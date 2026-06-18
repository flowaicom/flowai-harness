//! Generic stream folding for eval/sample execution.
//!
//! This accumulator consumes `StreamPart` events and produces:
//! - typed `SampleOutput` for scoring/orchestration
//! - accumulated plain assistant text
//! - typed `MessagePart` values for thread/message persistence
//!
//! The behavior is framework-generic. Domain-specific scorers can project the
//! captured tool calls into richer semantics locally.

use crate::sample_executor::SampleOutput;
use crate::trace::{TraceActor, TracePayload, TraceStep};
use crate::types::TokenUsageSummary;
use agent_fw_core::{
    LatencySummary, MessagePart, MessagePartAccumulator, StreamPart, ToolAgentState,
    ToolInvocationState,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Captured tool call from agent execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedToolCall {
    /// Tool name (e.g. `draft_plan`).
    pub tool: String,
    /// Unique invocation ID, if provided by the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool arguments as JSON.
    pub args: serde_json::Value,
    /// Tool result, if observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

impl CapturedToolCall {
    pub fn new(tool: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            tool: tool.into(),
            tool_call_id: None,
            args,
            result: None,
        }
    }

    pub fn to_trace_step(&self, ordinal: u32) -> TraceStep {
        TraceStep {
            ordinal,
            actor: Some(TraceActor::Assistant),
            tool_name: self.tool.clone(),
            tool_call_id: self.tool_call_id.clone(),
            arguments: TracePayload::inline(self.args.clone()),
            result: self.result.clone().map(TracePayload::inline),
            started_at: None,
            completed_at: None,
            error: None,
            correlation_id: None,
        }
    }
}

/// Token usage source, ordered by quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TokenSource {
    None = 0,
    Finish = 1,
    Cost = 2,
    Latency = 3,
}

fn scoped_tool_call_key(agent_stack: &[String], tool_call_id: &str) -> String {
    if agent_stack.is_empty() {
        tool_call_id.to_string()
    } else {
        format!("{}::{tool_call_id}", agent_stack.join("/"))
    }
}

/// Finalized stream-capture result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamCaptureResult {
    pub sample_output: SampleOutput,
    pub response_text: String,
    pub message_parts: Vec<MessagePart>,
}

/// Pure fold accumulator for eval-relevant data from a `StreamPart` stream.
#[derive(Debug)]
pub struct StreamCapture {
    trajectory: Vec<String>,
    captured_tool_calls: Vec<CapturedToolCall>,
    seen_tool_call_ids: HashSet<String>,
    captured_tool_call_indices: HashMap<String, usize>,
    agent_stack: Vec<String>,
    token_usage: TokenUsageSummary,
    token_source: TokenSource,
    latency: Option<LatencySummary>,
    errors: Vec<String>,
    messages: MessagePartAccumulator,
}

impl StreamCapture {
    pub fn new() -> Self {
        Self {
            trajectory: Vec::new(),
            captured_tool_calls: Vec::new(),
            seen_tool_call_ids: HashSet::new(),
            captured_tool_call_indices: HashMap::new(),
            agent_stack: Vec::new(),
            token_usage: TokenUsageSummary::ZERO,
            token_source: TokenSource::None,
            latency: None,
            errors: Vec::new(),
            messages: MessagePartAccumulator::new(),
        }
    }

    pub fn step(&mut self, part: &StreamPart) {
        self.messages.push(part);

        match part {
            StreamPart::ToolInvocation(data) => {
                let scoped_key = scoped_tool_call_key(&self.agent_stack, &data.id);
                if matches!(data.state, ToolInvocationState::Call) {
                    if self.seen_tool_call_ids.insert(scoped_key.clone()) {
                        self.trajectory.push(data.name.clone());
                        self.captured_tool_calls.push(CapturedToolCall {
                            tool: data.name.clone(),
                            tool_call_id: Some(data.id.clone()),
                            args: data.args.clone(),
                            result: None,
                        });
                        self.captured_tool_call_indices
                            .insert(scoped_key.clone(), self.captured_tool_calls.len() - 1);
                    }
                }

                if let ToolInvocationState::Result { result } = &data.state {
                    if let Some(call) = self
                        .captured_tool_call_indices
                        .get(&scoped_key)
                        .and_then(|index| self.captured_tool_calls.get_mut(*index))
                        .filter(|call| call.result.is_none())
                    {
                        call.result = Some(result.clone());
                    }
                }
            }
            StreamPart::ToolAgent(data) if matches!(data.state, ToolAgentState::Call) => {
                self.agent_stack.push(data.invocation_id.clone());
            }
            StreamPart::ToolAgent(data) if matches!(data.state, ToolAgentState::Result) => {
                if let Some(position) = self
                    .agent_stack
                    .iter()
                    .rposition(|invocation_id| invocation_id == &data.invocation_id)
                {
                    self.agent_stack.truncate(position);
                }
            }
            StreamPart::ToolAgent(_) => {}
            StreamPart::DataLatencySummary { data } => {
                self.latency = Some(data.clone());
                if self.token_source < TokenSource::Latency {
                    self.token_usage = TokenUsageSummary::new(
                        data.token_metrics.input_tokens,
                        data.token_metrics.output_tokens,
                        data.token_metrics.cached_tokens,
                        data.token_metrics.cache_creation_tokens,
                    );
                    self.token_source = TokenSource::Latency;
                }
            }
            StreamPart::DataCostSummary { data } => {
                if self.token_source < TokenSource::Cost {
                    let usage = data.total_usage();
                    self.token_usage = usage.into();
                    self.token_source = TokenSource::Cost;
                }
            }
            StreamPart::Finish { usage, .. } => {
                if self.token_source < TokenSource::Finish {
                    self.token_usage = usage.clone().into();
                    self.token_source = TokenSource::Finish;
                }
            }
            StreamPart::Error { error } => {
                self.errors.push(error.message.clone());
            }
            StreamPart::StepStart
            | StreamPart::Text { .. }
            | StreamPart::Reasoning { .. }
            | StreamPart::ToolProgress(_)
            | StreamPart::DataToolAgent { .. }
            | StreamPart::DataFlowUI { .. }
            | StreamPart::DataFileRegistered { .. }
            | StreamPart::ApprovalRequired { .. }
            | StreamPart::ApprovalDecision { .. }
            | StreamPart::PlanStatusChange { .. }
            | StreamPart::Custom { .. } => {}
        }
    }

    pub fn finalize(self, duration_ms: u64, thread_id: Option<String>) -> StreamCaptureResult {
        let error = if self.errors.is_empty() {
            None
        } else {
            Some(self.errors.join("; "))
        };
        let (message_parts, response_text) = self.messages.finish();

        StreamCaptureResult {
            sample_output: SampleOutput {
                actual_trajectory: self.trajectory,
                captured_tool_calls: self.captured_tool_calls,
                duration_ms,
                token_usage: self.token_usage,
                error,
                thread_id,
                extra: None,
                latency: self.latency,
            },
            response_text,
            message_parts,
        }
    }
}

impl Default for StreamCapture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::{
        AgentUsage, CostSummary, FinishReason, LatencySummary, PhaseBreakdown, TokenMetrics,
        TokenUsage, ToolTiming,
    };
    use hegel::generators;

    #[test]
    fn empty_stream_produces_zero_output() {
        let capture = StreamCapture::new();
        let output = capture.finalize(0, None).sample_output;

        assert!(output.actual_trajectory.is_empty());
        assert!(output.captured_tool_calls.is_empty());
        assert_eq!(output.duration_ms, 0);
        assert_eq!(output.token_usage, TokenUsageSummary::ZERO);
        assert!(output.latency.is_none());
        assert!(output.error.is_none());
    }

    #[test]
    fn tool_calls_captured_in_order() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::tool_call(
            "c1",
            "search_entities",
            serde_json::json!({"q": "beer"}),
        ));
        capture.step(&StreamPart::tool_call(
            "c2",
            "draft_plan",
            serde_json::json!({"name": "test"}),
        ));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(
            output.actual_trajectory,
            vec!["search_entities", "draft_plan"]
        );
        assert_eq!(output.captured_tool_calls.len(), 2);
        assert_eq!(output.captured_tool_calls[0].tool, "search_entities");
        assert_eq!(output.captured_tool_calls[1].tool, "draft_plan");
    }

    #[test]
    fn tool_result_attached_to_matching_call() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::tool_call(
            "c1",
            "draft_plan",
            serde_json::json!({"name": "test"}),
        ));
        capture.step(&StreamPart::tool_result(
            "c1",
            "draft_plan",
            serde_json::json!({"name": "test"}),
            serde_json::json!({"planId": "p-123"}),
        ));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(output.captured_tool_calls.len(), 1);
        assert_eq!(
            output.captured_tool_calls[0].result.as_ref().unwrap()["planId"],
            "p-123"
        );
    }

    #[test]
    fn duplicate_live_tool_call_event_is_captured_once_by_id() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::tool_call(
            "provider-call-1",
            "searchProducts",
            serde_json::json!({"q": "beer"}),
        ));
        capture.step(&StreamPart::tool_call(
            "provider-call-1",
            "searchProducts",
            serde_json::json!({"q": "beer"}),
        ));
        capture.step(&StreamPart::tool_result(
            "provider-call-1",
            "searchProducts",
            serde_json::json!({"q": "beer"}),
            serde_json::json!({"found": 3}),
        ));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(output.actual_trajectory, vec!["searchProducts"]);
        assert_eq!(output.captured_tool_calls.len(), 1);
        assert_eq!(
            output.captured_tool_calls[0]
                .result
                .as_ref()
                .expect("result")["found"],
            3
        );
    }

    #[test]
    fn duplicate_tool_ids_are_scoped_by_agent_invocation() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::sub_agent_call("coordinator", "agent-root"));
        capture.step(&StreamPart::tool_call(
            "scripted-tool-1",
            "call_agent",
            serde_json::json!({"agent": "planner"}),
        ));
        capture.step(&StreamPart::sub_agent_call("planner", "agent-planner"));
        capture.step(&StreamPart::tool_call(
            "scripted-tool-1",
            "storePlan",
            serde_json::json!({"planId": "p1"}),
        ));
        capture.step(&StreamPart::tool_result(
            "scripted-tool-1",
            "storePlan",
            serde_json::json!({"planId": "p1"}),
            serde_json::json!({"id": "p1"}),
        ));
        capture.step(&StreamPart::sub_agent_result("planner", "agent-planner"));
        capture.step(&StreamPart::tool_result(
            "scripted-tool-1",
            "call_agent",
            serde_json::json!({"agent": "planner"}),
            serde_json::json!({"response": "stored"}),
        ));
        capture.step(&StreamPart::sub_agent_result("coordinator", "agent-root"));

        let output = capture.finalize(100, None).sample_output;

        assert_eq!(output.actual_trajectory, vec!["call_agent", "storePlan"]);
        assert_eq!(output.captured_tool_calls.len(), 2);
        assert_eq!(output.captured_tool_calls[0].tool, "call_agent");
        assert_eq!(
            output.captured_tool_calls[0]
                .result
                .as_ref()
                .expect("call_agent result")["response"],
            "stored"
        );
        assert_eq!(output.captured_tool_calls[1].tool, "storePlan");
        assert_eq!(
            output.captured_tool_calls[1]
                .result
                .as_ref()
                .expect("storePlan result")["id"],
            "p1"
        );
    }

    #[test]
    fn repeated_same_tool_calls_with_distinct_ids_are_preserved() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::tool_call(
            "provider-call-1",
            "searchProducts",
            serde_json::json!({"q": "beer"}),
        ));
        capture.step(&StreamPart::tool_call(
            "provider-call-2",
            "searchProducts",
            serde_json::json!({"q": "wine"}),
        ));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(
            output.actual_trajectory,
            vec!["searchProducts", "searchProducts"]
        );
        assert_eq!(output.captured_tool_calls.len(), 2);
        assert_eq!(
            output.captured_tool_calls[0].tool_call_id.as_deref(),
            Some("provider-call-1")
        );
        assert_eq!(
            output.captured_tool_calls[1].tool_call_id.as_deref(),
            Some("provider-call-2")
        );
    }

    #[test]
    fn out_of_order_results_match_by_id() {
        let mut capture = StreamCapture::new();

        capture.step(&StreamPart::tool_call(
            "c1",
            "search_entities",
            serde_json::json!({"q": "beer"}),
        ));
        capture.step(&StreamPart::tool_call(
            "c2",
            "draft_plan",
            serde_json::json!({"name": "test"}),
        ));
        capture.step(&StreamPart::tool_result(
            "c2",
            "draft_plan",
            serde_json::json!({"name": "test"}),
            serde_json::json!({"planId": "p-789"}),
        ));
        capture.step(&StreamPart::tool_result(
            "c1",
            "search_entities",
            serde_json::json!({"q": "beer"}),
            serde_json::json!({"found": 3}),
        ));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(
            output.captured_tool_calls[0].result.as_ref().unwrap()["found"],
            3
        );
        assert_eq!(
            output.captured_tool_calls[1].result.as_ref().unwrap()["planId"],
            "p-789"
        );
    }

    #[test]
    fn latency_event_captured() {
        let mut capture = StreamCapture::new();

        let summary = LatencySummary {
            total_duration_ms: 1500,
            phases: PhaseBreakdown::new(800, 400, 2),
            tool_timings: vec![ToolTiming::completed("query_data", "call-1", 250)],
            token_metrics: TokenMetrics {
                input_tokens: 1000,
                output_tokens: 200,
                cached_tokens: 300,
                cache_creation_tokens: 0,
            },
            ..Default::default()
        };
        capture.step(&StreamPart::latency_summary(summary));

        let output = capture.finalize(1500, None).sample_output;
        assert!(output.latency.is_some());
        assert_eq!(output.token_usage.input_tokens(), 1000);
        assert_eq!(output.token_usage.output_tokens(), 200);
        assert_eq!(output.token_usage.cached_tokens(), 300);
    }

    #[test]
    fn message_parts_and_plain_text_are_captured() {
        let mut capture = StreamCapture::new();
        capture.step(&StreamPart::text("Hello "));
        capture.step(&StreamPart::tool_call(
            "c1",
            "search_entities",
            serde_json::json!({}),
        ));
        capture.step(&StreamPart::text("world"));

        let output = capture.finalize(0, None);
        assert_eq!(output.response_text, "Hello world");
        assert!(output.message_parts.len() >= 3);
    }

    fn draw_tool_name(tc: &hegel::TestCase) -> String {
        tc.draw(generators::sampled_from(vec![
            "search_entities".to_string(),
            "draft_plan".to_string(),
            "executeQuery".to_string(),
            "query_data".to_string(),
            "approve_plan".to_string(),
        ]))
    }

    fn draw_stream_part(tc: &hegel::TestCase) -> StreamPart {
        let variant = tc.draw(generators::integers::<u32>().min_value(0).max_value(6));
        match variant {
            0 => {
                let name = draw_tool_name(tc);
                let id = tc.draw(generators::text().min_size(1).max_size(10));
                StreamPart::tool_call(id, &name, serde_json::json!({}))
            }
            1 => {
                let name = draw_tool_name(tc);
                let id = tc.draw(generators::text().min_size(1).max_size(10));
                StreamPart::tool_result(id, &name, serde_json::json!({}), serde_json::json!({}))
            }
            2 => StreamPart::text("hello"),
            3 => StreamPart::error("something failed"),
            4 => StreamPart::latency_summary(LatencySummary::default()),
            5 => StreamPart::StepStart,
            _ => StreamPart::tool_progress("draft_plan", None, "Resolving", 1, 4, None),
        }
    }

    fn draw_stream(tc: &hegel::TestCase) -> Vec<StreamPart> {
        let len = tc.draw(generators::integers::<usize>().min_value(0).max_value(29));
        (0..len).map(|_| draw_stream_part(tc)).collect()
    }

    fn fold(parts: &[StreamPart]) -> SampleOutput {
        let mut capture = StreamCapture::new();
        for part in parts {
            capture.step(part);
        }
        capture.finalize(0, None).sample_output
    }

    #[hegel::test]
    fn trajectory_preserves_call_order(tc: hegel::TestCase) {
        let parts = draw_stream(&tc);
        let output = fold(&parts);
        let mut seen = std::collections::HashSet::new();
        let expected: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInvocation(data)
                    if matches!(data.state, ToolInvocationState::Call)
                        && seen.insert(data.id.clone()) =>
                {
                    Some(data.name.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(output.actual_trajectory, expected);
    }

    #[hegel::test]
    fn latency_fidelity(tc: hegel::TestCase) {
        let parts = draw_stream(&tc);
        let has_latency = parts
            .iter()
            .any(|p| matches!(p, StreamPart::DataLatencySummary { .. }));
        let output = fold(&parts);
        if has_latency {
            assert!(output.latency.is_some());
        }
    }

    #[hegel::test]
    fn error_capture(tc: hegel::TestCase) {
        let parts = draw_stream(&tc);
        let has_error = parts.iter().any(|p| matches!(p, StreamPart::Error { .. }));
        let output = fold(&parts);
        if has_error {
            assert!(output.error.is_some());
        }
    }

    #[hegel::test]
    fn tool_calls_match_trajectory(tc: hegel::TestCase) {
        let parts = draw_stream(&tc);
        let output = fold(&parts);
        assert_eq!(
            output.actual_trajectory.len(),
            output.captured_tool_calls.len()
        );
    }

    #[test]
    fn cost_summary_extracts_tokens() {
        let mut capture = StreamCapture::new();

        let summary = CostSummary::new(vec![AgentUsage {
            agent_name: "coordinator".to_string(),
            model: "claude-opus-4-6".to_string(),
            usage: TokenUsage::simple(500, 200),
        }]);
        capture.step(&StreamPart::cost_summary(summary));

        let output = capture.finalize(100, None).sample_output;
        assert_eq!(output.token_usage.input_tokens(), 500);
        assert_eq!(output.token_usage.output_tokens(), 200);
        assert_eq!(output.token_usage.cached_tokens(), 0);
    }

    #[test]
    fn finish_event_extracts_tokens() {
        let mut capture = StreamCapture::new();
        capture.step(&StreamPart::finish(
            FinishReason::Stop,
            TokenUsage::simple(300, 100),
        ));

        let output = capture.finalize(80, None).sample_output;
        assert_eq!(output.token_usage.input_tokens(), 300);
        assert_eq!(output.token_usage.output_tokens(), 100);
        assert_eq!(output.token_usage.cached_tokens(), 0);
    }
}
