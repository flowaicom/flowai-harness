pub mod artifact;
pub mod sample_executor;

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agent_fw_core::{EvalRunId, TestCaseId, ThreadId, WorkspaceId};
use agent_fw_eval::{
    rebuild_summary_from_results, EvalMode, EvalScorer, EvalTestCase, RawSampleOutput,
    ResolvedModelConfig, SampleInput, SampleResult, ScoreWeights, TestCaseAggInput, TestCaseResult,
    TokenUsageSummary, TraceActor, TraceOmissionReason, TracePayload, TraceProvenance, TraceRecord,
    TraceScope, TraceStage, TraceStatus, TraceStep,
};
use agent_fw_eval::{ResultAggregator, StandardAggregator};
use chrono::{Duration as ChronoDuration, Utc};
use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::eval::presets::{
    add_default_final_response_weight, default_score_weights_for_preset_and_test_cases,
    materialize_score_weights, scorer_for_eval_test_case_with_config,
    validate_specialist_explicit_score_weights, PresetScorerError, PRESET_EXECUTOR, PRESET_PLANNER,
    PRESET_SEQUENTIAL, PRESET_SPECIALIST, PRESET_TEST_CASE_BUILDER, PRESET_TRAJECTORY_ONLY,
};
use crate::eval::{
    final_response_judge_results_extra, FinalResponseEvalSpec, FinalResponseScorerConfig,
    HarnessScorerConfig,
};

pub use artifact::{
    ArtifactMetadata, CostAgentBreakdown, EvalArtifact, EvalArtifactSummary, EvalRequest,
    EvalTraceRef, HarnessEvalEvent, HarnessEvalEventEnvelope, ModelInvocation, SampleArtifact,
    SampleCost, SampleLatency, SummaryCost, SummaryLatency, TestCaseArtifact,
};
pub use sample_executor::{
    CaptureSampleExecutor, FinalResponseJudgeCapture, RuntimeSampleCapture, RuntimeSampleExecutor,
};

use crate::Runtime;

type HarnessEventEmitter<'a> = dyn FnMut(HarnessEvalEventEnvelope) -> bool + Send + 'a;

pub type EvalEventStream = Pin<Box<dyn Stream<Item = HarnessEvalEventEnvelope> + Send + 'static>>;

#[derive(Debug)]
struct ScoredSampleCapture {
    sample: SampleResult,
    model_invocations: Vec<ModelInvocation>,
    response_text: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvalRunnerError {
    #[error("eval request tenant '{request}' does not match runtime tenant '{runtime}'")]
    TenantMismatch { request: String, runtime: String },
    #[error("eval mode '{0}' is not supported by the runtime eval runner")]
    ModeNotSupported(String),
    #[error("invalid scorer config: {0}")]
    Scorer(#[from] PresetScorerError),
    #[error("invalid score weights: {0}")]
    ScoreWeights(#[from] agent_fw_eval::ScoreWeightsError),
    #[error("sample execution failed: {0}")]
    SampleExecution(#[from] agent_fw_eval::SampleExecutionError),
    #[error("eval run was cancelled")]
    Cancelled,
}

pub struct EvalRunner {
    runtime: Arc<Runtime>,
}

impl EvalRunner {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }

    pub async fn run(&self, request: EvalRequest) -> Result<EvalArtifact, EvalRunnerError> {
        // Runtime owns tenant isolation directly. Workspace scope is recorded
        // on the artifact from EvalRequest; this runtime type does not yet
        // carry a workspace identity to assert against.
        if request.tenant_id != self.runtime.tenant {
            return Err(EvalRunnerError::TenantMismatch {
                request: request.tenant_id.as_str().to_string(),
                runtime: self.runtime.tenant.as_str().to_string(),
            });
        }

        run_with_capture_executor(
            request,
            RuntimeSampleExecutor::new(self.runtime.clone()),
            self.runtime.trace_sink().clone(),
        )
        .await
    }

    pub async fn run_with_events(
        &self,
        request: EvalRequest,
    ) -> Result<(EvalArtifact, Vec<HarnessEvalEventEnvelope>), EvalRunnerError> {
        // Runtime owns tenant isolation directly. Workspace scope is recorded
        // on the artifact from EvalRequest; this runtime type does not yet
        // carry a workspace identity to assert against.
        if request.tenant_id != self.runtime.tenant {
            return Err(EvalRunnerError::TenantMismatch {
                request: request.tenant_id.as_str().to_string(),
                runtime: self.runtime.tenant.as_str().to_string(),
            });
        }

        let mut events = Vec::new();
        let artifact = run_with_capture_executor_and_events(
            request,
            RuntimeSampleExecutor::new(self.runtime.clone()),
            self.runtime.trace_sink().clone(),
            Some(&mut |event| {
                events.push(event);
                true
            }),
        )
        .await?;
        Ok((artifact, events))
    }

    pub fn stream(self, request: EvalRequest) -> EvalEventStream {
        if request.tenant_id != self.runtime.tenant {
            return single_event_stream(HarnessEvalEventEnvelope {
                run_id: format!("eval-{}", Uuid::new_v4()),
                sequence: 0,
                event: HarnessEvalEvent::EvalFailed {
                    error: EvalRunnerError::TenantMismatch {
                        request: request.tenant_id.as_str().to_string(),
                        runtime: self.runtime.tenant.as_str().to_string(),
                    }
                    .to_string(),
                },
            });
        }

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let last_run_id = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
            let last_run_id_for_emit = last_run_id.clone();
            let tx_for_emit = tx.clone();
            let result = run_with_capture_executor_and_events(
                request,
                RuntimeSampleExecutor::new(self.runtime.clone()),
                self.runtime.trace_sink().clone(),
                Some(&mut |event| {
                    if let Ok(mut guard) = last_run_id_for_emit.lock() {
                        *guard = Some(event.run_id.clone());
                    }
                    tx_for_emit.send(event).is_ok()
                }),
            )
            .await;

            if let Err(error) = result {
                let run_id = last_run_id
                    .lock()
                    .ok()
                    .and_then(|guard| guard.clone())
                    .unwrap_or_else(|| format!("eval-{}", Uuid::new_v4()));
                let _ = tx.send(HarnessEvalEventEnvelope {
                    run_id,
                    sequence: 0,
                    event: HarnessEvalEvent::EvalFailed {
                        error: error.to_string(),
                    },
                });
            }
        });

        Box::pin(stream::unfold(rx, |mut rx| async {
            rx.recv().await.map(|event| (event, rx))
        }))
    }
}

async fn run_with_capture_executor<E: CaptureSampleExecutor>(
    request: EvalRequest,
    executor: E,
    trace_sink: crate::SharedTraceSink,
) -> Result<EvalArtifact, EvalRunnerError> {
    run_with_capture_executor_and_events(request, executor, trace_sink, None).await
}

async fn run_with_capture_executor_and_events<E: CaptureSampleExecutor>(
    request: EvalRequest,
    executor: E,
    trace_sink: crate::SharedTraceSink,
    mut emit: Option<&mut HarnessEventEmitter<'_>>,
) -> Result<EvalArtifact, EvalRunnerError> {
    if request.config.mode == EvalMode::TestCaseBuilder {
        return Err(EvalRunnerError::ModeNotSupported("testCaseBuilder".into()));
    }
    if request.scorer_preset.as_deref() == Some(PRESET_TRAJECTORY_ONLY) {
        return Err(EvalRunnerError::ModeNotSupported(
            "trajectory_only preset is only supported by Phase A score_sample".into(),
        ));
    }
    if let Some(preset) = request.scorer_preset.as_deref() {
        validate_scorer_preset_for_mode(preset, request.config.mode)?;
    }

    let run_id = format!("eval-{}", Uuid::new_v4());
    let mut sequence = 0;
    let preset = request
        .scorer_preset
        .clone()
        .unwrap_or_else(|| preset_for_mode(request.config.mode).to_string());
    let resolved_weights = resolve_score_weights(&request)?;
    let metadata = ArtifactMetadata::new(&preset, weights_to_map(&resolved_weights.weights));

    if !emit_envelope(
        emit.as_deref_mut(),
        &run_id,
        &mut sequence,
        HarnessEvalEvent::EvalStarted {
            artifact: empty_artifact_for_started_event(&request, &run_id, metadata.clone()),
        },
    ) {
        return Err(EvalRunnerError::Cancelled);
    }

    let model_config = ResolvedModelConfig {
        provider: request
            .config
            .provider
            .clone()
            .unwrap_or_else(|| "runtime".to_string()),
        model: request
            .config
            .model
            .clone()
            .unwrap_or_else(|| "runtime-default".to_string()),
    };
    let timeout = request
        .config
        .timeout_per_sample_secs
        .map(Duration::from_secs);
    let harness_scorer_config =
        HarnessScorerConfig::from_value(request.config.scorer_config.as_ref())
            .map_err(PresetScorerError::from)?;
    let aggregator = StandardAggregator;
    let mut test_case_results = Vec::with_capacity(request.test_cases.len());
    let mut test_case_artifacts = Vec::with_capacity(request.test_cases.len());

    for test_case in &request.test_cases {
        if !emit_envelope(
            emit.as_deref_mut(),
            &run_id,
            &mut sequence,
            HarnessEvalEvent::TestCaseStarted {
                test_case_id: test_case.id.as_str().to_string(),
            },
        ) {
            return Err(EvalRunnerError::Cancelled);
        }

        // Assemble the scorer per test case so an unpopulated action bucket
        // contributes no component (no freebie 1.0 dilution).
        let scorer = scorer_for_eval_test_case_with_config(
            &preset,
            resolved_weights.weights.clone(),
            test_case,
            request.config.scorer_config.as_ref(),
            !resolved_weights.drop_empty_trajectory,
        )?;

        let mut sample_results = Vec::with_capacity(request.config.samples_per_case as usize);
        let mut sample_invocations: BTreeMap<u32, Vec<ModelInvocation>> = BTreeMap::new();
        let mut sample_response_text: BTreeMap<u32, String> = BTreeMap::new();
        let concurrency = (request.config.concurrency as usize).max(1);
        let mut completed_samples = stream::iter(0..request.config.samples_per_case)
            .map(|sample_index| {
                let executor = &executor;
                let scorer = scorer.clone();
                let test_case = test_case.clone();
                let model_config = model_config.clone();
                let final_response_config = harness_scorer_config.final_response.clone();
                let run_id = run_id.clone();
                let workspace_id = request.workspace_id.clone();
                let trace_sink = trace_sink.clone();
                let mode = request.config.mode;
                let target_agent_id = request.config.target_agent_id.clone();
                let pass_threshold = request.config.pass_threshold;
                async move {
                    execute_and_score_sample(
                        executor,
                        scorer,
                        test_case,
                        sample_index,
                        mode,
                        target_agent_id,
                        run_id,
                        workspace_id,
                        trace_sink,
                        &model_config,
                        &final_response_config,
                        timeout,
                        pass_threshold,
                    )
                    .await
                }
            })
            .buffer_unordered(concurrency);

        while let Some(capture) = completed_samples.next().await {
            let sample_artifact = sample_artifact_from_result(
                &capture.sample,
                capture.model_invocations.clone(),
                capture.response_text.clone(),
            );
            if !emit_envelope(
                emit.as_deref_mut(),
                &run_id,
                &mut sequence,
                HarnessEvalEvent::SampleCompleted {
                    sample: sample_artifact,
                },
            ) {
                return Err(EvalRunnerError::Cancelled);
            }
            if let Some(response_text) = capture.response_text {
                sample_response_text.insert(capture.sample.sample_index, response_text);
            }
            sample_invocations.insert(capture.sample.sample_index, capture.model_invocations);
            sample_results.push(capture.sample);
        }
        sample_results.sort_by_key(|sample| sample.sample_index);

        let sample_scores: Vec<f64> = sample_results.iter().map(sample_aggregate_score).collect();
        let sample_passed: Vec<bool> = sample_results.iter().map(|sample| sample.passed).collect();
        let tc_agg = aggregator.aggregate_test_case(&TestCaseAggInput {
            test_case_id: test_case.id.clone(),
            sample_scores,
            sample_passed,
            k_values: request.config.k_values.clone(),
            samples_per_case: request.config.samples_per_case,
            aggregation_strategy: request.config.aggregation_strategy,
        });
        let tc_result = TestCaseResult {
            test_case_id: test_case.id.clone(),
            input: Some(test_case.input.clone()),
            samples: sample_results,
            pass_at_k: tc_agg.pass_at_k,
            aggregate_score: tc_agg.aggregate_score,
        };
        let tc_artifact = TestCaseArtifact::from_framework_result(&tc_result, |sample_index| {
            sample_invocations
                .get(&sample_index)
                .cloned()
                .unwrap_or_default()
        });
        let tc_artifact = TestCaseArtifact {
            samples: tc_artifact
                .samples
                .into_iter()
                .map(|mut sample| {
                    sample.response_text = sample_response_text
                        .get(&sample.sample_index)
                        .filter(|text| !text.is_empty())
                        .cloned();
                    sample
                })
                .collect(),
            ..tc_artifact
        };
        if !emit_envelope(
            emit.as_deref_mut(),
            &run_id,
            &mut sequence,
            HarnessEvalEvent::TestCaseCompleted {
                test_case: tc_artifact.clone(),
            },
        ) {
            return Err(EvalRunnerError::Cancelled);
        }

        test_case_results.push(tc_result);
        test_case_artifacts.push(tc_artifact);
    }

    let framework_summary = rebuild_summary_from_results(&test_case_results, &request.config, None);
    let mut summary = EvalArtifactSummary::from_framework_summary(&framework_summary);
    summary.cost = summary_cost_from_test_cases(&test_case_artifacts);

    let artifact = EvalArtifact {
        run_id,
        tenant_id: request.tenant_id,
        workspace_id: request.workspace_id,
        mode: request.config.mode,
        summary,
        test_cases: test_case_artifacts,
        metadata,
    };

    if !emit_envelope(
        emit.as_deref_mut(),
        &artifact.run_id,
        &mut sequence,
        HarnessEvalEvent::EvalCompleted {
            artifact: artifact.clone(),
        },
    ) {
        return Err(EvalRunnerError::Cancelled);
    }

    Ok(artifact)
}

fn emit_envelope(
    emit: Option<&mut HarnessEventEmitter<'_>>,
    run_id: &str,
    sequence: &mut u64,
    event: HarnessEvalEvent,
) -> bool {
    let Some(emit) = emit else {
        return true;
    };
    let envelope = HarnessEvalEventEnvelope {
        run_id: run_id.to_string(),
        sequence: *sequence,
        event,
    };
    *sequence += 1;
    emit(envelope)
}

fn single_event_stream(event: HarnessEvalEventEnvelope) -> EvalEventStream {
    Box::pin(stream::once(async move { event }))
}

fn failed_sample_result(
    sample_index: u32,
    error: agent_fw_eval::SampleExecutionError,
) -> SampleResult {
    SampleResult {
        sample_index,
        passed: false,
        scores: vec![],
        actual_trajectory: vec![],
        response_text: None,
        duration_ms: 0,
        token_usage: TokenUsageSummary::ZERO,
        error: Some(error.to_string()),
        retry_count: 0,
        thread_id: None,
        trace: None,
        metadata: Some(crate::eval::resolved_actions_extra(&[])),
        latency: None,
    }
}

async fn execute_and_score_sample<E: CaptureSampleExecutor>(
    executor: &E,
    scorer: Arc<dyn EvalScorer>,
    test_case: EvalTestCase,
    sample_index: u32,
    mode: EvalMode,
    target_agent_id: Option<String>,
    run_id: String,
    workspace_id: WorkspaceId,
    trace_sink: crate::SharedTraceSink,
    model_config: &ResolvedModelConfig,
    final_response_config: &FinalResponseScorerConfig,
    timeout: Option<Duration>,
    pass_threshold: f64,
) -> ScoredSampleCapture {
    match executor
        .execute_capture(
            SampleInput {
                test_case: test_case.clone(),
                sample_index,
                eval_mode: mode,
                target_agent_id,
                run_id: run_id.clone(),
            },
            model_config,
            timeout,
        )
        .await
    {
        Ok(capture) => {
            let trace = trace_record_for_sample(
                workspace_id.as_str(),
                &run_id,
                &test_case,
                sample_index,
                &capture.output,
            );
            if let Err(error) = trace_sink.record_trace(trace.clone()).await {
                tracing::warn!(
                    error = %error,
                    trace_id = %trace.trace_id,
                    "failed to persist eval sample trace"
                );
            }
            let mut extra = capture.output.extra.clone();
            let mut model_invocations = capture.model_invocations;
            let response_text = capture.response_text;
            if let Some(final_response_spec) = parsed_final_response_spec(&test_case) {
                let judge_capture = executor
                    .execute_final_response_judges(
                        &final_response_spec,
                        &response_text,
                        &judge_run_context(&test_case, sample_index),
                        model_config,
                        final_response_config,
                        timeout,
                    )
                    .await;
                if !judge_capture.results.is_empty() {
                    extra = Some(sample_executor::merge_extra_objects(
                        extra,
                        final_response_judge_results_extra(&judge_capture.results),
                    ));
                }
                model_invocations.extend(judge_capture.model_invocations);
            }
            let raw_output = RawSampleOutput {
                actual_trajectory: capture.output.actual_trajectory.clone(),
                response_text: Some(response_text.clone()),
                extra: extra.clone(),
            };
            let scored = scorer.score(&test_case, &raw_output);
            ScoredSampleCapture {
                sample: SampleResult {
                    sample_index,
                    passed: scored.aggregate >= pass_threshold,
                    scores: scored.component_scores.into_vec(),
                    actual_trajectory: capture.output.actual_trajectory,
                    response_text: Some(response_text.clone()),
                    duration_ms: capture.output.duration_ms,
                    token_usage: capture.output.token_usage,
                    error: capture.output.error,
                    retry_count: 0,
                    thread_id: capture.output.thread_id,
                    trace: Some(trace),
                    metadata: extra,
                    latency: capture.output.latency,
                },
                model_invocations,
                response_text: Some(response_text).filter(|text| !text.is_empty()),
            }
        }
        Err(error) => {
            let trace = failed_trace_record_for_sample(
                workspace_id.as_str(),
                &run_id,
                &test_case,
                sample_index,
                &error,
            );
            if let Err(sink_error) = trace_sink.record_trace(trace.clone()).await {
                tracing::warn!(
                    error = %sink_error,
                    trace_id = %trace.trace_id,
                    "failed to persist failed eval sample trace"
                );
            }
            let mut sample = failed_sample_result(sample_index, error);
            sample.trace = Some(trace);
            ScoredSampleCapture {
                sample,
                model_invocations: Vec::new(),
                response_text: None,
            }
        }
    }
}

fn sample_artifact_from_result(
    sample: &SampleResult,
    model_invocations: Vec<ModelInvocation>,
    response_text: Option<String>,
) -> SampleArtifact {
    let mut artifact = SampleArtifact::from_framework_result(sample, model_invocations);
    artifact.response_text = response_text.filter(|text| !text.is_empty());
    artifact
}

fn trace_record_for_sample(
    workspace_id: &str,
    run_id: &str,
    test_case: &EvalTestCase,
    sample_index: u32,
    output: &agent_fw_eval::SampleExecutorOutput,
) -> TraceRecord {
    let thread_id = output
        .thread_id
        .as_ref()
        .map(|id| ThreadId::new_unchecked(id.clone()));
    let steps = trace_steps_for_output(output);
    let status = match (output.error.is_some(), steps.is_empty()) {
        (false, _) => TraceStatus::Completed,
        (true, false) => TraceStatus::Partial,
        (true, true) => TraceStatus::Failed,
    };
    let completed_at = Utc::now();
    let started_at = ChronoDuration::from_std(Duration::from_millis(output.duration_ms))
        .ok()
        .and_then(|elapsed| completed_at.checked_sub_signed(elapsed));
    let eval_run_id = EvalRunId::new_unchecked(run_id.to_string());
    let test_case_id = TestCaseId::new_unchecked(test_case.id.as_str().to_string());

    TraceRecord {
        trace_id: format!("trace-{}", Uuid::new_v4()),
        workspace_id: WorkspaceId::new_unchecked(workspace_id.to_string()),
        stage: TraceStage::Runtime,
        status,
        scope: TraceScope {
            eval_run_id: Some(eval_run_id.clone()),
            test_case_id: Some(test_case_id.clone()),
            thread_id: thread_id.clone(),
            sample_index: Some(sample_index),
            ..TraceScope::default()
        },
        steps,
        started_at,
        completed_at: Some(completed_at),
        provenance: TraceProvenance::EvalSample {
            eval_run_id,
            test_case_id,
            sample_index,
            stage: TraceStage::Runtime,
            thread_id,
        },
    }
}

fn trace_steps_for_output(output: &agent_fw_eval::SampleExecutorOutput) -> Vec<TraceStep> {
    if !output.captured_tool_calls.is_empty() {
        return output
            .captured_tool_calls
            .iter()
            .enumerate()
            .map(|(ordinal, call)| call.to_trace_step(ordinal as u32))
            .collect();
    }

    output
        .actual_trajectory
        .iter()
        .enumerate()
        .map(|(ordinal, tool_name)| TraceStep {
            ordinal: ordinal as u32,
            actor: Some(TraceActor::Assistant),
            tool_name: tool_name.clone(),
            tool_call_id: None,
            arguments: TracePayload::omitted(TraceOmissionReason::NotCaptured),
            result: None,
            started_at: None,
            completed_at: None,
            error: None,
            correlation_id: None,
        })
        .collect()
}

fn failed_trace_record_for_sample(
    workspace_id: &str,
    run_id: &str,
    test_case: &EvalTestCase,
    sample_index: u32,
    error: &agent_fw_eval::SampleExecutionError,
) -> TraceRecord {
    let status = match error {
        agent_fw_eval::SampleExecutionError::TimedOut { .. } => TraceStatus::TimedOut,
        agent_fw_eval::SampleExecutionError::Cancelled => TraceStatus::Cancelled,
        agent_fw_eval::SampleExecutionError::AgentFailed(_)
        | agent_fw_eval::SampleExecutionError::Internal(_) => TraceStatus::Failed,
    };
    let eval_run_id = EvalRunId::new_unchecked(run_id.to_string());
    let test_case_id = TestCaseId::new_unchecked(test_case.id.as_str().to_string());
    let completed_at = Utc::now();

    TraceRecord {
        trace_id: format!("trace-{}", Uuid::new_v4()),
        workspace_id: WorkspaceId::new_unchecked(workspace_id.to_string()),
        stage: TraceStage::Runtime,
        status,
        scope: TraceScope {
            eval_run_id: Some(eval_run_id.clone()),
            test_case_id: Some(test_case_id.clone()),
            sample_index: Some(sample_index),
            ..TraceScope::default()
        },
        steps: vec![TraceStep {
            ordinal: 0,
            actor: Some(TraceActor::System),
            tool_name: "sampleExecution".to_string(),
            tool_call_id: None,
            arguments: TracePayload::omitted(TraceOmissionReason::NotCaptured),
            result: None,
            started_at: None,
            completed_at: Some(completed_at),
            error: Some(error.to_string()),
            correlation_id: None,
        }],
        started_at: None,
        completed_at: Some(completed_at),
        provenance: TraceProvenance::EvalSample {
            eval_run_id,
            test_case_id,
            sample_index,
            stage: TraceStage::Runtime,
            thread_id: None,
        },
    }
}

fn parsed_final_response_spec(test_case: &EvalTestCase) -> Option<FinalResponseEvalSpec> {
    test_case
        .final_response
        .as_ref()
        .and_then(|value| FinalResponseEvalSpec::from_value(value).ok())
}

fn judge_run_context(test_case: &EvalTestCase, sample_index: u32) -> serde_json::Value {
    serde_json::json!({
        "testCaseId": test_case.id.as_str(),
        "sampleIndex": sample_index,
        "input": test_case.input,
    })
}

fn empty_artifact_for_started_event(
    request: &EvalRequest,
    run_id: &str,
    metadata: ArtifactMetadata,
) -> EvalArtifact {
    EvalArtifact {
        run_id: run_id.to_string(),
        tenant_id: request.tenant_id.clone(),
        workspace_id: request.workspace_id.clone(),
        mode: request.config.mode,
        summary: EvalArtifactSummary {
            total_test_cases: request.test_cases.len() as u32,
            passed: 0,
            failed: 0,
            skipped: 0,
            aggregate_score: 0.0,
            pass_rate: 0.0,
            pass_at_k: vec![],
            total_duration_ms: 0,
            total_usage: TokenUsageSummary::ZERO,
            cost: None,
            latency: None,
            metadata: None,
        },
        test_cases: vec![],
        metadata,
    }
}

#[derive(Debug, Clone)]
struct ResolvedScoreWeights {
    weights: ScoreWeights,
    drop_empty_trajectory: bool,
}

fn resolve_score_weights(request: &EvalRequest) -> Result<ResolvedScoreWeights, EvalRunnerError> {
    let preset = request
        .scorer_preset
        .as_deref()
        .unwrap_or_else(|| preset_for_mode(request.config.mode));
    let default_executor_trajectory = preset == PRESET_EXECUTOR
        && request
            .test_cases
            .iter()
            .any(|case| !case.expected_trajectory.is_empty());

    let (weights, drop_empty_trajectory) = if let Some(weights) = &request.score_weights {
        let weights = ScoreWeights::new(weights.iter().map(|(k, v)| (k.clone(), *v)).collect())?;
        if preset == PRESET_SPECIALIST {
            validate_specialist_explicit_score_weights(&weights, &request.test_cases)?;
        }
        (weights, false)
    } else if let Some(weights) = &request.config.score_weights {
        if preset == PRESET_SPECIALIST {
            validate_specialist_explicit_score_weights(weights, &request.test_cases)?;
        }
        (weights.clone(), false)
    } else {
        let weights = default_score_weights_for_preset_and_test_cases(preset, &request.test_cases)?
            .expect("all runtime eval presets have default weights");
        if request
            .test_cases
            .iter()
            .any(|case| case.final_response.is_some())
        {
            (
                add_default_final_response_weight(weights)?,
                default_executor_trajectory,
            )
        } else {
            (weights, default_executor_trajectory)
        }
    };

    Ok(ResolvedScoreWeights {
        weights: materialize_score_weights(weights)?,
        drop_empty_trajectory,
    })
}

fn preset_for_mode(mode: EvalMode) -> &'static str {
    match mode {
        EvalMode::Planner => PRESET_PLANNER,
        EvalMode::Executor => PRESET_EXECUTOR,
        EvalMode::Sequential => PRESET_SEQUENTIAL,
        EvalMode::Specialist => PRESET_SPECIALIST,
        EvalMode::TestCaseBuilder => PRESET_TEST_CASE_BUILDER,
    }
}

fn validate_scorer_preset_for_mode(preset: &str, mode: EvalMode) -> Result<(), PresetScorerError> {
    match (mode, preset) {
        (EvalMode::Specialist, PRESET_SPECIALIST) => Ok(()),
        (EvalMode::Specialist, _) | (_, PRESET_SPECIALIST) => {
            Err(PresetScorerError::PresetNotAllowedForMode {
                preset: preset.to_string(),
                mode: eval_mode_name(mode),
            })
        }
        _ => Ok(()),
    }
}

fn eval_mode_name(mode: EvalMode) -> &'static str {
    match mode {
        EvalMode::Planner => "planner",
        EvalMode::Executor => "executor",
        EvalMode::Sequential => "sequential",
        EvalMode::Specialist => "specialist",
        EvalMode::TestCaseBuilder => "testCaseBuilder",
    }
}

fn weights_to_map(weights: &ScoreWeights) -> BTreeMap<String, f64> {
    weights
        .iter()
        .map(|(name, weight)| (name.clone(), *weight))
        .collect()
}

fn sample_aggregate_score(sample: &SampleResult) -> f64 {
    sample
        .scores
        .last()
        .map(|score| score.score)
        .unwrap_or(if sample.passed { 1.0 } else { 0.0 })
}

fn summary_cost_from_test_cases(test_cases: &[TestCaseArtifact]) -> Option<SummaryCost> {
    let mut per_agent: BTreeMap<(String, Option<String>, String), CostAgentBreakdown> =
        BTreeMap::new();
    let mut total = 0.0;
    let mut has_known_cost = false;

    for invocation in test_cases
        .iter()
        .flat_map(|tc| tc.samples.iter())
        .flat_map(|sample| sample.model_invocations.iter())
    {
        if let Some(cost) = invocation.estimated_cost_usd {
            has_known_cost = true;
            total += cost;
        }

        let key = (
            invocation.agent.clone(),
            invocation.provider.clone(),
            invocation.model.clone(),
        );
        let usage = agent_fw_eval::TokenUsageSummary::new(
            invocation.input_tokens,
            invocation.output_tokens,
            invocation.cached_tokens,
            invocation.cache_creation_tokens,
        );
        per_agent
            .entry(key)
            .and_modify(|existing| {
                existing.usage = existing.usage.combine(&usage);
                existing.estimated_cost_usd =
                    sum_optional_cost(existing.estimated_cost_usd, invocation.estimated_cost_usd);
            })
            .or_insert_with(|| CostAgentBreakdown {
                agent: invocation.agent.clone(),
                provider: invocation.provider.clone(),
                model: invocation.model.clone(),
                usage,
                estimated_cost_usd: invocation.estimated_cost_usd,
            });
    }

    has_known_cost.then_some(SummaryCost {
        estimated_cost_usd: total,
        per_agent: per_agent.into_values().collect(),
    })
}

fn sum_optional_cost(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use agent_fw_agent::{ChatInterpreter, ChatProgram};
    use agent_fw_algebra::CancellationToken;
    use agent_fw_core::tenant::TenantContext;
    use agent_fw_core::{FinishReason, StreamPart, TenantId, TestCaseId, TokenUsage, WorkspaceId};
    use agent_fw_eval::{
        EvalConfig, EvalTestCase, SampleExecutionError, SampleExecutorOutput, TestCaseSource,
        TokenUsageSummary, TrajectoryMode,
    };
    use futures::{stream, Stream};

    use crate::{
        AgentRole, AgentSpec, ModelSpec, ProviderConfig, RecordingTraceSink, RuntimeDeps,
        RuntimeSpec, TenantIdentity, TraceSink, TraceStatus,
    };

    struct NoopInterpreter;

    impl ChatInterpreter for NoopInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(10, 5, 0, 0)),
            ]))
        }
    }

    struct ScriptedCaptureExecutor;

    #[async_trait::async_trait]
    impl CaptureSampleExecutor for ScriptedCaptureExecutor {
        async fn execute_capture(
            &self,
            input: SampleInput,
            _model_config: &ResolvedModelConfig,
            _timeout: Option<Duration>,
        ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
            Ok(RuntimeSampleCapture {
                output: SampleExecutorOutput {
                    actual_trajectory: vec![],
                    captured_tool_calls: vec![],
                    duration_ms: 10,
                    token_usage: TokenUsageSummary::new(10, 5, 0, 0),
                    error: None,
                    thread_id: Some(format!("thread-{}", input.sample_index)),
                    extra: Some(crate::eval::resolved_actions_extra(&[])),
                    latency: None,
                },
                model_invocations: vec![ModelInvocation {
                    agent: "coordinator".to_string(),
                    provider: Some("anthropic".to_string()),
                    model: "claude-sonnet-4-6".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                    cached_tokens: 0,
                    cache_creation_tokens: 0,
                    estimated_cost_usd: Some(0.02),
                }],
                response_text: format!("sample {} final response", input.sample_index),
            })
        }
    }

    struct ScriptedJudgeExecutor;

    #[async_trait::async_trait]
    impl CaptureSampleExecutor for ScriptedJudgeExecutor {
        async fn execute_capture(
            &self,
            input: SampleInput,
            model_config: &ResolvedModelConfig,
            timeout: Option<Duration>,
        ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
            ScriptedCaptureExecutor
                .execute_capture(input, model_config, timeout)
                .await
        }

        async fn execute_final_response_judges(
            &self,
            _spec: &FinalResponseEvalSpec,
            response_text: &str,
            run_context: &serde_json::Value,
            model_config: &ResolvedModelConfig,
            _final_response_config: &crate::eval::FinalResponseScorerConfig,
            _timeout: Option<Duration>,
        ) -> FinalResponseJudgeCapture {
            assert_eq!(response_text, "sample 0 final response");
            assert_eq!(run_context["testCaseId"], serde_json::json!("tc-1"));
            assert_eq!(run_context["sampleIndex"], serde_json::json!(0));
            FinalResponseJudgeCapture {
                results: BTreeMap::from([(
                    "judge_similarity".to_string(),
                    crate::eval::JudgeResponseScoringData::new(crate::eval::JudgeResponseVerdict {
                        passed: true,
                        selected_rubric_score: 1,
                        reason: "The response matches the reference.".to_string(),
                    })
                    .with_judge_run(crate::eval::JudgeRunMetadata {
                        schema_version: 1,
                        provider: model_config.provider.clone(),
                        model: model_config.model.clone(),
                        prompt_sha256: "p".repeat(64),
                        context_sha256: "c".repeat(64),
                    }),
                )]),
                model_invocations: vec![ModelInvocation {
                    agent: "judge".to_string(),
                    provider: Some(model_config.provider.clone()),
                    model: model_config.model.clone(),
                    input_tokens: 20,
                    output_tokens: 6,
                    cached_tokens: 0,
                    cache_creation_tokens: 0,
                    estimated_cost_usd: Some(0.01),
                }],
            }
        }
    }

    struct FailingSecondSampleExecutor;

    #[async_trait::async_trait]
    impl CaptureSampleExecutor for FailingSecondSampleExecutor {
        async fn execute_capture(
            &self,
            input: SampleInput,
            model_config: &ResolvedModelConfig,
            timeout: Option<Duration>,
        ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
            if input.sample_index == 1 {
                return Err(SampleExecutionError::TimedOut {
                    timeout: Duration::from_millis(5),
                });
            }
            ScriptedCaptureExecutor
                .execute_capture(input, model_config, timeout)
                .await
        }
    }

    struct ConcurrencyTrackingExecutor {
        current: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
    }

    impl ConcurrencyTrackingExecutor {
        fn new() -> Self {
            Self {
                current: Arc::new(AtomicUsize::new(0)),
                max_seen: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl CaptureSampleExecutor for ConcurrencyTrackingExecutor {
        async fn execute_capture(
            &self,
            input: SampleInput,
            model_config: &ResolvedModelConfig,
            timeout: Option<Duration>,
        ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
            let in_flight = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(in_flight, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);

            ScriptedCaptureExecutor
                .execute_capture(input, model_config, timeout)
                .await
        }
    }

    fn runtime() -> Arc<Runtime> {
        let mut providers = BTreeMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new(serde_json::json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
        );
        let mut spec = RuntimeSpec {
            tenant: TenantIdentity::new("tenant-1", "v1"),
            agents: vec![
                AgentSpec::new(
                    "coordinator",
                    AgentRole::Coordinator,
                    ModelSpec::new("claude-sonnet-4-6"),
                    "Coordinate.",
                ),
                AgentSpec::new(
                    "planner",
                    AgentRole::Planner,
                    ModelSpec::new("claude-sonnet-4-6"),
                    "Plan.",
                ),
            ],
            references: vec![],
            plans: vec![],
            toolkits: vec![],
            approval_policies: Default::default(),
            approval_overrides: Default::default(),
            storage_factories: Default::default(),
            providers,
        };
        spec.agents[0].routes = vec!["planner".to_string()];

        Arc::new(
            Runtime::new(
                spec,
                RuntimeDeps::new(
                    Arc::new(NoopInterpreter),
                    Arc::new(agent_fw_algebra::testing::NullEventSink),
                    TenantContext::new(TenantId::new_unchecked("tenant-1")),
                    Arc::new(agent_fw_interpreter::DashMapKVStore::new()),
                ),
            )
            .expect("runtime should build"),
        )
    }

    fn request(tenant: &str) -> EvalRequest {
        EvalRequest {
            tenant_id: TenantId::new_unchecked(tenant),
            workspace_id: WorkspaceId::new("workspace-main").expect("workspace id"),
            config: EvalConfig {
                mode: EvalMode::Sequential,
                test_case_source: TestCaseSource::Set("inline".to_string()),
                samples_per_case: 1,
                pass_threshold: 0.5,
                concurrency: 1,
                k_values: vec![1],
                timeout_per_sample_secs: Some(5),
                ..Default::default()
            },
            test_cases: vec![EvalTestCase {
                id: TestCaseId::new_unchecked("tc-1"),
                tags: vec![],
                input: "hello".to_string(),
                expected_trajectory: vec![],
                trajectory_mode: TrajectoryMode::Unordered,
                ground_truth: None,
                final_response: None,
                source_thread_id: None,
            }],
            scorer_preset: None,
            score_weights: None,
        }
    }

    fn noop_trace_sink() -> crate::SharedTraceSink {
        Arc::new(crate::NoopTraceSink)
    }

    #[tokio::test]
    async fn eval_runner_builds_artifact_with_model_invocations() {
        let trace_sink = Arc::new(RecordingTraceSink::new());
        let artifact = run_with_capture_executor(
            request("tenant-1"),
            ScriptedCaptureExecutor,
            trace_sink.clone(),
        )
        .await
        .expect("eval should run");

        assert_eq!(artifact.tenant_id.as_str(), "tenant-1");
        assert_eq!(artifact.workspace_id.as_str(), "workspace-main");
        assert_eq!(artifact.mode, EvalMode::Sequential);
        assert_eq!(artifact.test_cases.len(), 1);
        assert_eq!(artifact.test_cases[0].samples.len(), 1);
        assert_eq!(artifact.test_cases[0].pass_at_k[0].k, 1);
        assert_eq!(
            artifact.metadata.score_weights.get("trajectory").copied(),
            Some(0.5)
        );
        assert_eq!(
            artifact
                .metadata
                .score_weights
                .get("planned_actions")
                .copied(),
            Some(0.25)
        );
        assert_eq!(
            artifact
                .metadata
                .score_weights
                .get("executed_actions")
                .copied(),
            Some(0.25)
        );
        let sample = &artifact.test_cases[0].samples[0];
        assert_eq!(
            sample.response_text.as_deref(),
            Some("sample 0 final response")
        );
        assert_eq!(sample.resolved_actions, vec![]);
        assert!(!sample.model_invocations.is_empty());
        assert_eq!(sample.model_invocations[0].agent, "coordinator");
        assert_eq!(
            sample.model_invocations[0].provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            artifact
                .summary
                .cost
                .as_ref()
                .expect("summary cost")
                .estimated_cost_usd,
            0.02
        );
        let trace_ref = sample.trace.as_ref().expect("sample trace ref");
        let trace = trace_sink
            .get_trace(&trace_ref.trace_id)
            .await
            .expect("trace sink should read")
            .expect("sample trace should be stored");
        assert_eq!(
            trace.scope.eval_run_id.as_ref().map(|id| id.as_str()),
            Some(artifact.run_id.as_str())
        );
        assert_eq!(
            trace.scope.test_case_id.as_ref().map(|id| id.as_str()),
            Some("tc-1")
        );
        assert_eq!(trace.scope.sample_index, Some(0));
        assert_eq!(
            trace.scope.thread_id.as_ref().map(|id| id.as_str()),
            Some("thread-0")
        );
    }

    #[tokio::test]
    async fn eval_runner_scores_authored_final_response() {
        let mut request = request("tenant-1");
        request.test_cases[0].final_response = Some(serde_json::json!({
            "scorers": [
                {
                    "id": "mentions_sample",
                    "method": "contains",
                    "text": "sample 0 final response"
                }
            ],
            "passThreshold": 1.0
        }));

        let artifact =
            run_with_capture_executor(request, ScriptedCaptureExecutor, noop_trace_sink())
                .await
                .expect("eval should run");

        let final_response_weight = artifact
            .metadata
            .score_weights
            .get(crate::eval::SCORER_FINAL_RESPONSE)
            .copied()
            .expect("final_response weight");
        assert!((final_response_weight - (1.0 / 3.0)).abs() < 1e-10);

        let sample = &artifact.test_cases[0].samples[0];
        let names: Vec<&str> = sample
            .component_scores
            .iter()
            .map(|score| score.scorer_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                crate::eval::SCORER_TRAJECTORY,
                crate::eval::SCORER_FINAL_RESPONSE,
                "composite"
            ]
        );
        let final_response_eval = sample
            .final_response_eval
            .as_ref()
            .expect("final response eval details");
        assert_eq!(final_response_eval["passed"], serde_json::json!(true));
        assert_eq!(
            final_response_eval["responseScorers"][0]["id"],
            serde_json::json!("mentions_sample")
        );
    }

    #[tokio::test]
    async fn eval_runner_uses_specialist_preset_for_specialist_mode() {
        let mut request = request("tenant-1");
        request.config.mode = EvalMode::Specialist;
        request.config.target_agent_id = Some("insights".to_string());
        request.test_cases[0].final_response = Some(serde_json::json!({
            "scorers": [
                {
                    "id": "mentions_sample",
                    "method": "contains",
                    "text": "sample 0 final response"
                }
            ],
            "passThreshold": 1.0
        }));

        let artifact =
            run_with_capture_executor(request, ScriptedCaptureExecutor, noop_trace_sink())
                .await
                .expect("specialist eval should run");

        assert_eq!(artifact.mode, EvalMode::Specialist);
        assert_eq!(artifact.metadata.scorer_preset, PRESET_SPECIALIST);
        assert_eq!(
            artifact
                .metadata
                .score_weights
                .get(crate::eval::SCORER_FINAL_RESPONSE)
                .copied(),
            Some(1.0)
        );
        let names: Vec<&str> = artifact.test_cases[0].samples[0]
            .component_scores
            .iter()
            .map(|score| score.scorer_name.as_str())
            .collect();
        assert_eq!(names, vec![crate::eval::SCORER_FINAL_RESPONSE, "composite"]);
    }

    #[tokio::test]
    async fn eval_runner_rejects_specialist_without_scoreable_expectations() {
        let mut request = request("tenant-1");
        request.config.mode = EvalMode::Specialist;
        request.config.target_agent_id = Some("insights".to_string());

        let error = run_with_capture_executor(request, ScriptedCaptureExecutor, noop_trace_sink())
            .await
            .expect_err("empty specialist expectations should be rejected");

        assert!(
            error.to_string().contains("has no scoreable expectations"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn eval_runner_rejects_specialist_final_response_weight_without_spec() {
        let mut request = request("tenant-1");
        request.config.mode = EvalMode::Specialist;
        request.config.target_agent_id = Some("insights".to_string());
        request.config.score_weights = Some(
            ScoreWeights::new(vec![(crate::eval::SCORER_FINAL_RESPONSE.to_string(), 1.0)])
                .expect("weights should build"),
        );

        let error = run_with_capture_executor(request, ScriptedCaptureExecutor, noop_trace_sink())
            .await
            .expect_err("specialist final_response scorer should require finalResponse");

        assert!(
            error.to_string().contains("requires test case 'tc-1'"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn eval_runner_scores_judge_final_response_from_precomputed_verdict() {
        let mut request = request("tenant-1");
        request.config.model = Some("claude-sonnet-4-6".to_string());
        request.config.provider = Some("anthropic".to_string());
        request.test_cases[0].final_response = Some(serde_json::json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "sample 0 final response"
                }
            ],
            "passThreshold": 1.0
        }));

        let artifact = run_with_capture_executor(request, ScriptedJudgeExecutor, noop_trace_sink())
            .await
            .expect("eval should run");

        let sample = &artifact.test_cases[0].samples[0];
        let final_response_eval = sample
            .final_response_eval
            .as_ref()
            .expect("final response eval details");
        assert_eq!(final_response_eval["passed"], serde_json::json!(true));
        assert_eq!(
            final_response_eval["responseScorers"][0]["details"]["verdict"]
                ["selected_rubric_score"],
            serde_json::json!(1)
        );
        assert_eq!(
            final_response_eval["responseScorers"][0]["details"]["judgeRun"]["provider"],
            serde_json::json!("anthropic")
        );
        assert_eq!(
            final_response_eval["responseScorers"][0]["details"]["judgeRun"]["promptSha256"]
                .as_str()
                .expect("prompt hash")
                .len(),
            64
        );
        assert!(sample
            .model_invocations
            .iter()
            .any(|invocation| invocation.agent == "judge"));
        assert_eq!(
            artifact
                .summary
                .cost
                .as_ref()
                .expect("summary cost")
                .estimated_cost_usd,
            0.03
        );
    }

    #[tokio::test]
    async fn eval_runner_emits_contract_event_sequence() {
        let mut events = Vec::new();
        let artifact = run_with_capture_executor_and_events(
            request("tenant-1"),
            ScriptedCaptureExecutor,
            noop_trace_sink(),
            Some(&mut |event| {
                events.push(event);
                true
            }),
        )
        .await
        .expect("eval should run");

        assert_eq!(events.len(), 5);
        assert!(events.iter().all(|event| event.run_id == artifact.run_id));
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert!(matches!(
            events[0].event,
            HarnessEvalEvent::EvalStarted { .. }
        ));
        match &events[0].event {
            HarnessEvalEvent::EvalStarted { artifact } => {
                assert_eq!(artifact.tenant_id.as_str(), "tenant-1");
                assert_eq!(artifact.workspace_id.as_str(), "workspace-main");
                assert_eq!(artifact.metadata.scorer_preset, "sequential");
                assert_eq!(
                    artifact.metadata.score_weights.get("trajectory").copied(),
                    Some(0.5)
                );
            }
            other => panic!("expected evalStarted event, got {other:?}"),
        }
        assert!(matches!(
            events[1].event,
            HarnessEvalEvent::TestCaseStarted { .. }
        ));
        assert!(matches!(
            events[2].event,
            HarnessEvalEvent::SampleCompleted { .. }
        ));
        assert!(matches!(
            events[3].event,
            HarnessEvalEvent::TestCaseCompleted { .. }
        ));
        assert!(matches!(
            events[4].event,
            HarnessEvalEvent::EvalCompleted { .. }
        ));
    }

    #[tokio::test]
    async fn eval_runner_records_sample_errors_and_continues() {
        let mut request = request("tenant-1");
        request.config.samples_per_case = 3;
        let trace_sink = Arc::new(RecordingTraceSink::new());

        let artifact =
            run_with_capture_executor(request, FailingSecondSampleExecutor, trace_sink.clone())
                .await
                .expect("sample error should not abort eval");

        let samples = &artifact.test_cases[0].samples;
        assert_eq!(samples.len(), 3);
        assert!(samples[0].passed);
        assert!(!samples[1].passed);
        assert!(samples[2].passed);
        assert!(samples[1]
            .error
            .as_deref()
            .expect("failed sample error")
            .contains("timed out"));
        assert_eq!(samples[1].resolved_actions, vec![]);
        assert!(samples[1].model_invocations.is_empty());
        let failed_trace_id = samples[1]
            .trace
            .as_ref()
            .expect("failed sample should carry a trace ref")
            .trace_id
            .clone();
        let failed_trace = trace_sink
            .get_trace(&failed_trace_id)
            .await
            .expect("trace sink should read")
            .expect("failed trace should be recorded");
        assert_eq!(failed_trace.status, TraceStatus::TimedOut);
        assert_eq!(failed_trace.steps[0].tool_name, "sampleExecution");
        assert!(failed_trace.steps[0]
            .error
            .as_deref()
            .expect("failed trace should include error")
            .contains("timed out"));
    }

    #[tokio::test]
    async fn eval_runner_respects_configured_sample_concurrency() {
        let mut request = request("tenant-1");
        request.config.samples_per_case = 4;
        request.config.concurrency = 2;
        let executor = ConcurrencyTrackingExecutor::new();
        let max_seen = executor.max_seen.clone();

        let artifact = run_with_capture_executor(request, executor, noop_trace_sink())
            .await
            .expect("eval should run");

        assert_eq!(artifact.test_cases[0].samples.len(), 4);
        assert_eq!(max_seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn eval_runner_stops_when_event_consumer_cancels() {
        let mut request = request("tenant-1");
        request.config.samples_per_case = 3;
        let executor = ConcurrencyTrackingExecutor::new();
        let max_seen = executor.max_seen.clone();
        let mut emitted = 0;

        let err = run_with_capture_executor_and_events(
            request,
            executor,
            noop_trace_sink(),
            Some(&mut |_event| {
                emitted += 1;
                false
            }),
        )
        .await
        .expect_err("cancelled event consumer should stop eval");

        assert!(matches!(err, EvalRunnerError::Cancelled));
        assert_eq!(emitted, 1);
        assert_eq!(max_seen.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn eval_runner_rejects_tenant_mismatch_before_running_samples() {
        let err = EvalRunner::new(runtime())
            .run(request("different-tenant"))
            .await
            .expect_err("tenant mismatch should fail");

        assert!(matches!(err, EvalRunnerError::TenantMismatch { .. }));
    }

    #[tokio::test]
    async fn eval_runner_rejects_trajectory_only_full_eval_preset() {
        let mut request = request("tenant-1");
        request.scorer_preset = Some(PRESET_TRAJECTORY_ONLY.to_string());

        let err = run_with_capture_executor(request, ScriptedCaptureExecutor, noop_trace_sink())
            .await
            .expect_err("trajectory_only is score_sample-only");

        assert!(matches!(err, EvalRunnerError::ModeNotSupported(_)));
    }
}
