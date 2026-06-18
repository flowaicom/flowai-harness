//! Result aggregation algebra.
//!
//! # Laws
//!
//! - **L1 Purity**: No side effects
//! - **L2 Determinism**: Same inputs → same outputs
//! - **L3 Bounded**: All scores in [0.0, 1.0]
//! - **L4 Monoid**: Token usage combines correctly
//! - **L5 Consistency**: `num_correct <= num_samples`

use crate::cost::ModelPricing;
use crate::scoring::{pass_at_k_simple, pass_at_k_unbiased};
use crate::types::{
    AggregationStrategy, EvalConfig, EvalCostSummary, EvalSummary, PassAtKResult, TestCaseResult,
    TokenUsageSummary,
};
use agent_fw_core::{fold_latency, LatencySummary, TestCaseId};

/// Input for test-case-level aggregation.
#[derive(Debug, Clone)]
pub struct TestCaseAggInput {
    /// Test case ID.
    pub test_case_id: TestCaseId,
    /// Per-sample scores (one per sample).
    pub sample_scores: Vec<f64>,
    /// Per-sample pass/fail (one per sample).
    pub sample_passed: Vec<bool>,
    /// k values for pass@k computation.
    pub k_values: Vec<u32>,
    /// Number of samples per case (from config).
    pub samples_per_case: u32,
    /// Aggregation strategy.
    pub aggregation_strategy: AggregationStrategy,
}

/// Output from test-case-level aggregation.
#[derive(Debug, Clone)]
pub struct TestCaseAggOutput {
    /// pass@k results for each k.
    pub pass_at_k: Vec<PassAtKResult>,
    /// Aggregate score for this test case.
    pub aggregate_score: f64,
    /// Number of correct samples (passed).
    pub num_correct: u32,
}

/// Input for run-level aggregation.
#[derive(Debug, Clone)]
pub struct RunAggInput {
    /// Per-test-case aggregate scores.
    pub tc_aggregate_scores: Vec<f64>,
    /// Total token usage across all samples.
    pub total_usage: TokenUsageSummary,
    /// Total wall-clock duration in ms.
    pub total_duration_ms: u64,
    /// k values for pass@k.
    pub k_values: Vec<u32>,
    /// Pass threshold.
    pub pass_threshold: f64,
    /// Aggregation strategy.
    pub aggregation_strategy: AggregationStrategy,
    /// Optional cost summary supplied by the caller.
    pub cost: Option<EvalCostSummary>,
    /// Structured latency summaries collected across all samples.
    pub latency_summaries: Vec<LatencySummary>,
}

impl RunAggInput {
    pub fn from_test_case_results(
        results: &[TestCaseResult],
        config: &EvalConfig,
        cost: Option<EvalCostSummary>,
    ) -> Self {
        let tc_aggregate_scores: Vec<f64> = results.iter().map(|r| r.aggregate_score).collect();

        let total_usage = results
            .iter()
            .flat_map(|r| r.samples.iter())
            .fold(TokenUsageSummary::ZERO, |acc, s| {
                acc.combine(&s.token_usage)
            });

        let total_duration_ms: u64 = results
            .iter()
            .flat_map(|r| r.samples.iter())
            .map(|s| s.duration_ms)
            .sum();

        let latency_summaries = results
            .iter()
            .flat_map(|r| r.samples.iter())
            .filter_map(|s| s.latency.clone())
            .collect();

        Self {
            tc_aggregate_scores,
            total_usage,
            total_duration_ms,
            k_values: config.k_values.clone(),
            pass_threshold: config.pass_threshold,
            aggregation_strategy: config.aggregation_strategy,
            cost,
            latency_summaries,
        }
    }

    pub fn with_cost(mut self, cost: EvalCostSummary) -> Self {
        self.cost = Some(cost);
        self
    }

    pub fn with_single_agent_cost(
        mut self,
        agent_name: impl Into<String>,
        pricing: ModelPricing,
    ) -> Self {
        self.cost = Some(EvalCostSummary::from_single_agent_usage(
            agent_name,
            &self.total_usage,
            &pricing,
        ));
        self
    }

    pub fn with_latency_summaries(mut self, latency_summaries: Vec<LatencySummary>) -> Self {
        self.latency_summaries = latency_summaries;
        self
    }
}

pub fn rebuild_summary_from_results(
    results: &[TestCaseResult],
    config: &EvalConfig,
    cost: Option<EvalCostSummary>,
) -> EvalSummary {
    StandardAggregator.aggregate_run(&RunAggInput::from_test_case_results(results, config, cost))
}

/// Trait for aggregating eval results.
///
/// Synchronous and pure — no IO, no mutation.
pub trait ResultAggregator: Send + Sync {
    /// Aggregate sample results for a single test case.
    fn aggregate_test_case(&self, input: &TestCaseAggInput) -> TestCaseAggOutput;

    /// Aggregate test-case results into a run-level summary.
    fn aggregate_run(&self, input: &RunAggInput) -> EvalSummary;
}

/// Standard aggregator implementation.
///
/// Uses pass@k (simple + unbiased) and the configured `AggregationStrategy`.
pub struct StandardAggregator;

impl ResultAggregator for StandardAggregator {
    fn aggregate_test_case(&self, input: &TestCaseAggInput) -> TestCaseAggOutput {
        let n = input.sample_scores.len() as u32;
        let num_correct = input.sample_passed.iter().filter(|p| **p).count() as u32;

        let pass_at_k: Vec<PassAtKResult> = input
            .k_values
            .iter()
            .map(|&k| {
                let simple = pass_at_k_simple(n, num_correct, k);
                let unbiased = pass_at_k_unbiased(n, num_correct, k);
                PassAtKResult {
                    k,
                    simple_estimate: simple,
                    unbiased_estimate: unbiased,
                    num_samples: n,
                    num_correct,
                }
            })
            .collect();

        let aggregate_score = match input.aggregation_strategy {
            AggregationStrategy::PassRate => {
                if n == 0 {
                    0.0
                } else {
                    num_correct as f64 / n as f64
                }
            }
            AggregationStrategy::MeanScore => {
                if input.sample_scores.is_empty() {
                    0.0
                } else {
                    input.sample_scores.iter().sum::<f64>() / input.sample_scores.len() as f64
                }
            }
        };

        TestCaseAggOutput {
            pass_at_k,
            aggregate_score,
            num_correct,
        }
    }

    fn aggregate_run(&self, input: &RunAggInput) -> EvalSummary {
        let total_test_cases = input.tc_aggregate_scores.len() as u32;
        let passed = input
            .tc_aggregate_scores
            .iter()
            .filter(|s| **s >= input.pass_threshold)
            .count() as u32;
        let failed = total_test_cases.saturating_sub(passed);

        let aggregate_score = if total_test_cases == 0 {
            0.0
        } else {
            match input.aggregation_strategy {
                AggregationStrategy::PassRate => passed as f64 / total_test_cases as f64,
                AggregationStrategy::MeanScore => {
                    input.tc_aggregate_scores.iter().sum::<f64>() / total_test_cases as f64
                }
            }
        };

        let pass_at_k: Vec<PassAtKResult> = input
            .k_values
            .iter()
            .map(|&k| {
                let simple = pass_at_k_simple(total_test_cases, passed, k);
                let unbiased = pass_at_k_unbiased(total_test_cases, passed, k);
                PassAtKResult {
                    k,
                    simple_estimate: simple,
                    unbiased_estimate: unbiased,
                    num_samples: total_test_cases,
                    num_correct: passed,
                }
            })
            .collect();

        let latency = if input.latency_summaries.is_empty() {
            None
        } else {
            let summary = fold_latency(input.latency_summaries.iter());
            if summary == LatencySummary::zero() {
                None
            } else {
                Some(summary)
            }
        };

        EvalSummary {
            total_test_cases,
            passed,
            failed,
            skipped: 0,
            aggregate_score,
            pass_at_k,
            total_duration_ms: input.total_duration_ms,
            total_usage: input.total_usage.clone(),
            cost: input.cost.clone().unwrap_or_else(EvalCostSummary::zero),
            latency,
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tc_input(scores: Vec<f64>, threshold: f64) -> TestCaseAggInput {
        let passed: Vec<bool> = scores.iter().map(|s| *s >= threshold).collect();
        TestCaseAggInput {
            test_case_id: TestCaseId::new_unchecked("tc-1"),
            sample_scores: scores,
            sample_passed: passed,
            k_values: vec![1, 3],
            samples_per_case: 3,
            aggregation_strategy: AggregationStrategy::MeanScore,
        }
    }

    /// L2: Determinism
    #[test]
    fn deterministic() {
        let agg = StandardAggregator;
        let input = make_tc_input(vec![0.8, 0.6, 0.9], 0.7);
        let r1 = agg.aggregate_test_case(&input);
        let r2 = agg.aggregate_test_case(&input);
        assert_eq!(r1.aggregate_score, r2.aggregate_score);
        assert_eq!(r1.num_correct, r2.num_correct);
    }

    /// L3: Bounded
    #[test]
    fn bounded_scores() {
        let agg = StandardAggregator;
        let input = make_tc_input(vec![0.0, 0.5, 1.0], 0.7);
        let output = agg.aggregate_test_case(&input);
        assert!(output.aggregate_score >= 0.0 && output.aggregate_score <= 1.0);
        for pak in &output.pass_at_k {
            assert!(pak.simple_estimate >= 0.0 && pak.simple_estimate <= 1.0);
        }
    }

    /// L5: Consistency
    #[test]
    fn num_correct_leq_num_samples() {
        let agg = StandardAggregator;
        let input = make_tc_input(vec![0.8, 0.9, 1.0], 0.7);
        let output = agg.aggregate_test_case(&input);
        assert!(output.num_correct <= input.sample_scores.len() as u32);
    }

    #[test]
    fn pass_rate_strategy() {
        let agg = StandardAggregator;
        let input = TestCaseAggInput {
            test_case_id: TestCaseId::new_unchecked("tc-1"),
            sample_scores: vec![0.8, 0.3, 0.9],
            sample_passed: vec![true, false, true],
            k_values: vec![1],
            samples_per_case: 3,
            aggregation_strategy: AggregationStrategy::PassRate,
        };
        let output = agg.aggregate_test_case(&input);
        // 2 out of 3 passed
        assert!((output.aggregate_score - 2.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn mean_score_strategy() {
        let agg = StandardAggregator;
        let input = make_tc_input(vec![0.8, 0.6, 1.0], 0.7);
        let output = agg.aggregate_test_case(&input);
        assert!((output.aggregate_score - 0.8).abs() < 1e-10);
    }

    #[test]
    fn unbiased_estimate_populated() {
        let agg = StandardAggregator;
        let input = make_tc_input(vec![0.8, 0.6, 0.9], 0.7);
        let output = agg.aggregate_test_case(&input);
        let pak_1 = output.pass_at_k.iter().find(|p| p.k == 1).unwrap();
        assert!(pak_1.unbiased_estimate.is_some());
    }

    #[test]
    fn run_aggregation() {
        let agg = StandardAggregator;
        let input = RunAggInput {
            tc_aggregate_scores: vec![0.8, 0.5, 0.9],
            total_usage: TokenUsageSummary::new(1000, 500, 100, 0),
            total_duration_ms: 5000,
            k_values: vec![1],
            pass_threshold: 0.7,
            aggregation_strategy: AggregationStrategy::MeanScore,
            cost: Some(EvalCostSummary::from_single_agent_usage(
                "coordinator",
                &TokenUsageSummary::new(1000, 500, 100, 0),
                &ModelPricing::sonnet_4(),
            )),
            latency_summaries: vec![LatencySummary::zero()],
        };
        let summary = agg.aggregate_run(&input);
        assert_eq!(summary.total_test_cases, 3);
        assert_eq!(summary.passed, 2); // 0.8 and 0.9 >= 0.7
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.total_duration_ms, 5000);
        assert!(summary.cost.estimated_cost_usd > 0.0);
        assert!(summary.latency.is_none());
    }

    #[test]
    fn run_pass_rate_strategy_uses_passed_test_cases() {
        let agg = StandardAggregator;
        let input = RunAggInput {
            tc_aggregate_scores: vec![0.9, 0.2, 0.8],
            total_usage: TokenUsageSummary::ZERO,
            total_duration_ms: 1200,
            k_values: vec![1, 2],
            pass_threshold: 0.7,
            aggregation_strategy: AggregationStrategy::PassRate,
            cost: None,
            latency_summaries: vec![],
        };

        let summary = agg.aggregate_run(&input);
        assert!((summary.aggregate_score - 2.0 / 3.0).abs() < 1e-10);
        assert_eq!(summary.pass_at_k[0].num_samples, 3);
        assert_eq!(summary.pass_at_k[0].num_correct, 2);
    }

    #[test]
    fn run_agg_input_from_results_collects_usage_duration_and_latency() {
        let results = vec![TestCaseResult {
            test_case_id: TestCaseId::new_unchecked("tc-1"),
            input: Some("hello".into()),
            samples: vec![
                crate::types::SampleResult {
                    sample_index: 0,
                    passed: true,
                    scores: vec![],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 12,
                    token_usage: TokenUsageSummary::new(5, 7, 1, 0),
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: Some(LatencySummary {
                        total_duration_ms: 40,
                        phases: agent_fw_core::PhaseBreakdown::new(10, 20, 1)
                            .with_sub_agent_time(30),
                        ..LatencySummary::zero()
                    }),
                },
                crate::types::SampleResult {
                    sample_index: 1,
                    passed: false,
                    scores: vec![],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 8,
                    token_usage: TokenUsageSummary::new(2, 3, 0, 0),
                    error: None,
                    retry_count: 0,
                    thread_id: None,
                    trace: None,
                    metadata: None,
                    latency: None,
                },
            ],
            pass_at_k: vec![],
            aggregate_score: 0.75,
        }];

        let config = EvalConfig {
            k_values: vec![1, 3],
            pass_threshold: 0.7,
            aggregation_strategy: AggregationStrategy::MeanScore,
            ..Default::default()
        };

        let input = RunAggInput::from_test_case_results(&results, &config, None);

        assert_eq!(input.tc_aggregate_scores, vec![0.75]);
        assert_eq!(input.total_duration_ms, 20);
        assert_eq!(input.total_usage, TokenUsageSummary::new(7, 10, 1, 0));
        assert_eq!(input.latency_summaries.len(), 1);
    }

    #[test]
    fn rebuild_summary_from_results_uses_standard_aggregation() {
        let results = vec![
            TestCaseResult {
                test_case_id: TestCaseId::new_unchecked("tc-1"),
                input: Some("one".into()),
                samples: vec![],
                pass_at_k: vec![],
                aggregate_score: 0.9,
            },
            TestCaseResult {
                test_case_id: TestCaseId::new_unchecked("tc-2"),
                input: Some("two".into()),
                samples: vec![],
                pass_at_k: vec![],
                aggregate_score: 0.4,
            },
        ];

        let config = EvalConfig {
            k_values: vec![1],
            pass_threshold: 0.7,
            aggregation_strategy: AggregationStrategy::MeanScore,
            ..Default::default()
        };

        let summary = rebuild_summary_from_results(&results, &config, None);

        assert_eq!(summary.total_test_cases, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert!((summary.aggregate_score - 0.65).abs() < 1e-9);
        assert_eq!(summary.cost, EvalCostSummary::zero());
    }
}
