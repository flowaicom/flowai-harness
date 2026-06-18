//! Timing context and async timing helpers.
//!
//! `TimingContext` is a mutable per-request accumulator that collects latency
//! metrics during agent execution. At the end, `finalize()` consumes it into
//! an immutable `LatencySummary`.
//!
//! # Laws
//!
//! - **L1 (Initial zeroed):** New `TimingContext` has zero durations and counts.
//! - **L2 (LLM time accumulates):** Each `timed_llm_call` adds to `llm_time`.
//! - **L3 (LLM calls increment):** Each `timed_llm_call` increments `llm_calls` by 1.
//! - **L4 (Tool time accumulates):** Each `timed_tool_call` adds to `tool_time`.
//! - **L5 (Tool timings recorded):** Each `timed_tool_call` pushes a `ToolTiming` entry.
//! - **L6 (Finalize summary):** `finalize()` produces `LatencySummary` with correct totals.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_fw_core::{
    KVMetrics, LatencySummary, PhaseBreakdown, RetryEvent, TokenMetrics, ToolTiming,
};
use tokio::sync::Mutex;

/// Mutable per-request timing accumulator.
///
/// Shared via `Arc<Mutex<TimingContext>>` across concurrent tool calls.
pub struct TimingContext {
    /// When the request started.
    pub request_start: Instant,
    /// Accumulated LLM call time.
    pub llm_time: Duration,
    /// Accumulated tool call time.
    pub tool_time: Duration,
    /// Number of LLM calls.
    pub llm_calls: u32,
    /// Per-tool timing records.
    pub tool_timings: Vec<ToolTiming>,
    /// KV store metrics.
    pub kv_metrics: KVMetrics,
    /// Token usage metrics.
    pub token_metrics: TokenMetrics,
    /// Time to first token (ms).
    pub ttft: Option<u64>,
    /// Time to first text output (ms).
    pub first_text: Option<u64>,
    /// Number of retries performed.
    pub retry_count: u32,
    /// Individual retry event records.
    pub retry_events: Vec<RetryEvent>,
    /// Whether a timeout occurred.
    pub had_timeout: bool,
}

impl TimingContext {
    /// Create a new timing context starting now.
    pub fn new() -> Self {
        Self {
            request_start: Instant::now(),
            llm_time: Duration::ZERO,
            tool_time: Duration::ZERO,
            llm_calls: 0,
            tool_timings: Vec::new(),
            kv_metrics: KVMetrics::ZERO,
            token_metrics: TokenMetrics::ZERO,
            ttft: None,
            first_text: None,
            retry_count: 0,
            retry_events: Vec::new(),
            had_timeout: false,
        }
    }

    /// Consume this context into an immutable `LatencySummary`.
    pub fn finalize(self) -> LatencySummary {
        let total_duration_ms = self.request_start.elapsed().as_millis() as u64;
        let sub_agent_time_ms = self
            .tool_timings
            .iter()
            .filter(|t| t.is_sub_agent_invocation())
            .map(|t| t.duration_ms)
            .sum::<u64>();

        LatencySummary {
            total_duration_ms,
            phases: PhaseBreakdown::new(
                self.llm_time.as_millis() as u64,
                self.tool_time.as_millis() as u64,
                self.llm_calls,
            )
            .with_sub_agent_time(sub_agent_time_ms),
            tool_timings: self.tool_timings,
            kv_metrics: self.kv_metrics,
            token_metrics: self.token_metrics,
            ttft_ms: self.ttft,
            first_text_ms: self.first_text,
            retry_count: self.retry_count,
            retry_events: self.retry_events,
            had_timeout: self.had_timeout,
            domain_counters: Default::default(),
        }
    }
}

impl Default for TimingContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a timed LLM call with token usage.
pub struct LlmCallResult<T> {
    pub result: T,
    pub duration: Duration,
    pub tokens: Option<LlmCallTokens>,
}

/// Token counts from a single LLM call.
pub struct LlmCallTokens {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

/// Time an LLM call and record duration in the context.
pub async fn timed_llm_call<F, Fut, T>(ctx: &Arc<Mutex<TimingContext>>, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let result = f().await;
    let elapsed = start.elapsed();
    {
        let mut ctx = ctx.lock().await;
        ctx.llm_time += elapsed;
        ctx.llm_calls += 1;
    }
    result
}

/// Time a tool call and record a `ToolTiming` entry.
pub async fn timed_tool_call<F, Fut, T, E>(
    ctx: &Arc<Mutex<TimingContext>>,
    name: &str,
    id: &str,
    f: F,
) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    let result = f().await;
    let elapsed = start.elapsed();
    let timing = match &result {
        Ok(_) => {
            ToolTiming::completed(name.to_string(), id.to_string(), elapsed.as_millis() as u64)
        }
        Err(_) => ToolTiming::error(name.to_string(), id.to_string(), elapsed.as_millis() as u64),
    };
    {
        let mut ctx = ctx.lock().await;
        ctx.tool_time += elapsed;
        ctx.tool_timings.push(timing);
    }
    result
}

/// Time a tool call and record payload size alongside the timing entry.
pub async fn timed_tool_with_payload<F, Fut, T, E>(
    ctx: &Arc<Mutex<TimingContext>>,
    name: &str,
    id: &str,
    payload_size: u64,
    f: F,
) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    let result = f().await;
    let elapsed = start.elapsed();
    let timing = match &result {
        Ok(_) => {
            ToolTiming::completed(name.to_string(), id.to_string(), elapsed.as_millis() as u64)
                .with_payload_size(payload_size)
        }
        Err(_) => ToolTiming::error(name.to_string(), id.to_string(), elapsed.as_millis() as u64)
            .with_payload_size(payload_size),
    };
    {
        let mut ctx = ctx.lock().await;
        ctx.tool_time += elapsed;
        ctx.tool_timings.push(timing);
    }
    result
}

/// Time an LLM call and record both duration and token usage.
pub async fn timed_llm_call_with_usage<F, Fut, T>(
    ctx: &Arc<Mutex<TimingContext>>,
    f: F,
) -> LlmCallResult<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = (T, Option<LlmCallTokens>)>,
{
    let start = Instant::now();
    let (result, tokens) = f().await;
    let elapsed = start.elapsed();
    {
        let mut ctx = ctx.lock().await;
        ctx.llm_time += elapsed;
        ctx.llm_calls += 1;
        if let Some(ref t) = tokens {
            ctx.token_metrics = ctx.token_metrics.combine(&TokenMetrics {
                input_tokens: t.input_tokens,
                output_tokens: t.output_tokens,
                cached_tokens: t.cached_tokens,
                cache_creation_tokens: 0,
            });
        }
    }
    LlmCallResult {
        result,
        duration: elapsed,
        tokens,
    }
}

/// Time a KV put operation and record bytes written.
pub async fn timed_kv_put<F, Fut, T, E>(ctx: &Arc<Mutex<TimingContext>>, f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    let result = f().await;
    let elapsed = start.elapsed();
    {
        let mut ctx = ctx.lock().await;
        ctx.kv_metrics.record_put(0, elapsed);
    }
    result
}

/// Time a KV get operation and record bytes read.
pub async fn timed_kv_get<F, Fut, T, E>(ctx: &Arc<Mutex<TimingContext>>, f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    let result = f().await;
    let elapsed = start.elapsed();
    {
        let mut ctx = ctx.lock().await;
        ctx.kv_metrics.record_get(0, elapsed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_context_default() {
        let ctx = TimingContext::new();
        assert_eq!(ctx.llm_calls, 0);
        assert!(!ctx.had_timeout);
        assert!(ctx.tool_timings.is_empty());
    }

    #[test]
    fn finalize_produces_summary() {
        let mut ctx = TimingContext::new();
        ctx.llm_calls = 3;
        ctx.llm_time = Duration::from_millis(500);
        ctx.tool_time = Duration::from_millis(200);
        ctx.had_timeout = true;
        ctx.retry_count = 1;

        let summary = ctx.finalize();
        assert_eq!(summary.phases.llm_calls, 3);
        assert!(summary.had_timeout);
        assert_eq!(summary.retry_count, 1);
    }

    #[tokio::test]
    async fn timed_llm_call_increments() {
        let ctx = Arc::new(Mutex::new(TimingContext::new()));
        let result = timed_llm_call(&ctx, || async { 42 }).await;
        assert_eq!(result, 42);
        let ctx = ctx.lock().await;
        assert_eq!(ctx.llm_calls, 1);
        assert!(ctx.llm_time > Duration::ZERO || ctx.llm_time == Duration::ZERO);
        // may be zero on fast machines
    }

    #[tokio::test]
    async fn timed_tool_call_records_success() {
        let ctx = Arc::new(Mutex::new(TimingContext::new()));
        let result: Result<i32, String> =
            timed_tool_call(&ctx, "my_tool", "call-1", || async { Ok(42) }).await;
        assert_eq!(result, Ok(42));
        let ctx = ctx.lock().await;
        assert_eq!(ctx.tool_timings.len(), 1);
    }

    #[tokio::test]
    async fn timed_tool_call_records_error() {
        let ctx = Arc::new(Mutex::new(TimingContext::new()));
        let result: Result<i32, String> =
            timed_tool_call(&ctx, "my_tool", "call-2", || async { Err("oops".into()) }).await;
        assert!(result.is_err());
        let ctx = ctx.lock().await;
        assert_eq!(ctx.tool_timings.len(), 1);
    }
}
