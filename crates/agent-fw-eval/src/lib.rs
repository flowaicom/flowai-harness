//! First-class evaluation system: scoring, test cases, ground truth, event bus.
//!
//! # Architecture
//!
//! This crate provides the framework-generic evaluation infrastructure.
//! Domain-specific scorers, action types, and tool catalogs are defined
//! by consuming crates.
//!
//! # Modules
//!
//! - [`types`] — Core eval types (EvalStatus, EvalConfig, TokenUsageSummary, etc.)
//! - [`scoring`] — Pure scoring functions (f-beta, jaccard, pass@k, trajectory)
//! - [`ground_truth`] — Generic ground truth envelope (Text / Structured)
//! - [`test_case`] — Test case authoring (AuthoredTestCase, TrajectoryStep, ToolCatalog)
//! - [`scorer`] — EvalScorer algebra + shipped interpreters
//! - [`event_bus`] — Broadcast event delivery for eval progress
//! - [`comparison`] — Run comparison utilities
//! - [`sample_executor`] — SampleExecutor trait for running samples against agents
//! - [`result_aggregator`] — ResultAggregator trait for aggregating eval results
//! - [`expand_token`] — Dynamic test case injection into running evals
//! - [`cost`] — Cache-aware cost estimation (CostEstimate commutative monoid, ModelPricing)
//! - [`trace`] — Durable trace and provenance types captured during eval runs

pub mod builder_session_store;
pub mod builder_tools;
pub mod comparison;
pub mod cost;
pub mod event_bus;
pub mod expand_token;
pub mod ground_truth;
pub mod orchestration_executor;
pub mod orchestrator;
pub mod result_aggregator;
pub mod sample_executor;
pub mod scorer;
pub mod scoring;
pub mod stream_capture;
pub mod test_case;
pub mod trace;
pub mod types;

// Re-export key types at crate root
pub use builder_session_store::{
    load_builder_session, load_or_create_builder_session, mutate_builder_session,
    save_builder_session, BuilderSessionFactoryFn, BuilderSessionKeyFn, BuilderSessionStore,
    BuilderSessionStoreConfig, MutateBuilderSessionError,
};
pub use builder_tools::{ComposedCatalog, DefaultBuilderCatalog, BUILDER_TOOLS};
pub use comparison::{
    compare_runs, compare_runs_at_threshold, compare_runs_with_thresholds, ComparisonOutcome,
    RunComparison, TestCaseComparison,
};
pub use cost::{
    estimate_eval_cost, estimate_profiling_cost, CostEstimate, ModelPricing, ModelPricingResolver,
    ProfilingCostEstimate, StaticPricingResolver,
};
pub use event_bus::{EvalEventBus, Sequenced, SequencedBus};
pub use expand_token::ExpandToken;
pub use ground_truth::GroundTruth;
pub use ground_truth::NonEmptyText;
pub use orchestration_executor::OrchestrationSampleExecutor;
pub use orchestrator::{EvalOrchestrator, EvalOrchestratorError, EvalPersistence, EvalPlan};
pub use result_aggregator::{
    rebuild_summary_from_results, ResultAggregator, RunAggInput, StandardAggregator,
    TestCaseAggInput, TestCaseAggOutput,
};
pub use sample_executor::StubSampleExecutor;
#[cfg(any(test, feature = "test-support"))]
pub use sample_executor::TimeoutSampleExecutor;
pub use sample_executor::{
    ResolvedModelConfig, SampleExecutionError, SampleExecutor, SampleInput,
    SampleOutput as SampleExecutorOutput,
};
pub use scorer::{
    CompositeError, CompositeScorer, EvalScorer, RawSampleOutput, ScoreWeights, ScoreWeightsError,
    ScoredSample, TrajectoryScorer, WeightedChild, WeightedChildError,
};
pub use scoring::{
    aggregate_scorer_results, f_beta_score, jaccard_similarity, pass_at_k_simple,
    pass_at_k_unbiased, ratio_score, score_trajectory, trajectory_score_details, ConfusionCounts,
    TrajectoryScoreDetails, TrajectorySimilarityDiagnostics,
};
pub use stream_capture::{CapturedToolCall, StreamCapture, StreamCaptureResult};
pub use test_case::{
    canonicalize_expected_trajectory, extract_tool_calls_from_messages,
    extract_tool_calls_from_raw_messages, manual_trajectory_steps, messages_to_pairs,
    validate_test_case_fields, AuthoredTestCase, BaselineTrajectory,
    ComposeAuthoredTestCaseOptions, FinalizeAuthoredTestCaseOptions, MessagePair,
    RemappedTrajectory, SessionSummary, TestCaseBuilderError, TestCaseBuilderSession,
    TestCaseStatus, TestCaseValidationError, ToolCallEntry, ToolCatalog, ToolCatalogEntry,
    TrajectoryCanonicalizationError, TrajectorySource, TrajectoryStep, TrajectoryStepSource,
    VecToolCatalog,
};
pub use trace::{
    RedactedPayload, TraceActor, TraceOmissionReason, TracePayload, TraceProvenance, TraceRecord,
    TraceRef, TraceScope, TraceSegmentRef, TraceStage, TraceStatus, TraceStep,
};
pub use types::{
    AgentCostBreakdown, AggregationStrategy, EvalConfig, EvalConfigError, EvalCostSummary,
    EvalEvent, EvalMode, EvalProgress, EvalRequestOverrides, EvalRetryConfig, EvalRun,
    EvalRunSummary, EvalStatus, EvalSummary, EvalTestCase, EvalThreadFork, FBetaScore,
    JaccardScore, PassAtKResult, SampleResult, ScorerResult, TestCaseResult, TestCaseSet,
    TestCaseSource, TestCaseState, TestCaseStateEntry, TokenUsageSummary, TokenUsageSummaryError,
    TrajectoryMode, ValidatedEvalConfig, ValidationIssue, ValidationResult, ValidationSeverity,
    MAX_CONCURRENCY, MAX_SAMPLES_PER_CASE, MAX_TEST_CASES,
};
