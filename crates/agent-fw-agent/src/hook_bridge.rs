//! Bridge between agent hooks and EventSink with metrics accumulation.
//!
//! This module provides `HookBridge`, which implements streaming hooks and
//! accumulates metrics for emission at stream end.
//!
//! # Design Principles
//!
//! 1. **Single Source of Truth**: All metrics flow through this bridge
//! 2. **Push-Based**: Hooks are synchronous callbacks (no async in hot path)
//! 3. **Non-Blocking**: Never blocks on emission — graceful degradation on closed sink
//!
//! # Laws
//!
//! - **L1 (Order)**: Tool calls are emitted before their results
//! - **L2 (Totality)**: Operations never panic — graceful degradation on closed sink
//! - **L3 (Idempotency)**: TTFT is recorded once (first call wins)
//! - **L4 (Monoid Accumulation)**: Metrics combine via monoid operations

use agent_fw_algebra::event_sink::EventSink;
use agent_fw_core::latency::{
    KVMetrics, KVTimingEvent, LatencySummary, PhaseBreakdown, RetryEvent, RetryReason,
    TokenMetrics, ToolStatus, ToolTiming,
};
use agent_fw_core::stream_part::{AgentUsage, CostSummary, FinishReason};
use agent_fw_core::{StreamPart, TokenUsage};
use agent_fw_tool::{CommandCardPayload, HookChannel};
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

/// Pending tool call being tracked for timing.
#[derive(Debug, Clone)]
struct PendingToolCall {
    name: String,
    args: serde_json::Value,
    start_time: Instant,
}

/// Accumulated metrics for the current stream.
#[derive(Debug, Clone, Default)]
struct StreamMetrics {
    tokens: TokenMetrics,
    kv_metrics: KVMetrics,
    phases: PhaseBreakdown,
    tool_timings: Vec<ToolTiming>,
    retry_count: u32,
    retry_events: Vec<RetryEvent>,
    had_timeout: bool,
    domain_counters: BTreeMap<String, u64>,
    ttft_recorded: bool,
    first_text_recorded: bool,
    request_start: Option<Instant>,
    llm_start: Option<Instant>,
    ttft: Option<Duration>,
    first_text: Option<Duration>,
    turn_tokens_captured: bool,
}

impl StreamMetrics {
    fn new() -> Self {
        Self {
            request_start: Some(Instant::now()),
            ..Default::default()
        }
    }

    /// Record TTFT (time to first token). First call wins.
    fn record_ttft(&mut self) {
        if !self.ttft_recorded {
            if let Some(start) = self.request_start {
                self.ttft = Some(start.elapsed());
            }
            self.ttft_recorded = true;
        }
    }

    /// Record first text delta. First call wins.
    fn record_text_delta(&mut self) {
        self.record_ttft(); // TTFT is first token (any kind)
        if !self.first_text_recorded {
            if let Some(start) = self.request_start {
                self.first_text = Some(start.elapsed());
            }
            self.first_text_recorded = true;
        }
    }

    /// Add tool timing record.
    fn add_tool_timing(&mut self, timing: ToolTiming) {
        self.tool_timings.push(timing);
    }

    /// Accumulate token usage from an LLM call.
    fn add_tokens(&mut self, usage: &TokenMetrics) {
        self.tokens = self.tokens.combine(usage);
        self.phases.llm_calls += 1;
    }

    /// Add token usage only when the current turn has not already provided an
    /// aggregated usage record from the provider.
    fn add_tokens_if_uncaptured(&mut self, usage: &TokenMetrics) {
        if !self.turn_tokens_captured {
            self.turn_tokens_captured = true;
            self.tokens = self.tokens.combine(usage);
        }
    }

    /// Overwrite token metrics with a provider-aggregated view and mark the
    /// current turn as captured.
    fn set_aggregated_tokens(&mut self, usage: TokenMetrics) {
        self.turn_tokens_captured = true;
        self.tokens = usage;
    }

    /// Record LLM call start.
    fn record_llm_start(&mut self) {
        self.llm_start = Some(Instant::now());
    }

    /// Record LLM call end.
    fn record_llm_end(&mut self) {
        if let Some(start) = self.llm_start.take() {
            let duration_ms = start.elapsed().as_millis() as u64;
            self.phases.llm_time_ms = self.phases.llm_time_ms.saturating_add(duration_ms);
        }
    }

    /// Record a retry event.
    fn record_retry(&mut self, reason: RetryReason, duration_ms: u64) {
        self.retry_count = self.retry_count.saturating_add(1);
        self.retry_events.push(RetryEvent {
            reason,
            attempt: self.retry_count,
            duration_ms,
        });
    }

    /// Mark the request as having timed out.
    fn record_timeout(&mut self) {
        self.had_timeout = true;
    }

    /// Set or replace a named domain counter.
    fn set_domain_counter(&mut self, key: impl Into<String>, value: u64) {
        self.domain_counters.insert(key.into(), value);
    }

    /// Build final latency summary.
    fn to_latency_summary(&self) -> LatencySummary {
        let total_duration_ms = self
            .request_start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        LatencySummary {
            total_duration_ms,
            phases: self.phases.clone(),
            tool_timings: self.tool_timings.clone(),
            kv_metrics: self.kv_metrics.clone(),
            token_metrics: self.tokens.clone(),
            ttft_ms: self.ttft.map(|d| d.as_millis() as u64),
            first_text_ms: self.first_text.map(|d| d.as_millis() as u64),
            retry_count: self.retry_count,
            retry_events: self.retry_events.clone(),
            had_timeout: self.had_timeout,
            domain_counters: self.domain_counters.clone(),
            ..LatencySummary::zero()
        }
    }
}

/// Outcome of a tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Success,
    Error,
}

impl ToolOutcome {
    /// Check if the tool execution was successful.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Bridge between agent hooks and EventSink with metrics accumulation.
///
/// # Laws
///
/// - **L1 (Order)**: Tool calls are emitted before their results
/// - **L2 (Totality)**: Operations never panic — graceful degradation on closed sink
/// - **L3 (Idempotency)**: TTFT is recorded once (first call wins)
/// - **L4 (Monoid Accumulation)**: Metrics combine via monoid operations
///
/// # Example
///
/// ```ignore
/// let sink = Arc::new(ChannelEventSink::new());
/// let bridge = HookBridge::new(sink.clone());
///
/// // During LLM streaming:
/// bridge.on_text_delta("Hello");
/// bridge.on_tool_call("draft_plan", "call-123", args);
/// bridge.on_tool_result("call-123", result, ToolOutcome::Success);
///
/// // At end:
/// let summary = bridge.finalize();
/// ```
pub struct HookBridge<S: EventSink> {
    sink: Arc<S>,
    metrics: Arc<Mutex<StreamMetrics>>,
    pending_tools: Arc<Mutex<HashMap<String, PendingToolCall>>>,
    finalized: Arc<AtomicBool>,
    suppress_text: Arc<AtomicBool>,
    tool_call_id_cell: Option<Arc<Mutex<Option<String>>>>,
    pending_card_cell: Option<Arc<Mutex<Option<CommandCardPayload>>>>,
}

impl<S: EventSink> HookBridge<S> {
    /// Create a new bridge wrapping an EventSink.
    pub fn new(sink: Arc<S>) -> Self {
        Self {
            sink,
            metrics: Arc::new(Mutex::new(StreamMetrics::new())),
            pending_tools: Arc::new(Mutex::new(HashMap::new())),
            finalized: Arc::new(AtomicBool::new(false)),
            suppress_text: Arc::new(AtomicBool::new(false)),
            tool_call_id_cell: None,
            pending_card_cell: None,
        }
    }

    /// Share hook channel state with tools so tool-call IDs and buffered cards
    /// can flow through the same framework-owned path.
    pub fn with_hook_channel(mut self, hook: &HookChannel) -> Self {
        self.tool_call_id_cell = Some(hook.tool_call_id_cell());
        self.pending_card_cell = Some(hook.pending_card_cell());
        self
    }

    /// Wire a shared tool-call-ID cell.
    pub fn with_tool_call_id_cell(mut self, cell: Arc<Mutex<Option<String>>>) -> Self {
        self.tool_call_id_cell = Some(cell);
        self
    }

    /// Wire a shared pending-card cell.
    pub fn with_pending_card_cell(mut self, cell: Arc<Mutex<Option<CommandCardPayload>>>) -> Self {
        self.pending_card_cell = Some(cell);
        self
    }

    /// Total token usage represented in AI-SDK prompt/completion terms.
    pub fn total_usage(&self) -> TokenUsage {
        let metrics = self.metrics.lock().unwrap().tokens.clone();
        TokenUsage::new(
            metrics.input_tokens,
            metrics.output_tokens,
            metrics.cached_tokens,
            metrics.cache_creation_tokens,
        )
    }

    /// Detailed token metrics accumulated for this request.
    pub fn token_metrics(&self) -> TokenMetrics {
        self.metrics.lock().unwrap().tokens.clone()
    }

    /// Build an `AgentUsage` record for this hook's accumulated usage.
    pub fn agent_usage(
        &self,
        agent_name: impl Into<String>,
        model: impl Into<String>,
    ) -> AgentUsage {
        AgentUsage {
            agent_name: agent_name.into(),
            model: model.into(),
            usage: self.total_usage(),
        }
    }

    /// Build a single-agent cost summary from the accumulated usage.
    pub fn cost_summary(
        &self,
        agent_name: impl Into<String>,
        model: impl Into<String>,
    ) -> CostSummary {
        CostSummary::new(vec![self.agent_usage(agent_name, model)])
    }

    /// Overwrite token metrics with a provider-aggregated view.
    pub fn set_aggregated_usage(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.metrics
            .lock()
            .unwrap()
            .set_aggregated_tokens(TokenMetrics {
                input_tokens,
                output_tokens,
                cached_tokens,
                cache_creation_tokens,
            });
    }

    /// Add token metrics for the current request.
    pub fn add_token_metrics(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.metrics.lock().unwrap().add_tokens(&TokenMetrics {
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        });
    }

    /// Add token metrics only if the current turn has not already been captured
    /// through aggregated usage from the provider.
    pub fn add_token_metrics_if_uncaptured(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.metrics
            .lock()
            .unwrap()
            .add_tokens_if_uncaptured(&TokenMetrics {
                input_tokens,
                output_tokens,
                cached_tokens,
                cache_creation_tokens,
            });
    }

    /// Reset the per-turn provider-capture flag before a new model call.
    pub fn reset_turn_capture(&self) {
        self.metrics.lock().unwrap().turn_tokens_captured = false;
    }

    /// Read the current latency summary without finalizing the bridge.
    pub fn latency_summary(&self) -> LatencySummary {
        self.metrics.lock().unwrap().to_latency_summary()
    }

    /// Record a retry using the generic unknown reason bucket.
    pub fn record_retry(&self) {
        self.metrics
            .lock()
            .unwrap()
            .record_retry(RetryReason::Unknown, 0);
    }

    /// Record a categorized retry event.
    pub fn record_retry_with_reason(&self, reason: RetryReason, duration: Duration) {
        self.metrics
            .lock()
            .unwrap()
            .record_retry(reason, duration.as_millis() as u64);
    }

    /// Mark the request as having timed out.
    pub fn record_timeout(&self) {
        self.metrics.lock().unwrap().record_timeout();
    }

    /// Set a named domain counter for the final latency summary.
    pub fn set_domain_counter(&self, key: impl Into<String>, value: u64) {
        self.metrics.lock().unwrap().set_domain_counter(key, value);
    }

    /// Read a named domain counter if it has been set.
    pub fn domain_counter(&self, key: &str) -> Option<u64> {
        self.metrics
            .lock()
            .unwrap()
            .domain_counters
            .get(key)
            .copied()
    }

    /// Reset all accumulated metrics and internal state for reuse.
    pub fn reset(&self) {
        *self.metrics.lock().unwrap() = StreamMetrics::new();
        self.pending_tools.lock().unwrap().clear();
        self.finalized.store(false, Ordering::SeqCst);
        self.suppress_text.store(false, Ordering::Release);
        if let Some(ref cell) = self.tool_call_id_cell {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        if let Some(ref cell) = self.pending_card_cell {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Whether text output is currently suppressed after a command card.
    pub fn is_suppressed(&self) -> bool {
        self.suppress_text.load(Ordering::Acquire)
    }

    /// Called when LLM emits a tool call.
    ///
    /// Emits `ToolInvocation::Call` and tracks pending tool for timing.
    ///
    /// # Law L1
    /// Tool calls are always emitted before their results.
    pub fn on_tool_call(&self, name: &str, id: String, args: serde_json::Value) {
        let start = Instant::now();

        // Track pending tool
        self.pending_tools.lock().unwrap().insert(
            id.clone(),
            PendingToolCall {
                name: name.to_string(),
                args: args.clone(),
                start_time: start,
            },
        );

        if let Some(ref cell) = self.tool_call_id_cell {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
        }

        // Emit event
        self.emit(StreamPart::tool_call(id, name, args));
    }

    /// Resolve the most recent pending tool-call ID for a given tool name.
    pub fn pending_tool_id_by_name(&self, tool_name: &str) -> Option<String> {
        self.pending_tools
            .lock()
            .unwrap()
            .iter()
            .find(|(_, pending)| pending.name == tool_name)
            .map(|(id, _)| id.clone())
    }

    /// Called when tool execution completes.
    ///
    /// Emits `ToolInvocation::Result` and updates timing metrics.
    ///
    /// # Law L1
    /// Tool results are emitted after their corresponding calls.
    pub fn on_tool_result(&self, id: &str, result: serde_json::Value, outcome: ToolOutcome) {
        // Extract pending tool for timing
        let pending = self.pending_tools.lock().unwrap().remove(id);

        let (tool_name, args, timing) = if let Some(p) = pending {
            let duration = p.start_time.elapsed();
            let status = if outcome.is_success() {
                ToolStatus::Completed
            } else {
                ToolStatus::Error
            };

            let timing = ToolTiming {
                tool_name: p.name.clone(),
                tool_call_id: id.to_string(),
                duration_ms: duration.as_millis() as u64,
                status,
                payload_size: None,
            };

            (p.name, p.args, Some(timing))
        } else {
            // Orphan result — still emit but without timing
            tracing::warn!(tool_id = %id, "Tool result without matching call");
            ("unknown".to_string(), serde_json::Value::Null, None)
        };

        // Update metrics
        if let Some(t) = timing {
            self.metrics.lock().unwrap().add_tool_timing(t);
        }

        if let Some(ref cell) = self.tool_call_id_cell {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }

        let mut result = result;
        let ui = extract_precomputed_ui(&mut result);

        // Emit event
        self.emit(StreamPart::tool_result(id, tool_name, args, result));
        self.emit_card_events(&ui);
    }

    /// Called when LLM emits text.
    ///
    /// Tracks TTFT (time to first token) on first call.
    pub fn on_text_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.metrics.lock().unwrap().record_text_delta();
        if !self.suppress_text.load(Ordering::Acquire) {
            self.emit(StreamPart::text(text));
        }
    }

    /// Called when LLM emits reasoning.
    ///
    /// Tracks TTFT on first call.
    pub fn on_reasoning_delta(&self, text: &str) {
        self.metrics.lock().unwrap().record_ttft();
        self.emit(StreamPart::reasoning(text));
    }

    /// Called when LLM call starts.
    ///
    /// Records start time for LLM phase timing.
    pub fn on_llm_start(&self) {
        self.metrics.lock().unwrap().record_llm_start();
    }

    /// Called when an LLM call ends without directly supplying token metrics.
    pub fn on_llm_end(&self) {
        self.metrics.lock().unwrap().record_llm_end();
    }

    /// Called when LLM call completes.
    ///
    /// Accumulates token usage.
    pub fn on_llm_complete(&self, usage: TokenMetrics) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.record_llm_end();
        metrics.add_tokens(&usage);
    }

    /// Record a completed LLM call and its token usage in one step.
    pub fn record_llm_call_with_metrics(
        &self,
        duration: Duration,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.phases.llm_time_ms = metrics
            .phases
            .llm_time_ms
            .saturating_add(duration.as_millis() as u64);
        metrics.add_tokens(&TokenMetrics {
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        });
    }

    /// Record a direct KV put operation.
    pub fn record_kv_put(&self, bytes: u64, duration: Duration) {
        self.metrics
            .lock()
            .unwrap()
            .kv_metrics
            .record_put(bytes, duration);
    }

    /// Record a direct KV get operation.
    pub fn record_kv_get(&self, bytes: u64, duration: Duration) {
        self.metrics
            .lock()
            .unwrap()
            .kv_metrics
            .record_get(bytes, duration);
    }

    /// Consume a KV timing event from an instrumented KV interpreter.
    pub fn on_kv_timing(&self, event: KVTimingEvent) {
        event.apply_to(&mut self.metrics.lock().unwrap().kv_metrics);
    }

    /// End an in-flight LLM phase if one is currently open.
    pub fn flush_pending_llm_timing(&self) {
        self.metrics.lock().unwrap().record_llm_end();
    }

    /// Emit a sub-agent call event.
    pub fn on_sub_agent_call(&self, agent_name: &str, invocation_id: &str) {
        self.emit(StreamPart::sub_agent_call(agent_name, invocation_id));
    }

    /// Emit a sub-agent result event.
    pub fn on_sub_agent_result(&self, agent_name: &str, invocation_id: &str) {
        self.emit(StreamPart::sub_agent_result(agent_name, invocation_id));
    }

    /// Emit a tool progress event.
    pub fn on_tool_progress(
        &self,
        tool_name: &str,
        tool_call_id: Option<&str>,
        label: &str,
        phase_index: u8,
        total_phases: u8,
        milestone: Option<serde_json::Value>,
    ) {
        self.emit(StreamPart::tool_progress(
            tool_name,
            tool_call_id.map(|s| s.to_string()),
            label,
            phase_index,
            total_phases,
            milestone,
        ));
    }

    /// Emit a finish event.
    pub fn on_finish(&self, reason: agent_fw_core::stream_part::FinishReason, usage: TokenUsage) {
        self.emit(StreamPart::finish(reason, usage));
    }

    /// Emit an error event.
    pub fn on_error(&self, message: impl Into<String>) {
        self.emit(StreamPart::error(message));
    }

    /// Emit a data flow UI event (approval cards, etc.).
    pub fn on_flow_ui(&self, dsl: impl Into<String>) {
        self.emit(StreamPart::data_flow_ui(dsl));
    }

    /// Emit a step start event.
    pub fn on_step_start(&self) {
        self.emit(StreamPart::StepStart);
    }

    /// Finalize and emit cost/latency summaries with the supplied agent identity.
    ///
    /// Must be called once at end of request.
    /// Returns the final metrics summary.
    ///
    /// # Law L2 (Totality)
    /// Never panics — returns default summary if already finalized.
    pub fn finalize_as(
        &self,
        agent_name: impl Into<String>,
        model: impl Into<String>,
    ) -> MetricsSummary {
        // Ensure we only finalize once
        if self.finalized.swap(true, Ordering::SeqCst) {
            tracing::warn!("HookBridge finalized multiple times");
            return MetricsSummary::default();
        }

        let metrics = self.metrics.lock().unwrap().clone();

        let cost = self.cost_summary(agent_name, model);
        self.emit(StreamPart::cost_summary(cost));

        // Emit latency summary
        let latency = metrics.to_latency_summary();
        self.emit(StreamPart::latency_summary(latency.clone()));

        MetricsSummary {
            tokens: metrics.tokens,
            latency,
        }
    }

    /// Finalize and emit cost/latency summaries using a generic identity.
    pub fn finalize(&self) -> MetricsSummary {
        self.finalize_as("main", "unknown")
    }

    /// Finalize and emit cost, latency, and finish events in one step.
    pub fn finalize_with_finish(
        &self,
        agent_name: impl Into<String>,
        model: impl Into<String>,
        reason: FinishReason,
    ) -> MetricsSummary {
        let summary = self.finalize_as(agent_name, model);
        self.on_finish(reason, summary.token_usage());
        summary
    }

    /// Check if the underlying sink is open.
    pub fn is_open(&self) -> bool {
        self.sink.is_open()
    }

    /// Close the underlying sink.
    pub fn close(&self) {
        self.sink.close();
    }

    /// Internal emit helper with graceful degradation on closed sink.
    fn emit(&self, part: StreamPart) {
        if self.sink.is_open() {
            self.sink.emit(part);
        }
        // Silent drop if closed — upholds L2 (totality)
    }

    fn emit_card_events(&self, ui: &PrecomputedUi) {
        if ui.card_emitted {
            if let Some(ref cell) = self.pending_card_cell {
                if let Some(card) = cell.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    if let Some(summary) = card.display_summary {
                        self.emit(StreamPart::text(&summary));
                    }
                    if let Some(dsl) = card.approval_dsl {
                        self.emit(StreamPart::data_flow_ui(dsl));
                    }
                }
            }
            self.suppress_text.store(true, Ordering::Release);
        } else {
            if let Some(ref summary) = ui.display_summary {
                self.emit(StreamPart::text(summary));
            }
            if let Some(ref dsl) = ui.flow_dsl {
                self.emit(StreamPart::data_flow_ui(dsl.clone()));
                self.suppress_text.store(true, Ordering::Release);
            }
        }
    }
}

impl<S: EventSink> Clone for HookBridge<S> {
    fn clone(&self) -> Self {
        Self {
            sink: Arc::clone(&self.sink),
            metrics: Arc::clone(&self.metrics),
            pending_tools: Arc::clone(&self.pending_tools),
            finalized: Arc::clone(&self.finalized),
            suppress_text: Arc::clone(&self.suppress_text),
            tool_call_id_cell: self.tool_call_id_cell.clone(),
            pending_card_cell: self.pending_card_cell.clone(),
        }
    }
}

struct PrecomputedUi {
    flow_dsl: Option<String>,
    display_summary: Option<String>,
    card_emitted: bool,
}

fn extract_precomputed_ui(result: &mut serde_json::Value) -> PrecomputedUi {
    let obj = match result.as_object_mut() {
        Some(obj) => obj,
        None => {
            return PrecomputedUi {
                flow_dsl: None,
                display_summary: None,
                card_emitted: false,
            };
        }
    };

    let card_emitted = matches!(
        obj.remove("_cardEmitted"),
        Some(serde_json::Value::Bool(true))
    );

    let flow_dsl = match obj.remove("approvalDsl") {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(other) => {
            obj.insert("approvalDsl".to_string(), other);
            None
        }
        None => None,
    };

    let display_summary = match obj.remove("displaySummary") {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(other) => {
            obj.insert("displaySummary".to_string(), other);
            None
        }
        None => None,
    };

    PrecomputedUi {
        flow_dsl,
        display_summary,
        card_emitted,
    }
}

/// Summary of metrics collected during a request.
#[derive(Debug, Clone, Default)]
pub struct MetricsSummary {
    /// Token usage metrics.
    pub tokens: TokenMetrics,
    /// Latency metrics.
    pub latency: LatencySummary,
}

impl MetricsSummary {
    /// Create an empty summary.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Token usage rendered in AI-SDK prompt/completion terms.
    pub fn token_usage(&self) -> TokenUsage {
        TokenUsage::new(
            self.tokens.input_tokens,
            self.tokens.output_tokens,
            self.tokens.cached_tokens,
            self.tokens.cache_creation_tokens,
        )
    }

    /// Get total tokens (input + output).
    pub fn total_tokens(&self) -> u64 {
        self.tokens.total_tokens()
    }

    /// Get total request duration in milliseconds.
    pub fn total_duration_ms(&self) -> u64 {
        self.latency.total_duration_ms
    }

    /// Get LLM time in milliseconds.
    pub fn llm_time_ms(&self) -> u64 {
        self.latency.phases.llm_time_ms
    }

    /// Get tool time in milliseconds.
    pub fn tool_time_ms(&self) -> u64 {
        self.latency.phases.tool_time_ms
    }

    /// Get number of completed tools.
    pub fn completed_tools(&self) -> usize {
        self.latency.completed_tool_count()
    }

    /// Get number of failed tools.
    pub fn failed_tools(&self) -> usize {
        self.latency.failed_tool_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Simple test sink that captures events.
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

        fn events(&self) -> Vec<StreamPart> {
            self.events.lock().unwrap().clone()
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
    fn text_delta_emits_and_tracks_ttft() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_text_delta("Hello");
        bridge.on_text_delta(" world");

        let events = sink.events();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], StreamPart::Text { .. }));
        assert!(matches!(events[1], StreamPart::Text { .. }));

        // TTFT should be recorded
        let metrics = bridge.metrics.lock().unwrap();
        assert!(metrics.ttft_recorded);
        assert!(metrics.first_text_recorded);
        assert!(metrics.ttft.is_some());
        assert!(metrics.first_text.is_some());
    }

    #[test]
    fn tool_call_result_pairing() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_tool_call(
            "testTool",
            "call-1".to_string(),
            serde_json::json!({"x": 1}),
        );
        bridge.on_tool_result(
            "call-1",
            serde_json::json!({"result": 42}),
            ToolOutcome::Success,
        );

        let events = sink.events();
        assert_eq!(events.len(), 2);

        // First should be call
        assert!(events[0].is_tool_call());
        // Second should be result
        assert!(events[1].is_tool_result());
    }

    #[test]
    fn metrics_accumulate_on_finalize() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_tool_call("testTool", "call-1".to_string(), serde_json::json!({}));
        bridge.on_tool_result("call-1", serde_json::json!({}), ToolOutcome::Success);
        bridge.on_llm_complete(TokenMetrics {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });

        let summary = bridge.finalize();

        assert_eq!(summary.tokens.input_tokens, 100);
        assert_eq!(summary.tokens.output_tokens, 50);
        assert_eq!(summary.latency.completed_tool_count(), 1);
    }

    #[test]
    fn cost_summary_uses_supplied_agent_identity() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink);

        bridge.add_token_metrics(120, 45, 0, 0);
        let cost = bridge.cost_summary("planner", "claude-test");

        assert_eq!(cost.agents.len(), 1);
        assert_eq!(cost.agents[0].agent_name, "planner");
        assert_eq!(cost.agents[0].model, "claude-test");
        assert_eq!(cost.agents[0].usage.prompt_tokens, 120);
        assert_eq!(cost.agents[0].usage.completion_tokens, 45);
    }

    #[test]
    fn finalize_with_finish_emits_finish_with_accumulated_usage() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.add_token_metrics(80, 30, 0, 0);
        let summary = bridge.finalize_with_finish("planner", "claude-test", FinishReason::Stop);

        assert_eq!(summary.token_usage().prompt_tokens, 80);
        assert_eq!(summary.token_usage().completion_tokens, 30);

        let events = sink.events();
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::Finish { .. })));
    }

    #[test]
    fn kv_timing_accumulates_into_latency_summary() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink);

        bridge.on_kv_timing(KVTimingEvent::Put {
            key_len: 4,
            value_len: 128,
            duration_ms: 12,
        });
        bridge.on_kv_timing(KVTimingEvent::Get {
            key_len: 4,
            hit: true,
            duration_ms: 7,
        });

        let summary = bridge.finalize();
        assert_eq!(summary.latency.kv_metrics.bytes_written, 128);
        assert_eq!(summary.latency.kv_metrics.put_count, 1);
        assert_eq!(summary.latency.kv_metrics.get_count, 1);
        assert_eq!(summary.latency.kv_metrics.kv_duration_ms, 19);
    }

    #[test]
    fn finalize_emits_summaries() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_text_delta("test");
        bridge.finalize();

        let events = sink.events();
        // text, cost summary, latency summary
        assert_eq!(events.len(), 3);

        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataCostSummary { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamPart::DataLatencySummary { .. })));
    }

    #[test]
    fn double_finalize_returns_empty() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_text_delta("test");
        let first = bridge.finalize();
        let second = bridge.finalize();

        // First should have text-related data (TTFT recorded)
        assert!(first.latency.ttft_ms.is_some());
        // Second should be empty (default)
        assert_eq!(second.latency.total_duration_ms, 0);
        assert!(second.latency.ttft_ms.is_none());
    }

    #[test]
    fn closed_sink_gracefully_drops_events() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_text_delta("before");
        sink.close();
        bridge.on_text_delta("after");

        let events = sink.events();
        assert_eq!(events.len(), 1); // Only "before" emitted
    }

    #[test]
    fn orphan_result_emits_warning() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        // Result without preceding call
        bridge.on_tool_result("orphan-id", serde_json::json!({}), ToolOutcome::Success);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(events[0].is_tool_result());

        // No timing recorded
        let metrics = bridge.metrics.lock().unwrap();
        assert!(metrics.tool_timings.is_empty());
    }

    #[test]
    fn tool_call_id_cell_is_set_and_cleared() {
        let sink = Arc::new(TestSink::new());
        let hook = HookChannel::new();
        let bridge = HookBridge::new(sink).with_hook_channel(&hook);

        bridge.on_tool_call(
            "draft_plan",
            "call-123".to_string(),
            serde_json::json!({"x": 1}),
        );
        assert_eq!(hook.current_tool_call_id(), Some("call-123".to_string()));

        bridge.on_tool_result(
            "call-123",
            serde_json::json!({"ok": true}),
            ToolOutcome::Success,
        );
        assert_eq!(hook.current_tool_call_id(), None);
    }

    #[test]
    fn tool_result_emits_buffered_card_after_result() {
        let sink = Arc::new(TestSink::new());
        let hook = HookChannel::new();
        let bridge = HookBridge::new(sink.clone()).with_hook_channel(&hook);

        bridge.on_tool_call("draft_plan", "call-1".to_string(), serde_json::json!({}));
        hook.buffer_card(Some("summary".to_string()), Some("{dsl}".to_string()));
        bridge.on_tool_result(
            "call-1",
            serde_json::json!({"_cardEmitted": true}),
            ToolOutcome::Success,
        );

        let events = sink.events();
        assert_eq!(events.len(), 4);
        assert!(events[0].is_tool_call());
        assert!(events[1].is_tool_result());
        assert!(matches!(events[2], StreamPart::Text { .. }));
        assert!(matches!(events[3], StreamPart::DataFlowUI { .. }));
        assert!(hook.take_pending_card().is_none());
    }

    #[test]
    fn tool_result_extracts_inline_card_payloads() {
        let sink = Arc::new(TestSink::new());
        let bridge = HookBridge::new(sink.clone());

        bridge.on_tool_call("draft_plan", "call-1".to_string(), serde_json::json!({}));
        bridge.on_tool_result(
            "call-1",
            serde_json::json!({
                "approvalDsl": "{dsl}",
                "displaySummary": "summary",
                "data": 1
            }),
            ToolOutcome::Success,
        );

        let events = sink.events();
        assert_eq!(events.len(), 4);
        assert!(events[0].is_tool_call());
        assert!(events[1].is_tool_result());
        assert!(matches!(events[2], StreamPart::Text { .. }));
        assert!(matches!(events[3], StreamPart::DataFlowUI { .. }));
    }
}
