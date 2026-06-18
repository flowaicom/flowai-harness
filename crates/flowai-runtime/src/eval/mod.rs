pub mod action_ground_truth;
pub mod action_projection;
pub mod presets;
pub mod response_eval;
pub mod runner;
pub mod scorer_config;
pub mod trajectory_projection;

pub use action_ground_truth::{
    extract_executed_expected_actions, extract_planned_expected_actions, normalize_ground_truth,
    ActionGroundTruth, ActionPayloadMatchMode, ExpectedAction, GroundTruthNormalizationError,
};
pub use action_projection::{
    extract_planned_actions, extract_planned_actions_from_sample, extract_resolved_actions,
    extract_resolved_actions_from_sample, planned_actions_extra, project_from_captured_tool_calls,
    project_planned_from_captured_tool_calls, resolved_actions_extra, ResolvedAction,
};
pub use presets::{
    default_score_weights_for_preset, default_score_weights_for_preset_and_test_cases,
    materialize_score_weights, scorer_for_eval_test_case_with_config, scorer_for_mode,
    scorer_for_preset, scorer_for_test_case, scorer_for_test_case_with_config,
    validate_specialist_explicit_score_weights, ActionMatchResult, ActionScorer, ActionSource,
    ActionStatus, ComparisonSummary, HarnessTrajectoryScorer, PresetScorerError,
    DEFAULT_EXECUTOR_ACTION_WEIGHT, DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT,
    DEFAULT_PLANNED_ACTION_WEIGHT, DEFAULT_SEQUENTIAL_ACTION_WEIGHT, DEFAULT_TRAJECTORY_WEIGHT,
    PRESET_EXECUTOR, PRESET_PLANNER, PRESET_SEQUENTIAL, PRESET_SPECIALIST,
    PRESET_TEST_CASE_BUILDER, PRESET_TRAJECTORY_ONLY, SCORER_EXECUTED_ACTIONS,
    SCORER_PLANNED_ACTIONS, SCORER_TRAJECTORY,
};
pub use response_eval::{
    build_judge_prompt, default_judge_rubric, final_response_judge_results_extra,
    final_response_judge_results_from_extra, final_response_judge_verdicts_extra,
    final_response_judge_verdicts_from_extra, judge_context_for_hash, sha256_hex,
    stable_json_sha256, FinalResponseEvalError, FinalResponseEvalSpec, FinalResponseScorer,
    JudgeResponseErrorKind, JudgeResponseScoringData, JudgeResponseVerdict,
    JudgeResponseVerdictError, JudgeRunMetadata, JudgeTrace, ResponseScorerMethod,
    ResponseScorerSpec, DEFAULT_FINAL_RESPONSE_WEIGHT, FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY,
    SCORER_FINAL_RESPONSE,
};
pub use runner::{
    ArtifactMetadata, CostAgentBreakdown, EvalArtifact, EvalArtifactSummary, EvalEventStream,
    EvalRequest, EvalRunner, EvalRunnerError, EvalTraceRef, HarnessEvalEvent,
    HarnessEvalEventEnvelope, ModelInvocation, RuntimeSampleExecutor, SampleArtifact, SampleCost,
    SampleLatency, SummaryCost, SummaryLatency, TestCaseArtifact,
};
pub use scorer_config::{FinalResponseScorerConfig, HarnessScorerConfig, TrajectoryScorerConfig};
pub use trajectory_projection::{
    project_trajectory, trajectory_events_extra, TrajectoryEvent, TrajectoryEventCapture,
    TrajectoryEventKind, TrajectoryProjection, TRAJECTORY_EVENTS_EXTRA_KEY,
};
