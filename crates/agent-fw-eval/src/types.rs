//! Core evaluation types.
//!
//! All types are pure data structures with zero IO dependencies.
//!
//! # Algebraic Structures
//!
//! | Type | Structure | Laws |
//! |------|-----------|------|
//! | `TokenUsageSummary` | Commutative Monoid | Identity, Associativity, Commutativity |
//! | `EvalStatus` | Sum type (7 variants) | Exhaustive matching |
//! | `EvalEvent` | Sum type (11 variants) | Exhaustive matching |

use crate::cost::{CostEstimate, ModelPricing};
use crate::ground_truth::GroundTruth;
use crate::scorer::ScoreWeights;
use crate::trace::{TraceRecord, TraceRef};
use agent_fw_core::{
    EvalRunId, TestCaseId, ToolCompositionOverride, ToolDispatchOverrides, ToolRegistryOverride,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use thiserror::Error;

// =============================================================================
// Enums
// =============================================================================

/// Evaluation mode label for an eval run.
///
/// The framework treats these as stable config labels carried through to
/// sample executors and harness-level scorer factories. Concrete agent-role
/// semantics live outside `agent-fw-eval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvalMode {
    /// Planner-oriented eval run.
    Planner,
    /// Executor-oriented eval run.
    Executor,
    /// Sequential or end-to-end eval run.
    Sequential,
    /// Direct specialist-agent eval run.
    Specialist,
    /// Interactive test-case authoring / baseline-construction run.
    TestCaseBuilder,
}

/// Trajectory matching mode.
///
/// Public wire names follow standard trajectory matching terminology:
/// `strict`, `unordered`, `subset`, `superset`, and `subsequence`.
/// Legacy aliases `anyOrder` and `inOrder` are accepted on input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrajectoryMode {
    /// Exact sequence equality: same tools, same order, same length.
    Strict,
    /// Exact multiset equality: same tools, order ignored.
    Unordered,
    /// Actual trajectory must be a subset of expected.
    Subset,
    /// Actual trajectory must be a superset of expected.
    Superset,
    /// Expected tools must appear in actual in order, with gaps allowed.
    Subsequence,
}

impl std::str::FromStr for TrajectoryMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "strict" | "Strict" => Ok(Self::Strict),
            "unordered" | "Unordered" | "anyOrder" | "any_order" | "Any Order" => {
                Ok(Self::Unordered)
            }
            "subset" => Ok(Self::Subset),
            "superset" => Ok(Self::Superset),
            "subsequence" | "Subsequence" => Ok(Self::Subsequence),
            // Legacy alias. The previous implementation used LCS-based partial
            // scoring; the standard ordered/extras-allowed mode is subsequence.
            "inOrder" | "in_order" | "In Order" => Ok(Self::Subsequence),
            _ => Err(format!("unknown trajectory mode: {s}")),
        }
    }
}

impl std::fmt::Display for TrajectoryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Strict => "strict",
            Self::Unordered => "unordered",
            Self::Subset => "subset",
            Self::Superset => "superset",
            Self::Subsequence => "subsequence",
        })
    }
}

impl Serialize for TrajectoryMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TrajectoryMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl TryFrom<String> for TrajectoryMode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

fn deserialize_timeout_secs<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Option<u64> = Option::deserialize(deserializer)?;
    Ok(v.filter(|&n| n > 0))
}

// =============================================================================
// Token Usage Summary (Monoid)
// =============================================================================

/// Token usage summary (Commutative Monoid: identity + associativity + commutativity).
///
/// Uses u64 to support aggregate counts across many samples.
/// `input_tokens` includes cache read/write tokens (both are subsets of input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSummary {
    input_tokens: u64,
    /// Cache-read tokens (prompt cache hits).
    output_tokens: u64,
    /// Cache-read tokens (prompt cache hits).
    cached_tokens: u64,
    /// Cache-write tokens (prompt cache population).
    #[serde(default)]
    cache_creation_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TokenUsageSummaryError {
    #[error(
        "cache tokens exceed input tokens: input={input_tokens}, cached={cached_tokens}, cache_creation={cache_creation_tokens}"
    )]
    CacheTokensExceedInput {
        input_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    },
}

impl TokenUsageSummary {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        cache_creation_tokens: 0,
    };

    /// Smart constructor that enforces the cache subset invariant.
    pub fn try_new(
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) -> Result<Self, TokenUsageSummaryError> {
        let total_cached = cached_tokens.checked_add(cache_creation_tokens).ok_or(
            TokenUsageSummaryError::CacheTokensExceedInput {
                input_tokens,
                cached_tokens,
                cache_creation_tokens,
            },
        )?;

        if total_cached > input_tokens {
            return Err(TokenUsageSummaryError::CacheTokensExceedInput {
                input_tokens,
                cached_tokens,
                cache_creation_tokens,
            });
        }
        Ok(Self {
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        })
    }

    /// Constructor for trusted internal call sites.
    pub fn new(
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cache_creation_tokens: u64,
    ) -> Self {
        Self::try_new(
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_creation_tokens,
        )
        .expect("TokenUsageSummary invariant violated")
    }

    pub fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    pub fn cached_tokens(&self) -> u64 {
        self.cached_tokens
    }

    pub fn cache_creation_tokens(&self) -> u64 {
        self.cache_creation_tokens
    }

    pub fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens - self.cached_tokens - self.cache_creation_tokens
    }

    /// Monoid combine (saturating addition).
    pub fn combine(&self, other: &Self) -> Self {
        Self::new(
            self.input_tokens.saturating_add(other.input_tokens),
            self.output_tokens.saturating_add(other.output_tokens),
            self.cached_tokens.saturating_add(other.cached_tokens),
            self.cache_creation_tokens
                .saturating_add(other.cache_creation_tokens),
        )
    }

    /// Total tokens (derived).
    ///
    /// `input_tokens` already includes cache read/write tokens (they are subsets
    /// of input).
    /// Total = input + output (NOT input + output + cached, which would double-count).
    pub fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Cache hit rate as percentage (0.0 to 100.0).
    ///
    /// Returns `None` if `input_tokens == 0` (division by zero).
    /// Mirrors `TokenMetrics::cache_hit_rate_percent()`.
    pub fn cache_hit_rate_percent(&self) -> Option<f64> {
        if self.input_tokens == 0 {
            return None;
        }
        Some(self.cached_tokens as f64 / self.input_tokens as f64 * 100.0)
    }
}

/// Lossless conversion from core token usage into eval token usage summary.
impl From<agent_fw_core::TokenUsage> for TokenUsageSummary {
    fn from(u: agent_fw_core::TokenUsage) -> Self {
        Self::new(
            u.prompt_tokens,
            u.completion_tokens,
            u.cache_read_input_tokens,
            u.cache_creation_input_tokens,
        )
    }
}

impl Default for TokenUsageSummary {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<'de> Deserialize<'de> for TokenUsageSummary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Helper {
            input_tokens: u64,
            output_tokens: u64,
            cached_tokens: u64,
            #[serde(default)]
            cache_creation_tokens: u64,
        }

        let helper = Helper::deserialize(deserializer)?;
        TokenUsageSummary::try_new(
            helper.input_tokens,
            helper.output_tokens,
            helper.cached_tokens,
            helper.cache_creation_tokens,
        )
        .map_err(serde::de::Error::custom)
    }
}

// =============================================================================
// F-Beta & Jaccard Score Types
// =============================================================================

/// F-beta score result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FBetaScore {
    pub precision: f64,
    pub recall: f64,
    pub f_score: f64,
    pub beta: f64,
}

/// Jaccard similarity result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JaccardScore {
    pub similarity: f64,
    pub intersection_size: usize,
    pub union_size: usize,
}

// =============================================================================
// ScorerResult — flat struct with opaque details
// =============================================================================

/// Result from a single scorer.
///
/// Uses a flat struct instead of a closed enum, allowing domain-specific
/// scorers to emit arbitrary results without modifying the framework.
///
/// # Invariant
///
/// `score` is always in [0.0, 1.0] (L4 Bounded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScorerResult {
    /// Scorer name (e.g., "trajectory", "final_response", "domain_action_match").
    /// Defaults to `""` for backward-compatibility with eval results persisted
    /// before this field was added.
    #[serde(default)]
    pub scorer_name: String,
    /// Numeric score in [0.0, 1.0].
    pub score: f64,
    /// Domain-specific payload for UI rendering and diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ScorerResult {
    /// Create a scorer result with just a name and score.
    pub fn new(scorer_name: impl Into<String>, score: f64) -> Self {
        Self {
            scorer_name: scorer_name.into(),
            score: score.clamp(0.0, 1.0),
            details: None,
        }
    }

    /// Create a scorer result with details.
    pub fn with_details(
        scorer_name: impl Into<String>,
        score: f64,
        details: serde_json::Value,
    ) -> Self {
        Self {
            scorer_name: scorer_name.into(),
            score: score.clamp(0.0, 1.0),
            details: Some(details),
        }
    }
}

// =============================================================================
// Progress & State Types
// =============================================================================

/// Live progress during an eval run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalProgress {
    pub completed_samples: u32,
    pub total_samples: u32,
    pub completed_test_cases: u32,
    pub total_test_cases: u32,
    pub current_test_case_id: Option<String>,
    /// Elapsed wall-clock time in milliseconds.
    pub elapsed_ms: u64,
    /// Estimated remaining time in milliseconds.
    pub estimated_remaining_ms: Option<u64>,
    /// Per-test-case state machine entries.
    pub test_case_states: Vec<TestCaseStateEntry>,
}

/// Per-test-case state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum TestCaseState {
    Queued,
    Running {
        started_at_ms: u64,
        completed_samples: u32,
        total_samples: u32,
    },
    Completed {
        duration_ms: u64,
        passed: bool,
        aggregate_score: f64,
    },
    Failed {
        duration_ms: u64,
        error: String,
    },
}

/// Entry linking a test case ID to its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseStateEntry {
    pub test_case_id: String,
    pub state: TestCaseState,
}

// =============================================================================
// EvalTestCase — lightweight for eval execution
// =============================================================================

/// Lightweight test case for eval execution.
///
/// Converted from [`AuthoredTestCase`] via `to_eval_test_case()`, which
/// strips provenance and applies any trajectory transformations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "EvalTestCaseWire", into = "EvalTestCaseWire")]
pub struct EvalTestCase {
    pub id: TestCaseId,
    pub tags: Vec<String>,
    pub input: String,
    /// Expected tool trajectory for scoring.
    ///
    /// Intentionally `Vec<String>` (allows empty): an empty trajectory means
    /// "no trajectory expectation" — vacuous truth (score = 1.0 via
    /// `f_beta_score(0,0,0,_)`). Action-specific requirements live in
    /// `GroundTruth::Structured` and are interpreted by runtime crates.
    pub expected_trajectory: Vec<String>,
    pub trajectory_mode: TrajectoryMode,
    /// Typed structured ground truth — parsed at boundaries, never re-parsed
    /// by consumers. Canonical wire payloads use `structuredGroundTruth`.
    pub ground_truth: Option<GroundTruth>,
    /// Harness-owned final-response eval spec. The generic eval crate only
    /// transports this payload; runtime-specific harnesses interpret it.
    pub final_response: Option<JsonValue>,
    /// Optional authored-thread provenance retained for runtime continuity.
    ///
    /// This remains lightweight execution metadata, not a generic replay of all
    /// authored provenance. It allows sample executors to recover any
    /// thread-bound source selection when an eval case originated from chat.
    pub source_thread_id: Option<String>,
}

impl EvalTestCase {
    /// The expected trajectory — ready for eval scoring.
    pub fn expected_trajectory(&self) -> &[String] {
        &self.expected_trajectory
    }

    /// Authored thread provenance, when this case originated from chat.
    pub fn source_thread_id(&self) -> Option<&str> {
        self.source_thread_id.as_deref()
    }
}

/// Canonical wire format for [`EvalTestCase`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvalTestCaseWire {
    id: TestCaseId,
    tags: Vec<String>,
    input: String,
    expected_trajectory: Vec<String>,
    trajectory_mode: TrajectoryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    structured_ground_truth: Option<GroundTruth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_response: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_thread_id: Option<String>,
}

impl From<EvalTestCaseWire> for EvalTestCase {
    fn from(value: EvalTestCaseWire) -> Self {
        Self {
            id: value.id,
            tags: value.tags,
            input: value.input,
            expected_trajectory: value.expected_trajectory,
            trajectory_mode: value.trajectory_mode,
            ground_truth: value.structured_ground_truth,
            final_response: value.final_response,
            source_thread_id: value.source_thread_id,
        }
    }
}

impl From<EvalTestCase> for EvalTestCaseWire {
    fn from(value: EvalTestCase) -> Self {
        Self {
            id: value.id,
            tags: value.tags,
            input: value.input,
            expected_trajectory: value.expected_trajectory,
            trajectory_mode: value.trajectory_mode,
            structured_ground_truth: value.ground_truth,
            final_response: value.final_response,
            source_thread_id: value.source_thread_id,
        }
    }
}

// =============================================================================
// Validation DTOs
// =============================================================================

/// Pre-eval validation severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationSeverity {
    Error,
    Warning,
    Info,
}

impl ValidationSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

/// Pre-eval validation issue for a test case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub message: String,
}

impl ValidationIssue {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            message: message.into(),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            message: message.into(),
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Info,
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == ValidationSeverity::Error
    }
}

/// Result of pre-eval test case validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    /// Construct from a list of issues. `valid` iff no errors.
    pub fn from_issues(issues: Vec<ValidationIssue>) -> Self {
        let valid = !issues.iter().any(|i| i.is_error());
        Self { valid, issues }
    }
}

// =============================================================================
// Test Case Set
// =============================================================================

/// A named set of test cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestCaseSet<T = EvalTestCase> {
    pub id: String,
    pub name: String,
    pub description: String,
    pub test_cases: Vec<T>,
    pub created_at: String,
}

/// Persisted thread fork metadata for an eval test case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalThreadFork {
    pub id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_at_message_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_at: String,
}

// =============================================================================
// Sample & Test Case Results
// =============================================================================

/// Result from a single sample execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleResult {
    pub sample_index: u32,
    pub passed: bool,
    pub scores: Vec<ScorerResult>,
    pub actual_trajectory: Vec<String>,
    /// Final user-facing response text produced by the sample run, when
    /// available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    pub duration_ms: u64,
    pub token_usage: TokenUsageSummary,
    pub error: Option<String>,
    /// Number of retries before this result was obtained (0 = first attempt).
    #[serde(default)]
    pub retry_count: u32,
    /// Thread ID for conversation replay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Canonical persisted trace for this sample execution.
    ///
    /// When payload-rich tool capture is unavailable, this still stores a
    /// trajectory-derived trace with explicit omission markers so callers do
    /// not need to infer whether evidence was dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceRecord>,
    /// Domain-specific metadata (latency breakdown, captured tool calls, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Structured latency breakdown from agent execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<agent_fw_core::LatencySummary>,
}

/// Aggregated result for a test case across all samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseResult {
    pub test_case_id: TestCaseId,
    /// The original user query — surfaced in the matrix view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    pub samples: Vec<SampleResult>,
    pub pass_at_k: Vec<PassAtKResult>,
    pub aggregate_score: f64,
}

/// Per-agent cost breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCostBreakdown {
    pub agent_name: String,
    pub model: String,
    pub usage: TokenUsageSummary,
    pub cost_usd: f64,
}

impl AgentCostBreakdown {
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            agent_name: self.agent_name.clone(),
            model: self.model.clone(),
            usage: self.usage.combine(&other.usage),
            cost_usd: self.cost_usd + other.cost_usd,
        }
    }
}

/// Cost summary for an eval run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCostSummary {
    #[serde(default)]
    pub total_usage: TokenUsageSummary,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub per_agent: Vec<AgentCostBreakdown>,
}

impl EvalCostSummary {
    pub fn zero() -> Self {
        Self {
            total_usage: TokenUsageSummary::ZERO,
            estimated_cost_usd: 0.0,
            per_agent: Vec::new(),
        }
    }

    pub fn from_single_agent_usage(
        agent_name: impl Into<String>,
        usage: &TokenUsageSummary,
        pricing: &ModelPricing,
    ) -> Self {
        let estimate = CostEstimate::from_usage(usage, pricing);

        Self {
            total_usage: usage.clone(),
            estimated_cost_usd: estimate.total_usd,
            per_agent: vec![AgentCostBreakdown {
                agent_name: agent_name.into(),
                model: pricing.model.clone(),
                usage: usage.clone(),
                cost_usd: estimate.total_usd,
            }],
        }
    }

    pub fn combine(&self, other: &Self) -> Self {
        use std::collections::BTreeMap;

        let mut per_agent: BTreeMap<(String, String), AgentCostBreakdown> = BTreeMap::new();
        for item in self.per_agent.iter().chain(other.per_agent.iter()) {
            let key = (item.agent_name.clone(), item.model.clone());
            per_agent
                .entry(key)
                .and_modify(|acc| *acc = acc.combine(item))
                .or_insert_with(|| item.clone());
        }

        Self {
            total_usage: self.total_usage.combine(&other.total_usage),
            estimated_cost_usd: self.estimated_cost_usd + other.estimated_cost_usd,
            per_agent: per_agent.into_values().collect(),
        }
    }
}

impl SampleResult {
    /// Monoid identity element.
    pub fn empty(sample_index: u32) -> Self {
        Self {
            sample_index,
            passed: false,
            scores: Vec::new(),
            actual_trajectory: Vec::new(),
            response_text: None,
            duration_ms: 0,
            token_usage: TokenUsageSummary::ZERO,
            error: None,
            retry_count: 0,
            thread_id: None,
            trace: None,
            metadata: None,
            latency: None,
        }
    }

    pub fn trace_ref(&self) -> Option<TraceRef> {
        self.trace.as_ref().map(TraceRecord::trace_ref)
    }
}

impl TestCaseResult {
    /// Monoid identity element for a given test case ID.
    pub fn empty(test_case_id: TestCaseId) -> Self {
        Self {
            test_case_id,
            input: None,
            samples: Vec::new(),
            pass_at_k: Vec::new(),
            aggregate_score: 0.0,
        }
    }

    /// Monoid combine: merge two results for the same test case.
    ///
    /// Concatenates samples and recomputes the aggregate score
    /// as the mean of per-sample scores (each sample weighted equally,
    /// regardless of how many scorers it has).
    ///
    /// Per-sample score: if the sample has scorer results, it is the mean
    /// of those scorer scores; otherwise it is 1.0 if passed, 0.0 if not.
    ///
    /// # Laws
    ///
    /// - **L1 (Identity)**: `combine(empty, x) == x` (modulo sample indices)
    /// - **L2 (Associativity)**: `combine(combine(a, b), c) == combine(a, combine(b, c))`
    ///   (for aggregate_score, within floating-point tolerance)
    ///
    /// # Non-monoidal fields
    ///
    /// - `pass_at_k`: Kept from `self` if non-empty, else from `other`.
    ///   Not recomputed — callers needing accurate pass@k should use
    ///   `ResultAggregator` on the merged samples.
    pub fn combine(&self, other: &Self) -> Self {
        let mut merged_samples: Vec<SampleResult> =
            Vec::with_capacity(self.samples.len() + other.samples.len());
        merged_samples.extend(self.samples.iter().cloned());
        merged_samples.extend(other.samples.iter().cloned());

        // Compute per-sample score: mean of scorer results, or pass/fail.
        // Each sample contributes exactly one value (equal weighting).
        let per_sample_scores: Vec<f64> = merged_samples
            .iter()
            .map(|s| {
                if s.scores.is_empty() {
                    if s.passed {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    let sum: f64 = s.scores.iter().map(|sr| sr.score).sum();
                    sum / s.scores.len() as f64
                }
            })
            .collect();

        let aggregate_score = if per_sample_scores.is_empty() {
            0.0
        } else {
            per_sample_scores.iter().sum::<f64>() / per_sample_scores.len() as f64
        };

        // Merge pass_at_k (keep self's if both have entries)
        let pass_at_k = if self.pass_at_k.is_empty() {
            other.pass_at_k.clone()
        } else {
            self.pass_at_k.clone()
        };

        Self {
            test_case_id: self.test_case_id.clone(),
            input: self.input.clone().or_else(|| other.input.clone()),
            samples: merged_samples,
            pass_at_k,
            aggregate_score,
        }
    }
}

/// Pass@K result for a specific k value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassAtKResult {
    pub k: u32,
    pub simple_estimate: f64,
    pub unbiased_estimate: Option<f64>,
    pub num_samples: u32,
    pub num_correct: u32,
}

// =============================================================================
// Eval Summary
// =============================================================================

/// Overall eval summary (emitted on completion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalSummary {
    pub total_test_cases: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub aggregate_score: f64,
    #[serde(default)]
    pub pass_at_k: Vec<PassAtKResult>,
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub total_usage: TokenUsageSummary,
    #[serde(default = "EvalCostSummary::zero")]
    pub cost: EvalCostSummary,
    /// Aggregated latency across all samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<agent_fw_core::LatencySummary>,
    /// Domain-specific metadata (cost breakdown, latency aggregate, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl EvalSummary {
    /// Monoid identity element.
    pub fn empty() -> Self {
        Self {
            total_test_cases: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            aggregate_score: 0.0,
            pass_at_k: Vec::new(),
            total_duration_ms: 0,
            total_usage: TokenUsageSummary::ZERO,
            cost: EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        }
    }

    /// Partial monoid combine: merge two eval summaries.
    ///
    /// The monoidal fields (total_test_cases, passed, failed, skipped,
    /// total_duration_ms, total_usage) combine associatively with `empty()`
    /// as identity. `aggregate_score` uses a weighted average by
    /// `total_test_cases`, which is associative because it tracks
    /// `(sum_of_weighted_scores, total_weight)` implicitly.
    ///
    /// # Non-monoidal fields (lossy)
    ///
    /// - `pass_at_k`: Cleared to `Vec::new()`. Pass@k does not compose
    ///   from independently-computed sub-runs; it must be recomputed from
    ///   raw sample data. Callers needing pass@k should use
    ///   `ResultAggregator` instead.
    /// - `metadata`: Cleared to `None`. Domain-specific metadata has no
    ///   generic merge strategy.
    ///
    /// # Laws (monoidal fields only)
    ///
    /// - **L1 (Identity)**: `combine(empty(), x).total_test_cases == x.total_test_cases`
    /// - **L2 (Associativity)**: `combine(combine(a, b), c).total_test_cases ==
    ///   combine(a, combine(b, c)).total_test_cases`
    /// - **L3 (Weighted average)**: `combine(a, b).aggregate_score ==
    ///   (a.score * a.count + b.score * b.count) / (a.count + b.count)`
    pub fn combine(&self, other: &Self) -> Self {
        let total = self.total_test_cases.saturating_add(other.total_test_cases);
        // Weighted average of aggregate scores — associative because
        // it's equivalent to tracking (sum_of_weighted, weight) pair.
        let aggregate_score = if total == 0 {
            0.0
        } else {
            (self.aggregate_score * self.total_test_cases as f64
                + other.aggregate_score * other.total_test_cases as f64)
                / total as f64
        };

        Self {
            total_test_cases: total,
            passed: self.passed.saturating_add(other.passed),
            failed: self.failed.saturating_add(other.failed),
            skipped: self.skipped.saturating_add(other.skipped),
            aggregate_score,
            pass_at_k: Vec::new(), // lossy — not monoidal (see doc)
            total_duration_ms: self
                .total_duration_ms
                .saturating_add(other.total_duration_ms),
            total_usage: self.total_usage.combine(&other.total_usage),
            cost: self.cost.combine(&other.cost),
            latency: match (&self.latency, &other.latency) {
                (Some(left), Some(right)) => Some(left.combine(right)),
                (Some(left), None) => Some(left.clone()),
                (None, Some(right)) => Some(right.clone()),
                (None, None) => None,
            },
            metadata: None, // lossy — not monoidal (see doc)
        }
    }
}

// =============================================================================
// Eval Status (7-variant state machine)
// =============================================================================

/// Evaluation run status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum EvalStatus {
    /// Waiting to start.
    Queued,
    /// Currently executing.
    Running { progress: EvalProgress },
    /// Paused by user (cooperative checkpoint).
    Paused { progress: EvalProgress },
    /// Finished successfully.
    Completed { summary: EvalSummary },
    /// Failed with error.
    Failed { error: String },
    /// Cancelled by user.
    Cancelled,
    /// Skipped (e.g., tag filter excluded all test cases).
    Skipped { reason: String },
}

// =============================================================================
// Aggregation Strategy
// =============================================================================

/// How to aggregate per-sample scores into a test-case aggregate score.
///
/// # Law: Bounded — both strategies produce scores in [0.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AggregationStrategy {
    /// Binary pass/fail counting: aggregate_score = num_passed / num_samples.
    PassRate,
    /// Continuous mean: aggregate_score = mean(sample_scores).
    MeanScore,
}

impl Default for AggregationStrategy {
    fn default() -> Self {
        Self::PassRate
    }
}

/// Which test cases to run — either a named set or an explicit list of IDs.
#[derive(Debug, Clone, PartialEq)]
pub enum TestCaseSource {
    Set(String),
    Individual(Vec<String>),
}

impl Default for TestCaseSource {
    fn default() -> Self {
        Self::Set(String::new())
    }
}

impl Serialize for TestCaseSource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            TestCaseSource::Set(id) => {
                map.serialize_entry("testCaseSetId", id)?;
                map.serialize_entry("testCaseIds", &None::<Vec<String>>)?;
            }
            TestCaseSource::Individual(ids) => {
                map.serialize_entry("testCaseSetId", "")?;
                map.serialize_entry("testCaseIds", &Some(ids))?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for TestCaseSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            #[serde(default)]
            test_case_set_id: String,
            test_case_ids: Option<Vec<String>>,
        }

        let raw = Raw::deserialize(deserializer)?;
        if let Some(ids) = raw.test_case_ids {
            Ok(TestCaseSource::Individual(ids))
        } else {
            Ok(TestCaseSource::Set(raw.test_case_set_id))
        }
    }
}

/// Optional request-level chat overrides applied during sample execution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalRequestOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_prompts: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "ToolDispatchOverrides::is_empty")]
    pub tool_dispatch_overrides: ToolDispatchOverrides,
    #[serde(default, skip_serializing_if = "ToolCompositionOverride::is_empty")]
    pub tool_composition_override: ToolCompositionOverride,
    #[serde(default, skip_serializing_if = "ToolRegistryOverride::is_empty")]
    pub tool_registry_override: ToolRegistryOverride,
}

// =============================================================================
// Eval Config
// =============================================================================

/// Eval configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalConfig {
    pub mode: EvalMode,
    #[serde(default)]
    pub target_agent_id: Option<String>,
    #[serde(flatten)]
    pub test_case_source: TestCaseSource,
    pub samples_per_case: u32,
    pub pass_threshold: f64,
    pub concurrency: u32,
    pub k_values: Vec<u32>,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_timeout_secs")]
    pub timeout_per_sample_secs: Option<u64>,
    pub tags_filter: Option<Vec<String>>,
    /// Retry policy for failed samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<EvalRetryConfig>,
    /// How to aggregate sample scores into test-case aggregate.
    #[serde(default)]
    pub aggregation_strategy: AggregationStrategy,
    /// Common scorer weights keyed by scorer name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_weights: Option<ScoreWeights>,
    /// Domain-specific scorer configuration (score weights, thresholds, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer_config: Option<serde_json::Value>,
    /// Optional per-request chat overrides used by the execution interpreter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_overrides: Option<EvalRequestOverrides>,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            mode: EvalMode::Sequential,
            target_agent_id: None,
            test_case_source: TestCaseSource::default(),
            samples_per_case: 3,
            pass_threshold: 0.7,
            concurrency: 2,
            k_values: vec![1, 3],
            provider: None,
            model: None,
            timeout_per_sample_secs: Some(120),
            tags_filter: None,
            retry_policy: None,
            aggregation_strategy: AggregationStrategy::default(),
            score_weights: None,
            scorer_config: None,
            request_overrides: None,
        }
    }
}

/// Retry configuration for sample execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRetryConfig {
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub backoff_multiplier: f64,
}

// =============================================================================
// Config Validation (Newtype pattern)
// =============================================================================

/// Upper bound for `samples_per_case`.
pub const MAX_SAMPLES_PER_CASE: u32 = 20;
/// Upper bound for `concurrency`.
pub const MAX_CONCURRENCY: u32 = 10;
/// Upper bound for `test_case_ids` count.
pub const MAX_TEST_CASES: usize = 500;

/// Validation errors for `EvalConfig`.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalConfigError {
    SamplesPerCaseZero,
    SamplesPerCaseTooHigh(u32),
    PassThresholdOutOfRange(f64),
    ConcurrencyZero,
    ConcurrencyTooHigh(u32),
    KValuesEmpty,
    KValueExceedsSamples { k: u32, samples_per_case: u32 },
    BackoffMultiplierTooLow(f64),
    TooManyTestCases(usize),
    EmptyTestCaseIds,
    NoTestCaseSource,
}

impl std::fmt::Display for EvalConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SamplesPerCaseZero => write!(f, "samples_per_case must be > 0"),
            Self::SamplesPerCaseTooHigh(v) => {
                write!(
                    f,
                    "samples_per_case {} exceeds max of {}",
                    v, MAX_SAMPLES_PER_CASE
                )
            }
            Self::PassThresholdOutOfRange(v) => {
                write!(f, "pass_threshold {} is not in [0, 1]", v)
            }
            Self::ConcurrencyZero => write!(f, "concurrency must be > 0"),
            Self::ConcurrencyTooHigh(v) => {
                write!(f, "concurrency {} exceeds max of {}", v, MAX_CONCURRENCY)
            }
            Self::KValuesEmpty => write!(f, "k_values must not be empty"),
            Self::KValueExceedsSamples {
                k,
                samples_per_case,
            } => {
                write!(f, "k={} exceeds samples_per_case={}", k, samples_per_case)
            }
            Self::BackoffMultiplierTooLow(v) => {
                write!(f, "backoff_multiplier {} must be >= 1.0", v)
            }
            Self::TooManyTestCases(n) => {
                write!(
                    f,
                    "test_case_ids count {} exceeds max of {}",
                    n, MAX_TEST_CASES
                )
            }
            Self::EmptyTestCaseIds => write!(f, "test_case_ids must not be empty when provided"),
            Self::NoTestCaseSource => {
                write!(
                    f,
                    "either test_case_ids or a non-empty test_case_set_id is required"
                )
            }
        }
    }
}

impl std::error::Error for EvalConfigError {}

/// Validated eval config — invariants enforced at construction.
///
/// # Invariants
/// - `samples_per_case ∈ [1, MAX_SAMPLES_PER_CASE]`
/// - `pass_threshold ∈ [0, 1]`
/// - `concurrency ∈ [1, MAX_CONCURRENCY]`
/// - `k_values` is non-empty
/// - each `k ≤ samples_per_case`
/// - `retry_policy.backoff_multiplier >= 1.0` (if provided)
/// - `test_case_ids.len() ≤ MAX_TEST_CASES` (if provided)
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedEvalConfig(EvalConfig);

impl ValidatedEvalConfig {
    /// Validate an `EvalConfig`, collecting all errors (applicative).
    pub fn validate(config: EvalConfig) -> Result<Self, Vec<EvalConfigError>> {
        let mut errors = Vec::new();

        if config.samples_per_case == 0 {
            errors.push(EvalConfigError::SamplesPerCaseZero);
        } else if config.samples_per_case > MAX_SAMPLES_PER_CASE {
            errors.push(EvalConfigError::SamplesPerCaseTooHigh(
                config.samples_per_case,
            ));
        }
        if !(0.0..=1.0).contains(&config.pass_threshold) {
            errors.push(EvalConfigError::PassThresholdOutOfRange(
                config.pass_threshold,
            ));
        }
        if config.concurrency == 0 {
            errors.push(EvalConfigError::ConcurrencyZero);
        } else if config.concurrency > MAX_CONCURRENCY {
            errors.push(EvalConfigError::ConcurrencyTooHigh(config.concurrency));
        }
        if config.k_values.is_empty() {
            errors.push(EvalConfigError::KValuesEmpty);
        }
        for &k in &config.k_values {
            if config.samples_per_case > 0 && k > config.samples_per_case {
                errors.push(EvalConfigError::KValueExceedsSamples {
                    k,
                    samples_per_case: config.samples_per_case,
                });
            }
        }
        if let Some(ref retry) = config.retry_policy {
            if retry.backoff_multiplier < 1.0 {
                errors.push(EvalConfigError::BackoffMultiplierTooLow(
                    retry.backoff_multiplier,
                ));
            }
        }
        match &config.test_case_source {
            TestCaseSource::Individual(ids) if ids.is_empty() => {
                errors.push(EvalConfigError::EmptyTestCaseIds);
            }
            TestCaseSource::Individual(ids) if ids.len() > MAX_TEST_CASES => {
                errors.push(EvalConfigError::TooManyTestCases(ids.len()));
            }
            TestCaseSource::Set(id) if id.is_empty() => {
                errors.push(EvalConfigError::NoTestCaseSource);
            }
            _ => {}
        }

        if errors.is_empty() {
            Ok(Self(config))
        } else {
            Err(errors)
        }
    }

    /// Borrow the inner config.
    pub fn inner(&self) -> &EvalConfig {
        &self.0
    }

    /// Consume and return the inner config.
    pub fn into_inner(self) -> EvalConfig {
        self.0
    }
}

// =============================================================================
// Eval Run
// =============================================================================

/// An eval run (persisted in KV store).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRun {
    pub id: EvalRunId,
    pub config: EvalConfig,
    pub status: EvalStatus,
    pub results: Vec<TestCaseResult>,
    pub created_at: String,
    pub updated_at: String,
    /// Links a re-run to its parent run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<EvalRunId>,
    /// Which test cases were re-run (subset of parent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerun_test_case_ids: Option<Vec<TestCaseId>>,
}

/// Lightweight projection of `EvalRun` for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunSummary {
    pub id: EvalRunId,
    pub config: EvalConfig,
    pub status: EvalStatus,
    pub result_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<EvalRunId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerun_test_case_ids: Option<Vec<TestCaseId>>,
}

impl EvalRun {
    /// Create a new eval run with the given config.
    pub fn new(config: EvalConfig) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: EvalRunId::new_unchecked(uuid::Uuid::new_v4().to_string()),
            config,
            status: EvalStatus::Queued,
            results: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            parent_run_id: None,
            rerun_test_case_ids: None,
        }
    }

    /// Project to a lightweight summary (strips results).
    pub fn to_summary(&self) -> EvalRunSummary {
        EvalRunSummary {
            id: self.id.clone(),
            config: self.config.clone(),
            status: self.status.clone(),
            result_count: self.results.len(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            parent_run_id: self.parent_run_id.clone(),
            rerun_test_case_ids: self.rerun_test_case_ids.clone(),
        }
    }
}

// =============================================================================
// Eval Event (SSE protocol)
// =============================================================================

/// SSE events emitted during an eval run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EvalEvent {
    /// Eval run started.
    #[serde(rename_all = "camelCase")]
    Started {
        run_id: EvalRunId,
        config: EvalConfig,
    },
    /// Progress update.
    Progress { progress: EvalProgress },
    /// A test case is about to start.
    #[serde(rename_all = "camelCase")]
    TestCaseStarted {
        test_case_id: TestCaseId,
        test_case_index: u32,
    },
    /// Sample-level progress within a test case.
    #[serde(rename_all = "camelCase")]
    SampleProgress {
        test_case_id: TestCaseId,
        sample_index: u32,
        completed_samples: u32,
        total_samples: u32,
    },
    /// A single sample completed.
    #[serde(rename_all = "camelCase")]
    SampleComplete {
        test_case_id: TestCaseId,
        sample: SampleResult,
    },
    /// A full test case completed (all samples done).
    TestCaseComplete { result: TestCaseResult },
    /// Eval run completed.
    Completed { summary: EvalSummary },
    /// Eval paused by user.
    Paused { progress: EvalProgress },
    /// Eval resumed by user.
    Resumed { progress: EvalProgress },
    /// A test case was skipped.
    #[serde(rename_all = "camelCase")]
    TestCaseSkipped {
        test_case_id: TestCaseId,
        reason: String,
    },
    /// Error occurred.
    Error { message: String },
    /// Progress event forwarded from a child eval run (rerun).
    ///
    /// Wraps an inner event from a child run so the parent SSE stream
    /// has visibility into child progress without polling.
    #[serde(rename_all = "camelCase")]
    ChildProgress {
        child_run_id: EvalRunId,
        event: Box<EvalEvent>,
    },
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TokenUsageSummary Monoid Laws
    // =========================================================================

    #[test]
    fn token_usage_zero_is_identity() {
        let usage = TokenUsageSummary::new(100, 50, 25, 0);
        assert_eq!(usage.combine(&TokenUsageSummary::ZERO), usage);
        assert_eq!(TokenUsageSummary::ZERO.combine(&usage), usage);
    }

    #[test]
    fn token_usage_rejects_cache_tokens_exceeding_input() {
        assert_eq!(
            TokenUsageSummary::try_new(10, 5, 7, 4),
            Err(TokenUsageSummaryError::CacheTokensExceedInput {
                input_tokens: 10,
                cached_tokens: 7,
                cache_creation_tokens: 4,
            })
        );
    }

    #[test]
    fn token_usage_deserialization_rejects_invalid_cache_shape() {
        let err = serde_json::from_value::<TokenUsageSummary>(serde_json::json!({
            "inputTokens": 10,
            "outputTokens": 4,
            "cachedTokens": 8,
            "cacheCreationTokens": 3
        }))
        .expect_err("invalid cache/input relationship must fail deserialization");

        assert!(err.to_string().contains("cache tokens exceed input tokens"));
    }

    /// EvalSummary identity-identity: combine(empty, empty) == empty.
    #[test]
    fn eval_summary_empty_combine_empty() {
        let result = EvalSummary::empty().combine(&EvalSummary::empty());
        assert_eq!(result.total_test_cases, 0);
        assert_eq!(result.passed, 0);
        assert_eq!(result.aggregate_score, 0.0);
    }

    #[test]
    fn eval_summary_deserializes_legacy_shape_without_usage_or_cost() {
        let summary: EvalSummary = serde_json::from_value(serde_json::json!({
            "totalTestCases": 2,
            "passed": 1,
            "failed": 1,
            "skipped": 0,
            "aggregateScore": 0.5
        }))
        .unwrap();

        assert_eq!(summary.total_test_cases, 2);
        assert_eq!(summary.total_usage, TokenUsageSummary::ZERO);
        assert_eq!(summary.cost, EvalCostSummary::zero());
        assert!(summary.pass_at_k.is_empty());
        assert_eq!(summary.total_duration_ms, 0);
    }

    #[test]
    fn eval_run_deserializes_legacy_completed_status_without_total_usage() {
        let run: EvalRun = serde_json::from_value(serde_json::json!({
            "id": "run-1",
            "config": {
                "mode": "sequential",
                "testCaseSetId": "set-1",
                "testCaseIds": null,
                "samplesPerCase": 1,
                "passThreshold": 0.7,
                "concurrency": 1,
                "kValues": [1],
                "provider": null,
                "model": null,
                "timeoutPerSampleSecs": 120,
                "tagsFilter": null,
                "retryPolicy": null,
                "aggregationStrategy": "passRate",
                "scoreWeights": null,
                "scorerConfig": null
            },
            "status": {
                "status": "completed",
                "summary": {
                    "totalTestCases": 1,
                    "passed": 1,
                    "failed": 0,
                    "skipped": 0,
                    "aggregateScore": 1.0
                }
            },
            "results": [],
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();

        match run.status {
            EvalStatus::Completed { summary } => {
                assert_eq!(summary.total_usage, TokenUsageSummary::ZERO);
                assert_eq!(summary.cost, EvalCostSummary::zero());
            }
            other => panic!("expected completed status, got {other:?}"),
        }
    }

    #[test]
    fn token_usage_associativity() {
        let a = TokenUsageSummary::new(10, 20, 5, 1);
        let b = TokenUsageSummary::new(30, 40, 10, 2);
        let c = TokenUsageSummary::new(50, 60, 15, 3);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[test]
    fn token_usage_commutativity() {
        let a = TokenUsageSummary::new(10, 20, 5, 1);
        let b = TokenUsageSummary::new(30, 40, 10, 2);
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    #[test]
    fn token_usage_total() {
        let usage = TokenUsageSummary::new(100, 50, 25, 10);
        assert_eq!(usage.total(), 150); // 100 + 50, NOT 100 + 50 + 25
        assert_eq!(usage.uncached_input_tokens(), 65);
    }

    #[test]
    fn token_usage_from_token_usage() {
        let usage = agent_fw_core::TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        let summary: TokenUsageSummary = usage.into();
        assert_eq!(summary.input_tokens(), 100);
        assert_eq!(summary.output_tokens(), 50);
        assert_eq!(summary.cached_tokens(), 0);
    }

    // =========================================================================
    // TrajectoryMode
    // =========================================================================

    #[test]
    fn trajectory_mode_from_str_roundtrip() {
        for mode in [
            TrajectoryMode::Unordered,
            TrajectoryMode::Strict,
            TrajectoryMode::Subset,
            TrajectoryMode::Superset,
            TrajectoryMode::Subsequence,
        ] {
            let s = mode.to_string();
            let parsed: TrajectoryMode = s.parse().unwrap();
            assert_eq!(mode, parsed);
        }
    }

    #[test]
    fn trajectory_mode_accepts_legacy_aliases() {
        assert_eq!(
            "anyOrder".parse::<TrajectoryMode>().unwrap(),
            TrajectoryMode::Unordered
        );
        assert_eq!(
            "inOrder".parse::<TrajectoryMode>().unwrap(),
            TrajectoryMode::Subsequence
        );
    }

    #[test]
    fn trajectory_mode_from_str_unknown() {
        assert!("bogus".parse::<TrajectoryMode>().is_err());
    }

    // =========================================================================
    // EvalMode
    // =========================================================================

    #[test]
    fn eval_mode_roundtrip() {
        for mode in [
            EvalMode::Planner,
            EvalMode::Executor,
            EvalMode::Sequential,
            EvalMode::Specialist,
            EvalMode::TestCaseBuilder,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let parsed: EvalMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, parsed);
        }
    }

    #[test]
    fn eval_mode_test_case_builder_camel_case() {
        let json = serde_json::to_string(&EvalMode::TestCaseBuilder).unwrap();
        assert_eq!(json, "\"testCaseBuilder\"");
    }

    #[test]
    fn eval_mode_specialist_snake_case() {
        let json = serde_json::to_string(&EvalMode::Specialist).unwrap();
        assert_eq!(json, "\"specialist\"");
    }

    // =========================================================================
    // EvalTestCase wire contract
    // =========================================================================

    #[test]
    fn eval_test_case_rejects_legacy_ground_truth_field() {
        let json = serde_json::json!({
            "id": "tc-1",
            "tags": [],
            "input": "test",
            "expectedTrajectory": [],
            "trajectoryMode": "anyOrder",
            "groundTruth": {"kind":"text","text":"legacy"}
        });
        let err = serde_json::from_value::<EvalTestCase>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn eval_test_case_accepts_structured_ground_truth_field() {
        let json = serde_json::json!({
            "id": "tc-2",
            "tags": [],
            "input": "test",
            "expectedTrajectory": [],
            "trajectoryMode": "anyOrder",
            "structuredGroundTruth": {"kind":"text","text":"expected output"}
        });
        let tc: EvalTestCase = serde_json::from_value(json).unwrap();
        assert!(tc.ground_truth.is_some());
    }

    #[test]
    fn eval_test_case_accepts_source_thread_id_field() {
        let json = serde_json::json!({
            "id": "tc-3",
            "tags": [],
            "input": "test",
            "expectedTrajectory": [],
            "trajectoryMode": "anyOrder",
            "sourceThreadId": "thread-alpha"
        });
        let tc: EvalTestCase = serde_json::from_value(json).unwrap();
        assert_eq!(tc.source_thread_id(), Some("thread-alpha"));
    }

    #[test]
    fn validation_result_invalid_when_any_error_present() {
        let result = ValidationResult::from_issues(vec![
            ValidationIssue::info("heads up"),
            ValidationIssue::error("broken"),
        ]);
        assert!(!result.valid);
    }

    #[test]
    fn test_case_set_rejects_unknown_fields() {
        let json = serde_json::json!({
            "id": "set-1",
            "name": "Set 1",
            "description": "desc",
            "testCases": [],
            "createdAt": "2026-03-09T00:00:00Z",
            "extra": true
        });
        let err = serde_json::from_value::<TestCaseSet>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    // =========================================================================
    // ScorerResult
    // =========================================================================

    #[test]
    fn scorer_result_clamps_score() {
        let r = ScorerResult::new("test", 1.5);
        assert_eq!(r.score, 1.0);

        let r = ScorerResult::new("test", -0.5);
        assert_eq!(r.score, 0.0);
    }

    #[test]
    fn scorer_result_with_details_roundtrip() {
        let r = ScorerResult::with_details(
            "trajectory",
            0.85,
            serde_json::json!({"precision": 0.9, "recall": 0.8}),
        );
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ScorerResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, parsed);
    }

    // =========================================================================
    // EvalStatus serialization
    // =========================================================================

    #[test]
    fn eval_status_queued_roundtrip() {
        let status = EvalStatus::Queued;
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"status\":\"queued\""));
        let parsed: EvalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn eval_status_failed_roundtrip() {
        let status = EvalStatus::Failed {
            error: "timeout".into(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: EvalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    // =========================================================================
    // EvalConfig validation
    // =========================================================================

    #[test]
    fn validated_config_default_is_invalid() {
        // Default has an empty set source → NoTestCaseSource
        let result = ValidatedEvalConfig::validate(EvalConfig::default());
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.contains(&EvalConfigError::NoTestCaseSource));
    }

    #[test]
    fn validated_config_valid() {
        let mut config = EvalConfig::default();
        config.test_case_source = TestCaseSource::Set("set-1".into());
        let result = ValidatedEvalConfig::validate(config);
        assert!(result.is_ok());
    }

    #[test]
    fn validated_config_collects_all_errors() {
        let config = EvalConfig {
            samples_per_case: 0,
            pass_threshold: 2.0,
            concurrency: 0,
            k_values: vec![],
            ..Default::default()
        };
        let errors = ValidatedEvalConfig::validate(config).unwrap_err();
        assert!(errors.len() >= 4);
    }

    #[test]
    fn validated_config_k_exceeds_samples() {
        let config = EvalConfig {
            samples_per_case: 3,
            k_values: vec![1, 5],
            test_case_source: TestCaseSource::Set("set-1".into()),
            ..Default::default()
        };
        let errors = ValidatedEvalConfig::validate(config).unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            EvalConfigError::KValueExceedsSamples {
                k: 5,
                samples_per_case: 3
            }
        )));
    }

    #[test]
    fn validated_config_backoff_too_low() {
        let config = EvalConfig {
            retry_policy: Some(EvalRetryConfig {
                max_retries: 3,
                initial_backoff_ms: 100,
                backoff_multiplier: 0.5,
            }),
            test_case_source: TestCaseSource::Set("set-1".into()),
            ..Default::default()
        };
        let errors = ValidatedEvalConfig::validate(config).unwrap_err();
        assert!(errors.contains(&EvalConfigError::BackoffMultiplierTooLow(0.5)));
    }

    // =========================================================================
    // EvalEvent serialization
    // =========================================================================

    #[test]
    fn eval_event_error_roundtrip() {
        let event = EvalEvent::Error {
            message: "test error".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"error\""));
        let parsed: EvalEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn eval_event_started_camel_case() {
        let event = EvalEvent::Started {
            run_id: EvalRunId::new_unchecked("run-1"),
            config: EvalConfig {
                test_case_source: TestCaseSource::Set("set-1".into()),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"started\""));
        assert!(json.contains("\"runId\""));
    }

    // =========================================================================
    // EvalRun
    // =========================================================================

    #[test]
    fn eval_run_to_summary_strips_results() {
        let run = EvalRun {
            id: EvalRunId::new_unchecked("run-1"),
            config: EvalConfig {
                test_case_source: TestCaseSource::Set("set-1".into()),
                ..Default::default()
            },
            status: EvalStatus::Queued,
            results: vec![TestCaseResult {
                test_case_id: TestCaseId::new_unchecked("tc-1"),
                input: None,
                samples: vec![],
                pass_at_k: vec![],
                aggregate_score: 0.0,
            }],
            created_at: "2025-01-01".into(),
            updated_at: "2025-01-01".into(),
            parent_run_id: None,
            rerun_test_case_ids: None,
        };
        let summary = run.to_summary();
        assert_eq!(summary.result_count, 1);
    }

    // =========================================================================
    // Hegel property-based tests
    // =========================================================================

    use hegel::generators;

    fn draw_token_usage_quad(tc: &hegel::TestCase, max: u64) -> (u64, u64, u64, u64) {
        let input = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let output = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cached_raw = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cache_creation_raw = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cached = cached_raw.min(input);
        let cache_creation = cache_creation_raw.min(input.saturating_sub(cached));
        (input, output, cached, cache_creation)
    }

    #[hegel::test]
    fn token_usage_summary_monoid_identity(tc: hegel::TestCase) {
        let (input, output, cached, cache_creation) = draw_token_usage_quad(&tc, 1_000_000);
        let a = TokenUsageSummary::new(input, output, cached, cache_creation);
        assert_eq!(a.combine(&TokenUsageSummary::ZERO), a.clone());
        assert_eq!(TokenUsageSummary::ZERO.combine(&a), a);
    }

    #[hegel::test]
    fn token_usage_summary_monoid_associativity(tc: hegel::TestCase) {
        let (ai, ao, ac, acw) = draw_token_usage_quad(&tc, 100_000);
        let (bi, bo, bc, bcw) = draw_token_usage_quad(&tc, 100_000);
        let (ci, co, cc, ccw) = draw_token_usage_quad(&tc, 100_000);
        let a = TokenUsageSummary::new(ai, ao, ac, acw);
        let b = TokenUsageSummary::new(bi, bo, bc, bcw);
        let c = TokenUsageSummary::new(ci, co, cc, ccw);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    // =====================================================================
    // TestCaseResult monoid laws
    // =====================================================================

    /// L1 (Identity): combine(empty, x) recomputes aggregate from x's samples.
    #[hegel::test]
    fn test_case_result_identity(tc: hegel::TestCase) {
        let s0_passed = tc.draw(generators::booleans());
        let s0_score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let s1_passed = tc.draw(generators::booleans());
        let s1_score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let id = TestCaseId::new_unchecked("tc-prop");
        let x = TestCaseResult {
            test_case_id: id.clone(),
            input: None,
            samples: vec![
                SampleResult {
                    sample_index: 0,
                    passed: s0_passed,
                    scores: vec![ScorerResult::new("s", s0_score)],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 0,
                    token_usage: TokenUsageSummary::ZERO,
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: None,
                },
                SampleResult {
                    sample_index: 1,
                    passed: s1_passed,
                    scores: vec![ScorerResult::new("s", s1_score)],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 0,
                    token_usage: TokenUsageSummary::ZERO,
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: None,
                },
            ],
            pass_at_k: vec![],
            aggregate_score: 0.0, // doesn't matter — combine recomputes
        };
        let empty = TestCaseResult::empty(id);
        let result = empty.combine(&x);
        assert_eq!(result.samples.len(), x.samples.len());
        // Recomputed aggregate should match independent computation
        let expected = (s0_score.clamp(0.0, 1.0) + s1_score.clamp(0.0, 1.0)) / 2.0;
        assert!(
            (result.aggregate_score - expected).abs() < 1e-10,
            "identity: {} vs expected {}",
            result.aggregate_score,
            expected
        );
    }

    /// L2 (Associativity): combine(combine(a,b),c) == combine(a,combine(b,c)).
    ///
    /// Vec concatenation is associative and the fold order is identical,
    /// so aggregate_score should be bit-identical (not just epsilon-close).
    #[hegel::test]
    fn test_case_result_associativity(tc: hegel::TestCase) {
        let sa_score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let sb_score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let sc_score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let sa_passed = tc.draw(generators::booleans());
        let sb_passed = tc.draw(generators::booleans());
        let sc_passed = tc.draw(generators::booleans());
        let id = TestCaseId::new_unchecked("tc-assoc");
        let mk = |idx: u32, passed: bool, score: f64| -> TestCaseResult {
            TestCaseResult {
                test_case_id: id.clone(),
                input: None,
                samples: vec![SampleResult {
                    sample_index: idx,
                    passed,
                    scores: vec![ScorerResult::new("s", score)],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 0,
                    token_usage: TokenUsageSummary::ZERO,
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: None,
                }],
                pass_at_k: vec![],
                aggregate_score: 0.0,
            }
        };
        let a = mk(0, sa_passed, sa_score);
        let b = mk(1, sb_passed, sb_score);
        let c = mk(2, sc_passed, sc_score);
        let ab_c = a.combine(&b).combine(&c);
        let a_bc = a.combine(&b.combine(&c));
        assert_eq!(ab_c.samples.len(), a_bc.samples.len());
        assert_eq!(ab_c.samples.len(), 3);
        // Bit-identical: same values, same fold order
        assert!(
            (ab_c.aggregate_score - a_bc.aggregate_score).abs() == 0.0,
            "associativity violated: {} vs {}",
            ab_c.aggregate_score,
            a_bc.aggregate_score
        );
    }

    /// TestCaseResult combine with scorerless samples (pass/fail binary).
    #[hegel::test]
    fn test_case_result_scorerless_associativity(tc: hegel::TestCase) {
        let sa = tc.draw(generators::booleans());
        let sb = tc.draw(generators::booleans());
        let sc = tc.draw(generators::booleans());
        let id = TestCaseId::new_unchecked("tc-noscorer");
        let mk = |idx: u32, passed: bool| -> TestCaseResult {
            TestCaseResult {
                test_case_id: id.clone(),
                input: None,
                samples: vec![SampleResult {
                    sample_index: idx,
                    passed,
                    scores: vec![], // no scorers — uses pass/fail binary
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 0,
                    token_usage: TokenUsageSummary::ZERO,
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: None,
                }],
                pass_at_k: vec![],
                aggregate_score: 0.0,
            }
        };
        let a = mk(0, sa);
        let b = mk(1, sb);
        let c = mk(2, sc);
        let ab_c = a.combine(&b).combine(&c);
        let a_bc = a.combine(&b.combine(&c));
        assert!(
            (ab_c.aggregate_score - a_bc.aggregate_score).abs() == 0.0,
            "scorerless associativity violated: {} vs {}",
            ab_c.aggregate_score,
            a_bc.aggregate_score
        );
        // Verify the score is correct: mean of pass/fail binary
        let expected =
            (if sa { 1.0 } else { 0.0 } + if sb { 1.0 } else { 0.0 } + if sc { 1.0 } else { 0.0 })
                / 3.0;
        assert!(
            (ab_c.aggregate_score - expected).abs() < 1e-10,
            "expected {} got {}",
            expected,
            ab_c.aggregate_score
        );
    }

    // =====================================================================
    // EvalSummary partial monoid laws (monoidal fields only)
    // =====================================================================

    /// L1 (Identity): combine(empty(), x) preserves monoidal fields.
    ///
    /// Uses total_test_cases >= 1 because aggregate_score is only
    /// meaningful when total > 0. For total == 0, the weighted average
    /// formula correctly returns 0.0 (no data → no score).
    #[hegel::test]
    fn eval_summary_identity(tc: hegel::TestCase) {
        let total = tc.draw(generators::integers::<u32>().min_value(1).max_value(99));
        let passed = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let failed = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let skipped = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let score = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let dur = tc.draw(
            generators::integers::<u64>()
                .min_value(0)
                .max_value(100_000),
        );
        let (ti, to_, tc_, tcw) = draw_token_usage_quad(&tc, 100_000);
        let x = EvalSummary {
            total_test_cases: total,
            passed,
            failed,
            skipped,
            aggregate_score: score,
            pass_at_k: Vec::new(),
            total_duration_ms: dur,
            total_usage: TokenUsageSummary::new(ti, to_, tc_, tcw),
            cost: EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        };
        let left = EvalSummary::empty().combine(&x);
        assert_eq!(left.total_test_cases, x.total_test_cases);
        assert_eq!(left.passed, x.passed);
        assert_eq!(left.failed, x.failed);
        assert_eq!(left.skipped, x.skipped);
        assert_eq!(left.total_duration_ms, x.total_duration_ms);
        assert_eq!(left.total_usage, x.total_usage);
        // aggregate_score: empty has weight 0, so weighted avg = x.score
        assert!((left.aggregate_score - x.aggregate_score).abs() < 1e-10);
    }

    /// L2 (Associativity): monoidal fields are associative.
    #[hegel::test]
    fn eval_summary_associativity(tc: hegel::TestCase) {
        let at = tc.draw(generators::integers::<u32>().min_value(1).max_value(49));
        let ap = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let af = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let ask = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let ascore = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let adur = tc.draw(generators::integers::<u64>().min_value(0).max_value(50_000));
        let bt = tc.draw(generators::integers::<u32>().min_value(1).max_value(49));
        let bp = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let bf = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let bsk = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let bscore = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let bdur = tc.draw(generators::integers::<u64>().min_value(0).max_value(50_000));
        let ct = tc.draw(generators::integers::<u32>().min_value(1).max_value(49));
        let cp = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let cf = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let csk = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let cscore = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let cdur = tc.draw(generators::integers::<u64>().min_value(0).max_value(50_000));
        let a = EvalSummary {
            total_test_cases: at,
            passed: ap,
            failed: af,
            skipped: ask,
            aggregate_score: ascore,
            pass_at_k: Vec::new(),
            total_duration_ms: adur,
            total_usage: TokenUsageSummary::ZERO,
            cost: EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        };
        let b = EvalSummary {
            total_test_cases: bt,
            passed: bp,
            failed: bf,
            skipped: bsk,
            aggregate_score: bscore,
            pass_at_k: Vec::new(),
            total_duration_ms: bdur,
            total_usage: TokenUsageSummary::ZERO,
            cost: EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        };
        let c = EvalSummary {
            total_test_cases: ct,
            passed: cp,
            failed: cf,
            skipped: csk,
            aggregate_score: cscore,
            pass_at_k: Vec::new(),
            total_duration_ms: cdur,
            total_usage: TokenUsageSummary::ZERO,
            cost: EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        };
        let ab_c = a.combine(&b).combine(&c);
        let a_bc = a.combine(&b.combine(&c));
        assert_eq!(ab_c.total_test_cases, a_bc.total_test_cases);
        assert_eq!(ab_c.passed, a_bc.passed);
        assert_eq!(ab_c.failed, a_bc.failed);
        assert_eq!(ab_c.skipped, a_bc.skipped);
        assert_eq!(ab_c.total_duration_ms, a_bc.total_duration_ms);
        // Weighted average associativity: allow f64 tolerance
        assert!(
            (ab_c.aggregate_score - a_bc.aggregate_score).abs() < 1e-10,
            "aggregate_score not associative: {} vs {}",
            ab_c.aggregate_score,
            a_bc.aggregate_score
        );
    }
}
