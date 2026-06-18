//! Reusable stream-runtime helpers for local sub-agent execution.
//!
//! Applications that execute a sub-agent locally typically need the same
//! mechanics:
//! - drain a `StreamPart` stream to text/usage/error/latency output
//! - forward nested tool events to a parent sink with an agent-name prefix
//! - preserve the protocol framing while keeping app-side code small
//!
//! This module owns that generic ceremony so applications only wire domain
//! prompts, tools, and concrete interpreters.

use std::sync::Arc;

use agent_fw_algebra::EventSink;
use agent_fw_core::{LatencySummary, StreamPart, TokenUsage};
use futures::{Stream, StreamExt};

/// Collected result from draining a sub-agent event stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CollectedStreamResult {
    pub response_text: String,
    pub usage: TokenUsage,
    pub error_message: Option<String>,
    pub latency: Option<LatencySummary>,
}

impl CollectedStreamResult {
    /// Whether an error event was observed while draining the stream.
    pub fn had_error(&self) -> bool {
        self.error_message.is_some()
    }
}

/// Drain a sub-agent stream, collecting the final result and forwarding nested
/// tool activity to a parent sink.
///
/// The parent sink receives prefixed tool call/result/progress events, so a UI
/// can render `planner > draft_plan` instead of only `draft_plan`.
pub async fn collect_sub_agent_stream<St>(
    stream: &mut St,
    parent_sink: Option<&Arc<dyn EventSink>>,
    agent_name: &str,
) -> CollectedStreamResult
where
    St: Stream<Item = StreamPart> + Unpin,
{
    let mut result = CollectedStreamResult::default();

    while let Some(part) = stream.next().await {
        match part {
            StreamPart::Text { text } => result.response_text.push_str(&text),
            StreamPart::Finish { usage, .. } => {
                result.usage = TokenUsage::simple(usage.prompt_tokens, usage.completion_tokens);
            }
            StreamPart::Error { error } => {
                result.error_message = Some(error.message);
            }
            StreamPart::DataLatencySummary { data } => {
                result.latency = Some(data);
            }
            StreamPart::ToolInvocation(mut data) => {
                if let Some(sink) = parent_sink {
                    data.name = format!("{agent_name} > {}", data.name);
                    sink.emit(StreamPart::ToolInvocation(data));
                }
            }
            StreamPart::ToolProgress(data) => {
                if let Some(sink) = parent_sink {
                    sink.emit(StreamPart::tool_progress(
                        format!("{agent_name} > {}", data.tool_name),
                        data.tool_call_id,
                        data.label,
                        data.phase_index,
                        data.total_phases,
                        data.milestone,
                    ));
                }
            }
            // Approval events forward unscoped so the host's
            // `respond_to_approval(id, decision)` resolves the same
            // `ApprovalId` the sub-agent's gate is awaiting (pre-dispatch approval
            // review fix; mirror of `TeeEventSink`).
            StreamPart::ApprovalRequired { .. }
            | StreamPart::ApprovalDecision { .. }
            | StreamPart::PlanStatusChange { .. } => {
                if let Some(sink) = parent_sink {
                    sink.emit(part);
                }
            }
            StreamPart::StepStart
            | StreamPart::Reasoning { .. }
            | StreamPart::ToolAgent(_)
            | StreamPart::DataToolAgent { .. }
            | StreamPart::DataFileRegistered { .. }
            | StreamPart::DataCostSummary { .. }
            | StreamPart::DataFlowUI { .. }
            | StreamPart::Custom { .. } => {}
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::testing::RecordingEventSink;
    use agent_fw_core::latency::LatencySummary;
    use agent_fw_core::stream_part::ToolInvocationState;
    use std::sync::Arc;

    #[tokio::test]
    async fn collect_sub_agent_stream_collects_and_forwards() {
        let parent_sink = Arc::new(RecordingEventSink::new());
        let parent_sink_dyn: Arc<dyn EventSink> = parent_sink.clone();
        let mut stream = tokio_stream::iter(vec![
            StreamPart::StepStart,
            StreamPart::text("Hello "),
            StreamPart::tool_call("call-1", "draft_plan", serde_json::json!({"a": 1})),
            StreamPart::tool_progress(
                "draft_plan",
                Some("call-1".to_string()),
                "Resolving products",
                0,
                2,
                Some(serde_json::json!({"matched": 3})),
            ),
            StreamPart::tool_result(
                "call-1",
                "draft_plan",
                serde_json::json!({"a": 1}),
                serde_json::json!({"ok": true}),
            ),
            StreamPart::text("world"),
            StreamPart::latency_summary(LatencySummary {
                total_duration_ms: 17,
                ..Default::default()
            }),
            StreamPart::finish(
                agent_fw_core::stream_part::FinishReason::Stop,
                TokenUsage::simple(5, 3),
            ),
        ]);

        let result = collect_sub_agent_stream(&mut stream, Some(&parent_sink_dyn), "planner").await;

        assert_eq!(result.response_text, "Hello world");
        assert_eq!(result.usage, TokenUsage::simple(5, 3));
        assert_eq!(
            result.latency.as_ref().map(|l| l.total_duration_ms),
            Some(17)
        );
        assert!(!result.had_error());

        let events = parent_sink.events();
        assert_eq!(events.len(), 3);
        match &events[0] {
            StreamPart::ToolInvocation(data) => {
                assert_eq!(data.name, "planner > draft_plan");
                assert!(matches!(data.state, ToolInvocationState::Call));
            }
            other => panic!("expected ToolInvocation call, got {other:?}"),
        }
        match &events[1] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.tool_name, "planner > draft_plan");
            }
            other => panic!("expected ToolProgress, got {other:?}"),
        }
        match &events[2] {
            StreamPart::ToolInvocation(data) => {
                assert_eq!(data.name, "planner > draft_plan");
                assert!(matches!(data.state, ToolInvocationState::Result { .. }));
            }
            other => panic!("expected ToolInvocation result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn collect_sub_agent_stream_captures_error() {
        let mut stream =
            tokio_stream::iter(vec![StreamPart::text("partial"), StreamPart::error("boom")]);

        let result = collect_sub_agent_stream(&mut stream, None, "planner").await;

        assert_eq!(result.response_text, "partial");
        assert_eq!(result.error_message.as_deref(), Some("boom"));
        assert!(result.had_error());
    }

    /// pre-dispatch approval review fix: approval events from a sub-agent must reach
    /// the parent sink so the host can render and resolve them.
    /// Forwarded unscoped — the `ApprovalId` is the resolution key.
    #[tokio::test]
    async fn collect_sub_agent_stream_forwards_approval_events() {
        use agent_fw_core::approval::{
            ApprovalDecision, ApprovalKind, ApprovalRequest, PlanStatusChange,
        };
        use agent_fw_core::{ApprovalId, TenantId, ThreadId};

        let parent_sink = Arc::new(RecordingEventSink::new());
        let parent_sink_dyn: Arc<dyn EventSink> = parent_sink.clone();

        let req = ApprovalRequest {
            id: ApprovalId::new_unchecked("apr-xyz"),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: serde_json::json!({}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("acme"),
            thread_id: ThreadId::new_unchecked("th-1"),
            correlation_id: Some("tool_use_42".into()),
        };
        let mut stream = tokio_stream::iter(vec![
            StreamPart::approval_required(req.clone()),
            StreamPart::approval_decision(ApprovalDecision::approve(ApprovalId::new_unchecked(
                "apr-xyz",
            ))),
            StreamPart::PlanStatusChange {
                data: PlanStatusChange {
                    plan_id: "plan-1".into(),
                    from: "draft".into(),
                    to: "pending_approval".into(),
                },
            },
            StreamPart::finish(
                agent_fw_core::stream_part::FinishReason::Stop,
                TokenUsage::simple(0, 0),
            ),
        ]);

        let _result =
            collect_sub_agent_stream(&mut stream, Some(&parent_sink_dyn), "planner").await;

        let events = parent_sink.events();
        assert_eq!(
            events.len(),
            3,
            "all three approval variants forwarded; finish is not"
        );
        match &events[0] {
            StreamPart::ApprovalRequired { data } => {
                assert_eq!(data.id.as_str(), "apr-xyz");
                assert_eq!(data.target, "create_scenario");
                assert_eq!(data.correlation_id.as_deref(), Some("tool_use_42"));
            }
            other => panic!("expected ApprovalRequired, got {other:?}"),
        }
        match &events[1] {
            StreamPart::ApprovalDecision { data } => {
                assert_eq!(data.id.as_str(), "apr-xyz");
            }
            other => panic!("expected ApprovalDecision, got {other:?}"),
        }
        match &events[2] {
            StreamPart::PlanStatusChange { data } => {
                assert_eq!(data.plan_id, "plan-1");
                assert_eq!(data.to, "pending_approval");
            }
            other => panic!("expected PlanStatusChange, got {other:?}"),
        }
    }
}
