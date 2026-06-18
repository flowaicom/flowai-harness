//! Run comparison utilities.
//!
//! Compare two eval runs to surface regressions and improvements.
//!
//! # Wire contract
//!
//! The frontend (`RunComparisonSummary` in `studio/app/lib/api/evals.ts`)
//! expects camelCase JSON with fields: `leftId`, `rightId`,
//! `testCaseComparisons`, `scoreDelta`, `passRateDelta`, `leftAvgScore`,
//! `rightAvgScore`, `leftPassRate`, `rightPassRate`.
//!
//! `TestCaseComparison` includes `leftScore`, `rightScore`, `scoreDelta`,
//! `leftPass`, `rightPass`, `regression`, `improvement`.

use crate::types::{EvalRun, TestCaseResult};
use serde::ser::SerializeMap;
use serde::Serialize;

// =============================================================================
// ComparisonOutcome — 3-inhabitant sum type
// =============================================================================

/// Classification of a test case comparison.
///
/// Replaces the `(regression: bool, improvement: bool)` pair which admitted
/// the impossible state `(true, true)`. This 3-inhabitant enum makes that
/// illegal state unrepresentable at the type level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOutcome {
    Regressed,
    Improved,
    Unchanged,
}

impl ComparisonOutcome {
    /// Classify from pass/fail status. Total over all 9 cases of
    /// `Option<bool> x Option<bool>`.
    ///
    /// | left_pass   | right_pass  | outcome   | rationale                    |
    /// |-------------|-------------|-----------|------------------------------|
    /// | Some(true)  | Some(false) | Regressed | was passing, now failing     |
    /// | Some(false) | Some(true)  | Improved  | was failing, now passing     |
    /// | None        | Some(true)  | Improved  | new test case, passing       |
    /// | all other 6 | combos      | Unchanged | no pass/fail status change   |
    ///
    /// Deliberate asymmetry: `(None, Some(true))` is `Improved` because a new
    /// passing test is positive signal. `(Some(true), None)` is NOT `Regressed`
    /// because a missing test case is typically a config difference, not
    /// quality regression.
    pub fn classify(left_pass: Option<bool>, right_pass: Option<bool>) -> Self {
        match (left_pass, right_pass) {
            (Some(true), Some(false)) => ComparisonOutcome::Regressed,
            (Some(false), Some(true)) => ComparisonOutcome::Improved,
            (None, Some(true)) => ComparisonOutcome::Improved,
            _ => ComparisonOutcome::Unchanged,
        }
    }

    /// Whether this outcome represents a regression.
    pub fn is_regression(self) -> bool {
        matches!(self, ComparisonOutcome::Regressed)
    }

    /// Whether this outcome represents an improvement.
    pub fn is_improvement(self) -> bool {
        matches!(self, ComparisonOutcome::Improved)
    }
}

// =============================================================================
// TestCaseComparison
// =============================================================================

/// Comparison between two test case results.
///
/// Uses `ComparisonOutcome` as the source of truth. Custom `Serialize` projects
/// to the wire contract's `regression: bool, improvement: bool` for backwards
/// compatibility with the frontend.
#[derive(Debug, Clone, PartialEq)]
pub struct TestCaseComparison {
    pub test_case_id: String,
    pub left_score: Option<f64>,
    pub right_score: Option<f64>,
    pub score_delta: f64,
    pub left_pass: Option<bool>,
    pub right_pass: Option<bool>,
    pub outcome: ComparisonOutcome,
}

impl Serialize for TestCaseComparison {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(8))?;
        map.serialize_entry("testCaseId", &self.test_case_id)?;
        map.serialize_entry("leftScore", &self.left_score)?;
        map.serialize_entry("rightScore", &self.right_score)?;
        map.serialize_entry("scoreDelta", &self.score_delta)?;
        map.serialize_entry("leftPass", &self.left_pass)?;
        map.serialize_entry("rightPass", &self.right_pass)?;
        map.serialize_entry("regression", &self.outcome.is_regression())?;
        map.serialize_entry("improvement", &self.outcome.is_improvement())?;
        map.end()
    }
}

// =============================================================================
// Aggregate helpers
// =============================================================================

/// Mean aggregate score across results. Returns `None` for empty slices.
fn mean_score(results: &[TestCaseResult]) -> Option<f64> {
    if results.is_empty() {
        return None;
    }
    Some(results.iter().map(|r| r.aggregate_score).sum::<f64>() / results.len() as f64)
}

/// Pass rate (fraction of results scoring >= threshold). Returns `None` for empty slices.
fn pass_rate(results: &[TestCaseResult], threshold: f64) -> Option<f64> {
    if results.is_empty() {
        return None;
    }
    let passed = results
        .iter()
        .filter(|r| r.aggregate_score >= threshold)
        .count();
    Some(passed as f64 / results.len() as f64)
}

// =============================================================================
// RunComparison
// =============================================================================

/// Run-level comparison summary.
///
/// Field names and casing match the frontend `RunComparisonSummary` interface.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunComparison {
    pub left_id: String,
    pub right_id: String,
    pub test_case_comparisons: Vec<TestCaseComparison>,
    pub score_delta: f64,
    pub pass_rate_delta: f64,
    pub left_avg_score: f64,
    pub right_avg_score: f64,
    pub left_pass_rate: f64,
    pub right_pass_rate: f64,
}

/// Compare two eval runs using each run's own configured pass threshold.
pub fn compare_runs(left: &EvalRun, right: &EvalRun) -> RunComparison {
    compare_runs_with_thresholds(
        left,
        right,
        left.config.pass_threshold,
        right.config.pass_threshold,
    )
}

/// Compare two eval runs using one explicit pass threshold for both sides.
pub fn compare_runs_at_threshold(
    left: &EvalRun,
    right: &EvalRun,
    pass_threshold: f64,
) -> RunComparison {
    compare_runs_with_thresholds(left, right, pass_threshold, pass_threshold)
}

/// Compare two eval runs using explicit pass thresholds per side.
pub fn compare_runs_with_thresholds(
    left: &EvalRun,
    right: &EvalRun,
    left_threshold: f64,
    right_threshold: f64,
) -> RunComparison {
    use std::collections::HashMap;

    let left_map: HashMap<String, &TestCaseResult> = left
        .results
        .iter()
        .map(|r| (r.test_case_id.as_str().to_string(), r))
        .collect();

    let right_map: HashMap<String, &TestCaseResult> = right
        .results
        .iter()
        .map(|r| (r.test_case_id.as_str().to_string(), r))
        .collect();

    let mut all_ids: Vec<String> = left_map.keys().chain(right_map.keys()).cloned().collect();
    all_ids.sort();
    all_ids.dedup();

    let mut comparisons = Vec::new();

    for id in &all_ids {
        let left_result = left_map.get(id);
        let right_result = right_map.get(id);

        let left_score = left_result.map(|r| r.aggregate_score);
        let right_score = right_result.map(|r| r.aggregate_score);

        let left_pass = left_score.map(|s| s >= left_threshold);
        let right_pass = right_score.map(|s| s >= right_threshold);

        // Honest score_delta: don't conflate None with zero.
        let score_delta = match (left_score, right_score) {
            (Some(l), Some(r)) => r - l,
            (None, Some(r)) => r,
            (Some(l), None) => -l,
            (None, None) => 0.0,
        };

        let outcome = ComparisonOutcome::classify(left_pass, right_pass);

        comparisons.push(TestCaseComparison {
            test_case_id: id.clone(),
            left_score,
            right_score,
            score_delta,
            left_pass,
            right_pass,
            outcome,
        });
    }

    // Aggregate scores — None for empty runs, projected to 0.0 for wire contract.
    let left_avg_score = mean_score(&left.results).unwrap_or(0.0);
    let right_avg_score = mean_score(&right.results).unwrap_or(0.0);
    let left_pass_rate = pass_rate(&left.results, left_threshold).unwrap_or(0.0);
    let right_pass_rate = pass_rate(&right.results, right_threshold).unwrap_or(0.0);

    RunComparison {
        left_id: left.id.as_str().to_string(),
        right_id: right.id.as_str().to_string(),
        test_case_comparisons: comparisons,
        score_delta: right_avg_score - left_avg_score,
        pass_rate_delta: right_pass_rate - left_pass_rate,
        left_avg_score,
        right_avg_score,
        left_pass_rate,
        right_pass_rate,
    }
}

/// Merge re-run results into the parent run.
///
/// For each test case in `rerun`, replace the matching result in `parent`.
/// Test cases not in the rerun are preserved unchanged.
pub fn merge_rerun_results(
    parent_results: &[TestCaseResult],
    rerun_results: &[TestCaseResult],
) -> Vec<TestCaseResult> {
    use std::collections::HashMap;

    let rerun_map: HashMap<String, &TestCaseResult> = rerun_results
        .iter()
        .map(|r| (r.test_case_id.as_str().to_string(), r))
        .collect();

    parent_results
        .iter()
        .map(|parent| {
            let id = parent.test_case_id.as_str().to_string();
            match rerun_map.get(&id) {
                Some(rerun) => (*rerun).clone(),
                None => parent.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EvalConfig, EvalRun, EvalStatus, TestCaseResult};
    use agent_fw_core::{EvalRunId, TestCaseId};

    fn make_result(id: &str, score: f64) -> TestCaseResult {
        TestCaseResult {
            test_case_id: TestCaseId::new_unchecked(id),
            input: None,
            samples: vec![],
            pass_at_k: vec![],
            aggregate_score: score,
        }
    }

    fn make_run(id: &str, results: Vec<TestCaseResult>) -> EvalRun {
        EvalRun {
            id: EvalRunId::new_unchecked(id),
            config: EvalConfig {
                test_case_source: crate::types::TestCaseSource::Set("set-1".into()),
                ..Default::default()
            },
            status: EvalStatus::Queued,
            results,
            created_at: "2025-01-01".into(),
            updated_at: "2025-01-01".into(),
            parent_run_id: None,
            rerun_test_case_ids: None,
        }
    }

    // =========================================================================
    // ComparisonOutcome::classify — exhaustive 9-case truth table
    // =========================================================================

    #[test]
    fn classify_true_false_is_regressed() {
        assert_eq!(
            ComparisonOutcome::classify(Some(true), Some(false)),
            ComparisonOutcome::Regressed
        );
    }

    #[test]
    fn classify_false_true_is_improved() {
        assert_eq!(
            ComparisonOutcome::classify(Some(false), Some(true)),
            ComparisonOutcome::Improved
        );
    }

    #[test]
    fn classify_none_true_is_improved() {
        assert_eq!(
            ComparisonOutcome::classify(None, Some(true)),
            ComparisonOutcome::Improved
        );
    }

    #[test]
    fn classify_remaining_six_are_unchanged() {
        // (true, true), (false, false), (true, None), (false, None),
        // (None, false), (None, None)
        let unchanged_cases = [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (Some(true), None),
            (Some(false), None),
            (None, Some(false)),
            (None, None),
        ];
        for (left, right) in unchanged_cases {
            assert_eq!(
                ComparisonOutcome::classify(left, right),
                ComparisonOutcome::Unchanged,
                "Expected Unchanged for ({left:?}, {right:?})"
            );
        }
    }

    // =========================================================================
    // compare_runs tests
    // =========================================================================

    #[test]
    fn compare_runs_identical() {
        let run = make_run(
            "run-1",
            vec![make_result("tc-1", 0.8), make_result("tc-2", 0.6)],
        );
        let cmp = compare_runs_at_threshold(&run, &run, 0.5);
        assert_eq!(cmp.test_case_comparisons.len(), 2);
        assert!(cmp
            .test_case_comparisons
            .iter()
            .all(|c| c.outcome == ComparisonOutcome::Unchanged));
        assert!((cmp.score_delta).abs() < 1e-10);
        assert!((cmp.pass_rate_delta).abs() < 1e-10);
    }

    #[test]
    fn compare_runs_improvement() {
        let baseline = make_run("run-1", vec![make_result("tc-1", 0.3)]);
        let candidate = make_run("run-2", vec![make_result("tc-1", 0.9)]);
        let cmp = compare_runs_at_threshold(&baseline, &candidate, 0.5);
        assert_eq!(cmp.test_case_comparisons.len(), 1);
        let tc = &cmp.test_case_comparisons[0];
        assert_eq!(tc.outcome, ComparisonOutcome::Improved);
        assert_eq!(tc.left_pass, Some(false));
        assert_eq!(tc.right_pass, Some(true));
        assert!((tc.score_delta - 0.6).abs() < 1e-10);
    }

    #[test]
    fn compare_runs_regression() {
        let baseline = make_run("run-1", vec![make_result("tc-1", 0.9)]);
        let candidate = make_run("run-2", vec![make_result("tc-1", 0.3)]);
        let cmp = compare_runs_at_threshold(&baseline, &candidate, 0.5);
        let tc = &cmp.test_case_comparisons[0];
        assert_eq!(tc.outcome, ComparisonOutcome::Regressed);
    }

    #[test]
    fn compare_runs_left_only() {
        let baseline = make_run("run-1", vec![make_result("tc-1", 0.8)]);
        let candidate = make_run("run-2", vec![]);
        let cmp = compare_runs_at_threshold(&baseline, &candidate, 0.5);
        assert_eq!(cmp.test_case_comparisons.len(), 1);
        let tc = &cmp.test_case_comparisons[0];
        assert_eq!(tc.left_score, Some(0.8));
        assert_eq!(tc.right_score, None);
        // Honest score_delta: -0.8 (not 0.0 - 0.8)
        assert!((tc.score_delta - (-0.8)).abs() < 1e-10);
    }

    #[test]
    fn compare_runs_right_only() {
        let baseline = make_run("run-1", vec![]);
        let candidate = make_run("run-2", vec![make_result("tc-1", 0.8)]);
        let cmp = compare_runs_at_threshold(&baseline, &candidate, 0.5);
        assert_eq!(cmp.test_case_comparisons.len(), 1);
        let tc = &cmp.test_case_comparisons[0];
        assert_eq!(tc.left_score, None);
        assert_eq!(tc.right_score, Some(0.8));
        assert_eq!(tc.outcome, ComparisonOutcome::Improved); // None → pass
                                                             // Honest score_delta: 0.8 (not 0.8 - 0.0)
        assert!((tc.score_delta - 0.8).abs() < 1e-10);
    }

    #[test]
    fn compare_runs_pass_rates() {
        let baseline = make_run(
            "run-1",
            vec![make_result("tc-1", 0.8), make_result("tc-2", 0.3)],
        );
        let candidate = make_run(
            "run-2",
            vec![make_result("tc-1", 0.9), make_result("tc-2", 0.7)],
        );
        let cmp = compare_runs_at_threshold(&baseline, &candidate, 0.5);
        assert!((cmp.left_pass_rate - 0.5).abs() < 1e-10);
        assert!((cmp.right_pass_rate - 1.0).abs() < 1e-10);
        assert!((cmp.pass_rate_delta - 0.5).abs() < 1e-10);
    }

    #[test]
    fn compare_runs_uses_each_run_threshold_by_default() {
        let mut baseline = make_run("run-1", vec![make_result("tc-1", 0.6)]);
        baseline.config.pass_threshold = 0.7;
        let mut candidate = make_run("run-2", vec![make_result("tc-1", 0.6)]);
        candidate.config.pass_threshold = 0.5;

        let cmp = compare_runs(&baseline, &candidate);
        let tc = &cmp.test_case_comparisons[0];
        assert_eq!(tc.left_pass, Some(false));
        assert_eq!(tc.right_pass, Some(true));
        assert_eq!(tc.outcome, ComparisonOutcome::Improved);
    }

    // =========================================================================
    // Wire contract (serde)
    // =========================================================================

    #[test]
    fn serde_produces_camel_case() {
        let cmp = RunComparison {
            left_id: "a".into(),
            right_id: "b".into(),
            test_case_comparisons: vec![],
            score_delta: 0.1,
            pass_rate_delta: 0.2,
            left_avg_score: 0.5,
            right_avg_score: 0.6,
            left_pass_rate: 0.7,
            right_pass_rate: 0.9,
        };
        let json = serde_json::to_value(&cmp).unwrap();
        assert!(json.get("leftId").is_some());
        assert!(json.get("rightId").is_some());
        assert!(json.get("testCaseComparisons").is_some());
        assert!(json.get("scoreDelta").is_some());
        assert!(json.get("passRateDelta").is_some());
        assert!(json.get("leftAvgScore").is_some());
        assert!(json.get("rightAvgScore").is_some());
        assert!(json.get("leftPassRate").is_some());
        assert!(json.get("rightPassRate").is_some());
        // Ensure old field names are NOT present
        assert!(json.get("baseline_run_id").is_none());
        assert!(json.get("candidate_run_id").is_none());
    }

    #[test]
    fn test_case_comparison_serde_camel_case() {
        let tc = TestCaseComparison {
            test_case_id: "tc-1".into(),
            left_score: Some(0.8),
            right_score: Some(0.9),
            score_delta: 0.1,
            left_pass: Some(true),
            right_pass: Some(true),
            outcome: ComparisonOutcome::Unchanged,
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert!(json.get("testCaseId").is_some());
        assert!(json.get("leftScore").is_some());
        assert!(json.get("rightScore").is_some());
        assert!(json.get("scoreDelta").is_some());
        assert!(json.get("leftPass").is_some());
        assert!(json.get("rightPass").is_some());
        assert!(json.get("regression").is_some());
        assert!(json.get("improvement").is_some());
        // Wire projects from outcome enum:
        assert_eq!(json.get("regression").unwrap(), false);
        assert_eq!(json.get("improvement").unwrap(), false);
    }

    #[test]
    fn test_case_comparison_serde_regression() {
        let tc = TestCaseComparison {
            test_case_id: "tc-1".into(),
            left_score: Some(0.9),
            right_score: Some(0.3),
            score_delta: -0.6,
            left_pass: Some(true),
            right_pass: Some(false),
            outcome: ComparisonOutcome::Regressed,
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json.get("regression").unwrap(), true);
        assert_eq!(json.get("improvement").unwrap(), false);
    }

    #[test]
    fn test_case_comparison_serde_improvement() {
        let tc = TestCaseComparison {
            test_case_id: "tc-1".into(),
            left_score: Some(0.3),
            right_score: Some(0.9),
            score_delta: 0.6,
            left_pass: Some(false),
            right_pass: Some(true),
            outcome: ComparisonOutcome::Improved,
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json.get("regression").unwrap(), false);
        assert_eq!(json.get("improvement").unwrap(), true);
    }

    // =========================================================================
    // mean_score / pass_rate helpers
    // =========================================================================

    #[test]
    fn mean_score_empty_is_none() {
        assert_eq!(mean_score(&[]), None);
    }

    #[test]
    fn mean_score_computes_average() {
        let results = vec![make_result("a", 0.4), make_result("b", 0.6)];
        assert!((mean_score(&results).unwrap() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn pass_rate_empty_is_none() {
        assert_eq!(pass_rate(&[], 0.5), None);
    }

    #[test]
    fn pass_rate_computes_fraction() {
        let results = vec![make_result("a", 0.8), make_result("b", 0.3)];
        assert!((pass_rate(&results, 0.5).unwrap() - 0.5).abs() < 1e-10);
    }

    // =========================================================================
    // merge_rerun
    // =========================================================================

    #[test]
    fn merge_rerun_replaces_matching() {
        let parent = vec![make_result("tc-1", 0.5), make_result("tc-2", 0.6)];
        let rerun = vec![make_result("tc-1", 0.9)];
        let merged = merge_rerun_results(&parent, &rerun);
        assert_eq!(merged[0].aggregate_score, 0.9);
        assert_eq!(merged[1].aggregate_score, 0.6);
    }

    #[test]
    fn merge_rerun_preserves_non_rerun() {
        let parent = vec![make_result("tc-1", 0.5), make_result("tc-2", 0.6)];
        let rerun = vec![];
        let merged = merge_rerun_results(&parent, &rerun);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].aggregate_score, 0.5);
    }
}
