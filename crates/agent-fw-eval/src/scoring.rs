//! Pure scoring functions (total functions, no IO).
//!
//! # Vacuous Truth Convention
//!
//! All scoring functions in this module follow a uniform policy:
//! **empty inputs ⟹ perfect score (1.0)**.
//!
//! | Function | Empty case | Returns |
//! |----------|-----------|---------|
//! | `f_beta_score(0, 0, 0, _)` | No predictions, no ground truth | 1.0 |
//! | `jaccard_similarity([], [])` | Both sets empty | 1.0 |
//! | `aggregate_scorer_results([])` | No scorer results | 1.0 |
//! | `ratio_score(0, 0)` | No items | 1.0 |
//!
//! This is consistent with the domain semantics: "nothing expected,
//! nothing produced" is a perfect match by vacuous truth.
//!
//! # Functions
//!
//! - [`f_beta_score`] — precision/recall → FBetaScore (vacuous truth: 0,0,0 → 1.0)
//! - [`jaccard_similarity`] — set intersection / union
//! - [`pass_at_k_simple`] — simple pass@k estimator
//! - [`pass_at_k_unbiased`] — Codex unbiased estimator (Chen et al., 2021)
//! - [`score_trajectory`] — expected × actual × mode → FBetaScore
//! - [`aggregate_scorer_results`] — weighted average of scorer results
//! - [`ratio_score`] — matching / total (1.0 when total is 0)

use std::collections::HashMap;

use crate::types::{FBetaScore, JaccardScore, ScorerResult, TrajectoryMode};
use serde::Serialize;

// =============================================================================
// ratio_score
// =============================================================================

/// Compute a ratio score: matching / total, defaulting to 1.0 when total is 0.
pub fn ratio_score(matching: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        matching as f64 / total as f64
    }
}

// =============================================================================
// F-Beta Score
// =============================================================================

/// The F-beta formula as a pure function of precision and recall.
///
/// # Laws
/// - **Bounded**: result ∈ [0.0, 1.0]
/// - **Identity**: `compute_f_beta(1.0, 1.0, _) == 1.0`
/// - **Zero**: `compute_f_beta(0.0, _, _) == 0.0` and `compute_f_beta(_, 0.0, _) == 0.0`
/// - **Symmetry at β=1**: `compute_f_beta(p, r, 1.0) == compute_f_beta(r, p, 1.0)`
pub fn compute_f_beta(precision: f64, recall: f64, beta: f64) -> f64 {
    if !precision.is_finite() || !recall.is_finite() || !beta.is_finite() {
        return 0.0;
    }
    if precision + recall == 0.0 {
        return 0.0;
    }
    let beta_sq = beta * beta;
    (1.0 + beta_sq) * precision * recall / (beta_sq * precision + recall)
}

/// Compute F-beta score from true positives, false positives, false negatives.
///
/// Default beta=2 (recall-weighted, standard for evals).
///
/// # Laws
/// - `f_beta_score(0, 0, 0, _)` → vacuous truth: precision=1, recall=1, fScore=1
/// - `f_beta_score(tp, 0, 0, _)` → precision=1, recall=1, fScore=1
/// - `0 ≤ fScore ≤ 1`
pub fn f_beta_score(tp: u32, fp: u32, fn_: u32, beta: f64) -> FBetaScore {
    // Vacuous truth: no predictions and no ground truth → perfect match.
    if tp == 0 && fp == 0 && fn_ == 0 {
        return FBetaScore {
            precision: 1.0,
            recall: 1.0,
            f_score: 1.0,
            beta,
        };
    }

    let tp_f = tp as f64;
    let fp_f = fp as f64;
    let fn_f = fn_ as f64;

    let precision = if tp + fp == 0 {
        0.0
    } else {
        tp_f / (tp_f + fp_f)
    };

    let recall = if tp + fn_ == 0 {
        0.0
    } else {
        tp_f / (tp_f + fn_f)
    };

    FBetaScore {
        precision,
        recall,
        f_score: compute_f_beta(precision, recall, beta),
        beta,
    }
}

// =============================================================================
// Jaccard Similarity
// =============================================================================

/// Compute Jaccard similarity between two sets of strings.
///
/// Jaccard = |A ∩ B| / |A ∪ B|
///
/// # Laws
/// - `jaccard_similarity(a, a)` → similarity = 1.0
/// - `jaccard_similarity([], [])` → similarity = 1.0 (vacuous)
/// - `0 ≤ similarity ≤ 1`
pub fn jaccard_similarity(a: &[String], b: &[String]) -> JaccardScore {
    use std::collections::HashSet;

    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();

    let intersection_size = set_a.intersection(&set_b).count();
    let union_size = set_a.union(&set_b).count();

    let similarity = if union_size == 0 {
        1.0 // Both empty → identical
    } else {
        intersection_size as f64 / union_size as f64
    };

    JaccardScore {
        similarity,
        intersection_size,
        union_size,
    }
}

// =============================================================================
// Pass@K Estimators
// =============================================================================

/// Simple pass@k estimator.
///
/// Returns 1.0 when enough samples pass, 0.0 when none pass.
pub fn pass_at_k_simple(n: u32, c: u32, k: u32) -> f64 {
    if k == 0 || n == 0 {
        return 0.0;
    }
    if c >= n {
        return 1.0;
    }
    if c == 0 {
        return 0.0;
    }
    if k == 1 {
        return c as f64 / n as f64;
    }
    // 1 - prod_{i=0}^{k-1} (n-c-i) / (n-i)
    let mut result = 1.0;
    for i in 0..k {
        if n as i64 - c as i64 - i as i64 <= 0 {
            return 1.0; // Not enough failures to fill k slots
        }
        result *= (n - c - i) as f64 / (n - i) as f64;
    }
    1.0 - result
}

/// Unbiased pass@k estimator from the Codex paper (Chen et al., 2021).
///
/// Computes 1 - C(n-c, k) / C(n, k) using log-space arithmetic
/// to prevent overflow for large n, c, k values.
///
/// Returns None if n < k (insufficient samples).
///
/// # Law: Bounded — result in [0.0, 1.0] when Some.
pub fn pass_at_k_unbiased(n: u32, c: u32, k: u32) -> Option<f64> {
    if k > n {
        return None;
    }
    if c >= n {
        return Some(1.0);
    }
    if c == 0 {
        return Some(0.0);
    }
    if n - c < k {
        return Some(1.0); // not enough failures to fill k slots
    }

    let mut log_ratio = 0.0f64;
    for i in 0..k {
        log_ratio += ((n - c - i) as f64).ln() - ((n - i) as f64).ln();
    }
    Some((1.0 - log_ratio.exp()).clamp(0.0, 1.0))
}

// =============================================================================
// Trajectory Scoring
// =============================================================================

/// Confusion matrix counts for multiset comparison.
///
/// Named fields prevent the silent swap bug that `(u32, u32, u32)` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfusionCounts {
    pub true_positives: u32,
    pub false_positives: u32,
    pub false_negatives: u32,
}

/// Diagnostic-only trajectory similarity metrics.
///
/// These explain how close an observed trajectory was without changing the
/// binary trajectory predicate score returned by [`score_trajectory`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectorySimilarityDiagnostics {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub f2: f64,
}

/// JSON-ready trajectory scorer details for UI/debugging.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryScoreDetails {
    pub mode: TrajectoryMode,
    pub passed: bool,
    pub expected: Vec<String>,
    pub actual: Vec<String>,
    pub matched: Vec<String>,
    pub missing: Vec<String>,
    pub unexpected: Vec<String>,
    pub diagnostics: TrajectorySimilarityDiagnostics,
}

/// Build a multiset (frequency map) from a slice of tool names.
fn multiset_counts<'a>(tools: &'a [String]) -> HashMap<&'a str, u32> {
    let mut counts = HashMap::new();
    for tool in tools {
        *counts.entry(tool.as_str()).or_insert(0) += 1;
    }
    counts
}

fn multiset_is_subset(left: &[String], right: &[String]) -> bool {
    let left_counts = multiset_counts(left);
    let right_counts = multiset_counts(right);
    left_counts
        .iter()
        .all(|(tool, count)| right_counts.get(tool).copied().unwrap_or(0) >= *count)
}

fn is_subsequence(expected: &[String], actual: &[String]) -> bool {
    if expected.is_empty() {
        return true;
    }
    let mut expected_iter = expected.iter();
    let mut next_expected = expected_iter.next();
    for actual_tool in actual {
        if next_expected.is_some_and(|expected_tool| expected_tool == actual_tool) {
            next_expected = expected_iter.next();
            if next_expected.is_none() {
                return true;
            }
        }
    }
    false
}

fn binary_trajectory_score(pass: bool, beta: f64) -> FBetaScore {
    if pass {
        f_beta_score(1, 0, 0, beta)
    } else {
        FBetaScore {
            precision: 0.0,
            recall: 0.0,
            f_score: 0.0,
            beta,
        }
    }
}

fn multiset_trajectory_diff(
    expected: &[String],
    actual: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut remaining_expected = multiset_counts(expected);
    let mut matched = Vec::new();
    let mut unexpected = Vec::new();

    for tool in actual {
        let remaining = remaining_expected.get(tool.as_str()).copied().unwrap_or(0);
        if remaining > 0 {
            remaining_expected.insert(tool.as_str(), remaining - 1);
            matched.push(tool.clone());
        } else {
            unexpected.push(tool.clone());
        }
    }

    let mut missing = Vec::new();
    for tool in expected {
        let remaining = remaining_expected.get(tool.as_str()).copied().unwrap_or(0);
        if remaining > 0 {
            remaining_expected.insert(tool.as_str(), remaining - 1);
            missing.push(tool.clone());
        }
    }

    (matched, missing, unexpected)
}

fn ordered_trajectory_diff(
    expected: &[String],
    actual: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut lcs = vec![vec![0usize; actual.len() + 1]; expected.len() + 1];
    for (i, expected_tool) in expected.iter().enumerate() {
        for (j, actual_tool) in actual.iter().enumerate() {
            if expected_tool == actual_tool {
                lcs[i + 1][j + 1] = lcs[i][j] + 1;
            } else {
                lcs[i + 1][j + 1] = lcs[i][j + 1].max(lcs[i + 1][j]);
            }
        }
    }

    let mut matched_expected = vec![false; expected.len()];
    let mut matched_actual = vec![false; actual.len()];
    let mut i = expected.len();
    let mut j = actual.len();
    while i > 0 && j > 0 {
        if expected[i - 1] == actual[j - 1] {
            matched_expected[i - 1] = true;
            matched_actual[j - 1] = true;
            i -= 1;
            j -= 1;
        } else if lcs[i - 1][j] >= lcs[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    let matched = actual
        .iter()
        .enumerate()
        .filter(|(index, _)| matched_actual[*index])
        .map(|(_, tool)| tool.clone())
        .collect();
    let missing = expected
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched_expected[*index])
        .map(|(_, tool)| tool.clone())
        .collect();
    let unexpected = actual
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched_actual[*index])
        .map(|(_, tool)| tool.clone())
        .collect();

    (matched, missing, unexpected)
}

fn trajectory_similarity_diagnostics(
    matched: usize,
    missing: usize,
    unexpected: usize,
) -> TrajectorySimilarityDiagnostics {
    let true_positives = matched as u32;
    let false_positives = unexpected as u32;
    let false_negatives = missing as u32;
    let f1 = f_beta_score(true_positives, false_positives, false_negatives, 1.0);
    let f2 = f_beta_score(true_positives, false_positives, false_negatives, 2.0);

    TrajectorySimilarityDiagnostics {
        precision: f2.precision,
        recall: f2.recall,
        f1: f1.f_score,
        f2: f2.f_score,
    }
}

/// Build diagnostic trajectory details without changing the binary score.
///
/// `passed` should come from [`score_trajectory`]. The diff and similarity
/// values are explanatory only, so modes that allow extras can pass while still
/// reporting `unexpected` tool calls.
pub fn trajectory_score_details(
    expected: &[String],
    actual: &[String],
    mode: TrajectoryMode,
    passed: bool,
) -> TrajectoryScoreDetails {
    let (matched, missing, unexpected) = match mode {
        TrajectoryMode::Strict | TrajectoryMode::Subsequence => {
            ordered_trajectory_diff(expected, actual)
        }
        TrajectoryMode::Unordered | TrajectoryMode::Subset | TrajectoryMode::Superset => {
            multiset_trajectory_diff(expected, actual)
        }
    };
    let diagnostics =
        trajectory_similarity_diagnostics(matched.len(), missing.len(), unexpected.len());

    TrajectoryScoreDetails {
        mode,
        passed,
        expected: expected.to_vec(),
        actual: actual.to_vec(),
        matched,
        missing,
        unexpected,
        diagnostics,
    }
}

/// Score an actual trajectory against expected tools using the given mode.
///
/// Trajectory modes are exact predicates. The `beta` parameter is preserved on
/// the returned score for compatibility with the shared `FBetaScore` shape.
pub fn score_trajectory(
    expected: &[String],
    actual: &[String],
    mode: TrajectoryMode,
    beta: f64,
) -> FBetaScore {
    match mode {
        TrajectoryMode::Strict => binary_trajectory_score(expected == actual, beta),
        TrajectoryMode::Unordered => binary_trajectory_score(
            expected.len() == actual.len() && multiset_is_subset(expected, actual),
            beta,
        ),
        TrajectoryMode::Subset => {
            binary_trajectory_score(multiset_is_subset(actual, expected), beta)
        }
        TrajectoryMode::Superset => {
            binary_trajectory_score(multiset_is_subset(expected, actual), beta)
        }
        TrajectoryMode::Subsequence => {
            binary_trajectory_score(is_subsequence(expected, actual), beta)
        }
    }
}

// =============================================================================
// Aggregate Scorer Results
// =============================================================================

/// Aggregate multiple scorer results into a single score.
///
/// Strategy:
/// - Empty: 1.0 (vacuous truth — no evidence = perfect)
/// - Single result: extract its score directly
/// - Multiple results: weighted average using provided weights;
///   falls back to equal weighting when weights absent
///
/// # Law: Bounded — result always in [0.0, 1.0]
pub fn aggregate_scorer_results(scores: &[ScorerResult], weights: Option<&[f64]>) -> f64 {
    if scores.is_empty() {
        return 1.0; // Vacuous truth: no evidence = perfect
    }
    if scores.len() == 1 {
        return scores[0].score;
    }

    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;
    for (i, s) in scores.iter().enumerate() {
        let w = weights.and_then(|ws| ws.get(i).copied()).unwrap_or(1.0);
        weighted_sum += s.score * w;
        total_weight += w;
    }

    if total_weight == 0.0 {
        0.0
    } else {
        (weighted_sum / total_weight).clamp(0.0, 1.0)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // compute_f_beta (shared kernel)
    // =========================================================================

    #[test]
    fn compute_f_beta_identity() {
        assert_eq!(compute_f_beta(1.0, 1.0, 2.0), 1.0);
    }

    #[test]
    fn compute_f_beta_zero_precision() {
        assert_eq!(compute_f_beta(0.0, 1.0, 2.0), 0.0);
    }

    #[test]
    fn compute_f_beta_zero_recall() {
        assert_eq!(compute_f_beta(1.0, 0.0, 2.0), 0.0);
    }

    #[test]
    fn compute_f_beta_zero_both() {
        assert_eq!(compute_f_beta(0.0, 0.0, 2.0), 0.0);
    }

    #[test]
    fn compute_f_beta_symmetry_at_beta_one() {
        let a = compute_f_beta(0.8, 0.6, 1.0);
        let b = compute_f_beta(0.6, 0.8, 1.0);
        assert!(
            (a - b).abs() < 1e-12,
            "F1 should be symmetric: {} vs {}",
            a,
            b
        );
    }

    #[test]
    fn compute_f_beta_nan_returns_zero() {
        assert_eq!(compute_f_beta(f64::NAN, 0.5, 2.0), 0.0);
        assert_eq!(compute_f_beta(0.5, f64::NAN, 2.0), 0.0);
        assert_eq!(compute_f_beta(0.5, 0.5, f64::NAN), 0.0);
    }

    #[test]
    fn compute_f_beta_infinite_returns_zero() {
        assert_eq!(compute_f_beta(f64::INFINITY, 0.5, 2.0), 0.0);
        assert_eq!(compute_f_beta(0.5, f64::NEG_INFINITY, 2.0), 0.0);
        assert_eq!(compute_f_beta(0.5, 0.5, f64::INFINITY), 0.0);
    }

    // =========================================================================
    // f_beta_score
    // =========================================================================

    #[test]
    fn f_beta_vacuous_truth() {
        let r = f_beta_score(0, 0, 0, 2.0);
        assert_eq!(r.precision, 1.0);
        assert_eq!(r.recall, 1.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn f_beta_perfect() {
        let r = f_beta_score(5, 0, 0, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn f_beta_partial() {
        let r = f_beta_score(3, 1, 1, 2.0);
        assert!(r.f_score > 0.0 && r.f_score < 1.0);
    }

    #[test]
    fn f_beta_zero() {
        let r = f_beta_score(0, 5, 5, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn f_beta_beta_variation() {
        // Beta=1 weights precision and recall equally
        let r1 = f_beta_score(3, 1, 2, 1.0);
        // Beta=2 weights recall more
        let r2 = f_beta_score(3, 1, 2, 2.0);
        // With more FN (recall hurt), beta=2 should give lower f_score
        assert!(r2.f_score < r1.f_score);
    }

    // =========================================================================
    // jaccard_similarity
    // =========================================================================

    #[test]
    fn jaccard_identical() {
        let a = vec!["a".into(), "b".into(), "c".into()];
        let r = jaccard_similarity(&a, &a);
        assert_eq!(r.similarity, 1.0);
    }

    #[test]
    fn jaccard_empty() {
        let r = jaccard_similarity(&[], &[]);
        assert_eq!(r.similarity, 1.0);
    }

    #[test]
    fn jaccard_disjoint() {
        let a = vec!["a".into(), "b".into()];
        let b = vec!["c".into(), "d".into()];
        let r = jaccard_similarity(&a, &b);
        assert_eq!(r.similarity, 0.0);
    }

    #[test]
    fn jaccard_partial() {
        let a = vec!["a".into(), "b".into(), "c".into()];
        let b = vec!["b".into(), "c".into(), "d".into()];
        let r = jaccard_similarity(&a, &b);
        assert_eq!(r.intersection_size, 2);
        assert_eq!(r.union_size, 4);
        assert_eq!(r.similarity, 0.5);
    }

    // =========================================================================
    // pass_at_k
    // =========================================================================

    #[test]
    fn pass_at_k_simple_k1() {
        assert_eq!(pass_at_k_simple(10, 3, 1), 0.3);
    }

    #[test]
    fn pass_at_k_simple_all_pass() {
        assert_eq!(pass_at_k_simple(5, 5, 1), 1.0);
    }

    #[test]
    fn pass_at_k_simple_none_pass() {
        assert_eq!(pass_at_k_simple(5, 0, 1), 0.0);
    }

    #[test]
    fn pass_at_k_simple_k0() {
        assert_eq!(pass_at_k_simple(5, 3, 0), 0.0);
    }

    #[test]
    fn pass_at_k_unbiased_insufficient_samples() {
        assert!(pass_at_k_unbiased(3, 2, 5).is_none());
    }

    #[test]
    fn pass_at_k_unbiased_all_pass() {
        assert_eq!(pass_at_k_unbiased(5, 5, 3), Some(1.0));
    }

    #[test]
    fn pass_at_k_unbiased_none_pass() {
        assert_eq!(pass_at_k_unbiased(5, 0, 1), Some(0.0));
    }

    #[test]
    fn pass_at_k_unbiased_bounded() {
        if let Some(v) = pass_at_k_unbiased(10, 3, 2) {
            assert!(v >= 0.0 && v <= 1.0);
        }
    }

    // =========================================================================
    // score_trajectory
    // =========================================================================

    #[test]
    fn trajectory_unordered_accepts_reordered_exact_multiset() {
        let expected = vec!["a".into(), "b".into()];
        let actual = vec!["b".into(), "a".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Unordered, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_strict_accepts_exact_sequence() {
        let expected = vec!["a".into(), "b".into(), "c".into()];
        let actual = vec!["a".into(), "b".into(), "c".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Strict, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_strict_rejects_reordered_sequence() {
        let expected = vec!["a".into(), "b".into()];
        let actual = vec!["b".into(), "a".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Strict, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_strict_rejects_extra_actual_tools() {
        let expected = vec!["a".into(), "c".into()];
        let actual = vec!["a".into(), "b".into(), "c".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Strict, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_subset_accepts_actual_subset_of_expected() {
        let expected = vec!["a".into(), "b".into(), "c".into()];
        let actual = vec!["a".into(), "b".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subset, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_subset_rejects_unexpected_extra_actual_tools() {
        let expected = vec!["a".into(), "b".into()];
        let actual = vec!["a".into(), "b".into(), "c".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subset, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_subset_rejects_duplicate_overuse() {
        let expected = vec!["a".into(), "a".into(), "b".into()];
        let actual = vec!["a".into(), "a".into(), "a".into(), "b".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subset, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_superset_accepts_extra_actual_tools() {
        let expected = vec!["a".into(), "b".into()];
        let actual = vec!["a".into(), "b".into(), "c".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Superset, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_superset_rejects_missing_expected_tools() {
        let expected = vec!["a".into(), "b".into(), "c".into()];
        let actual = vec!["a".into(), "b".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Superset, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_subsequence_accepts_ordered_milestones_with_gaps() {
        let expected = vec!["a".into(), "c".into()];
        let actual = vec!["a".into(), "b".into(), "c".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subsequence, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_subsequence_rejects_reordered_milestones() {
        let expected = vec!["a".into(), "c".into()];
        let actual = vec!["c".into(), "a".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subsequence, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_superset_empty_actual_is_not_perfect_when_expected_exists() {
        let expected = vec!["draft_plan".into()];
        let actual = vec![];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Superset, 2.0);
        assert_eq!(r.f_score, 0.0);
        assert_eq!(r.precision, 0.0);
        assert_eq!(r.recall, 0.0);
    }

    #[test]
    fn trajectory_empty_empty_vacuous() {
        let r = score_trajectory(&[], &[], TrajectoryMode::Unordered, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[test]
    fn trajectory_multiset_duplicates() {
        let expected = vec!["a".into(), "a".into(), "b".into()];
        let actual = vec!["a".into(), "b".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Unordered, 2.0);
        assert_eq!(r.f_score, 0.0);
    }

    #[test]
    fn trajectory_details_report_partial_failure_similarity() {
        let expected = vec!["storePlan".into(), "executePlan".into()];
        let actual = vec!["storePlan".into(), "searchProducts".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Subsequence, 2.0);
        let details = trajectory_score_details(
            &expected,
            &actual,
            TrajectoryMode::Subsequence,
            r.f_score == 1.0,
        );

        assert_eq!(r.f_score, 0.0);
        assert!(!details.passed);
        assert_eq!(details.matched, vec!["storePlan"]);
        assert_eq!(details.missing, vec!["executePlan"]);
        assert_eq!(details.unexpected, vec!["searchProducts"]);
        assert_eq!(details.diagnostics.precision, 0.5);
        assert_eq!(details.diagnostics.recall, 0.5);
        assert_eq!(details.diagnostics.f1, 0.5);
        assert_eq!(details.diagnostics.f2, 0.5);
    }

    #[test]
    fn trajectory_details_keep_extras_for_passing_superset() {
        let expected = vec!["a".into(), "b".into()];
        let actual = vec!["a".into(), "lookup".into(), "b".into()];
        let r = score_trajectory(&expected, &actual, TrajectoryMode::Superset, 2.0);
        let details = trajectory_score_details(
            &expected,
            &actual,
            TrajectoryMode::Superset,
            r.f_score == 1.0,
        );

        assert_eq!(r.f_score, 1.0);
        assert!(details.passed);
        assert_eq!(details.matched, vec!["a", "b"]);
        assert!(details.missing.is_empty());
        assert_eq!(details.unexpected, vec!["lookup"]);
        assert!((details.diagnostics.precision - (2.0 / 3.0)).abs() < 1e-12);
        assert_eq!(details.diagnostics.recall, 1.0);
        assert!((details.diagnostics.f1 - 0.8).abs() < 1e-12);
        assert!((details.diagnostics.f2 - (10.0 / 11.0)).abs() < 1e-12);
    }

    #[test]
    fn trajectory_details_respect_duplicate_counts() {
        let expected = vec!["a".into(), "a".into(), "b".into()];
        let actual = vec!["a".into(), "b".into(), "b".into()];
        let details =
            trajectory_score_details(&expected, &actual, TrajectoryMode::Unordered, false);

        assert_eq!(details.matched, vec!["a", "b"]);
        assert_eq!(details.missing, vec!["a"]);
        assert_eq!(details.unexpected, vec!["b"]);
    }

    // =========================================================================
    // aggregate_scorer_results
    // =========================================================================

    #[test]
    fn aggregate_empty() {
        // Vacuous truth: no scorer results = perfect score
        assert_eq!(aggregate_scorer_results(&[], None), 1.0);
    }

    #[test]
    fn aggregate_single() {
        let scores = vec![ScorerResult::new("test", 0.8)];
        assert_eq!(aggregate_scorer_results(&scores, None), 0.8);
    }

    #[test]
    fn aggregate_equal_weights() {
        let scores = vec![ScorerResult::new("a", 0.8), ScorerResult::new("b", 0.6)];
        let r = aggregate_scorer_results(&scores, None);
        assert!((r - 0.7).abs() < 1e-10);
    }

    #[test]
    fn aggregate_explicit_weights() {
        let scores = vec![ScorerResult::new("a", 1.0), ScorerResult::new("b", 0.0)];
        let r = aggregate_scorer_results(&scores, Some(&[3.0, 1.0]));
        assert!((r - 0.75).abs() < 1e-10);
    }

    // =========================================================================
    // ratio_score
    // =========================================================================

    #[test]
    fn ratio_score_zero_total() {
        assert_eq!(ratio_score(0, 0), 1.0);
    }

    #[test]
    fn ratio_score_perfect() {
        assert_eq!(ratio_score(5, 5), 1.0);
    }

    #[test]
    fn ratio_score_partial() {
        assert_eq!(ratio_score(3, 6), 0.5);
    }

    // =========================================================================
    // Hegel property-based tests
    // =========================================================================

    use hegel::generators;

    fn draw_tool_vec(tc: &hegel::TestCase) -> Vec<String> {
        tc.draw(
            generators::vecs(generators::sampled_from(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]))
            .max_size(5),
        )
    }

    #[hegel::test]
    fn f_beta_bounded(tc: hegel::TestCase) {
        let tp = tc.draw(generators::integers::<u32>().min_value(0).max_value(19));
        let fp = tc.draw(generators::integers::<u32>().min_value(0).max_value(19));
        let fn_ = tc.draw(generators::integers::<u32>().min_value(0).max_value(19));
        let r = f_beta_score(tp, fp, fn_, 2.0);
        assert!(r.f_score >= 0.0 && r.f_score <= 1.0);
        assert!(r.precision >= 0.0 && r.precision <= 1.0);
        assert!(r.recall >= 0.0 && r.recall <= 1.0);
    }

    #[hegel::test]
    fn trajectory_unordered_identity(tc: hegel::TestCase) {
        let tools = draw_tool_vec(&tc);
        let r = score_trajectory(&tools, &tools, TrajectoryMode::Unordered, 2.0);
        assert_eq!(r.f_score, 1.0);
    }

    #[hegel::test]
    fn trajectory_score_bounded(tc: hegel::TestCase) {
        let expected = draw_tool_vec(&tc);
        let actual = draw_tool_vec(&tc);
        for mode in [
            TrajectoryMode::Unordered,
            TrajectoryMode::Strict,
            TrajectoryMode::Subset,
            TrajectoryMode::Superset,
            TrajectoryMode::Subsequence,
        ] {
            let r = score_trajectory(&expected, &actual, mode, 2.0);
            assert!(
                r.f_score >= 0.0 && r.f_score <= 1.0,
                "mode={:?} score={}",
                mode,
                r.f_score
            );
        }
    }

    #[hegel::test]
    fn pass_at_k_unbiased_bounded_prop(tc: hegel::TestCase) {
        let n = tc.draw(generators::integers::<u32>().min_value(1).max_value(19));
        let c = tc.draw(generators::integers::<u32>().min_value(0).max_value(19));
        let k = tc.draw(generators::integers::<u32>().min_value(1).max_value(19));
        if let Some(v) = pass_at_k_unbiased(n, c.min(n), k.min(n)) {
            assert!(v >= 0.0 && v <= 1.0, "value={}", v);
        }
    }

    /// aggregate_scorer_results output is always in [0.0, 1.0] given bounded scores.
    #[hegel::test]
    fn aggregate_bounded_prop(tc: hegel::TestCase) {
        let s1 = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let s2 = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(1.0));
        let w1 = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(10.0));
        let w2 = tc.draw(generators::floats::<f64>().min_value(0.0).max_value(10.0));
        let scores = vec![ScorerResult::new("a", s1), ScorerResult::new("b", s2)];
        let weights = vec![w1, w2];
        let result = aggregate_scorer_results(&scores, Some(&weights));
        assert!(
            result >= 0.0 && result <= 1.0,
            "aggregate should be in [0,1], got {}",
            result
        );
    }
}
