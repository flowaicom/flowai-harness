//! Pure metrics accumulator — a fold over StreamPart events.
//!
//! Separates metrics collection from event forwarding (HookBridge).
//! This enables testing metric logic in isolation without `Arc<Mutex<>>`.
//!
//! # Laws
//!
//! **L1 (TTFT idempotence)**: Time-to-first-token is recorded once;
//!     subsequent text events do not overwrite it.
//!
//! **L2 (Token monotonicity)**: Token counts are monotonically
//!     non-decreasing across accumulation calls.
//!
//! **L3 (Finalization idempotence)**: Calling `snapshot()` multiple times
//!     returns identical `StreamMetrics` values.

use agent_fw_core::latency::{
    KVMetrics, KVTimingEvent, LatencySummary, PhaseBreakdown, RetryEvent, RetryReason,
    TokenMetrics, ToolTiming,
};
use agent_fw_core::StreamPart;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Snapshot of accumulated metrics at a point in time.
///
/// This is an owned, immutable value returned by [`MetricsAccumulator::snapshot`].
/// It mirrors the shape of the internal accumulator state but without any
/// timing anchors (`Instant`) — all durations are resolved at snapshot time.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    /// Token metrics accumulated across LLM calls.
    pub tokens: TokenMetrics,
    /// Provider-aggregated token metrics (overrides `tokens` when present).
    pub aggregated_tokens: Option<TokenMetrics>,
    /// Time to first token (any kind — text or reasoning).
    pub ttft: Option<Duration>,
    /// Time to first text delta (may differ from TTFT if reasoning comes first).
    pub first_text: Option<Duration>,
    /// Per-tool timing records for completed tools.
    pub tool_timings: Vec<ToolTiming>,
    /// Durations for tools tracked via `record_tool_start`/`record_tool_end`.
    pub named_tool_timings: BTreeMap<String, Duration>,
    /// KV store operation metrics.
    pub kv_metrics: KVMetrics,
    /// Phase breakdown (LLM time, tool time, call counts).
    pub phases: PhaseBreakdown,
    /// Number of retries recorded.
    pub retries: u32,
    /// Categorized retry events.
    pub retry_events: Vec<RetryEvent>,
    /// Whether a timeout was recorded.
    pub had_timeout: bool,
    /// Domain-specific counters (e.g., "tool_calls", "api_calls").
    pub domain_counters: BTreeMap<String, u64>,
    /// Total elapsed time since the accumulator was created.
    pub elapsed: Duration,
}

impl MetricsSnapshot {
    /// Build a [`LatencySummary`] suitable for stream emission.
    pub fn to_latency_summary(&self) -> LatencySummary {
        LatencySummary {
            total_duration_ms: self.elapsed.as_millis() as u64,
            phases: self.phases.clone(),
            tool_timings: self.tool_timings.clone(),
            kv_metrics: self.kv_metrics.clone(),
            token_metrics: self.effective_tokens().clone(),
            ttft_ms: self.ttft.map(|d| d.as_millis() as u64),
            first_text_ms: self.first_text.map(|d| d.as_millis() as u64),
            retry_count: self.retries,
            retry_events: self.retry_events.clone(),
            had_timeout: self.had_timeout,
            domain_counters: self.domain_counters.clone(),
        }
    }

    /// Returns aggregated tokens when available, otherwise the incrementally
    /// accumulated tokens.
    pub fn effective_tokens(&self) -> &TokenMetrics {
        self.aggregated_tokens.as_ref().unwrap_or(&self.tokens)
    }
}

/// Pure metrics accumulator that folds over stream events.
///
/// All mutation is synchronous and non-blocking. The accumulator carries no
/// `Arc`, `Mutex`, or IO — it is a plain value that can be embedded in
/// whatever concurrency wrapper the caller needs.
///
/// # Usage
///
/// ```ignore
/// let mut acc = MetricsAccumulator::new(Instant::now());
/// acc.accumulate(&part);           // fold a StreamPart
/// acc.add_tokens(token_metrics);   // record LLM token usage
/// let snap = acc.snapshot();       // immutable read
/// ```
pub struct MetricsAccumulator {
    /// Anchor instant for computing elapsed durations.
    request_start: Instant,

    // -- TTFT tracking --
    ttft: Option<Duration>,
    ttft_recorded: bool,
    first_text: Option<Duration>,
    first_text_recorded: bool,

    // -- Token metrics --
    tokens: TokenMetrics,
    aggregated_tokens: Option<TokenMetrics>,
    turn_tokens_captured: bool,

    // -- Tool timing (call-id correlated, from HookBridge flow) --
    tool_timings: Vec<ToolTiming>,

    // -- Named tool timing (start/end pairs, from caller) --
    active_tools: HashMap<String, Instant>,
    named_tool_timings: BTreeMap<String, Duration>,

    // -- Phase breakdown --
    phases: PhaseBreakdown,
    llm_start: Option<Instant>,

    // -- KV metrics --
    kv_metrics: KVMetrics,

    // -- Retries --
    retries: u32,
    retry_events: Vec<RetryEvent>,

    // -- Timeout --
    had_timeout: bool,

    // -- Domain counters --
    domain_counters: BTreeMap<String, u64>,
}

impl MetricsAccumulator {
    /// Create a new accumulator anchored at the given request start time.
    pub fn new(request_start: Instant) -> Self {
        Self {
            request_start,
            ttft: None,
            ttft_recorded: false,
            first_text: None,
            first_text_recorded: false,
            tokens: TokenMetrics::ZERO,
            aggregated_tokens: None,
            turn_tokens_captured: false,
            tool_timings: Vec::new(),
            active_tools: HashMap::new(),
            named_tool_timings: BTreeMap::new(),
            phases: PhaseBreakdown::ZERO,
            llm_start: None,
            kv_metrics: KVMetrics::ZERO,
            retries: 0,
            retry_events: Vec::new(),
            had_timeout: false,
            domain_counters: BTreeMap::new(),
        }
    }

    // =========================================================================
    // StreamPart fold
    // =========================================================================

    /// Fold a single [`StreamPart`] into the accumulator.
    ///
    /// This is the primary entry point for event-driven accumulation. It
    /// inspects the variant and updates TTFT, first-text, and token counters
    /// as appropriate. Tool timing correlation is **not** handled here because
    /// the accumulator does not own tool-call-ID state — that remains in
    /// `HookBridge`.
    pub fn accumulate(&mut self, part: &StreamPart) {
        match part {
            StreamPart::Text { .. } => {
                self.record_text_delta();
            }
            StreamPart::Reasoning { .. } => {
                self.record_ttft();
            }
            _ => {}
        }
    }

    // =========================================================================
    // TTFT
    // =========================================================================

    /// Record time-to-first-token. First call wins (L1 idempotence).
    pub fn record_ttft(&mut self) {
        if !self.ttft_recorded {
            self.ttft = Some(self.request_start.elapsed());
            self.ttft_recorded = true;
        }
    }

    /// Record first text delta. Also records TTFT if not yet recorded.
    pub fn record_text_delta(&mut self) {
        self.record_ttft();
        if !self.first_text_recorded {
            self.first_text = Some(self.request_start.elapsed());
            self.first_text_recorded = true;
        }
    }

    // =========================================================================
    // Token accumulation
    // =========================================================================

    /// Accumulate token usage from an LLM call and increment the call counter.
    pub fn add_tokens(&mut self, usage: TokenMetrics) {
        self.tokens = self.tokens.combine(&usage);
        self.phases.llm_calls += 1;
    }

    /// Add token usage only when the current turn has not already provided an
    /// aggregated usage record from the provider.
    pub fn add_tokens_if_uncaptured(&mut self, usage: TokenMetrics) {
        if !self.turn_tokens_captured {
            self.turn_tokens_captured = true;
            self.tokens = self.tokens.combine(&usage);
        }
    }

    /// Overwrite token metrics with a provider-aggregated view and mark the
    /// current turn as captured.
    pub fn set_aggregated_tokens(&mut self, usage: TokenMetrics) {
        self.turn_tokens_captured = true;
        self.aggregated_tokens = Some(usage);
    }

    /// Reset the per-turn provider-capture flag before a new model call.
    pub fn reset_turn_capture(&mut self) {
        self.turn_tokens_captured = false;
    }

    /// Current accumulated token metrics (before aggregation override).
    pub fn token_metrics(&self) -> &TokenMetrics {
        &self.tokens
    }

    // =========================================================================
    // LLM phase timing
    // =========================================================================

    /// Record LLM call start.
    pub fn record_llm_start(&mut self) {
        self.llm_start = Some(Instant::now());
    }

    /// Record LLM call end — accumulates elapsed time into phase breakdown.
    pub fn record_llm_end(&mut self) {
        if let Some(start) = self.llm_start.take() {
            let duration_ms = start.elapsed().as_millis() as u64;
            self.phases.llm_time_ms = self.phases.llm_time_ms.saturating_add(duration_ms);
        }
    }

    /// Record a completed LLM call with explicit duration and token usage.
    pub fn record_llm_call(&mut self, duration: Duration, usage: TokenMetrics) {
        self.phases.llm_time_ms = self
            .phases
            .llm_time_ms
            .saturating_add(duration.as_millis() as u64);
        self.add_tokens(usage);
    }

    // =========================================================================
    // Tool timing (structured ToolTiming records)
    // =========================================================================

    /// Add a completed [`ToolTiming`] record.
    pub fn add_tool_timing(&mut self, timing: ToolTiming) {
        self.tool_timings.push(timing);
    }

    // =========================================================================
    // Named tool timing (start/end pairs)
    // =========================================================================

    /// Start timing a named tool. If the tool is already active, the previous
    /// start is overwritten.
    pub fn record_tool_start(&mut self, tool_name: &str) {
        self.active_tools
            .insert(tool_name.to_string(), Instant::now());
    }

    /// End timing a named tool. The elapsed duration is stored in
    /// `named_tool_timings`. If the tool was not previously started, this is
    /// a no-op.
    pub fn record_tool_end(&mut self, tool_name: &str) {
        if let Some(start) = self.active_tools.remove(tool_name) {
            let duration = start.elapsed();
            self.named_tool_timings
                .entry(tool_name.to_string())
                .and_modify(|d| *d += duration)
                .or_insert(duration);
        }
    }

    // =========================================================================
    // KV metrics
    // =========================================================================

    /// Record a KV put operation.
    pub fn record_kv_put(&mut self, bytes: u64, duration: Duration) {
        self.kv_metrics.record_put(bytes, duration);
    }

    /// Record a KV get operation.
    pub fn record_kv_get(&mut self, bytes: u64, duration: Duration) {
        self.kv_metrics.record_get(bytes, duration);
    }

    /// Apply a [`KVTimingEvent`] to the internal KV metrics.
    pub fn on_kv_timing(&mut self, event: KVTimingEvent) {
        event.apply_to(&mut self.kv_metrics);
    }

    // =========================================================================
    // Retries
    // =========================================================================

    /// Record a retry (unknown reason, zero duration).
    pub fn record_retry(&mut self) {
        self.retries = self.retries.saturating_add(1);
        self.retry_events.push(RetryEvent {
            reason: RetryReason::Unknown,
            attempt: self.retries,
            duration_ms: 0,
        });
    }

    /// Record a categorized retry event.
    pub fn record_retry_with_reason(&mut self, reason: RetryReason, duration: Duration) {
        self.retries = self.retries.saturating_add(1);
        self.retry_events.push(RetryEvent {
            reason,
            attempt: self.retries,
            duration_ms: duration.as_millis() as u64,
        });
    }

    // =========================================================================
    // Timeout
    // =========================================================================

    /// Mark the request as having timed out.
    pub fn record_timeout(&mut self) {
        self.had_timeout = true;
    }

    // =========================================================================
    // Domain counters
    // =========================================================================

    /// Increment a named domain counter by 1.
    pub fn increment_counter(&mut self, key: &str) {
        *self.domain_counters.entry(key.to_string()).or_insert(0) += 1;
    }

    /// Set a named domain counter to a specific value.
    pub fn set_domain_counter(&mut self, key: impl Into<String>, value: u64) {
        self.domain_counters.insert(key.into(), value);
    }

    /// Read a named domain counter if it has been set.
    pub fn domain_counter(&self, key: &str) -> Option<u64> {
        self.domain_counters.get(key).copied()
    }

    // =========================================================================
    // Snapshot (immutable read)
    // =========================================================================

    /// Produce an immutable snapshot of all accumulated metrics.
    ///
    /// This method is pure and can be called any number of times — it will
    /// return structurally identical values as long as no mutation occurs
    /// between calls (L3 finalization idempotence).
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            tokens: self.tokens.clone(),
            aggregated_tokens: self.aggregated_tokens.clone(),
            ttft: self.ttft,
            first_text: self.first_text,
            tool_timings: self.tool_timings.clone(),
            named_tool_timings: self.named_tool_timings.clone(),
            kv_metrics: self.kv_metrics.clone(),
            phases: self.phases.clone(),
            retries: self.retries,
            retry_events: self.retry_events.clone(),
            had_timeout: self.had_timeout,
            domain_counters: self.domain_counters.clone(),
            elapsed: self.request_start.elapsed(),
        }
    }

    /// Build a [`LatencySummary`] from the current state.
    ///
    /// Convenience method equivalent to `self.snapshot().to_latency_summary()`.
    pub fn to_latency_summary(&self) -> LatencySummary {
        self.snapshot().to_latency_summary()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::latency::ToolStatus;
    use std::time::Instant;

    #[test]
    fn ttft_recorded_once() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_ttft();
        let first = acc.snapshot().ttft;
        std::thread::sleep(Duration::from_millis(1));
        acc.record_ttft();
        let second = acc.snapshot().ttft;
        assert_eq!(
            first, second,
            "L1: TTFT must not change after first recording"
        );
    }

    #[test]
    fn token_monotonicity() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens(TokenMetrics {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let s1 = acc.snapshot();
        acc.add_tokens(TokenMetrics {
            input_tokens: 5,
            output_tokens: 3,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let s2 = acc.snapshot();
        assert!(
            s2.tokens.input_tokens >= s1.tokens.input_tokens,
            "L2: input tokens must be monotonic"
        );
        assert!(
            s2.tokens.output_tokens >= s1.tokens.output_tokens,
            "L2: output tokens must be monotonic"
        );
    }

    #[test]
    fn snapshot_idempotence() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens(TokenMetrics {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        acc.record_ttft();
        let s1 = acc.snapshot();
        let s2 = acc.snapshot();
        assert_eq!(
            s1.tokens, s2.tokens,
            "L3: snapshot must be idempotent (tokens)"
        );
        assert_eq!(s1.ttft, s2.ttft, "L3: snapshot must be idempotent (ttft)");
        assert_eq!(
            s1.retries, s2.retries,
            "L3: snapshot must be idempotent (retries)"
        );
        assert_eq!(
            s1.domain_counters, s2.domain_counters,
            "L3: snapshot must be idempotent (domain_counters)"
        );
    }

    #[test]
    fn tool_timing_recorded() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_tool_start("build_plan");
        std::thread::sleep(Duration::from_millis(1));
        acc.record_tool_end("build_plan");
        let s = acc.snapshot();
        assert!(s.named_tool_timings.contains_key("build_plan"));
        assert!(s.named_tool_timings["build_plan"] > Duration::ZERO);
    }

    #[test]
    fn aggregated_tokens_override() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens(TokenMetrics {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        acc.set_aggregated_tokens(TokenMetrics {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let s = acc.snapshot();
        assert_eq!(
            s.aggregated_tokens.as_ref().map(|t| t.input_tokens),
            Some(100)
        );
        // effective_tokens should prefer aggregated
        assert_eq!(s.effective_tokens().input_tokens, 100);
    }

    #[test]
    fn retry_counting() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_retry();
        acc.record_retry();
        assert_eq!(acc.snapshot().retries, 2);
    }

    #[test]
    fn domain_counters() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.increment_counter("tool_calls");
        acc.increment_counter("tool_calls");
        acc.increment_counter("api_calls");
        let s = acc.snapshot();
        assert_eq!(s.domain_counters.get("tool_calls"), Some(&2));
        assert_eq!(s.domain_counters.get("api_calls"), Some(&1));
    }

    #[test]
    fn accumulate_text_records_ttft_and_first_text() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.accumulate(&StreamPart::text("hello"));
        let s = acc.snapshot();
        assert!(s.ttft.is_some(), "text should trigger TTFT");
        assert!(s.first_text.is_some(), "text should trigger first_text");
    }

    #[test]
    fn accumulate_reasoning_records_ttft_but_not_first_text() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.accumulate(&StreamPart::reasoning("thinking..."));
        let s = acc.snapshot();
        assert!(s.ttft.is_some(), "reasoning should trigger TTFT");
        assert!(
            s.first_text.is_none(),
            "reasoning should not trigger first_text"
        );
    }

    #[test]
    fn ttft_is_not_overwritten_by_text_after_reasoning() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.accumulate(&StreamPart::reasoning("thinking..."));
        let ttft_after_reasoning = acc.snapshot().ttft;
        std::thread::sleep(Duration::from_millis(1));
        acc.accumulate(&StreamPart::text("hello"));
        let ttft_after_text = acc.snapshot().ttft;
        assert_eq!(
            ttft_after_reasoning, ttft_after_text,
            "L1: TTFT set by reasoning must not be overwritten by text"
        );
    }

    #[test]
    fn add_tool_timing_shows_in_snapshot() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tool_timing(ToolTiming::completed("resolve", "call-1", 150));
        acc.add_tool_timing(ToolTiming::error("validate", "call-2", 30));
        let s = acc.snapshot();
        assert_eq!(s.tool_timings.len(), 2);
        assert_eq!(s.tool_timings[0].tool_name, "resolve");
        assert_eq!(s.tool_timings[0].status, ToolStatus::Completed);
        assert_eq!(s.tool_timings[1].tool_name, "validate");
        assert_eq!(s.tool_timings[1].status, ToolStatus::Error);
    }

    #[test]
    fn llm_phase_timing() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_llm_start();
        std::thread::sleep(Duration::from_millis(1));
        acc.record_llm_end();
        let s = acc.snapshot();
        assert!(
            s.phases.llm_time_ms > 0,
            "LLM phase timing should be recorded"
        );
    }

    #[test]
    fn record_llm_call_combines_duration_and_tokens() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_llm_call(
            Duration::from_millis(100),
            TokenMetrics {
                input_tokens: 50,
                output_tokens: 20,
                cached_tokens: 10,
                cache_creation_tokens: 0,
            },
        );
        let s = acc.snapshot();
        assert_eq!(s.phases.llm_time_ms, 100);
        assert_eq!(s.tokens.input_tokens, 50);
        assert_eq!(s.tokens.output_tokens, 20);
        assert_eq!(s.phases.llm_calls, 1);
    }

    #[test]
    fn kv_metrics_accumulate() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_kv_put(256, Duration::from_millis(5));
        acc.record_kv_get(128, Duration::from_millis(3));
        let s = acc.snapshot();
        assert_eq!(s.kv_metrics.bytes_written, 256);
        assert_eq!(s.kv_metrics.bytes_read, 128);
        assert_eq!(s.kv_metrics.put_count, 1);
        assert_eq!(s.kv_metrics.get_count, 1);
        assert_eq!(s.kv_metrics.kv_duration_ms, 8);
    }

    #[test]
    fn kv_timing_event_applied() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.on_kv_timing(KVTimingEvent::Put {
            key_len: 4,
            value_len: 100,
            duration_ms: 10,
        });
        let s = acc.snapshot();
        assert_eq!(s.kv_metrics.bytes_written, 100);
        assert_eq!(s.kv_metrics.put_count, 1);
    }

    #[test]
    fn timeout_recorded() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        assert!(!acc.snapshot().had_timeout);
        acc.record_timeout();
        assert!(acc.snapshot().had_timeout);
    }

    #[test]
    fn retry_with_reason() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_retry_with_reason(RetryReason::RateLimit, Duration::from_millis(500));
        let s = acc.snapshot();
        assert_eq!(s.retries, 1);
        assert_eq!(s.retry_events.len(), 1);
        assert_eq!(s.retry_events[0].reason, RetryReason::RateLimit);
        assert_eq!(s.retry_events[0].duration_ms, 500);
        assert_eq!(s.retry_events[0].attempt, 1);
    }

    #[test]
    fn set_domain_counter_and_read() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.set_domain_counter("products", 42);
        assert_eq!(acc.domain_counter("products"), Some(42));
        assert_eq!(acc.domain_counter("missing"), None);
    }

    #[test]
    fn to_latency_summary_mirrors_snapshot() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens(TokenMetrics {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 20,
            cache_creation_tokens: 0,
        });
        acc.record_ttft();
        acc.record_retry();
        acc.set_domain_counter("items", 10);

        let summary = acc.to_latency_summary();
        assert!(summary.ttft_ms.is_some());
        assert_eq!(summary.retry_count, 1);
        assert_eq!(summary.token_metrics.input_tokens, 100);
        assert_eq!(summary.domain_counters.get("items"), Some(&10));
    }

    #[test]
    fn add_tokens_if_uncaptured_first_call_wins() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens_if_uncaptured(TokenMetrics {
            input_tokens: 50,
            output_tokens: 25,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        acc.add_tokens_if_uncaptured(TokenMetrics {
            input_tokens: 100,
            output_tokens: 100,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let s = acc.snapshot();
        // Only the first call should have been applied
        assert_eq!(s.tokens.input_tokens, 50);
        assert_eq!(s.tokens.output_tokens, 25);
    }

    #[test]
    fn reset_turn_capture_allows_next_uncaptured() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens_if_uncaptured(TokenMetrics {
            input_tokens: 50,
            output_tokens: 25,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        acc.reset_turn_capture();
        acc.add_tokens_if_uncaptured(TokenMetrics {
            input_tokens: 30,
            output_tokens: 15,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let s = acc.snapshot();
        assert_eq!(s.tokens.input_tokens, 80);
        assert_eq!(s.tokens.output_tokens, 40);
    }

    #[test]
    fn named_tool_timing_accumulates_across_calls() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_tool_start("query");
        std::thread::sleep(Duration::from_millis(1));
        acc.record_tool_end("query");

        acc.record_tool_start("query");
        std::thread::sleep(Duration::from_millis(1));
        acc.record_tool_end("query");

        let s = acc.snapshot();
        assert!(s.named_tool_timings.contains_key("query"));
        // Two separate invocations should accumulate
        assert!(s.named_tool_timings["query"] >= Duration::from_millis(2));
    }

    #[test]
    fn record_tool_end_without_start_is_noop() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.record_tool_end("never_started");
        let s = acc.snapshot();
        assert!(s.named_tool_timings.is_empty());
    }

    #[test]
    fn latency_summary_uses_aggregated_tokens_when_present() {
        let mut acc = MetricsAccumulator::new(Instant::now());
        acc.add_tokens(TokenMetrics {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        acc.set_aggregated_tokens(TokenMetrics {
            input_tokens: 200,
            output_tokens: 100,
            cached_tokens: 50,
            cache_creation_tokens: 0,
        });
        let summary = acc.to_latency_summary();
        assert_eq!(summary.token_metrics.input_tokens, 200);
        assert_eq!(summary.token_metrics.output_tokens, 100);
    }
}
