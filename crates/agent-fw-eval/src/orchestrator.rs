//! Eval orchestrator — composes all eval primitives into a runnable pipeline.
//!
//! # Architecture (programs-as-values)
//!
//! [`EvalPlan`] (value) describes WHAT: which test cases, how many samples, which scorer.
//! [`EvalOrchestrator`] (interpreter) decides HOW: concurrency, progress, persistence.
//!
//! # Laws
//!
//! - **L1 (Progress monotonicity)**: `completed_samples` only increases
//! - **L2 (Completion)**: Returns iff all samples complete or pipeline cancelled
//! - **L3 (Result persistence)**: Every scored sample is persisted before next starts
//! - **L4 (Cancellation)**: Cancelled token stops new samples, awaits in-flight

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::trace::{
    TraceOmissionReason, TracePayload, TraceProvenance, TraceRecord, TraceScope, TraceStage,
    TraceStatus, TraceStep,
};
use agent_fw_algebra::{CancellationToken, KVStore, PauseToken};
use agent_fw_core::{EvalRunId, TenantId, ThreadId, WorkspaceId};
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

use crate::cost::{ModelPricing, ModelPricingResolver, StaticPricingResolver};
use crate::event_bus::EvalEventBus;
use crate::result_aggregator::{ResultAggregator, RunAggInput, TestCaseAggInput};
use crate::sample_executor::{
    ResolvedModelConfig, SampleExecutionError, SampleExecutor, SampleInput, SampleOutput,
};
use crate::scorer::{EvalScorer, RawSampleOutput};
use crate::types::{
    EvalEvent, EvalProgress, EvalRun, EvalRunSummary, EvalStatus, EvalTestCase, SampleResult,
    TestCaseResult, TestCaseState, TestCaseStateEntry, TokenUsageSummary, ValidatedEvalConfig,
};

// =============================================================================
// EvalPersistence — local trait to break workspace → eval cycle
// =============================================================================

/// Persistence layer for the eval orchestrator.
///
/// Captures the subset of workspace operations the orchestrator needs.
/// Defined here (not in `agent-fw-workspace`) to avoid a circular dependency.
/// Consumers implement this for their concrete store.
#[async_trait::async_trait]
pub trait EvalPersistence: Send + Sync {
    /// Persist a new eval run.
    async fn persist_run(
        &self,
        tenant: &TenantId,
        run: &EvalRun,
    ) -> Result<(), EvalOrchestratorError>;

    /// Update the status of an eval run.
    async fn update_status(
        &self,
        tenant: &TenantId,
        run_id: &str,
        status: &EvalStatus,
    ) -> Result<(), EvalOrchestratorError>;

    /// Persist a test case result.
    async fn persist_test_case_result(
        &self,
        tenant: &TenantId,
        run_id: &str,
        result: &TestCaseResult,
    ) -> Result<(), EvalOrchestratorError>;
}

// =============================================================================
// EvalPlan — pure value describing WHAT to run
// =============================================================================

/// Configuration for an eval run. Consumed by the orchestrator.
///
/// This is a value (programs-as-values): it describes WHAT to run.
/// The orchestrator's `run()` method takes it by value (moved, not mutated).
/// Execution state lives in the orchestrator's internal `EvalRunState`.
#[derive(Clone)]
pub struct EvalPlan {
    /// The eval run to execute.
    pub run: EvalRun,
    /// Test cases to evaluate.
    pub test_cases: Vec<EvalTestCase>,
    /// Scorer graph to apply to every sample in the run.
    pub scorer: Arc<dyn EvalScorer>,
    /// Model configuration for sample execution.
    pub model_config: ResolvedModelConfig,
    /// Validated eval configuration.
    pub config: ValidatedEvalConfig,
    /// Tenant context for persistence.
    pub tenant: TenantId,
}

impl std::fmt::Debug for EvalPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalPlan")
            .field("run", &self.run)
            .field("test_cases", &self.test_cases)
            .field("scorer", &self.scorer.name())
            .field("model_config", &self.model_config)
            .field("config", &self.config)
            .field("tenant", &self.tenant)
            .finish()
    }
}

// =============================================================================
// EvalOrchestratorError
// =============================================================================

/// Errors from the eval orchestrator.
#[derive(Debug, thiserror::Error)]
pub enum EvalOrchestratorError {
    #[error("eval was cancelled")]
    Cancelled,
    #[error("persistence error: {0}")]
    Persistence(String),
    #[error("internal error: {0}")]
    Internal(String),
}

// =============================================================================
// EvalOrchestrator
// =============================================================================

/// Eval orchestrator — composes all eval primitives into a runnable pipeline.
///
/// Constructed with the primitives it needs, then `run(plan)` executes the eval.
pub struct EvalOrchestrator {
    executor: Arc<dyn SampleExecutor>,
    aggregator: Arc<dyn ResultAggregator>,
    event_bus: Arc<EvalEventBus>,
    kv: Arc<dyn KVStore>,
    persistence: Option<Arc<dyn EvalPersistence>>,
    pricing_resolver: Option<Arc<dyn ModelPricingResolver>>,
    cancel: CancellationToken,
    pause: PauseToken,
}

impl EvalOrchestrator {
    /// Create a new eval orchestrator.
    pub fn new(
        executor: Arc<dyn SampleExecutor>,
        aggregator: Arc<dyn ResultAggregator>,
        event_bus: Arc<EvalEventBus>,
        kv: Arc<dyn KVStore>,
        cancel: CancellationToken,
        pause: PauseToken,
    ) -> Self {
        Self {
            executor,
            aggregator,
            event_bus,
            kv,
            persistence: None,
            pricing_resolver: None,
            cancel,
            pause,
        }
    }

    /// Set the optional workspace persistence layer (dual-write).
    pub fn with_persistence(mut self, persistence: Arc<dyn EvalPersistence>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Set the optional model-pricing resolver used to compute eval cost.
    ///
    /// No fallback pricing is applied. If the resolver cannot resolve a model,
    /// the run still completes but the summary cost remains the zero value.
    pub fn with_pricing_resolver(
        mut self,
        pricing_resolver: Arc<dyn ModelPricingResolver>,
    ) -> Self {
        self.pricing_resolver = Some(pricing_resolver);
        self
    }

    /// Convenience for supplying externally managed pricing entries directly.
    ///
    /// This is the default low-ceremony path for applications that refresh
    /// model costs outside the framework and hand the current table in here.
    pub fn with_model_pricing(mut self, pricing: impl IntoIterator<Item = ModelPricing>) -> Self {
        self.pricing_resolver = Some(Arc::new(StaticPricingResolver::new(pricing)));
        self
    }

    /// Run an evaluation to completion.
    ///
    /// Consumes the plan (moved, not mutated). Execution state lives in
    /// the internal `EvalRunState`.
    ///
    /// Returns the run summary on success, or an error if cancelled/failed.
    pub async fn run(&self, plan: EvalPlan) -> Result<EvalRunSummary, EvalOrchestratorError> {
        let config = plan.config.inner();
        let start = Instant::now();
        let scorer = plan.scorer.clone();

        // Separate plan (value) from execution state (mutable).
        // EvalPlan is consumed here; all subsequent mutations target `state`.
        struct EvalRunState {
            run: EvalRun,
            tc_states: BTreeMap<String, TestCaseState>,
        }
        let mut state = EvalRunState {
            run: plan.run,
            tc_states: plan
                .test_cases
                .iter()
                .map(|tc| (tc.id.as_str().to_string(), TestCaseState::Queued))
                .collect(),
        };

        // Step 1: Persist initial run (dual-write: KV primary, workspace best-effort)
        state.run.status = EvalStatus::Queued;
        self.persist_run_kv(&plan.tenant, &state.run).await;
        if let Some(ref ws) = self.persistence {
            if let Err(e) = ws.persist_run(&plan.tenant, &state.run).await {
                tracing::warn!(error = %e, "failed to persist initial eval run to workspace");
            }
        }

        // Emit started event
        self.event_bus.emit(EvalEvent::Started {
            run_id: state.run.id.clone(),
            config: config.clone(),
        });

        // Update status to Running
        let total_samples = plan.test_cases.len() as u32 * config.samples_per_case;
        let progress = EvalProgress {
            completed_samples: 0,
            total_samples,
            completed_test_cases: 0,
            total_test_cases: plan.test_cases.len() as u32,
            current_test_case_id: None,
            elapsed_ms: 0,
            estimated_remaining_ms: None,
            test_case_states: state
                .tc_states
                .iter()
                .map(|(id, s)| TestCaseStateEntry {
                    test_case_id: id.clone(),
                    state: s.clone(),
                })
                .collect(),
        };
        state.run.status = EvalStatus::Running {
            progress: progress.clone(),
        };
        self.persist_status_kv(&plan.tenant, &state.run).await;

        // Step 2: Execute test cases with bounded concurrency
        // Atomic counters: the current loop is sequential, but these are designed
        // to remain correct when upgraded to JoinSet concurrency.
        let completed_samples = Arc::new(AtomicU32::new(0));
        let completed_test_cases = Arc::new(AtomicU32::new(0));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency as usize));
        let timeout = config.timeout_per_sample_secs.map(Duration::from_secs);

        let mut tc_results: Vec<TestCaseResult> = Vec::with_capacity(plan.test_cases.len());

        for (tc_idx, test_case) in plan.test_cases.iter().enumerate() {
            let tc_id = test_case.id.as_str().to_string();

            // L4: Check cancellation before starting next test case
            if self.cancel.is_cancelled() {
                state.run.status = EvalStatus::Cancelled;
                self.persist_status_kv(&plan.tenant, &state.run).await;
                self.event_bus.emit(EvalEvent::Error {
                    message: "eval cancelled".into(),
                });
                return Err(EvalOrchestratorError::Cancelled);
            }

            // Cooperative pause check — select against cancellation to avoid deadlock
            // when pause is active and cancel is triggered simultaneously.
            tokio::select! {
                _ = self.pause.wait_if_paused() => {},
                _ = self.cancel.cancelled() => {
                    state.run.status = EvalStatus::Cancelled;
                    self.persist_status_kv(&plan.tenant, &state.run).await;
                    self.event_bus.emit(EvalEvent::Error {
                        message: "eval cancelled while paused".into(),
                    });
                    return Err(EvalOrchestratorError::Cancelled);
                },
            }

            // Update test case state to Running
            let tc_start = Instant::now();
            state.tc_states.insert(
                tc_id.clone(),
                TestCaseState::Running {
                    started_at_ms: start.elapsed().as_millis() as u64,
                    completed_samples: 0,
                    total_samples: config.samples_per_case,
                },
            );

            // Emit test case started
            self.event_bus.emit(EvalEvent::TestCaseStarted {
                test_case_id: test_case.id.clone(),
                test_case_index: tc_idx as u32,
            });

            // Execute samples for this test case
            let mut samples: Vec<SampleResult> =
                Vec::with_capacity(config.samples_per_case as usize);

            for sample_idx in 0..config.samples_per_case {
                // Check cancellation between samples
                if self.cancel.is_cancelled() {
                    break;
                }

                // Acquire semaphore permit for bounded concurrency
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|_| EvalOrchestratorError::Internal("semaphore closed".into()))?;

                // Execute sample
                let sample_input = SampleInput {
                    test_case: test_case.clone(),
                    sample_index: sample_idx,
                    eval_mode: config.mode,
                    target_agent_id: config.target_agent_id.clone(),
                    run_id: state.run.id.as_str().to_string(),
                };

                let sample_result = self
                    .execute_and_score_sample(
                        sample_input,
                        &plan.tenant,
                        &plan.model_config,
                        timeout,
                        &scorer,
                        test_case,
                        config.pass_threshold,
                    )
                    .await;

                // L1: Progress monotonicity — only increment
                let total_completed = completed_samples.fetch_add(1, Ordering::Relaxed) + 1;

                // Update test case state: increment completed_samples
                if let Some(TestCaseState::Running {
                    completed_samples: cs,
                    ..
                }) = state.tc_states.get_mut(&tc_id)
                {
                    *cs = sample_idx + 1;
                }

                // Emit sample complete
                self.event_bus.emit(EvalEvent::SampleComplete {
                    test_case_id: test_case.id.clone(),
                    sample: sample_result.clone(),
                });

                // Emit progress
                self.event_bus.emit(EvalEvent::SampleProgress {
                    test_case_id: test_case.id.clone(),
                    sample_index: sample_idx,
                    completed_samples: sample_idx + 1,
                    total_samples: config.samples_per_case,
                });

                // Update running progress
                let elapsed = start.elapsed().as_millis() as u64;
                let estimated_remaining = if total_completed > 0 {
                    let rate = elapsed as f64 / total_completed as f64;
                    let remaining = total_samples.saturating_sub(total_completed);
                    Some((rate * remaining as f64) as u64)
                } else {
                    None
                };

                self.event_bus.emit(EvalEvent::Progress {
                    progress: EvalProgress {
                        completed_samples: total_completed,
                        total_samples,
                        completed_test_cases: completed_test_cases.load(Ordering::Relaxed),
                        total_test_cases: plan.test_cases.len() as u32,
                        current_test_case_id: Some(tc_id.clone()),
                        elapsed_ms: elapsed,
                        estimated_remaining_ms: estimated_remaining,
                        test_case_states: state
                            .tc_states
                            .iter()
                            .map(|(id, s)| TestCaseStateEntry {
                                test_case_id: id.clone(),
                                state: s.clone(),
                            })
                            .collect(),
                    },
                });

                samples.push(sample_result);
            }

            // Aggregate test case results
            // Each SampleResult carries a pre-computed aggregate via ScoredSample.
            // For error samples (empty scores), fall back to pass/fail as 1.0/0.0.
            let sample_scores: Vec<f64> = samples
                .iter()
                .map(|s| {
                    if s.scores.is_empty() {
                        if s.passed {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        // Use the last score (composite aggregate) for backward compat
                        // with the ScoredSample convention: composite appends aggregate last.
                        s.scores.last().map(|sr| sr.score).unwrap_or(0.0)
                    }
                })
                .collect();
            let sample_passed: Vec<bool> = samples.iter().map(|s| s.passed).collect();

            let tc_agg_input = TestCaseAggInput {
                test_case_id: test_case.id.clone(),
                sample_scores,
                sample_passed,
                k_values: config.k_values.clone(),
                samples_per_case: config.samples_per_case,
                aggregation_strategy: config.aggregation_strategy,
            };
            let tc_agg_output = self.aggregator.aggregate_test_case(&tc_agg_input);

            let tc_result = TestCaseResult {
                test_case_id: test_case.id.clone(),
                input: Some(test_case.input.clone()),
                samples,
                pass_at_k: tc_agg_output.pass_at_k,
                aggregate_score: tc_agg_output.aggregate_score,
            };

            // Update test case state to Completed
            state.tc_states.insert(
                tc_id.clone(),
                TestCaseState::Completed {
                    duration_ms: tc_start.elapsed().as_millis() as u64,
                    passed: tc_agg_output.aggregate_score >= config.pass_threshold,
                    aggregate_score: tc_agg_output.aggregate_score,
                },
            );

            // L3: Persist before moving to next test case
            self.persist_tc_result_kv(&plan.tenant, &state.run, &tc_result)
                .await;
            if let Some(ref ws) = self.persistence {
                if let Err(e) = ws
                    .persist_test_case_result(&plan.tenant, state.run.id.as_str(), &tc_result)
                    .await
                {
                    tracing::warn!(error = %e, test_case_id = %tc_id, "failed to persist test case result to workspace");
                }
            }

            // Emit test case complete
            self.event_bus.emit(EvalEvent::TestCaseComplete {
                result: tc_result.clone(),
            });

            completed_test_cases.fetch_add(1, Ordering::Relaxed);
            tc_results.push(tc_result);
        }

        // Step 3: Aggregate run-level results
        let total_usage = tc_results
            .iter()
            .flat_map(|tc| tc.samples.iter())
            .fold(TokenUsageSummary::ZERO, |acc, s| {
                acc.combine(&s.token_usage)
            });

        let tc_aggregate_scores: Vec<f64> =
            tc_results.iter().map(|tc| tc.aggregate_score).collect();
        let latency_summaries = tc_results
            .iter()
            .flat_map(|tc| tc.samples.iter())
            .filter_map(|sample| sample.latency.clone())
            .collect();

        let mut run_agg_input = RunAggInput {
            tc_aggregate_scores,
            total_usage,
            total_duration_ms: start.elapsed().as_millis() as u64,
            k_values: config.k_values.clone(),
            pass_threshold: config.pass_threshold,
            aggregation_strategy: config.aggregation_strategy,
            cost: None,
            latency_summaries: vec![],
        }
        .with_latency_summaries(latency_summaries);
        if let Some(pricing_resolver) = &self.pricing_resolver {
            if let Some(pricing) = pricing_resolver.resolve_model_pricing(&plan.model_config.model)
            {
                run_agg_input = run_agg_input.with_single_agent_cost("coordinator", pricing);
            }
        }
        let summary = self.aggregator.aggregate_run(&run_agg_input);

        // Step 4: Finalize
        state.run.status = EvalStatus::Completed {
            summary: summary.clone(),
        };
        state.run.results = tc_results;
        state.run.updated_at = chrono::Utc::now().to_rfc3339();
        self.persist_run_kv(&plan.tenant, &state.run).await;
        if let Some(ref ws) = self.persistence {
            if let Err(e) = ws
                .update_status(&plan.tenant, state.run.id.as_str(), &state.run.status)
                .await
            {
                tracing::warn!(error = %e, "failed to update final eval status in workspace");
            }
        }

        // Emit completed
        self.event_bus.emit(EvalEvent::Completed {
            summary: summary.clone(),
        });

        Ok(state.run.to_summary())
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Execute a single sample and score it.
    async fn execute_and_score_sample(
        &self,
        input: SampleInput,
        tenant: &TenantId,
        model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
        scorer: &Arc<dyn EvalScorer>,
        test_case: &EvalTestCase,
        pass_threshold: f64,
    ) -> SampleResult {
        let sample_index = input.sample_index;
        let trace_input = input.clone();

        match self.executor.execute(input, model_config, timeout).await {
            Ok(output) => {
                // Preserve domain-specific sample metadata for harness scorers
                // that score structured runtime output, such as resolved actions.
                let raw_output = match output.extra.clone() {
                    Some(extra) => {
                        RawSampleOutput::with_extra(output.actual_trajectory.clone(), extra)
                    }
                    None => RawSampleOutput::new(output.actual_trajectory.clone()),
                };

                let scored = scorer.score(test_case, &raw_output);
                let trace = trace_record_for_output(tenant, &trace_input, &output);

                SampleResult {
                    sample_index,
                    passed: scored.aggregate >= pass_threshold,
                    scores: scored.component_scores.into_vec(),
                    actual_trajectory: output.actual_trajectory,
                    response_text: None,
                    duration_ms: output.duration_ms,
                    token_usage: output.token_usage,
                    error: output.error,
                    retry_count: 0,
                    thread_id: output.thread_id,
                    trace: Some(trace),
                    metadata: output.extra,
                    latency: output.latency,
                }
            }
            Err(e) => {
                // Execution failed — produce a zero-score result
                let error_msg = match &e {
                    SampleExecutionError::TimedOut { timeout } => {
                        format!("timed out after {}ms", timeout.as_millis())
                    }
                    SampleExecutionError::Cancelled => "cancelled".into(),
                    other => other.to_string(),
                };

                SampleResult {
                    sample_index,
                    passed: false,
                    scores: vec![],
                    actual_trajectory: vec![],
                    response_text: None,
                    duration_ms: 0,
                    token_usage: TokenUsageSummary::ZERO,
                    error: Some(error_msg),
                    retry_count: 0,
                    thread_id: None,
                    trace: Some(trace_record_for_error(tenant, &trace_input, &e)),
                    metadata: None,
                    latency: None,
                }
            }
        }
    }

    /// Persist run to KV (primary store, best-effort).
    async fn persist_run_kv(&self, tenant: &TenantId, run: &EvalRun) {
        let key = format!("eval:run:{}", run.id.as_str());
        match serde_json::to_value(run) {
            Ok(value) => {
                if let Err(e) = self.kv.put_json(tenant.as_str(), &key, value, None).await {
                    tracing::warn!(error = %e, key = %key, "failed to persist eval run to KV");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, key = %key, "failed to serialize eval run");
            }
        }
    }

    /// Persist run status update to KV.
    async fn persist_status_kv(&self, tenant: &TenantId, run: &EvalRun) {
        // For KV, we just overwrite the whole run (KV is primary)
        self.persist_run_kv(tenant, run).await;
    }

    /// Persist test case result to KV.
    async fn persist_tc_result_kv(
        &self,
        tenant: &TenantId,
        run: &EvalRun,
        result: &TestCaseResult,
    ) {
        let key = format!(
            "eval:result:{}:{}",
            run.id.as_str(),
            result.test_case_id.as_str()
        );
        match serde_json::to_value(result) {
            Ok(value) => {
                if let Err(e) = self.kv.put_json(tenant.as_str(), &key, value, None).await {
                    tracing::warn!(error = %e, key = %key, "failed to persist test case result to KV");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, key = %key, "failed to serialize test case result");
            }
        }
    }
}

fn trace_record_for_output(
    tenant: &TenantId,
    input: &SampleInput,
    output: &SampleOutput,
) -> TraceRecord {
    let eval_run_id = EvalRunId::new_unchecked(input.run_id.clone());
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

    TraceRecord {
        trace_id: format!("trace-{}", Uuid::new_v4()),
        workspace_id: workspace_id_for_tenant(tenant),
        stage: TraceStage::Runtime,
        status,
        scope: TraceScope {
            eval_run_id: Some(eval_run_id.clone()),
            test_case_id: Some(input.test_case.id.clone()),
            thread_id: thread_id.clone(),
            sample_index: Some(input.sample_index),
            ..TraceScope::default()
        },
        steps,
        started_at,
        completed_at: Some(completed_at),
        provenance: TraceProvenance::EvalSample {
            eval_run_id,
            test_case_id: input.test_case.id.clone(),
            sample_index: input.sample_index,
            stage: TraceStage::Runtime,
            thread_id,
        },
    }
}

fn trace_record_for_error(
    tenant: &TenantId,
    input: &SampleInput,
    error: &SampleExecutionError,
) -> TraceRecord {
    let eval_run_id = EvalRunId::new_unchecked(input.run_id.clone());
    let status = match error {
        SampleExecutionError::TimedOut { .. } => TraceStatus::TimedOut,
        SampleExecutionError::Cancelled => TraceStatus::Cancelled,
        SampleExecutionError::AgentFailed(_) | SampleExecutionError::Internal(_) => {
            TraceStatus::Failed
        }
    };

    TraceRecord {
        trace_id: format!("trace-{}", Uuid::new_v4()),
        workspace_id: workspace_id_for_tenant(tenant),
        stage: TraceStage::Runtime,
        status,
        scope: TraceScope {
            eval_run_id: Some(eval_run_id.clone()),
            test_case_id: Some(input.test_case.id.clone()),
            sample_index: Some(input.sample_index),
            ..TraceScope::default()
        },
        steps: Vec::new(),
        started_at: None,
        completed_at: Some(Utc::now()),
        provenance: TraceProvenance::EvalSample {
            eval_run_id,
            test_case_id: input.test_case.id.clone(),
            sample_index: input.sample_index,
            stage: TraceStage::Runtime,
            thread_id: None,
        },
    }
}

fn trace_steps_for_output(output: &SampleOutput) -> Vec<TraceStep> {
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
            actor: None,
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

fn workspace_id_for_tenant(tenant: &TenantId) -> WorkspaceId {
    WorkspaceId::new(tenant.as_str()).unwrap_or_else(WorkspaceId::default_workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result_aggregator::StandardAggregator;
    use crate::sample_executor::{StubSampleExecutor, TimeoutSampleExecutor};
    use crate::scorer::{CompositeScorer, EvalScorer, RawSampleOutput, ScoredSample};
    use crate::stream_capture::CapturedToolCall;
    use crate::types::TrajectoryMode;
    use agent_fw_core::{TestCaseId, ThreadId};
    use agent_fw_interpreter::DashMapKVStore;
    use serde_json::json;

    struct CapturingSampleExecutor;

    #[async_trait::async_trait]
    impl SampleExecutor for CapturingSampleExecutor {
        async fn execute(
            &self,
            _input: SampleInput,
            _model_config: &ResolvedModelConfig,
            _timeout: Option<Duration>,
        ) -> Result<SampleOutput, SampleExecutionError> {
            Ok(SampleOutput {
                actual_trajectory: vec!["draft_plan".into()],
                captured_tool_calls: vec![CapturedToolCall {
                    tool: "draft_plan".into(),
                    tool_call_id: Some("tool-call-1".into()),
                    args: json!({ "goal": "ship" }),
                    result: Some(json!({ "ok": true })),
                }],
                duration_ms: 42,
                token_usage: TokenUsageSummary::ZERO,
                error: None,
                thread_id: Some("thread-rich-1".into()),
                extra: Some(json!({ "note": "preserved" })),
                latency: None,
            })
        }
    }

    struct ExtraSampleExecutor;

    #[async_trait::async_trait]
    impl SampleExecutor for ExtraSampleExecutor {
        async fn execute(
            &self,
            _input: SampleInput,
            _model_config: &ResolvedModelConfig,
            _timeout: Option<Duration>,
        ) -> Result<SampleOutput, SampleExecutionError> {
            Ok(SampleOutput {
                actual_trajectory: vec!["executePlan".into()],
                captured_tool_calls: vec![],
                duration_ms: 10,
                token_usage: TokenUsageSummary::ZERO,
                error: None,
                thread_id: Some("thread-extra-1".into()),
                extra: Some(json!({
                    "resolvedActions": [{
                        "type": "price_change",
                        "changeType": "absolute",
                        "value": 10.0
                    }]
                })),
                latency: None,
            })
        }
    }

    #[derive(Debug)]
    struct ExtraAssertingScorer;

    impl EvalScorer for ExtraAssertingScorer {
        fn score(&self, _test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
            let has_expected_extra = output
                .extra
                .as_ref()
                .and_then(|extra| extra.get("resolvedActions"))
                .and_then(|actions| actions.as_array())
                .is_some_and(|actions| !actions.is_empty());

            ScoredSample::leaf(self.name(), if has_expected_extra { 1.0 } else { 0.0 })
        }

        fn name(&self) -> &str {
            "extra_asserting"
        }
    }

    fn make_test_cases(n: usize) -> Vec<EvalTestCase> {
        (0..n)
            .map(|i| EvalTestCase {
                id: TestCaseId::new_unchecked(format!("tc-{i}")),
                tags: vec![],
                input: format!("test query {i}"),
                expected_trajectory: vec!["draft_plan".into()],
                trajectory_mode: TrajectoryMode::Unordered,
                ground_truth: None,
                final_response: None,
                source_thread_id: None,
            })
            .collect()
    }

    fn make_plan(test_cases: Vec<EvalTestCase>) -> EvalPlan {
        let config = crate::types::EvalConfig {
            mode: crate::types::EvalMode::Sequential,
            test_case_source: crate::types::TestCaseSource::Set("set-1".into()),
            samples_per_case: 2,
            pass_threshold: 0.5,
            concurrency: 2,
            k_values: vec![1],
            timeout_per_sample_secs: Some(30),
            ..Default::default()
        };
        let validated = ValidatedEvalConfig::validate(config.clone()).unwrap();

        EvalPlan {
            run: EvalRun::new(config),
            test_cases,
            scorer: Arc::new(CompositeScorer::trajectory_only()),
            model_config: ResolvedModelConfig {
                provider: "test".into(),
                model: "stub".into(),
            },
            config: validated,
            tenant: TenantId::new_unchecked("test-tenant"),
        }
    }

    // =========================================================================
    // L1: Progress monotonicity
    // =========================================================================

    #[tokio::test]
    async fn l1_progress_monotonicity() {
        let bus = Arc::new(EvalEventBus::new(256));
        let mut rx = bus.subscribe();

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            bus.clone(),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(2));
        let result = orchestrator.run(plan).await;
        assert!(result.is_ok());

        // Collect all SampleProgress events and verify monotonicity
        let mut progress_values = Vec::new();
        while let Ok(sequenced) = rx.try_recv() {
            if let EvalEvent::Progress { progress } = sequenced.event {
                progress_values.push(progress.completed_samples);
            }
        }
        // Progress should be monotonically non-decreasing
        for pair in progress_values.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "progress regressed: {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    // =========================================================================
    // L2: Completion — returns when all samples done
    // =========================================================================

    #[tokio::test]
    async fn l2_completion() {
        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(3));
        let summary = orchestrator.run(plan).await.unwrap();

        // Should have completed status
        assert!(matches!(summary.status, EvalStatus::Completed { .. }));
    }

    #[tokio::test]
    async fn with_model_pricing_attaches_cost_without_custom_resolver() {
        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        )
        .with_model_pricing([ModelPricing::new("stub", 3.0, 15.0, 0.30)]);

        let plan = make_plan(make_test_cases(1));
        let summary = orchestrator.run(plan).await.unwrap();
        let EvalStatus::Completed { summary } = summary.status else {
            panic!("expected completed status");
        };
        assert!(summary.cost.estimated_cost_usd > 0.0);
        assert_eq!(summary.cost.per_agent.len(), 1);
        assert_eq!(summary.cost.per_agent[0].model, "stub");
    }

    // =========================================================================
    // L3: Result persistence
    // =========================================================================

    #[tokio::test]
    async fn l3_results_persisted_in_kv() {
        let kv = Arc::new(DashMapKVStore::new());
        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            kv.clone(),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(2));
        let run_id = plan.run.id.as_str().to_string();
        orchestrator.run(plan).await.unwrap();

        // Check that the run was persisted
        let run_key = format!("eval:run:{run_id}");
        let stored = kv.get_json("test-tenant", &run_key).await.unwrap();
        assert!(stored.is_some(), "run should be persisted in KV");

        // Check that test case results were persisted
        for i in 0..2 {
            let result_key = format!("eval:result:{run_id}:tc-{i}");
            let stored = kv.get_json("test-tenant", &result_key).await.unwrap();
            assert!(stored.is_some(), "tc-{i} result should be persisted");

            let stored: TestCaseResult =
                serde_json::from_value(stored.unwrap()).expect("test case result should decode");
            let expected_tc_id = format!("tc-{i}");
            for sample in &stored.samples {
                let trace = sample.trace.as_ref().expect("trace should be persisted");
                assert_eq!(trace.status, TraceStatus::Completed);
                assert_eq!(trace.scope.sample_index, Some(sample.sample_index));
                assert_eq!(
                    trace.scope.eval_run_id.as_ref().map(|id| id.as_str()),
                    Some(run_id.as_str())
                );
                assert_eq!(
                    trace.scope.test_case_id.as_ref().map(|id| id.as_str()),
                    Some(expected_tc_id.as_str())
                );
                assert_eq!(trace.steps.len(), 1);
                assert_eq!(trace.steps[0].tool_name, "draft_plan");
                assert_eq!(
                    trace.steps[0].arguments,
                    TracePayload::omitted(TraceOmissionReason::NotCaptured)
                );
                assert_eq!(sample.trace_ref(), Some(trace.trace_ref()));
            }
        }
    }

    #[tokio::test]
    async fn persists_rich_trace_payloads_from_captured_tool_calls() {
        let kv = Arc::new(DashMapKVStore::new());
        let orchestrator = EvalOrchestrator::new(
            Arc::new(CapturingSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            kv.clone(),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let mut plan = make_plan(make_test_cases(1));
        plan.config = ValidatedEvalConfig::validate(crate::types::EvalConfig {
            samples_per_case: 1,
            ..plan.config.inner().clone()
        })
        .expect("config should validate");
        let run_id = plan.run.id.as_str().to_string();

        orchestrator.run(plan).await.unwrap();

        let result_key = format!("eval:result:{run_id}:tc-0");
        let stored = kv
            .get_json("test-tenant", &result_key)
            .await
            .unwrap()
            .expect("test case result should exist");
        let stored: TestCaseResult =
            serde_json::from_value(stored).expect("test case result should decode");
        let sample = stored.samples.first().expect("sample should exist");
        let trace = sample.trace.as_ref().expect("trace should be persisted");

        assert_eq!(trace.status, TraceStatus::Completed);
        assert_eq!(
            trace.scope.thread_id.as_ref().map(ThreadId::as_str),
            Some("thread-rich-1")
        );
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].tool_name, "draft_plan");
        assert_eq!(trace.steps[0].tool_call_id.as_deref(), Some("tool-call-1"));
        assert_eq!(
            trace.steps[0].arguments,
            TracePayload::inline(json!({ "goal": "ship" }))
        );
        assert_eq!(
            trace.steps[0].result,
            Some(TracePayload::inline(json!({ "ok": true })))
        );
        assert_eq!(sample.metadata, Some(json!({ "note": "preserved" })));
    }

    #[tokio::test]
    async fn persists_timed_out_trace_status_for_failed_samples() {
        let orchestrator = EvalOrchestrator::new(
            Arc::new(TimeoutSampleExecutor {
                delay: Duration::from_millis(50),
            }),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let scorer: Arc<dyn EvalScorer> = Arc::new(CompositeScorer::trajectory_only());
        let test_case = make_test_cases(1).into_iter().next().expect("test case");

        // Force the executor to see a short timeout directly.
        let result = orchestrator
            .execute_and_score_sample(
                SampleInput {
                    test_case: test_case.clone(),
                    sample_index: 0,
                    eval_mode: crate::types::EvalMode::Sequential,
                    target_agent_id: None,
                    run_id: "run-timeout".into(),
                },
                &TenantId::new_unchecked("test-tenant"),
                &ResolvedModelConfig {
                    provider: "test".into(),
                    model: "stub".into(),
                },
                Some(Duration::from_millis(1)),
                &scorer,
                &test_case,
                0.5,
            )
            .await;

        let trace = result.trace.expect("trace should be recorded");
        assert_eq!(trace.status, TraceStatus::TimedOut);
        assert!(trace.steps.is_empty());
    }

    #[tokio::test]
    async fn scorer_input_preserves_sample_output_extra() {
        let orchestrator = EvalOrchestrator::new(
            Arc::new(ExtraSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let scorer: Arc<dyn EvalScorer> = Arc::new(ExtraAssertingScorer);
        let test_case = make_test_cases(1).into_iter().next().expect("test case");

        let result = orchestrator
            .execute_and_score_sample(
                SampleInput {
                    test_case: test_case.clone(),
                    sample_index: 0,
                    eval_mode: crate::types::EvalMode::Executor,
                    target_agent_id: None,
                    run_id: "run-extra".into(),
                },
                &TenantId::new_unchecked("test-tenant"),
                &ResolvedModelConfig {
                    provider: "test".into(),
                    model: "stub".into(),
                },
                None,
                &scorer,
                &test_case,
                0.5,
            )
            .await;

        assert!(result.passed);
        assert_eq!(result.scores[0].scorer_name, "extra_asserting");
        assert_eq!(result.scores[0].score, 1.0);
        assert_eq!(
            result
                .metadata
                .as_ref()
                .and_then(|extra| extra.get("resolvedActions"))
                .and_then(|actions| actions.as_array())
                .map(Vec::len),
            Some(1)
        );
    }

    // =========================================================================
    // L4: Cancellation
    // =========================================================================

    #[tokio::test]
    async fn l4_cancellation_stops_execution() {
        let cancel = CancellationToken::new();
        // Cancel immediately
        cancel.cancel();

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            cancel,
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(10));
        let result = orchestrator.run(plan).await;
        assert!(matches!(result, Err(EvalOrchestratorError::Cancelled)));
    }

    // =========================================================================
    // Pause + Cancel composition (no deadlock)
    // =========================================================================

    #[tokio::test]
    async fn pause_then_cancel_returns_cancelled() {
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();

        // Pause FIRST, then cancel after a delay
        pause.pause();

        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            Arc::new(EvalEventBus::new(256)),
            Arc::new(DashMapKVStore::new()),
            cancel,
            pause,
        );

        let plan = make_plan(make_test_cases(3));
        let result = tokio::time::timeout(Duration::from_secs(5), orchestrator.run(plan)).await;

        // Should NOT time out (no deadlock) — should return Cancelled
        let result = result.expect("should not deadlock (timeout 5s)");
        assert!(
            matches!(result, Err(EvalOrchestratorError::Cancelled)),
            "expected Cancelled, got: {:?}",
            result.err()
        );
    }

    // =========================================================================
    // Scorer integration
    // =========================================================================

    #[tokio::test]
    async fn scorer_produces_bounded_scores() {
        let bus = Arc::new(EvalEventBus::new(256));
        let mut rx = bus.subscribe();

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            bus.clone(),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(1));
        orchestrator.run(plan).await.unwrap();

        // Collect all sample results and verify scores are in [0,1]
        while let Ok(sequenced) = rx.try_recv() {
            if let EvalEvent::SampleComplete { sample, .. } = sequenced.event {
                for sr in &sample.scores {
                    assert!(
                        sr.score >= 0.0 && sr.score <= 1.0,
                        "score {} outside [0,1]",
                        sr.score
                    );
                }
            }
        }
    }

    // =========================================================================
    // Completed event emitted
    // =========================================================================

    #[tokio::test]
    async fn completed_event_emitted() {
        let bus = Arc::new(EvalEventBus::new(256));
        let mut rx = bus.subscribe();

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            bus.clone(),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(1));
        orchestrator.run(plan).await.unwrap();

        let mut found_completed = false;
        while let Ok(sequenced) = rx.try_recv() {
            if matches!(sequenced.event, EvalEvent::Completed { .. }) {
                found_completed = true;
            }
        }
        assert!(found_completed, "should emit Completed event");
    }

    // =========================================================================
    // 3.4: Progress events include populated test_case_states
    // =========================================================================

    #[tokio::test]
    async fn progress_events_include_test_case_states() {
        let bus = Arc::new(EvalEventBus::new(256));
        let mut rx = bus.subscribe();

        let orchestrator = EvalOrchestrator::new(
            Arc::new(StubSampleExecutor),
            Arc::new(StandardAggregator),
            bus.clone(),
            Arc::new(DashMapKVStore::new()),
            CancellationToken::new(),
            PauseToken::new(),
        );

        let plan = make_plan(make_test_cases(2));
        orchestrator.run(plan).await.unwrap();

        let mut found_states = false;
        while let Ok(seq) = rx.try_recv() {
            if let EvalEvent::Progress { progress } = seq.event {
                if !progress.test_case_states.is_empty() {
                    found_states = true;
                    // Every entry should have a valid state
                    for entry in &progress.test_case_states {
                        assert!(
                            matches!(
                                entry.state,
                                TestCaseState::Queued
                                    | TestCaseState::Running { .. }
                                    | TestCaseState::Completed { .. }
                                    | TestCaseState::Failed { .. }
                            ),
                            "unexpected test case state: {:?}",
                            entry.state
                        );
                    }
                }
            }
        }
        assert!(
            found_states,
            "progress events must include test_case_states"
        );
    }
}
