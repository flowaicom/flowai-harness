//! Latency metrics with monoid instance for composition.
//!
//! # Design: Backend Owns Timing
//!
//! All timing is captured server-side where we have precise control:
//! - LLM call boundaries (before/after API call)
//! - Tool execution boundaries (before/after tool handler)
//! - Network latency is excluded (client-side concern)
//!
//! # Laws (Monoid)
//!
//! LatencySummary satisfies monoid laws for aggregation across sub-agents:
//! - L1. Identity:      combine(ZERO, a) = a = combine(a, ZERO)
//! - L2. Associativity: combine(combine(a, b), c) = combine(a, combine(b, c))
//!
//! # Serialization
//!
//! All timing values are in milliseconds (u64). This provides sub-second
//! precision while avoiding floating point in JSON serialization.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

// ============================================================================
// Tool Timing
// ============================================================================

/// Status of a tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Completed,
    Error,
}

/// Timing record for a single tool execution.
///
/// Captured at tool execution boundaries in the agent loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolTiming {
    /// Tool name (e.g., "query_data", "storePlan")
    pub tool_name: String,

    /// Unique invocation ID for correlation
    pub tool_call_id: String,

    /// Execution duration in milliseconds
    pub duration_ms: u64,

    /// Completion status
    pub status: ToolStatus,

    /// Optional payload size (e.g., product count, plan bytes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_size: Option<u64>,
}

impl ToolTiming {
    /// Create a completed tool timing record.
    pub fn completed(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            duration_ms,
            status: ToolStatus::Completed,
            payload_size: None,
        }
    }

    /// Create an error tool timing record.
    pub fn error(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            duration_ms,
            status: ToolStatus::Error,
            payload_size: None,
        }
    }

    /// Add payload size (builder pattern).
    pub fn with_payload_size(mut self, size: u64) -> Self {
        self.payload_size = Some(size);
        self
    }

    /// Whether this timing is for a sub-agent invocation.
    pub fn is_sub_agent_invocation(&self) -> bool {
        self.tool_name.starts_with("invoke")
    }
}

// ============================================================================
// Phase Breakdown
// ============================================================================

/// Phase breakdown within a request.
///
/// Phases represent distinct time slices of request processing:
/// - `llm_time_ms`: Time spent in LLM API calls (prompt processing + generation)
/// - `tool_time_ms`: Wall-clock time executing tool handlers
/// - `llm_calls`: Number of LLM round-trips (useful for multi-step agents)
///
/// # Monoid Instance
///
/// PhaseBreakdown forms a monoid under component-wise addition,
/// enabling aggregation across sub-agents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseBreakdown {
    /// Time in LLM API calls (milliseconds)
    pub llm_time_ms: u64,

    /// Wall-clock time in tool execution (milliseconds)
    pub tool_time_ms: u64,

    /// Wall-clock time in delegation (sub-agent) tools (milliseconds)
    #[serde(default)]
    pub sub_agent_time_ms: u64,

    /// Number of LLM round-trips
    pub llm_calls: u32,
}

impl PhaseBreakdown {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        llm_time_ms: 0,
        tool_time_ms: 0,
        sub_agent_time_ms: 0,
        llm_calls: 0,
    };

    /// Create a new phase breakdown.
    pub fn new(llm_time_ms: u64, tool_time_ms: u64, llm_calls: u32) -> Self {
        Self {
            llm_time_ms,
            tool_time_ms,
            sub_agent_time_ms: 0,
            llm_calls,
        }
    }

    /// Set sub-agent time (builder pattern).
    pub fn with_sub_agent_time(mut self, ms: u64) -> Self {
        self.sub_agent_time_ms = ms;
        self
    }

    /// Monoid combine operation.
    ///
    /// Uses saturating arithmetic to prevent overflow.
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            llm_time_ms: self.llm_time_ms.saturating_add(other.llm_time_ms),
            tool_time_ms: self.tool_time_ms.saturating_add(other.tool_time_ms),
            sub_agent_time_ms: self
                .sub_agent_time_ms
                .saturating_add(other.sub_agent_time_ms),
            llm_calls: self.llm_calls.saturating_add(other.llm_calls),
        }
    }

    /// Check if this is the identity element.
    pub fn is_zero(&self) -> bool {
        self.llm_time_ms == 0
            && self.tool_time_ms == 0
            && self.sub_agent_time_ms == 0
            && self.llm_calls == 0
    }
}

// ============================================================================
// KV Metrics
// ============================================================================

/// KV store metrics for latency panel.
///
/// Tracks bytes read/written and operation counts for persistence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KVMetrics {
    /// Total bytes written to KV store.
    pub bytes_written: u64,

    /// Total bytes read from KV store.
    pub bytes_read: u64,

    /// Total KV operation duration (ms).
    pub kv_duration_ms: u64,

    /// Number of put operations.
    pub put_count: u32,

    /// Number of get operations.
    pub get_count: u32,
}

impl KVMetrics {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        bytes_written: 0,
        bytes_read: 0,
        kv_duration_ms: 0,
        put_count: 0,
        get_count: 0,
    };

    /// Monoid combine operation.
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            bytes_written: self.bytes_written.saturating_add(other.bytes_written),
            bytes_read: self.bytes_read.saturating_add(other.bytes_read),
            kv_duration_ms: self.kv_duration_ms.saturating_add(other.kv_duration_ms),
            put_count: self.put_count.saturating_add(other.put_count),
            get_count: self.get_count.saturating_add(other.get_count),
        }
    }

    /// Check if this is the identity element.
    pub fn is_zero(&self) -> bool {
        self.bytes_written == 0
            && self.bytes_read == 0
            && self.kv_duration_ms == 0
            && self.put_count == 0
            && self.get_count == 0
    }

    /// Total bytes (read + written).
    pub fn total_bytes(&self) -> u64 {
        self.bytes_written.saturating_add(self.bytes_read)
    }

    /// Record a put operation's bytes and duration.
    pub fn record_put(&mut self, bytes: u64, duration: Duration) {
        self.bytes_written = self.bytes_written.saturating_add(bytes);
        self.kv_duration_ms = self
            .kv_duration_ms
            .saturating_add(duration.as_millis() as u64);
        self.put_count = self.put_count.saturating_add(1);
    }

    /// Record a get operation's bytes and duration.
    pub fn record_get(&mut self, bytes: u64, duration: Duration) {
        self.bytes_read = self.bytes_read.saturating_add(bytes);
        self.kv_duration_ms = self
            .kv_duration_ms
            .saturating_add(duration.as_millis() as u64);
        self.get_count = self.get_count.saturating_add(1);
    }

    /// Record a delete operation's duration.
    pub fn record_delete(&mut self, duration: Duration) {
        self.kv_duration_ms = self
            .kv_duration_ms
            .saturating_add(duration.as_millis() as u64);
    }

    /// Record a get_many operation.
    pub fn record_get_many(&mut self, hit_count: u32, duration: Duration) {
        self.get_count = self.get_count.saturating_add(hit_count);
        self.kv_duration_ms = self
            .kv_duration_ms
            .saturating_add(duration.as_millis() as u64);
    }
}

/// Timing event emitted by an instrumented KV store.
///
/// This is a pure data event so both interpreters and hook/latency collectors
/// can share one canonical vocabulary without a crate-layering dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KVTimingEvent {
    /// A put operation completed.
    Put {
        key_len: usize,
        value_len: usize,
        duration_ms: u64,
    },
    /// A get-like operation completed.
    Get {
        key_len: usize,
        hit: bool,
        duration_ms: u64,
    },
    /// A delete operation completed.
    Delete { key_len: usize, duration_ms: u64 },
    /// A batched read/list operation completed.
    GetMany {
        key_count: usize,
        hit_count: usize,
        duration_ms: u64,
    },
}

impl KVTimingEvent {
    /// Apply this event to a mutable KV metrics accumulator.
    pub fn apply_to(&self, metrics: &mut KVMetrics) {
        match self {
            Self::Put {
                value_len,
                duration_ms,
                ..
            } => metrics.record_put(*value_len as u64, Duration::from_millis(*duration_ms)),
            Self::Get {
                hit, duration_ms, ..
            } => {
                let bytes = if *hit { 1 } else { 0 };
                metrics.record_get(bytes, Duration::from_millis(*duration_ms))
            }
            Self::Delete { duration_ms, .. } => {
                metrics.record_delete(Duration::from_millis(*duration_ms))
            }
            Self::GetMany {
                hit_count,
                duration_ms,
                ..
            } => metrics.record_get_many(*hit_count as u32, Duration::from_millis(*duration_ms)),
        }
    }
}

// ============================================================================
// Token Metrics
// ============================================================================

/// LLM token usage metrics.
///
/// Tracks input/output tokens for cost attribution and capacity planning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenMetrics {
    /// Total input tokens sent to LLM.
    pub input_tokens: u64,

    /// Total output tokens received from LLM.
    pub output_tokens: u64,

    /// Cache-read tokens served from a provider prompt cache.
    pub cached_tokens: u64,

    /// Cache-write tokens used to populate a provider prompt cache.
    #[serde(default)]
    pub cache_creation_tokens: u64,
}

impl TokenMetrics {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        cache_creation_tokens: 0,
    };

    /// Monoid combine operation.
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cached_tokens: self.cached_tokens.saturating_add(other.cached_tokens),
            cache_creation_tokens: self
                .cache_creation_tokens
                .saturating_add(other.cache_creation_tokens),
        }
    }

    /// Total tokens (input + output).
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Check if this is the identity element.
    pub fn is_zero(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cached_tokens == 0
            && self.cache_creation_tokens == 0
    }

    /// Cache hit rate as a ratio (0.0 to 1.0).
    ///
    /// Returns None if no input tokens (division by zero).
    pub fn cache_hit_rate(&self) -> Option<f64> {
        if self.input_tokens == 0 {
            return None;
        }
        Some(self.cached_tokens as f64 / self.input_tokens as f64)
    }

    /// Cache hit rate as percentage (0.0 to 100.0).
    pub fn cache_hit_rate_percent(&self) -> Option<f64> {
        self.cache_hit_rate().map(|r| r * 100.0)
    }

    /// Estimated cache savings ratio from cache reads only.
    ///
    /// Assumes cached tokens cost ~10% of uncached tokens (90% savings).
    pub fn cache_savings_ratio(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        const CACHE_DISCOUNT: f64 = 0.9;
        CACHE_DISCOUNT * (self.cached_tokens as f64 / self.input_tokens as f64)
    }
}

// ============================================================================
// Retry Reason
// ============================================================================

/// Categorized reasons for retries during request processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    /// Rate limit exceeded (429)
    RateLimit,
    /// Request timeout
    Timeout,
    /// Context length exceeded (input too long)
    ContextLength,
    /// Server error (5xx)
    ServerError,
    /// Network connectivity issue
    NetworkError,
    /// Content filtered by safety system
    ContentFilter,
    /// Unknown or uncategorized error
    Unknown,
}

impl RetryReason {
    /// Suggest whether to retry based on the reason.
    pub fn should_retry(&self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Timeout | Self::ServerError | Self::NetworkError
        )
    }

    /// Suggested backoff multiplier for this retry reason.
    pub fn backoff_multiplier(&self) -> f64 {
        match self {
            Self::RateLimit => 2.0,
            Self::Timeout => 1.5,
            Self::ServerError => 1.5,
            Self::NetworkError => 1.0,
            _ => 1.0,
        }
    }
}

/// A single retry event with reason and timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryEvent {
    /// Why the retry occurred
    pub reason: RetryReason,
    /// Which attempt this was (1-indexed)
    pub attempt: u32,
    /// Duration of the failed attempt in ms
    pub duration_ms: u64,
}

// ============================================================================
// Latency Summary
// ============================================================================

/// Complete latency summary for a request.
///
/// Emitted as `DataLatencySummary` stream part at request end,
/// alongside `DataCostSummary` for token usage.
///
/// # Monoid Instance
///
/// LatencySummary forms a monoid under combine(), enabling
/// aggregation across sub-agents in agent networks.
///
/// # Streaming Latency
///
/// - `ttft_ms`: Time to first token - critical for streaming UX
/// - `first_text_ms`: Time to first text delta (may differ from TTFT if reasoning comes first)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    /// Total request duration in milliseconds (server-side, excludes network)
    pub total_duration_ms: u64,

    /// Phase breakdown (LLM vs tool time)
    pub phases: PhaseBreakdown,

    /// Per-tool timing records
    pub tool_timings: Vec<ToolTiming>,

    /// KV store metrics (bytes read/written, operation counts)
    #[serde(default, skip_serializing_if = "KVMetrics::is_zero")]
    pub kv_metrics: KVMetrics,

    /// LLM token usage metrics
    #[serde(default, skip_serializing_if = "TokenMetrics::is_zero")]
    pub token_metrics: TokenMetrics,

    /// Time to first token in milliseconds (streaming latency)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,

    /// Time to first text delta in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_text_ms: Option<u64>,

    /// Number of retries during request processing
    pub retry_count: u32,

    /// Categorized retry events (for detailed analysis)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_events: Vec<RetryEvent>,

    /// Whether a timeout occurred
    pub had_timeout: bool,

    /// Domain-specific counters. Combined with first-wins semantics.
    /// Example: {"productSetSize": 75, "planPayloadBytes": 2048}
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub domain_counters: BTreeMap<String, u64>,
}

impl LatencySummary {
    /// Monoid identity element.
    pub fn zero() -> Self {
        Self {
            total_duration_ms: 0,
            phases: PhaseBreakdown::ZERO,
            tool_timings: Vec::new(),
            kv_metrics: KVMetrics::ZERO,
            token_metrics: TokenMetrics::ZERO,
            ttft_ms: None,
            first_text_ms: None,
            retry_count: 0,
            retry_events: Vec::new(),
            had_timeout: false,
            domain_counters: BTreeMap::new(),
        }
    }

    /// Monoid combine operation for sub-agent aggregation.
    ///
    /// - Durations are summed
    /// - Phases are combined via PhaseBreakdown::combine
    /// - Tool timings are concatenated
    /// - KV metrics are combined
    /// - Token metrics are combined
    /// - TTFT/first_text_ms use minimum (earliest) non-None value
    /// - Retry counts are summed
    /// - Retry events are concatenated
    /// - Timeout is OR'd (true if any sub-agent timed out)
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            total_duration_ms: self
                .total_duration_ms
                .saturating_add(other.total_duration_ms),
            phases: self.phases.combine(&other.phases),
            tool_timings: self
                .tool_timings
                .iter()
                .chain(other.tool_timings.iter())
                .cloned()
                .collect(),
            kv_metrics: self.kv_metrics.combine(&other.kv_metrics),
            token_metrics: self.token_metrics.combine(&other.token_metrics),
            ttft_ms: match (self.ttft_ms, other.ttft_ms) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, None) => a,
                (None, b) => b,
            },
            first_text_ms: match (self.first_text_ms, other.first_text_ms) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, None) => a,
                (None, b) => b,
            },
            retry_count: self.retry_count.saturating_add(other.retry_count),
            retry_events: self
                .retry_events
                .iter()
                .chain(other.retry_events.iter())
                .cloned()
                .collect(),
            had_timeout: self.had_timeout || other.had_timeout,
            domain_counters: {
                let mut merged = self.domain_counters.clone();
                for (k, v) in &other.domain_counters {
                    merged.entry(k.clone()).or_insert(*v);
                }
                merged
            },
        }
    }

    /// Check if this is the identity element.
    pub fn is_zero(&self) -> bool {
        self.total_duration_ms == 0
            && self.phases.is_zero()
            && self.tool_timings.is_empty()
            && self.kv_metrics.is_zero()
            && self.token_metrics.is_zero()
            && self.ttft_ms.is_none()
            && self.first_text_ms.is_none()
            && self.retry_count == 0
            && self.retry_events.is_empty()
            && !self.had_timeout
            && self.domain_counters.is_empty()
    }

    /// Get count of completed tools.
    pub fn completed_tool_count(&self) -> usize {
        self.tool_timings
            .iter()
            .filter(|t| t.status == ToolStatus::Completed)
            .count()
    }

    /// Get count of failed tools.
    pub fn failed_tool_count(&self) -> usize {
        self.tool_timings
            .iter()
            .filter(|t| t.status == ToolStatus::Error)
            .count()
    }

    /// Output tokens per second (throughput metric).
    pub fn tokens_per_second(&self) -> Option<f64> {
        if self.total_duration_ms == 0 || self.token_metrics.output_tokens == 0 {
            return None;
        }
        let duration_secs = self.total_duration_ms as f64 / 1000.0;
        Some(self.token_metrics.output_tokens as f64 / duration_secs)
    }

    /// LLM utilization ratio (LLM time / total time).
    pub fn llm_utilization(&self) -> Option<f64> {
        if self.total_duration_ms == 0 {
            return None;
        }
        Some(self.phases.llm_time_ms as f64 / self.total_duration_ms as f64)
    }

    /// Tool utilization ratio (tool time / total time).
    pub fn tool_utilization(&self) -> Option<f64> {
        if self.total_duration_ms == 0 {
            return None;
        }
        Some(self.phases.tool_time_ms as f64 / self.total_duration_ms as f64)
    }

    /// Average tokens per LLM call.
    pub fn tokens_per_llm_call(&self) -> Option<f64> {
        if self.phases.llm_calls == 0 {
            return None;
        }
        Some(self.token_metrics.total_tokens() as f64 / self.phases.llm_calls as f64)
    }

    /// Average LLM call latency in milliseconds.
    pub fn avg_llm_latency_ms(&self) -> Option<f64> {
        if self.phases.llm_calls == 0 {
            return None;
        }
        Some(self.phases.llm_time_ms as f64 / self.phases.llm_calls as f64)
    }

    /// Average tool latency in milliseconds.
    pub fn avg_tool_latency_ms(&self) -> Option<f64> {
        if self.tool_timings.is_empty() {
            return None;
        }
        let total: u64 = self.tool_timings.iter().map(|t| t.duration_ms).sum();
        Some(total as f64 / self.tool_timings.len() as f64)
    }

    /// Get retry events by reason.
    pub fn retries_by_reason(&self, reason: RetryReason) -> impl Iterator<Item = &RetryEvent> {
        self.retry_events.iter().filter(move |e| e.reason == reason)
    }

    /// Count retries by reason.
    pub fn retry_count_by_reason(&self, reason: RetryReason) -> usize {
        self.retries_by_reason(reason).count()
    }
}

impl Default for LatencySummary {
    fn default() -> Self {
        Self::zero()
    }
}

// ============================================================================
// Latency Distribution
// ============================================================================

/// A latency distribution for calculating percentiles.
///
/// Uses a sorted vector of observations. For production use with high volume,
/// consider a streaming algorithm like t-digest or HDR histogram.
#[derive(Debug, Clone, Default)]
pub struct LatencyDistribution {
    observations: Vec<u64>,
    sorted: bool,
}

impl LatencyDistribution {
    /// Create a new empty distribution.
    pub fn new() -> Self {
        Self {
            observations: Vec::new(),
            sorted: true,
        }
    }

    /// Create with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            observations: Vec::with_capacity(capacity),
            sorted: true,
        }
    }

    /// Add an observation (milliseconds).
    pub fn observe(&mut self, value_ms: u64) {
        self.observations.push(value_ms);
        self.sorted = false;
    }

    /// Add multiple observations.
    pub fn observe_many(&mut self, values: impl IntoIterator<Item = u64>) {
        self.observations.extend(values);
        self.sorted = false;
    }

    /// Get the number of observations.
    pub fn count(&self) -> usize {
        self.observations.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    fn ensure_sorted(&mut self) {
        if !self.sorted {
            self.observations.sort_unstable();
            self.sorted = true;
        }
    }

    /// Calculate a percentile (0.0 to 1.0).
    pub fn percentile(&mut self, p: f64) -> Option<u64> {
        if self.observations.is_empty() {
            return None;
        }

        self.ensure_sorted();

        let p = p.clamp(0.0, 1.0);
        let n = self.observations.len();

        let rank = p * (n - 1) as f64;
        let lower_idx = (rank.floor() as usize).min(n - 1);
        let upper_idx = (rank.ceil() as usize).min(n - 1);

        if lower_idx == upper_idx {
            return Some(self.observations[lower_idx]);
        }

        let frac = rank - lower_idx as f64;
        let lower = self.observations[lower_idx] as f64;
        let upper = self.observations[upper_idx] as f64;

        Some((lower + frac * (upper - lower)).round() as u64)
    }

    /// Get the median (p50).
    pub fn p50(&mut self) -> Option<u64> {
        self.percentile(0.50)
    }

    /// Get p90.
    pub fn p90(&mut self) -> Option<u64> {
        self.percentile(0.90)
    }

    /// Get p95.
    pub fn p95(&mut self) -> Option<u64> {
        self.percentile(0.95)
    }

    /// Get p99.
    pub fn p99(&mut self) -> Option<u64> {
        self.percentile(0.99)
    }

    /// Get the minimum value.
    pub fn min(&mut self) -> Option<u64> {
        if self.observations.is_empty() {
            return None;
        }
        self.ensure_sorted();
        Some(self.observations[0])
    }

    /// Get the maximum value.
    pub fn max(&mut self) -> Option<u64> {
        if self.observations.is_empty() {
            return None;
        }
        self.ensure_sorted();
        Some(self.observations[self.observations.len() - 1])
    }

    /// Get the mean (average).
    pub fn mean(&self) -> Option<f64> {
        if self.observations.is_empty() {
            return None;
        }
        let sum: u64 = self.observations.iter().sum();
        Some(sum as f64 / self.observations.len() as f64)
    }

    /// Get a summary of the distribution.
    pub fn summary(&mut self) -> Option<DistributionSummary> {
        if self.observations.is_empty() {
            return None;
        }

        Some(DistributionSummary {
            count: self.count(),
            min: self.min().unwrap(),
            max: self.max().unwrap(),
            mean: self.mean().unwrap(),
            p50: self.p50().unwrap(),
            p95: self.p95().unwrap(),
            p99: self.p99().unwrap(),
        })
    }

    /// Merge another distribution into this one.
    pub fn merge(&mut self, other: &Self) {
        self.observations.extend(other.observations.iter().copied());
        self.sorted = false;
    }

    /// Clear all observations.
    pub fn clear(&mut self) {
        self.observations.clear();
        self.sorted = true;
    }
}

/// Summary statistics from a distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionSummary {
    pub count: usize,
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
}

// ============================================================================
// Tool Distribution
// ============================================================================

/// Aggregated latency distribution per tool.
#[derive(Debug, Clone, Default)]
pub struct ToolDistributions {
    distributions: std::collections::HashMap<String, LatencyDistribution>,
}

impl ToolDistributions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tool timing.
    pub fn record(&mut self, tool_name: &str, duration_ms: u64) {
        self.distributions
            .entry(tool_name.to_string())
            .or_default()
            .observe(duration_ms);
    }

    /// Record timings from a LatencySummary.
    pub fn record_from_summary(&mut self, summary: &LatencySummary) {
        for timing in &summary.tool_timings {
            self.record(&timing.tool_name, timing.duration_ms);
        }
    }

    /// Get summaries for all tools, sorted by total time descending.
    pub fn summaries(&mut self) -> Vec<ToolDistributionSummary> {
        let mut results: Vec<_> = self
            .distributions
            .iter_mut()
            .filter_map(|(name, dist)| {
                dist.summary().map(|s| ToolDistributionSummary {
                    tool_name: name.clone(),
                    stats: s,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            let a_total = a.stats.mean * a.stats.count as f64;
            let b_total = b.stats.mean * b.stats.count as f64;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    /// Get the top N tools by total time.
    pub fn top_contributors(&mut self, n: usize) -> Vec<ToolDistributionSummary> {
        let mut summaries = self.summaries();
        summaries.truncate(n);
        summaries
    }

    /// Clear all distributions.
    pub fn clear(&mut self) {
        self.distributions.clear();
    }
}

/// Summary for a single tool's latency distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDistributionSummary {
    pub tool_name: String,
    pub stats: DistributionSummary,
}

// ============================================================================
// Free Functions
// ============================================================================

/// Get the identity element (zero latency).
pub fn zero_latency() -> LatencySummary {
    LatencySummary::zero()
}

/// Combine two latency summaries (monoid operation).
pub fn combine_latency(a: &LatencySummary, b: &LatencySummary) -> LatencySummary {
    a.combine(b)
}

/// Fold a collection of latency summaries into a single summary.
pub fn fold_latency<'a>(summaries: impl IntoIterator<Item = &'a LatencySummary>) -> LatencySummary {
    summaries
        .into_iter()
        .fold(LatencySummary::zero(), |acc, s| acc.combine(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_is_identity() {
        let summary = LatencySummary {
            total_duration_ms: 1000,
            phases: PhaseBreakdown::new(500, 300, 2),
            tool_timings: vec![ToolTiming::completed("test", "id-1", 100)],
            kv_metrics: KVMetrics {
                bytes_written: 100,
                bytes_read: 50,
                kv_duration_ms: 10,
                put_count: 1,
                get_count: 1,
            },
            token_metrics: TokenMetrics {
                input_tokens: 500,
                output_tokens: 100,
                cached_tokens: 50,
                cache_creation_tokens: 0,
            },
            ttft_ms: Some(120),
            first_text_ms: Some(150),
            retry_count: 1,
            retry_events: vec![],
            had_timeout: false,
            domain_counters: BTreeMap::from([("productSetSize".into(), 50)]),
        };

        assert_eq!(summary.combine(&LatencySummary::zero()), summary);
        assert_eq!(LatencySummary::zero().combine(&summary), summary);
    }

    #[test]
    fn phase_breakdown_combine() {
        let a = PhaseBreakdown::new(100, 50, 1);
        let b = PhaseBreakdown::new(200, 100, 2);
        let combined = a.combine(&b);

        assert_eq!(combined.llm_time_ms, 300);
        assert_eq!(combined.tool_time_ms, 150);
        assert_eq!(combined.llm_calls, 3);
    }

    #[test]
    fn tool_timing_builder() {
        let timing = ToolTiming::completed("query_data", "call-123", 250).with_payload_size(150);

        assert_eq!(timing.tool_name, "query_data");
        assert_eq!(timing.duration_ms, 250);
        assert_eq!(timing.status, ToolStatus::Completed);
        assert_eq!(timing.payload_size, Some(150));
    }

    #[test]
    fn serialization_roundtrip() {
        let summary = LatencySummary {
            total_duration_ms: 1500,
            phases: PhaseBreakdown::new(800, 400, 3),
            tool_timings: vec![
                ToolTiming::completed("tool1", "id-1", 200),
                ToolTiming::error("tool2", "id-2", 100),
            ],
            kv_metrics: KVMetrics {
                bytes_written: 2048,
                bytes_read: 512,
                kv_duration_ms: 25,
                put_count: 2,
                get_count: 1,
            },
            token_metrics: TokenMetrics {
                input_tokens: 1000,
                output_tokens: 200,
                cached_tokens: 100,
                cache_creation_tokens: 0,
            },
            ttft_ms: Some(100),
            first_text_ms: Some(120),
            retry_count: 0,
            retry_events: vec![],
            had_timeout: false,
            domain_counters: BTreeMap::new(),
        };

        let json = serde_json::to_string(&summary).unwrap();
        let parsed: LatencySummary = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.total_duration_ms, 1500);
        assert_eq!(parsed.phases.llm_calls, 3);
        assert_eq!(parsed.tool_timings.len(), 2);
        assert_eq!(parsed.kv_metrics.bytes_written, 2048);
        assert_eq!(parsed.token_metrics.input_tokens, 1000);
        assert_eq!(parsed.ttft_ms, Some(100));
        assert_eq!(parsed.first_text_ms, Some(120));
    }

    #[test]
    fn fold_latency_aggregates() {
        let summaries = vec![
            LatencySummary {
                total_duration_ms: 500,
                phases: PhaseBreakdown::new(300, 100, 1),
                tool_timings: vec![ToolTiming::completed("t1", "id-1", 100)],
                ..Default::default()
            },
            LatencySummary {
                total_duration_ms: 700,
                phases: PhaseBreakdown::new(400, 200, 2),
                tool_timings: vec![ToolTiming::completed("t2", "id-2", 200)],
                ..Default::default()
            },
        ];

        let total = fold_latency(&summaries);
        assert_eq!(total.total_duration_ms, 1200);
        assert_eq!(total.phases.llm_calls, 3);
        assert_eq!(total.tool_timings.len(), 2);
    }

    #[test]
    fn completed_and_failed_counts() {
        let summary = LatencySummary {
            tool_timings: vec![
                ToolTiming::completed("t1", "id-1", 100),
                ToolTiming::completed("t2", "id-2", 200),
                ToolTiming::error("t3", "id-3", 50),
            ],
            ..Default::default()
        };

        assert_eq!(summary.completed_tool_count(), 2);
        assert_eq!(summary.failed_tool_count(), 1);
    }

    // =========================================================================
    // Property-Based Tests (Hegel) — PhaseBreakdown monoid
    // =========================================================================

    use hegel::generators;

    fn draw_phase(tc: &hegel::TestCase) -> PhaseBreakdown {
        PhaseBreakdown {
            llm_time_ms: tc.draw(generators::integers::<u64>()),
            tool_time_ms: tc.draw(generators::integers::<u64>()),
            sub_agent_time_ms: tc.draw(generators::integers::<u64>()),
            llm_calls: tc.draw(generators::integers::<u32>()),
        }
    }

    #[hegel::test]
    fn phase_identity_left(tc: hegel::TestCase) {
        let a = draw_phase(&tc);
        assert_eq!(PhaseBreakdown::ZERO.combine(&a), a);
    }

    #[hegel::test]
    fn phase_identity_right(tc: hegel::TestCase) {
        let a = draw_phase(&tc);
        assert_eq!(a.combine(&PhaseBreakdown::ZERO), a);
    }

    #[hegel::test]
    fn phase_associativity(tc: hegel::TestCase) {
        let a = draw_phase(&tc);
        let b = draw_phase(&tc);
        let c = draw_phase(&tc);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[hegel::test]
    fn phase_commutativity(tc: hegel::TestCase) {
        let a = draw_phase(&tc);
        let b = draw_phase(&tc);
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    // =========================================================================
    // Distribution Tests
    // =========================================================================

    #[test]
    fn distribution_empty() {
        let mut dist = LatencyDistribution::new();
        assert!(dist.is_empty());
        assert_eq!(dist.p50(), None);
        assert_eq!(dist.mean(), None);
    }

    #[test]
    fn distribution_single_value() {
        let mut dist = LatencyDistribution::new();
        dist.observe(100);

        assert_eq!(dist.count(), 1);
        assert_eq!(dist.p50(), Some(100));
        assert_eq!(dist.min(), Some(100));
        assert_eq!(dist.max(), Some(100));
        assert_eq!(dist.mean(), Some(100.0));
    }

    #[test]
    fn distribution_percentiles() {
        let mut dist = LatencyDistribution::new();
        for i in 1..=100 {
            dist.observe(i);
        }

        assert_eq!(dist.count(), 100);
        assert_eq!(dist.min(), Some(1));
        assert_eq!(dist.max(), Some(100));
        assert_eq!(dist.p50(), Some(51));
        assert_eq!(dist.p95(), Some(95));
        assert_eq!(dist.p99(), Some(99));
    }

    #[test]
    fn distribution_merge() {
        let mut dist1 = LatencyDistribution::new();
        dist1.observe(100);
        dist1.observe(200);

        let mut dist2 = LatencyDistribution::new();
        dist2.observe(300);
        dist2.observe(400);

        dist1.merge(&dist2);

        assert_eq!(dist1.count(), 4);
        assert_eq!(dist1.min(), Some(100));
        assert_eq!(dist1.max(), Some(400));
    }

    #[test]
    fn tool_distributions_record() {
        let mut dists = ToolDistributions::new();

        dists.record("query_data", 100);
        dists.record("query_data", 150);
        dists.record("storePlan", 50);

        let summaries = dists.summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].tool_name, "query_data");
        assert_eq!(summaries[0].stats.count, 2);
    }

    // =========================================================================
    // KV Metrics Tests
    // =========================================================================

    #[test]
    fn kv_metrics_combine() {
        let a = KVMetrics {
            bytes_written: 100,
            bytes_read: 50,
            kv_duration_ms: 10,
            put_count: 2,
            get_count: 1,
        };
        let b = KVMetrics {
            bytes_written: 200,
            bytes_read: 100,
            kv_duration_ms: 15,
            put_count: 1,
            get_count: 2,
        };
        let combined = a.combine(&b);

        assert_eq!(combined.bytes_written, 300);
        assert_eq!(combined.bytes_read, 150);
        assert_eq!(combined.kv_duration_ms, 25);
        assert_eq!(combined.put_count, 3);
        assert_eq!(combined.get_count, 3);
    }

    // =========================================================================
    // Token Metrics Tests
    // =========================================================================

    #[test]
    fn token_metrics_cache_hit_rate() {
        let metrics = TokenMetrics {
            input_tokens: 1000,
            output_tokens: 200,
            cached_tokens: 800,
            cache_creation_tokens: 0,
        };
        assert_eq!(metrics.cache_hit_rate(), Some(0.8));
        assert_eq!(metrics.cache_hit_rate_percent(), Some(80.0));
    }

    #[test]
    fn token_metrics_cache_hit_rate_zero_input() {
        let metrics = TokenMetrics::ZERO;
        assert_eq!(metrics.cache_hit_rate(), None);
    }

    // =========================================================================
    // KV/Token Metrics Property Tests (Hegel)
    // =========================================================================

    fn draw_kv_metrics(tc: &hegel::TestCase) -> KVMetrics {
        KVMetrics {
            bytes_written: tc.draw(generators::integers::<u64>()),
            bytes_read: tc.draw(generators::integers::<u64>()),
            kv_duration_ms: tc.draw(generators::integers::<u64>()),
            put_count: tc.draw(generators::integers::<u32>()),
            get_count: tc.draw(generators::integers::<u32>()),
        }
    }

    fn draw_token_metrics(tc: &hegel::TestCase) -> TokenMetrics {
        TokenMetrics {
            input_tokens: tc.draw(generators::integers::<u64>()),
            output_tokens: tc.draw(generators::integers::<u64>()),
            cached_tokens: tc.draw(generators::integers::<u64>()),
            cache_creation_tokens: tc.draw(generators::integers::<u64>()),
        }
    }

    #[hegel::test]
    fn kv_metrics_identity_left(tc: hegel::TestCase) {
        let a = draw_kv_metrics(&tc);
        assert_eq!(KVMetrics::ZERO.combine(&a), a);
    }

    #[hegel::test]
    fn kv_metrics_identity_right(tc: hegel::TestCase) {
        let a = draw_kv_metrics(&tc);
        assert_eq!(a.combine(&KVMetrics::ZERO), a);
    }

    #[hegel::test]
    fn kv_metrics_associativity(tc: hegel::TestCase) {
        let a = draw_kv_metrics(&tc);
        let b = draw_kv_metrics(&tc);
        let c = draw_kv_metrics(&tc);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[hegel::test]
    fn kv_metrics_commutativity(tc: hegel::TestCase) {
        let a = draw_kv_metrics(&tc);
        let b = draw_kv_metrics(&tc);
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    #[hegel::test]
    fn token_metrics_identity_left(tc: hegel::TestCase) {
        let a = draw_token_metrics(&tc);
        assert_eq!(TokenMetrics::ZERO.combine(&a), a);
    }

    #[hegel::test]
    fn token_metrics_identity_right(tc: hegel::TestCase) {
        let a = draw_token_metrics(&tc);
        assert_eq!(a.combine(&TokenMetrics::ZERO), a);
    }

    #[hegel::test]
    fn token_metrics_associativity(tc: hegel::TestCase) {
        let a = draw_token_metrics(&tc);
        let b = draw_token_metrics(&tc);
        let c = draw_token_metrics(&tc);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[hegel::test]
    fn token_metrics_commutativity(tc: hegel::TestCase) {
        let a = draw_token_metrics(&tc);
        let b = draw_token_metrics(&tc);
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    // --- Cache hit rate: bounded when invariant (cached ≤ input) holds ---

    #[hegel::test]
    fn cache_hit_rate_bounded(tc: hegel::TestCase) {
        let input = tc.draw(generators::integers::<u64>());
        let cached = tc.draw(generators::integers::<u64>().max_value(input));
        let m = TokenMetrics {
            input_tokens: input,
            output_tokens: tc.draw(generators::integers::<u64>()),
            cached_tokens: cached,
            cache_creation_tokens: tc.draw(generators::integers::<u64>()),
        };
        if let Some(rate) = m.cache_hit_rate() {
            assert!(
                (0.0..=1.0).contains(&rate),
                "cache hit rate must be in [0, 1], got {rate}"
            );
        }
    }

    // =========================================================================
    // Retry Reason Tests
    // =========================================================================

    #[test]
    fn retry_reason_should_retry() {
        assert!(RetryReason::RateLimit.should_retry());
        assert!(RetryReason::Timeout.should_retry());
        assert!(RetryReason::ServerError.should_retry());
        assert!(RetryReason::NetworkError.should_retry());
        assert!(!RetryReason::ContextLength.should_retry());
        assert!(!RetryReason::ContentFilter.should_retry());
        assert!(!RetryReason::Unknown.should_retry());
    }

    // =========================================================================
    // LatencySummary Monoid Laws (Hegel)
    // =========================================================================

    fn draw_retry_reason(tc: &hegel::TestCase) -> RetryReason {
        let idx = tc.draw(generators::integers::<u8>().max_value(6));
        match idx {
            0 => RetryReason::RateLimit,
            1 => RetryReason::Timeout,
            2 => RetryReason::ContextLength,
            3 => RetryReason::ServerError,
            4 => RetryReason::NetworkError,
            5 => RetryReason::ContentFilter,
            _ => RetryReason::Unknown,
        }
    }

    fn draw_retry_event(tc: &hegel::TestCase) -> RetryEvent {
        RetryEvent {
            reason: draw_retry_reason(tc),
            attempt: tc.draw(generators::integers::<u32>()),
            duration_ms: tc.draw(generators::integers::<u64>()),
        }
    }

    fn draw_tool_timing(tc: &hegel::TestCase) -> ToolTiming {
        let status = if tc.draw(generators::booleans()) {
            ToolStatus::Completed
        } else {
            ToolStatus::Error
        };
        ToolTiming {
            tool_name: tc.draw(generators::text().min_size(1).max_size(10)),
            tool_call_id: tc.draw(generators::text().min_size(1).max_size(10)),
            duration_ms: tc.draw(generators::integers::<u64>()),
            status,
            payload_size: if tc.draw(generators::booleans()) {
                Some(tc.draw(generators::integers::<u64>()))
            } else {
                None
            },
        }
    }

    fn draw_latency_summary(tc: &hegel::TestCase) -> LatencySummary {
        let n_timings = tc.draw(generators::integers::<usize>().max_value(5));
        let n_retries = tc.draw(generators::integers::<usize>().max_value(3));
        let n_counters = tc.draw(generators::integers::<usize>().max_value(3));

        let tool_timings: Vec<ToolTiming> = (0..n_timings).map(|_| draw_tool_timing(tc)).collect();
        let retry_events: Vec<RetryEvent> = (0..n_retries).map(|_| draw_retry_event(tc)).collect();
        let domain_counters: BTreeMap<String, u64> = (0..n_counters)
            .map(|i| (format!("k{i}"), tc.draw(generators::integers::<u64>())))
            .collect();

        LatencySummary {
            total_duration_ms: tc.draw(generators::integers::<u64>()),
            phases: draw_phase(tc),
            tool_timings,
            kv_metrics: draw_kv_metrics(tc),
            token_metrics: draw_token_metrics(tc),
            ttft_ms: if tc.draw(generators::booleans()) {
                Some(tc.draw(generators::integers::<u64>()))
            } else {
                None
            },
            first_text_ms: if tc.draw(generators::booleans()) {
                Some(tc.draw(generators::integers::<u64>()))
            } else {
                None
            },
            retry_count: tc.draw(generators::integers::<u32>()),
            retry_events,
            had_timeout: tc.draw(generators::booleans()),
            domain_counters,
        }
    }

    #[hegel::test]
    fn latency_summary_identity_left(tc: hegel::TestCase) {
        let a = draw_latency_summary(&tc);
        assert_eq!(LatencySummary::zero().combine(&a), a);
    }

    #[hegel::test]
    fn latency_summary_identity_right(tc: hegel::TestCase) {
        let a = draw_latency_summary(&tc);
        assert_eq!(a.combine(&LatencySummary::zero()), a);
    }

    #[hegel::test]
    fn latency_summary_associativity(tc: hegel::TestCase) {
        let a = draw_latency_summary(&tc);
        let b = draw_latency_summary(&tc);
        let c = draw_latency_summary(&tc);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    // --- TTFT/first_text min semantics ---

    #[hegel::test]
    fn ttft_takes_min_of_both(tc: hegel::TestCase) {
        let a_ms = tc.draw(generators::integers::<u64>());
        let b_ms = tc.draw(generators::integers::<u64>());
        let a = LatencySummary {
            ttft_ms: Some(a_ms),
            ..LatencySummary::zero()
        };
        let b = LatencySummary {
            ttft_ms: Some(b_ms),
            ..LatencySummary::zero()
        };
        assert_eq!(a.combine(&b).ttft_ms, Some(a_ms.min(b_ms)));
    }

    #[hegel::test]
    fn ttft_none_is_identity(tc: hegel::TestCase) {
        let ms = tc.draw(generators::integers::<u64>());
        let a = LatencySummary {
            ttft_ms: Some(ms),
            ..LatencySummary::zero()
        };
        let b = LatencySummary::zero(); // ttft_ms = None
        assert_eq!(a.combine(&b).ttft_ms, Some(ms));
        assert_eq!(b.combine(&a).ttft_ms, Some(ms));
    }

    // --- had_timeout is OR ---

    #[hegel::test]
    fn had_timeout_is_or(tc: hegel::TestCase) {
        let a_timeout = tc.draw(generators::booleans());
        let b_timeout = tc.draw(generators::booleans());
        let a = LatencySummary {
            had_timeout: a_timeout,
            ..LatencySummary::zero()
        };
        let b = LatencySummary {
            had_timeout: b_timeout,
            ..LatencySummary::zero()
        };
        assert_eq!(a.combine(&b).had_timeout, a_timeout || b_timeout);
    }

    // --- domain_counters first-write-wins ---

    #[hegel::test]
    fn domain_counters_first_write_wins(tc: hegel::TestCase) {
        let val_a = tc.draw(generators::integers::<u64>());
        let val_b = tc.draw(generators::integers::<u64>());
        let mut counters_a = BTreeMap::new();
        counters_a.insert("shared_key".to_string(), val_a);
        let mut counters_b = BTreeMap::new();
        counters_b.insert("shared_key".to_string(), val_b);

        let a = LatencySummary {
            domain_counters: counters_a,
            ..LatencySummary::zero()
        };
        let b = LatencySummary {
            domain_counters: counters_b,
            ..LatencySummary::zero()
        };

        // First-write-wins: a's value takes precedence
        let combined = a.combine(&b);
        assert_eq!(combined.domain_counters.get("shared_key"), Some(&val_a));
    }

    // --- Serde roundtrip ---

    #[hegel::test]
    fn latency_summary_serde_roundtrip(tc: hegel::TestCase) {
        let original = draw_latency_summary(&tc);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: LatencySummary = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }
}
