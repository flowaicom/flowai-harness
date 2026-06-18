//! Rig streaming hook integration built on top of [`HookBridge`].
//!
//! This keeps the generic event/timing state machine in the framework while
//! offering a first-class adapter for applications that use Rig as their model
//! runtime. The result is a minimal app-side shell: wire a sink once, pass the
//! hook to Rig, and keep domain-specific counters local.

use crate::{HookBridge, ToolOutcome};
use agent_fw_algebra::{event_sink::EventSink, CancellationToken};
use agent_fw_core::latency::{KVTimingEvent, LatencySummary, RetryReason};
use agent_fw_core::stream_part::FinishReason;
use agent_fw_core::usage::TokenUsage;
use agent_fw_tool::{CommandCardPayload, HookChannel};
use futures::{Stream, StreamExt};
use rig::agent::MultiTurnStreamItem;
use rig::agent::{HookAction, PromptHook, ToolCallHookAction};
use rig::completion::{CompletionModel, GetTokenUsage};
use rig::message::Message;
use rig::streaming::StreamedAssistantContent;
use std::future::Future;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

/// Sentinel reason used when a command card has become the visible response and
/// the agent loop should stop before another LLM round-trip.
pub const COMMAND_CARD_TERMINATE_REASON: &str = "command_card_awaiting_approval";

/// Framework-owned Rig adapter around [`HookBridge`].
pub struct RigHookBridge<S: EventSink> {
    bridge: HookBridge<S>,
    agent_name: Arc<str>,
}

/// Generic outcome of consuming a Rig multi-turn stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RigStreamOutcome {
    pub response_len: usize,
    pub had_error: bool,
    pub was_cancelled: bool,
}

/// Result of running a complete Rig request through the framework-owned
/// request lifecycle.
#[derive(Debug, Clone)]
pub struct RigRequestResult {
    pub outcome: RigStreamOutcome,
    pub summary: crate::MetricsSummary,
}

impl<S: EventSink> RigHookBridge<S> {
    /// Create a new Rig hook bridge for the given sink.
    pub fn new(sink: Arc<S>, agent_name: impl Into<String>) -> Self {
        Self {
            bridge: HookBridge::new(sink),
            agent_name: Arc::<str>::from(agent_name.into()),
        }
    }

    /// Access the semantic name for this hook instance.
    pub fn agent_name(&self) -> &str {
        &self.agent_name
    }

    /// Wire shared hook-channel state so tool-call IDs and buffered approval
    /// cards flow through the canonical framework path.
    pub fn with_hook_channel(mut self, hook: &HookChannel) -> Self {
        self.bridge = self.bridge.with_hook_channel(hook);
        self
    }

    /// Wire a shared tool-call-ID cell.
    pub fn with_tool_call_id_cell(mut self, cell: Arc<Mutex<Option<String>>>) -> Self {
        self.bridge = self.bridge.with_tool_call_id_cell(cell);
        self
    }

    /// Wire a shared pending-card cell.
    pub fn with_pending_card_cell(mut self, cell: Arc<Mutex<Option<CommandCardPayload>>>) -> Self {
        self.bridge = self.bridge.with_pending_card_cell(cell);
        self
    }

    /// Bridge `InstrumentedKVStore` timing events into the shared metrics.
    pub fn record_kv_timing(&self, event: KVTimingEvent) {
        self.bridge.on_kv_timing(event);
    }

    /// Provider-aggregated usage is the authoritative token source when Rig
    /// exposes it, so we overwrite the current view for that turn.
    pub fn set_aggregated_usage(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.bridge.set_aggregated_usage(
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        );
    }

    /// Capture fallback token usage only when the provider did not already
    /// report aggregate usage for the turn.
    pub fn add_token_metrics_if_uncaptured(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.bridge.add_token_metrics_if_uncaptured(
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        );
    }

    /// Record a categorized retry event.
    pub fn record_retry_with_reason(&self, reason: RetryReason, duration: std::time::Duration) {
        self.bridge.record_retry_with_reason(reason, duration);
    }

    /// Mark the request as timed out.
    pub fn record_timeout(&self) {
        self.bridge.record_timeout();
    }

    /// Attach a domain counter to the final latency summary.
    pub fn set_domain_counter(&self, key: impl Into<String>, value: u64) {
        self.bridge.set_domain_counter(key, value);
    }

    /// Read the current latency summary without emitting it.
    pub fn finalize_timing(&self) -> LatencySummary {
        self.bridge.latency_summary()
    }

    /// Read the accumulated token usage.
    pub fn total_usage(&self) -> TokenUsage {
        self.bridge.total_usage()
    }

    /// Consume a Rig multi-turn stream using the framework-owned event, token,
    /// and cancellation semantics.
    pub async fn consume_multi_turn_stream<R, St, E>(
        &self,
        stream: &mut St,
        cancel: &CancellationToken,
        cancel_message: &str,
    ) -> RigStreamOutcome
    where
        St: Stream<Item = Result<MultiTurnStreamItem<R>, E>> + Unpin,
        E: std::fmt::Display,
    {
        let mut outcome = RigStreamOutcome::default();

        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    self.bridge.on_error(cancel_message);
                    outcome.was_cancelled = true;
                    break;
                }

                item = stream.next() => {
                    match item {
                        Some(Ok(item)) => {
                            outcome.response_len += handle_stream_item(self, &item);
                        }
                        Some(Err(err)) => {
                            self.bridge.on_error(err.to_string());
                            outcome.had_error = true;
                            break;
                        }
                        None => break,
                    }
                }
            }
        }

        outcome
    }

    /// Run a complete Rig request using a caller-supplied stream factory.
    ///
    /// This owns the generic request ceremony:
    /// - early cancellation before any provider call
    /// - `StepStart` emission
    /// - multi-turn consumption with cancellation/error handling
    /// - terminal cost/latency/finish emission
    /// - sink closure
    ///
    /// The caller only supplies the concrete Rig stream construction.
    pub async fn run_stream_request<R, St, E, F, Fut>(
        &self,
        cancel: &CancellationToken,
        cancel_message: &str,
        agent_name: impl Into<String>,
        model: impl Into<String>,
        reason: FinishReason,
        make_stream: F,
    ) -> RigRequestResult
    where
        St: Stream<Item = Result<MultiTurnStreamItem<R>, E>> + Unpin,
        E: std::fmt::Display,
        F: FnOnce(Self) -> Fut,
        Fut: Future<Output = St>,
    {
        if cancel.is_cancelled() {
            self.bridge.on_step_start();
            self.bridge.on_error(cancel_message);
            let summary = self.finalize_request(agent_name, model, reason);
            self.close();
            return RigRequestResult {
                outcome: RigStreamOutcome {
                    was_cancelled: true,
                    ..RigStreamOutcome::default()
                },
                summary,
            };
        }

        self.bridge.on_step_start();

        let mut stream = make_stream(self.clone()).await;
        let outcome = self
            .consume_multi_turn_stream(&mut stream, cancel, cancel_message)
            .await;
        let summary = self.finalize_request(agent_name, model, reason);
        self.close();

        RigRequestResult { outcome, summary }
    }

    /// Finalize and emit cost, latency, and finish events with an explicit
    /// identity for the agent represented by this hook.
    pub fn finalize_request(
        &self,
        agent_name: impl Into<String>,
        model: impl Into<String>,
        reason: FinishReason,
    ) -> crate::MetricsSummary {
        self.bridge.finalize_with_finish(agent_name, model, reason)
    }
}

impl<S: EventSink> Deref for RigHookBridge<S> {
    type Target = HookBridge<S>;

    fn deref(&self) -> &Self::Target {
        &self.bridge
    }
}

impl<S: EventSink> Clone for RigHookBridge<S> {
    fn clone(&self) -> Self {
        Self {
            bridge: self.bridge.clone(),
            agent_name: Arc::clone(&self.agent_name),
        }
    }
}

fn handle_stream_item<S: EventSink, R>(
    hook: &RigHookBridge<S>,
    item: &MultiTurnStreamItem<R>,
) -> usize {
    match item {
        MultiTurnStreamItem::FinalResponse(res) => {
            let usage = res.usage();
            hook.set_aggregated_usage(
                usage.input_tokens + usage.cached_input_tokens + usage.cache_creation_input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.cache_creation_input_tokens,
            );
            0
        }
        MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ReasoningDelta {
            reasoning,
            ..
        }) => {
            if !reasoning.is_empty() {
                hook.bridge.on_reasoning_delta(reasoning);
            }
            0
        }
        MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text)) => {
            text.text.len()
        }
        _ => 0,
    }
}

impl<S, M> PromptHook<M> for RigHookBridge<S>
where
    S: EventSink + Send + Sync + 'static,
    M: CompletionModel,
{
    fn on_completion_call(
        &self,
        _prompt: &Message,
        _history: &[Message],
    ) -> impl Future<Output = HookAction> + Send {
        self.bridge.reset_turn_capture();

        let action = if self.bridge.is_suppressed() {
            HookAction::terminate(COMMAND_CARD_TERMINATE_REASON)
        } else {
            self.bridge.on_llm_start();
            self.bridge.on_step_start();
            HookAction::cont()
        };

        async move { action }
    }

    fn on_text_delta(
        &self,
        text_delta: &str,
        _aggregated_text: &str,
    ) -> impl Future<Output = HookAction> + Send {
        self.bridge.on_text_delta(text_delta);
        async { HookAction::cont() }
    }

    fn on_tool_call_delta(
        &self,
        _tool_call_id: &str,
        _internal_call_id: &str,
        _tool_name: Option<&str>,
        _tool_call_delta: &str,
    ) -> impl Future<Output = HookAction> + Send {
        async { HookAction::cont() }
    }

    fn on_stream_completion_response_finish(
        &self,
        _prompt: &Message,
        response: &<M as CompletionModel>::StreamingResponse,
    ) -> impl Future<Output = HookAction> + Send {
        self.bridge.on_llm_end();

        if let Some(usage) = response.token_usage() {
            self.bridge.set_aggregated_usage(
                usage.input_tokens + usage.cached_input_tokens + usage.cache_creation_input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.cache_creation_input_tokens,
            );
        }

        async { HookAction::cont() }
    }

    fn on_tool_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        _internal_call_id: &str,
        args: &str,
    ) -> impl Future<Output = ToolCallHookAction> + Send {
        self.bridge.flush_pending_llm_timing();

        let tool_call_id = tool_call_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let args_value =
            serde_json::from_str(args).unwrap_or_else(|_| serde_json::json!({ "raw": args }));
        self.bridge
            .on_tool_call(tool_name, tool_call_id, args_value);

        async { ToolCallHookAction::cont() }
    }

    fn on_tool_result(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        _internal_call_id: &str,
        _args: &str,
        result: &str,
    ) -> impl Future<Output = HookAction> + Send {
        let tool_call_id = tool_call_id
            .or_else(|| self.bridge.pending_tool_id_by_name(tool_name))
            .unwrap_or_default();

        let result_value: serde_json::Value =
            serde_json::from_str(result).unwrap_or_else(|_| serde_json::json!(result));
        self.bridge
            .on_tool_result(&tool_call_id, result_value, ToolOutcome::Success);

        async { HookAction::cont() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::StreamPart;
    use rig::completion::Usage;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TestSink {
        events: Mutex<Vec<StreamPart>>,
        open: AtomicBool,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                open: AtomicBool::new(true),
            }
        }
    }

    impl EventSink for TestSink {
        fn emit(&self, part: StreamPart) -> bool {
            if !self.is_open() {
                return false;
            }
            self.events.lock().unwrap().push(part);
            true
        }

        fn close(&self) {
            self.open.store(false, Ordering::SeqCst);
        }

        fn is_open(&self) -> bool {
            self.open.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn wrapper_delegates_metrics_and_domain_counters() {
        let sink = Arc::new(TestSink::new());
        let hook = RigHookBridge::new(sink, "planner");

        hook.add_token_metrics_if_uncaptured(100, 50, 10, 0);
        hook.record_retry_with_reason(RetryReason::RateLimit, std::time::Duration::from_millis(25));
        hook.record_timeout();
        hook.set_domain_counter("productSetSize", 42);

        let usage = hook.total_usage();
        let latency = hook.finalize_timing();

        assert_eq!(hook.agent_name(), "planner");
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(latency.retry_count, 1);
        assert!(latency.had_timeout);
        assert_eq!(latency.domain_counters.get("productSetSize"), Some(&42));
    }

    #[tokio::test]
    async fn consume_multi_turn_stream_handles_reasoning_and_final_usage() {
        let sink = Arc::new(TestSink::new());
        let hook = RigHookBridge::new(sink.clone(), "planner");
        let cancel = CancellationToken::new();

        let items: Vec<
            Result<MultiTurnStreamItem<serde_json::Value>, rig::completion::CompletionError>,
        > = vec![
            Ok::<_, rig::completion::CompletionError>(
                MultiTurnStreamItem::<serde_json::Value>::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta {
                        id: None,
                        reasoning: "thinking".to_string(),
                    },
                ),
            ),
            Ok::<_, rig::completion::CompletionError>(
                MultiTurnStreamItem::<serde_json::Value>::StreamAssistantItem(
                    StreamedAssistantContent::Text(rig::message::Text {
                        text: "hello".to_string(),
                    }),
                ),
            ),
            Ok::<_, rig::completion::CompletionError>(
                MultiTurnStreamItem::<serde_json::Value>::final_response(
                    "hello",
                    Usage {
                        input_tokens: 10,
                        output_tokens: 4,
                        total_tokens: 14,
                        cached_input_tokens: 3,
                        cache_creation_input_tokens: 0,
                    },
                ),
            ),
        ];
        let mut stream = tokio_stream::iter(items);

        let outcome = hook
            .consume_multi_turn_stream(&mut stream, &cancel, "cancelled")
            .await;
        let summary = hook.finalize_request("planner", "claude-test", FinishReason::Stop);

        assert_eq!(outcome.response_len, 5);
        assert!(!outcome.had_error);
        assert!(!outcome.was_cancelled);
        assert_eq!(summary.token_usage().prompt_tokens, 13);
        assert_eq!(summary.token_usage().completion_tokens, 4);

        let events = sink.events.lock().unwrap().clone();
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::Reasoning { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataLatencySummary { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataCostSummary { .. })));
    }

    #[tokio::test]
    async fn run_stream_request_short_circuits_on_early_cancel() {
        let sink = Arc::new(TestSink::new());
        let hook = RigHookBridge::new(sink.clone(), "planner");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let called = Arc::new(AtomicBool::new(false));
        let called_inner = Arc::clone(&called);

        let result = hook
            .run_stream_request(
                &cancel,
                "cancelled",
                "planner",
                "claude-test",
                FinishReason::Stop,
                move |_hook| {
                    called_inner.store(true, Ordering::SeqCst);
                    async {
                        tokio_stream::iter::<
                            Vec<
                                Result<
                                    MultiTurnStreamItem<serde_json::Value>,
                                    rig::completion::CompletionError,
                                >,
                            >,
                        >(vec![])
                    }
                },
            )
            .await;

        assert!(!called.load(Ordering::SeqCst));
        assert!(result.outcome.was_cancelled);
        assert_eq!(result.summary.token_usage(), TokenUsage::ZERO);

        let events = sink.events.lock().unwrap().clone();
        assert!(matches!(events.first(), Some(StreamPart::StepStart)));
        assert!(matches!(events.get(1), Some(StreamPart::Error { .. })));
        assert!(matches!(events.last(), Some(StreamPart::Finish { .. })));
    }

    #[tokio::test]
    async fn run_stream_request_executes_and_finalizes() {
        let sink = Arc::new(TestSink::new());
        let hook = RigHookBridge::new(sink.clone(), "planner");
        let cancel = CancellationToken::new();

        let result = hook
            .run_stream_request(
                &cancel,
                "cancelled",
                "planner",
                "claude-test",
                FinishReason::Stop,
                |_hook| async {
                    tokio_stream::iter(vec![
                        Ok::<_, rig::completion::CompletionError>(MultiTurnStreamItem::<
                            serde_json::Value,
                        >::StreamAssistantItem(
                            StreamedAssistantContent::Text(rig::message::Text {
                                text: "hello".to_string(),
                            }),
                        )),
                        Ok::<_, rig::completion::CompletionError>(MultiTurnStreamItem::<
                            serde_json::Value,
                        >::final_response(
                            "hello",
                            Usage {
                                input_tokens: 10,
                                output_tokens: 4,
                                total_tokens: 14,
                                cached_input_tokens: 3,
                                cache_creation_input_tokens: 0,
                            },
                        )),
                    ])
                },
            )
            .await;

        assert_eq!(result.outcome.response_len, 5);
        assert!(!result.outcome.had_error);
        assert!(!result.outcome.was_cancelled);
        assert_eq!(result.summary.token_usage(), TokenUsage::new(13, 4, 3, 0));

        let events = sink.events.lock().unwrap().clone();
        assert!(matches!(events.first(), Some(StreamPart::StepStart)));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataLatencySummary { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataCostSummary { .. })));
        assert!(matches!(events.last(), Some(StreamPart::Finish { .. })));
    }
}
