//! Algebraic law harnesses for `EvalScorer` implementations.
//!
//! # Laws Tested
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L2 | Determinism | `score(tc, out) == score(tc, out)` |
//! | L3 | Non-empty | `scored.component_scores` is non-empty |
//! | L4 | Bounded | `scored.aggregate ∈ [0,1]` and `∀r: r.score ∈ [0,1]` |
//! | L6 | Aggregate consistency | leaf: `aggregate == component_scores[0].score` |
//!
//! L1 (Purity) is structural — enforced by the trait requiring no `&mut self`.
//! L5 (Composition) is tested inline in `CompositeScorer` tests.
//!
//! # Usage
//!
//! ```ignore
//! use agent_fw_test::eval_scorer_laws;
//! use agent_fw_eval::{TrajectoryScorer, EvalScorer};
//!
//! fn test_my_scorer() {
//!     let scorer = TrajectoryScorer::default();
//!     eval_scorer_laws::test_all(&scorer);
//! }
//! ```

use agent_fw_core::TestCaseId;
use agent_fw_eval::types::{EvalTestCase, TrajectoryMode};
use agent_fw_eval::{EvalScorer, GroundTruth, RawSampleOutput};

/// Generate a set of diverse test fixtures for scorer law verification.
///
/// Ground truth is placed INSIDE the test case (where it belongs),
/// not as a separate parameter.
fn fixtures() -> Vec<(EvalTestCase, RawSampleOutput)> {
    vec![
        // Empty expected + empty actual (vacuous truth)
        (
            make_tc(vec![], TrajectoryMode::Unordered, None),
            RawSampleOutput::new(vec![]),
        ),
        // Perfect match (AnyOrder)
        (
            make_tc(vec!["a", "b"], TrajectoryMode::Unordered, None),
            RawSampleOutput::new(vec!["b".into(), "a".into()]),
        ),
        // Partial match (InOrder)
        (
            make_tc(vec!["a", "b", "c"], TrajectoryMode::Strict, None),
            RawSampleOutput::new(vec!["a".into(), "c".into()]),
        ),
        // Subset mode
        (
            make_tc(vec!["a", "b", "c"], TrajectoryMode::Subset, None),
            RawSampleOutput::new(vec!["a".into()]),
        ),
        // No overlap
        (
            make_tc(vec!["a", "b"], TrajectoryMode::Unordered, None),
            RawSampleOutput::new(vec!["c".into(), "d".into()]),
        ),
        // With ground truth text (inside the test case)
        (
            make_tc(
                vec!["a"],
                TrajectoryMode::Unordered,
                GroundTruth::text("expected output"),
            ),
            RawSampleOutput::new(vec!["a".into()]),
        ),
        // Duplicates in trajectory
        (
            make_tc(vec!["a", "a", "b"], TrajectoryMode::Unordered, None),
            RawSampleOutput::new(vec!["a".into(), "b".into(), "a".into()]),
        ),
    ]
}

fn make_tc(
    expected: Vec<&str>,
    mode: TrajectoryMode,
    ground_truth: Option<GroundTruth>,
) -> EvalTestCase {
    EvalTestCase {
        id: TestCaseId::new_unchecked("tc-law-test"),
        tags: vec![],
        input: "law test input".into(),
        expected_trajectory: expected.into_iter().map(String::from).collect(),
        trajectory_mode: mode,
        ground_truth,
        final_response: None,
        source_thread_id: None,
    }
}

/// Test L2: Determinism — same inputs produce same outputs.
pub fn test_determinism(scorer: &dyn EvalScorer) {
    for (tc, output) in &fixtures() {
        let r1 = scorer.score(tc, output);
        let r2 = scorer.score(tc, output);
        assert_eq!(
            r1.aggregate,
            r2.aggregate,
            "L2 Determinism: aggregate differs for scorer '{}'",
            scorer.name()
        );
        assert_eq!(
            r1.component_scores.len(),
            r2.component_scores.len(),
            "L2 Determinism: component count differs for scorer '{}'",
            scorer.name()
        );
        for (a, b) in r1.component_scores.iter().zip(r2.component_scores.iter()) {
            assert_eq!(
                a.score,
                b.score,
                "L2 Determinism: scores differ for scorer '{}'",
                scorer.name()
            );
            assert_eq!(
                a.scorer_name,
                b.scorer_name,
                "L2 Determinism: scorer names differ for scorer '{}'",
                scorer.name()
            );
        }
    }
}

/// Test L3: Non-empty — score always produces at least one component result.
///
/// `ScoredSample` always has an aggregate, and component_scores are populated
/// by construction. This exercises the scorer for panic-freedom under diverse inputs.
pub fn test_non_empty(scorer: &dyn EvalScorer) {
    for (tc, output) in &fixtures() {
        let scored = scorer.score(tc, output);
        // NonEmpty guarantees non-emptiness at the type level — L3 is
        // structurally enforced. We exercise the scorer for panic-freedom.
        let _ = &scored.component_scores;
    }
}

/// Test L4: Bounded — aggregate and all component scores in [0.0, 1.0].
pub fn test_bounded(scorer: &dyn EvalScorer) {
    for (tc, output) in &fixtures() {
        let scored = scorer.score(tc, output);
        assert!(
            scored.aggregate >= 0.0 && scored.aggregate <= 1.0,
            "L4 Bounded: scorer '{}' produced aggregate {} (expected [0, 1])",
            scorer.name(),
            scored.aggregate
        );
        for r in &scored.component_scores {
            assert!(
                r.score >= 0.0 && r.score <= 1.0,
                "L4 Bounded: scorer '{}' produced component score {} (expected [0, 1])",
                r.scorer_name,
                r.score
            );
        }
    }
}

/// Test L6: Aggregate Consistency — leaf aggregate matches the single component score.
///
/// For leaf scorers: `scored.aggregate == scored.component_scores[0].score`.
/// This law verifies that leaf constructors (`ScoredSample::leaf`,
/// `ScoredSample::leaf_with_details`) maintain the invariant.
///
/// Note: composite aggregate consistency is tested inline in `CompositeScorer`
/// tests, since it requires knowledge of children and weights.
pub fn test_aggregate_consistency(scorer: &dyn EvalScorer) {
    for (tc, output) in &fixtures() {
        let scored = scorer.score(tc, output);
        if scored.component_scores.len().get() == 1 {
            // Leaf scorer: aggregate must equal the single component score
            assert_eq!(
                scored.aggregate,
                scored.component_scores.first().score,
                "L6 Aggregate Consistency: leaf scorer '{}' has aggregate {} != component {}",
                scorer.name(),
                scored.aggregate,
                scored.component_scores.first().score
            );
        }
    }
}

/// Run L4 (bounded) with hegel-generated inputs.
///
/// Generates random expected trajectories + actual trajectories and verifies
/// that aggregate and all component scores are in [0.0, 1.0].
pub fn test_bounded_hegel(scorer: &dyn EvalScorer) {
    use hegel::generators;

    hegel::Hegel::new(|tc: hegel::TestCase| {
        let labels = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let expected: Vec<String> =
            tc.draw(generators::vecs(generators::sampled_from(labels.clone())).max_size(7));
        let actual: Vec<String> =
            tc.draw(generators::vecs(generators::sampled_from(labels)).max_size(7));
        let eval_tc = EvalTestCase {
            id: TestCaseId::new_unchecked("hegel-test"),
            tags: vec![],
            input: "hegel".into(),
            expected_trajectory: expected,
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };
        let output = RawSampleOutput::new(actual);
        let scored = scorer.score(&eval_tc, &output);

        assert!(
            scored.aggregate >= 0.0 && scored.aggregate <= 1.0,
            "L4 hegel: aggregate={}",
            scored.aggregate
        );
        for r in &scored.component_scores {
            assert!(
                r.score >= 0.0 && r.score <= 1.0,
                "L4 hegel: component={}",
                r.score
            );
        }
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// Run all scorer laws (L2-L4, L6) against the given implementation.
pub fn test_all(scorer: &dyn EvalScorer) {
    test_determinism(scorer);
    test_non_empty(scorer);
    test_bounded(scorer);
    test_aggregate_consistency(scorer);
    test_bounded_hegel(scorer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_eval::{CompositeScorer, TrajectoryScorer};

    #[test]
    fn trajectory_scorer_satisfies_laws() {
        let scorer = TrajectoryScorer::default();
        test_all(&scorer);
    }

    #[test]
    fn composite_scorer_satisfies_laws() {
        let scorer = CompositeScorer::trajectory_only();
        test_all(&scorer);
    }
}
