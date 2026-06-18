//! EvalScorer algebra + shipped interpreters.
//!
//! # Laws
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L2 | Determinism | `score(tc, out) == score(tc, out)` |
//! | L3 | Non-empty | `score(...).len() >= 1` |
//! | L4 | Bounded | `∀r ∈ score(...): 0.0 ≤ r.score ≤ 1.0` |
//! | L5 | Composition | `CompositeScorer.score(...)` = concat of children results |
//!
//! L1 (Purity) is a structural property — no IO in `score()`.
//!
//! # Shipped Interpreters
//!
//! - [`TrajectoryScorer`] — binary trajectory predicates with diagnostics
//! - [`CompositeScorer`] — Weighted composition of child scorers

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::scoring::{score_trajectory, trajectory_score_details};
use crate::types::{EvalTestCase, ScorerResult};
use agent_fw_core::NonEmpty;

// =============================================================================
// ScoredSample — referential transparency for scoring output
// =============================================================================

/// Scoring output with an explicit aggregate.
///
/// `aggregate` is THE score — the single f64 that determines pass/fail.
/// `component_scores` are individual scorer results for diagnostics/UI.
/// Consumers read `aggregate`, never re-derive it from component_scores.
///
/// # Law — L6 Aggregate Consistency
///
/// ```text
/// For leaf scorers:  scored.aggregate == scored.component_scores[0].score
/// For composite:     scored.aggregate == weighted_avg(child.aggregate for child in children)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSample {
    /// THE score — determines pass/fail. Always in [0.0, 1.0].
    pub aggregate: f64,
    /// Individual scorer results for diagnostics/UI.
    /// `NonEmpty` enforces L3 (non-empty) at the type level — the runtime
    /// assertion at line 821 is replaced by a compile-time guarantee.
    pub component_scores: NonEmpty<ScorerResult>,
}

impl ScoredSample {
    /// Leaf scorer: one result, aggregate = that result's score.
    pub fn leaf(name: &str, score: f64) -> Self {
        let result = ScorerResult::new(name, score);
        Self {
            aggregate: result.score, // Use clamped value
            component_scores: NonEmpty::singleton(result),
        }
    }

    /// Leaf scorer with detail metadata.
    pub fn leaf_with_details(name: &str, score: f64, details: serde_json::Value) -> Self {
        let result = ScorerResult::with_details(name, score, details);
        Self {
            aggregate: result.score, // Use clamped value
            component_scores: NonEmpty::singleton(result),
        }
    }
}

/// Weighted average of f64 values with corresponding weights.
///
/// Precondition: `values.len() == weights.len()` and `sum(weights) > 0`.
fn weighted_average(values: &[f64], weights: &[f64]) -> f64 {
    let sum_weights: f64 = weights.iter().sum();
    if sum_weights == 0.0 {
        return 0.0;
    }
    let weighted_sum: f64 = values.iter().zip(weights.iter()).map(|(v, w)| v * w).sum();
    weighted_sum / sum_weights
}

// =============================================================================
// ScoreWeights — validated newtype for scorer weight configuration
// =============================================================================

/// Validated scorer weight configuration.
///
/// Smart constructor ensures:
/// - At least one weight (via `NonEmpty`)
/// - All weights are non-negative
/// - At least one weight is positive (not all zero)
///
/// # Law — Normalization invariant
///
/// ```text
/// ScoreWeights::new(ws).unwrap().normalized().iter().sum() == 1.0 (±ε)
/// ```
///
/// # Example
///
/// ```
/// # use agent_fw_eval::scorer::ScoreWeights;
/// let ws = ScoreWeights::new(vec![
///     ("trajectory".into(), 0.4),
///     ("final_response".into(), 0.6),
/// ]).unwrap();
/// assert_eq!(ws.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreWeights {
    entries: NonEmpty<(String, f64)>,
}

impl ScoreWeights {
    /// Construct validated score weights.
    ///
    /// Rejects:
    /// - Empty weights
    /// - Non-finite weights (NaN, ±Infinity)
    /// - Negative weights
    /// - All-zero weights (no signal)
    pub fn new(entries: Vec<(String, f64)>) -> Result<Self, ScoreWeightsError> {
        let entries = NonEmpty::from_vec(entries).ok_or(ScoreWeightsError::Empty)?;
        for (name, w) in entries.iter() {
            if w.is_nan() || w.is_infinite() {
                return Err(ScoreWeightsError::NonFinite {
                    name: name.clone(),
                    weight: *w,
                });
            }
            if *w < 0.0 {
                return Err(ScoreWeightsError::Negative {
                    name: name.clone(),
                    weight: *w,
                });
            }
        }
        let sum: f64 = entries.iter().map(|(_, w)| w).sum();
        if sum == 0.0 {
            return Err(ScoreWeightsError::AllZero);
        }
        Ok(Self { entries })
    }

    /// Equal weights for all scorers.
    pub fn equal(names: Vec<String>) -> Result<Self, ScoreWeightsError> {
        let ne_names = NonEmpty::from_vec(names).ok_or(ScoreWeightsError::Empty)?;
        let w = 1.0 / ne_names.len().get() as f64;
        let entries = ne_names.map(|n| (n, w));
        Ok(Self { entries })
    }

    /// Number of scorers. Always >= 1.
    pub fn len(&self) -> usize {
        self.entries.len().get()
    }

    /// Whether the weight set is empty (always false — NonEmpty guarantee).
    #[deprecated(note = "ScoreWeights is always non-empty by construction (NonEmpty)")]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Iterate over raw (unnormalized) weight entries.
    pub fn iter(&self) -> impl Iterator<Item = &(String, f64)> {
        self.entries.iter()
    }

    /// Normalized weights (sum to 1.0).
    pub fn normalized(&self) -> Vec<(String, f64)> {
        let sum: f64 = self.entries.iter().map(|(_, w)| w).sum();
        self.entries
            .iter()
            .map(|(name, w)| (name.clone(), w / sum))
            .collect()
    }

    /// Extract just the weight values (in order), for use with `aggregate_scorer_results`.
    pub fn values(&self) -> Vec<f64> {
        self.entries.iter().map(|(_, w)| *w).collect()
    }

    /// Build a `CompositeScorer` from these weights and the given scorers.
    ///
    /// Returns `Err(CompositeError::LengthMismatch)` if scorer count != weight count.
    /// Weights are already validated by `ScoreWeights::new()`, so
    /// `WeightedChild::new()` cannot fail here — the `expect` is a defense-in-depth assertion.
    /// Both `scorers` and `self.entries` are `NonEmpty`, so the result is structurally non-empty.
    pub fn into_composite(
        self,
        scorers: NonEmpty<std::sync::Arc<dyn EvalScorer>>,
    ) -> Result<CompositeScorer, CompositeError> {
        if scorers.len().get() != self.entries.len().get() {
            return Err(CompositeError::LengthMismatch {
                scorers: scorers.len().get(),
                weights: self.entries.len().get(),
            });
        }
        let children_vec: Vec<WeightedChild> = scorers
            .into_iter()
            .zip(self.entries.iter())
            .map(|(scorer, (_, weight))| {
                // Safe: ScoreWeights rejects negative/NaN/infinite weights
                WeightedChild::new(scorer, *weight).expect("ScoreWeights guarantees valid weights")
            })
            .collect();
        // Safety: scorers is NonEmpty and all entries resolved → children_vec is non-empty
        let children = NonEmpty::from_vec(children_vec).expect("NonEmpty input → non-empty output");
        CompositeScorer::new(children)
    }
}

/// Errors from `ScoreWeights` construction.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreWeightsError {
    /// No weights provided.
    Empty,
    /// A weight is non-finite (NaN or ±Infinity).
    NonFinite { name: String, weight: f64 },
    /// A weight is negative.
    Negative { name: String, weight: f64 },
    /// All weights are zero (no signal).
    AllZero,
    /// Unknown scorer name in weight configuration.
    UnknownScorer(String),
}

impl std::fmt::Display for ScoreWeightsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "ScoreWeights: at least one weight required"),
            Self::NonFinite { name, weight } => {
                write!(f, "ScoreWeights: non-finite weight for '{name}': {weight}")
            }
            Self::Negative { name, weight } => {
                write!(f, "ScoreWeights: negative weight for '{name}': {weight}")
            }
            Self::AllZero => write!(f, "ScoreWeights: all weights are zero (no signal)"),
            Self::UnknownScorer(name) => {
                write!(f, "ScoreWeights: unknown scorer name '{name}'")
            }
        }
    }
}

impl std::error::Error for ScoreWeightsError {}

impl Serialize for ScoreWeights {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(self.entries.len().get()))?;
        for (name, weight) in self.entries.iter() {
            map.serialize_entry(name, weight)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ScoreWeights {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = std::collections::BTreeMap::<String, f64>::deserialize(deserializer)?;
        ScoreWeights::new(entries.into_iter().collect()).map_err(serde::de::Error::custom)
    }
}

// =============================================================================
// RawSampleOutput
// =============================================================================

/// Raw output from a single sample execution.
///
/// This is the scorer's input: what the agent actually produced.
/// Domain-specific fields (e.g., captured tool calls) can be stored
/// in `extra` as opaque JSON.
#[derive(Debug, Clone)]
pub struct RawSampleOutput {
    /// The actual tool trajectory observed.
    pub actual_trajectory: Vec<String>,
    /// Final user-facing response text, when the executor captured one.
    pub response_text: Option<String>,
    /// Domain-specific extra data (captured tool calls, raw responses, etc.).
    pub extra: Option<serde_json::Value>,
}

impl RawSampleOutput {
    /// Create a sample output with just a trajectory.
    pub fn new(actual_trajectory: Vec<String>) -> Self {
        Self {
            actual_trajectory,
            response_text: None,
            extra: None,
        }
    }

    /// Create a sample output with trajectory and extra data.
    pub fn with_extra(actual_trajectory: Vec<String>, extra: serde_json::Value) -> Self {
        Self {
            actual_trajectory,
            response_text: None,
            extra: Some(extra),
        }
    }

    /// Attach final response text to the raw scorer input.
    pub fn with_response_text(mut self, response_text: impl Into<String>) -> Self {
        self.response_text = Some(response_text.into());
        self
    }
}

// =============================================================================
// EvalScorer trait
// =============================================================================

/// Eval scorer algebra (tagless final pattern).
///
/// Implementations score a single sample against expected behavior.
/// Each call produces one or more [`ScorerResult`] values.
///
/// # Laws
///
/// **L1 Purity**: `score` has no side effects (no IO, no mutation).
///
/// **L2 Determinism**: Same inputs → same outputs.
/// ```text
/// score(tc, out) == score(tc, out)
/// ```
///
/// **L3 Non-empty**: Always produces at least one result.
/// ```text
/// score(...).len() >= 1
/// ```
///
/// **L4 Bounded**: All scores in [0.0, 1.0].
/// ```text
/// ∀r ∈ score(...): 0.0 ≤ r.score ≤ 1.0
/// ```
///
/// **L5 Composition**: `CompositeScorer` results = concat of children.
pub trait EvalScorer: Send + Sync {
    /// Score a single sample.
    ///
    /// Returns [`ScoredSample`] with an explicit aggregate score and
    /// component scores for diagnostics. Consumers use `aggregate` for
    /// pass/fail decisions, never re-derive from component_scores.
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample;

    /// Human-readable scorer name (for diagnostics).
    fn name(&self) -> &str;
}

// =============================================================================
// TrajectoryScorer
// =============================================================================

/// Binary trajectory scorer.
///
/// Scores the actual tool trajectory against the expected trajectory
/// using the test case's configured trajectory mode. Similarity details are
/// emitted for debugging, but do not affect the binary score.
///
/// # Vacuous truth
///
/// When both expected and actual trajectories are empty,
/// the score is 1.0 (vacuous truth via `f_beta_score(0,0,0,_)`).
#[derive(Debug, Clone)]
pub struct TrajectoryScorer {
    /// F-beta parameter (default: 2.0, recall-weighted).
    pub beta: f64,
}

impl Default for TrajectoryScorer {
    fn default() -> Self {
        Self { beta: 2.0 }
    }
}

impl TrajectoryScorer {
    /// Create a trajectory scorer with the given beta parameter.
    pub fn with_beta(beta: f64) -> Self {
        Self { beta }
    }
}

impl EvalScorer for TrajectoryScorer {
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
        let f_beta = score_trajectory(
            &test_case.expected_trajectory,
            &output.actual_trajectory,
            test_case.trajectory_mode,
            self.beta,
        );

        ScoredSample::leaf_with_details(
            "trajectory",
            f_beta.f_score,
            serde_json::to_value(trajectory_score_details(
                &test_case.expected_trajectory,
                &output.actual_trajectory,
                test_case.trajectory_mode,
                f_beta.f_score == 1.0,
            ))
            .expect("trajectory score details serialize"),
        )
    }

    fn name(&self) -> &str {
        "trajectory"
    }
}

// =============================================================================
// CompositeScorer
// =============================================================================

/// A child scorer with an associated weight.
///
/// Smart constructor validates weight invariants (non-negative, finite).
#[derive(Clone)]
pub struct WeightedChild {
    pub scorer: std::sync::Arc<dyn EvalScorer>,
    pub weight: f64,
}

/// Errors from `WeightedChild` construction.
#[derive(Debug, thiserror::Error)]
pub enum WeightedChildError {
    #[error("weight must be non-negative, got {0}")]
    Negative(f64),
    #[error("weight must be finite, got {0}")]
    NonFinite(f64),
}

impl WeightedChild {
    /// Create a weighted child scorer.
    ///
    /// Rejects negative, NaN, and infinite weights to uphold scorer law L4 (bounded).
    pub fn new(
        scorer: std::sync::Arc<dyn EvalScorer>,
        weight: f64,
    ) -> Result<Self, WeightedChildError> {
        if weight < 0.0 {
            return Err(WeightedChildError::Negative(weight));
        }
        if weight.is_nan() || weight.is_infinite() {
            return Err(WeightedChildError::NonFinite(weight));
        }
        Ok(Self { scorer, weight })
    }
}

impl std::fmt::Debug for WeightedChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeightedChild")
            .field("scorer", &self.scorer.name())
            .field("weight", &self.weight)
            .finish()
    }
}

/// Errors from `CompositeScorer` construction.
#[derive(Debug, thiserror::Error)]
pub enum CompositeError {
    #[error("CompositeScorer requires at least one child")]
    Empty,
    #[error("all weights are zero (no signal)")]
    AllZeroWeights,
    #[error("invalid child weight: {0}")]
    InvalidChild(#[from] WeightedChildError),
    #[error("scorer count ({scorers}) != weight count ({weights})")]
    LengthMismatch { scorers: usize, weights: usize },
}

/// Composite scorer: orchestrates multiple child scorers.
///
/// Collects all child results and appends a synthetic "composite" result
/// with the weighted average score.
///
/// # Structural invariant
///
/// `children: NonEmpty<WeightedChild>` guarantees at least one child
/// by construction — emptiness is type-prevented, not runtime-checked.
///
/// # Law L5 (Composition)
///
/// `composite.score(...)` = concat of all children results + composite aggregate.
#[derive(Debug, Clone)]
pub struct CompositeScorer {
    children: NonEmpty<WeightedChild>,
}

impl CompositeScorer {
    /// Create a composite scorer from weighted children.
    ///
    /// `NonEmpty` guarantees at least one child. Only rejects all-zero weights.
    pub fn new(children: NonEmpty<WeightedChild>) -> Result<Self, CompositeError> {
        let sum: f64 = children.iter().map(|c| c.weight).sum();
        if sum == 0.0 {
            return Err(CompositeError::AllZeroWeights);
        }
        Ok(Self { children })
    }

    /// Convenience: trajectory scorer only (weight 1.0).
    pub fn trajectory_only() -> Self {
        // Weight 1.0 is hardcoded valid — unwrap is safe.
        Self::new(NonEmpty::singleton(
            WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), 1.0)
                .expect("weight 1.0 is valid"),
        ))
        .expect("single child with weight 1.0 is valid")
    }
}

impl EvalScorer for CompositeScorer {
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
        let mut child_aggregates = Vec::new();
        let mut child_weights = Vec::new();

        // Collect the first child's scores as the NonEmpty seed
        let first_child = self.children.first();
        let first_output = first_child.scorer.score(test_case, output);
        child_aggregates.push(first_output.aggregate);
        child_weights.push(first_child.weight);
        let mut component_scores = first_output.component_scores;

        // Extend with remaining children
        for child in self.children.iter().skip(1) {
            let child_output = child.scorer.score(test_case, output);
            child_aggregates.push(child_output.aggregate);
            child_weights.push(child.weight);
            component_scores.extend(child_output.component_scores);
        }

        let aggregate = weighted_average(&child_aggregates, &child_weights);
        component_scores.push(ScorerResult::new("composite", aggregate));

        ScoredSample {
            aggregate,
            component_scores,
        }
    }

    fn name(&self) -> &str {
        "composite"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TrajectoryMode;
    use agent_fw_core::TestCaseId;

    fn make_test_case(expected: Vec<&str>, mode: TrajectoryMode) -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked("tc-test"),
            tags: vec![],
            input: "test input".into(),
            expected_trajectory: expected.into_iter().map(String::from).collect(),
            trajectory_mode: mode,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        }
    }

    // =========================================================================
    // TrajectoryScorer
    // =========================================================================

    #[test]
    fn trajectory_scorer_perfect_match() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(
            vec!["draft_plan", "approve_plan"],
            TrajectoryMode::Unordered,
        );
        let output = RawSampleOutput::new(vec!["approve_plan".into(), "draft_plan".into()]);

        let scored = scorer.score(&tc, &output);
        assert_eq!(scored.component_scores.len().get(), 1);
        assert_eq!(scored.component_scores[0].scorer_name, "trajectory");
        assert_eq!(scored.aggregate, 1.0);
        assert_eq!(scored.aggregate, scored.component_scores[0].score); // L6
    }

    #[test]
    fn trajectory_scorer_rejects_incomplete_unordered_match() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec!["a", "b", "c"], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec!["a".into(), "b".into()]);

        let scored = scorer.score(&tc, &output);
        assert_eq!(scored.aggregate, 0.0);
    }

    #[test]
    fn trajectory_scorer_empty_vacuous() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec![], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec![]);

        let scored = scorer.score(&tc, &output);
        assert_eq!(scored.aggregate, 1.0);
    }

    #[test]
    fn trajectory_scorer_details_present() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec!["a"], TrajectoryMode::Strict);
        let output = RawSampleOutput::new(vec!["a".into()]);

        let scored = scorer.score(&tc, &output);
        let details = scored.component_scores[0].details.as_ref().unwrap();
        assert_eq!(details["mode"], "strict");
        assert_eq!(details["passed"], true);
        assert_eq!(details["expected"], serde_json::json!(["a"]));
        assert_eq!(details["actual"], serde_json::json!(["a"]));
        assert_eq!(details["matched"], serde_json::json!(["a"]));
        assert_eq!(details["missing"], serde_json::json!([]));
        assert_eq!(details["unexpected"], serde_json::json!([]));
        assert_eq!(details["diagnostics"]["precision"], serde_json::json!(1.0));
        assert!(details.get("fScore").is_none());
    }

    #[test]
    fn trajectory_scorer_details_explain_partial_failure() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(
            vec!["storePlan", "executePlan"],
            TrajectoryMode::Subsequence,
        );
        let output = RawSampleOutput::new(vec!["storePlan".into(), "searchProducts".into()]);

        let scored = scorer.score(&tc, &output);
        let details = scored.component_scores[0].details.as_ref().unwrap();

        assert_eq!(scored.aggregate, 0.0);
        assert_eq!(scored.component_scores[0].score, 0.0);
        assert_eq!(details["passed"], false);
        assert_eq!(details["matched"], serde_json::json!(["storePlan"]));
        assert_eq!(details["missing"], serde_json::json!(["executePlan"]));
        assert_eq!(details["unexpected"], serde_json::json!(["searchProducts"]));
        assert_eq!(details["diagnostics"]["precision"], serde_json::json!(0.5));
        assert_eq!(details["diagnostics"]["recall"], serde_json::json!(0.5));
        assert_eq!(details["diagnostics"]["f1"], serde_json::json!(0.5));
        assert_eq!(details["diagnostics"]["f2"], serde_json::json!(0.5));
    }

    #[test]
    fn trajectory_scorer_details_show_extras_for_passing_superset() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec!["a", "b"], TrajectoryMode::Superset);
        let output = RawSampleOutput::new(vec!["a".into(), "lookup".into(), "b".into()]);

        let scored = scorer.score(&tc, &output);
        let details = scored.component_scores[0].details.as_ref().unwrap();

        assert_eq!(scored.aggregate, 1.0);
        assert_eq!(scored.component_scores[0].score, 1.0);
        assert_eq!(details["passed"], true);
        assert_eq!(details["matched"], serde_json::json!(["a", "b"]));
        assert_eq!(details["missing"], serde_json::json!([]));
        assert_eq!(details["unexpected"], serde_json::json!(["lookup"]));
        assert_eq!(details["diagnostics"]["recall"], serde_json::json!(1.0));
        let f2 = details["diagnostics"]["f2"]
            .as_f64()
            .expect("diagnostic f2 should be numeric");
        assert!((f2 - (10.0 / 11.0)).abs() < 1e-12);
    }

    // =========================================================================
    // L2: Determinism
    // =========================================================================

    #[test]
    fn scorer_determinism() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec!["a", "b"], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec!["b".into(), "c".into()]);

        let r1 = scorer.score(&tc, &output);
        let r2 = scorer.score(&tc, &output);
        assert_eq!(r1.aggregate, r2.aggregate);
        assert_eq!(r1.component_scores.len(), r2.component_scores.len());
    }

    // =========================================================================
    // L3: Non-empty (structural via ScoredSample)
    // =========================================================================

    #[test]
    fn scorer_non_empty() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec![], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec![]);

        let scored = scorer.score(&tc, &output);
        // L3 is now a compile-time guarantee: NonEmpty<ScorerResult> cannot be empty.
        // This test verifies the structural property still holds at runtime.
        assert!(
            scored.component_scores.len().get() >= 1,
            "L3: compile-time guarantee — NonEmpty always has >= 1 element"
        );
    }

    // =========================================================================
    // L4: Bounded
    // =========================================================================

    #[test]
    fn scorer_bounded() {
        let scorer = TrajectoryScorer::default();
        let tc = make_test_case(vec!["a", "b", "c"], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec!["d".into(), "e".into()]);

        let scored = scorer.score(&tc, &output);
        assert!(scored.aggregate >= 0.0 && scored.aggregate <= 1.0);
        for r in &scored.component_scores {
            assert!(r.score >= 0.0 && r.score <= 1.0);
        }
    }

    // =========================================================================
    // CompositeScorer
    // =========================================================================

    #[test]
    fn composite_scorer_trajectory_only() {
        let scorer = CompositeScorer::trajectory_only();
        let tc = make_test_case(vec!["a", "b"], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec!["a".into(), "b".into()]);

        let scored = scorer.score(&tc, &output);
        // trajectory result + composite result
        assert_eq!(scored.component_scores.len().get(), 2);
        assert_eq!(scored.component_scores[0].scorer_name, "trajectory");
        assert_eq!(scored.component_scores[1].scorer_name, "composite");
        assert_eq!(scored.aggregate, 1.0);
        assert_eq!(scored.component_scores[0].score, 1.0);
    }

    #[test]
    fn composite_scorer_l5_composition() {
        let child1 =
            WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), 1.0).unwrap();

        let composite = CompositeScorer::new(NonEmpty::singleton(child1)).unwrap();
        let tc = make_test_case(vec!["a"], TrajectoryMode::Unordered);
        let output = RawSampleOutput::new(vec!["a".into()]);

        let scored = composite.score(&tc, &output);

        // Child results appear first, composite result last
        let child_results: Vec<_> = scored
            .component_scores
            .iter()
            .filter(|r| r.scorer_name != "composite")
            .collect();
        assert!(!child_results.is_empty());

        // Aggregate should match the weighted average of child aggregates
        assert!(scored.aggregate >= 0.0 && scored.aggregate <= 1.0);
    }

    // =========================================================================
    // WeightedChild / CompositeScorer validation tests
    // =========================================================================

    #[test]
    fn weighted_child_rejects_negative() {
        let result = WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), -1.0);
        assert!(matches!(result, Err(WeightedChildError::Negative(_))));
    }

    #[test]
    fn weighted_child_rejects_nan() {
        let result = WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), f64::NAN);
        assert!(matches!(result, Err(WeightedChildError::NonFinite(_))));
    }

    #[test]
    fn weighted_child_rejects_infinite() {
        let result = WeightedChild::new(
            std::sync::Arc::new(TrajectoryScorer::default()),
            f64::INFINITY,
        );
        assert!(matches!(result, Err(WeightedChildError::NonFinite(_))));
    }

    #[test]
    fn weighted_child_accepts_zero() {
        let result = WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), 0.0);
        assert!(result.is_ok());
    }

    // `composite_rejects_empty` is now a compile-time guarantee:
    // `NonEmpty<WeightedChild>` cannot be constructed from an empty vec.
    // The test is replaced by this doc-comment and the type system itself.

    #[test]
    fn composite_rejects_all_zero_weights() {
        let child =
            WeightedChild::new(std::sync::Arc::new(TrajectoryScorer::default()), 0.0).unwrap();
        let result = CompositeScorer::new(NonEmpty::singleton(child));
        assert!(matches!(result, Err(CompositeError::AllZeroWeights)));
    }

    // =========================================================================
    // Proptest
    // =========================================================================

    // =========================================================================
    // ScoreWeights
    // =========================================================================

    #[test]
    fn score_weights_valid() {
        let ws = ScoreWeights::new(vec![("trajectory".into(), 0.4), ("action".into(), 0.6)]);
        assert!(ws.is_ok());
        let ws = ws.unwrap();
        assert_eq!(ws.len(), 2);
        // is_empty() is deprecated — NonEmpty guarantees non-emptiness
    }

    #[test]
    fn score_weights_rejects_empty() {
        assert!(matches!(
            ScoreWeights::new(vec![]),
            Err(ScoreWeightsError::Empty)
        ));
    }

    #[test]
    fn score_weights_rejects_negative() {
        let ws = ScoreWeights::new(vec![("a".into(), 1.0), ("b".into(), -0.5)]);
        assert!(matches!(ws, Err(ScoreWeightsError::Negative { .. })));
    }

    #[test]
    fn score_weights_rejects_all_zero() {
        let ws = ScoreWeights::new(vec![("a".into(), 0.0), ("b".into(), 0.0)]);
        assert!(matches!(ws, Err(ScoreWeightsError::AllZero)));
    }

    #[test]
    fn score_weights_rejects_nan() {
        let ws = ScoreWeights::new(vec![("a".into(), 1.0), ("b".into(), f64::NAN)]);
        assert!(matches!(ws, Err(ScoreWeightsError::NonFinite { .. })));
    }

    #[test]
    fn score_weights_rejects_infinite() {
        let ws = ScoreWeights::new(vec![("a".into(), f64::INFINITY)]);
        assert!(matches!(ws, Err(ScoreWeightsError::NonFinite { .. })));
    }

    #[test]
    fn score_weights_rejects_neg_infinite() {
        let ws = ScoreWeights::new(vec![("a".into(), f64::NEG_INFINITY)]);
        assert!(matches!(ws, Err(ScoreWeightsError::NonFinite { .. })));
    }

    #[test]
    fn score_weights_normalized_sums_to_one() {
        let ws = ScoreWeights::new(vec![("a".into(), 3.0), ("b".into(), 7.0)]).unwrap();
        let norm = ws.normalized();
        let sum: f64 = norm.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-10);
        assert!((norm[0].1 - 0.3).abs() < 1e-10);
        assert!((norm[1].1 - 0.7).abs() < 1e-10);
    }

    #[test]
    fn score_weights_equal() {
        let ws = ScoreWeights::equal(vec!["a".into(), "b".into(), "c".into()]).unwrap();
        assert_eq!(ws.len(), 3);
        let vals = ws.values();
        for v in &vals {
            assert!((*v - 1.0 / 3.0).abs() < 1e-10);
        }
    }

    #[test]
    fn score_weights_into_composite() {
        let ws = ScoreWeights::new(vec![("trajectory".into(), 1.0)]).unwrap();
        let scorers = NonEmpty::singleton(
            std::sync::Arc::new(TrajectoryScorer::default()) as std::sync::Arc<dyn EvalScorer>
        );
        let composite = ws.into_composite(scorers).unwrap();
        assert_eq!(EvalScorer::name(&composite), "composite");
    }

    #[test]
    fn into_composite_length_mismatch() {
        let ws = ScoreWeights::new(vec![("a".into(), 0.5), ("b".into(), 0.5)]).unwrap();
        let scorers = NonEmpty::singleton(
            std::sync::Arc::new(TrajectoryScorer::default()) as std::sync::Arc<dyn EvalScorer>
        );
        let result = ws.into_composite(scorers);
        assert!(matches!(
            result,
            Err(CompositeError::LengthMismatch {
                scorers: 1,
                weights: 2
            })
        ));
    }

    use hegel::generators;

    #[hegel::test]
    fn trajectory_scorer_bounded_prop(tc: hegel::TestCase) {
        let expected: Vec<String> = tc.draw(
            generators::vecs(generators::sampled_from(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]))
            .max_size(5),
        );
        let actual: Vec<String> = tc.draw(
            generators::vecs(generators::sampled_from(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]))
            .max_size(5),
        );
        let scorer = TrajectoryScorer::default();
        let test_case = make_test_case(
            expected.iter().map(|s| s.as_str()).collect(),
            TrajectoryMode::Unordered,
        );
        let output = RawSampleOutput::new(actual);

        let scored = scorer.score(&test_case, &output);
        assert!(
            scored.aggregate >= 0.0 && scored.aggregate <= 1.0,
            "L4: aggregate bounded, got {}",
            scored.aggregate
        );
        for r in &scored.component_scores {
            assert!(
                r.score >= 0.0 && r.score <= 1.0,
                "L4: component bounded, got {}",
                r.score
            );
        }
    }

    /// Validated ScoreWeights entries are always finite and non-negative.
    #[hegel::test]
    fn score_weights_valid_entries_are_finite(tc: hegel::TestCase) {
        let w1 = tc.draw(generators::floats::<f64>());
        let w2 = tc.draw(generators::floats::<f64>());
        let entries = vec![("a".into(), w1), ("b".into(), w2)];
        if let Ok(ws) = ScoreWeights::new(entries) {
            for (_, w) in ws.iter() {
                assert!(w.is_finite(), "weight should be finite, got {}", w);
                assert!(*w >= 0.0, "weight should be non-negative, got {}", w);
            }
        }
    }
}
