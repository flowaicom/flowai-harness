use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_fw_agent::{
    parse_conversation, ChatInterpreter, ChatProgram, ModelId, ModelSettings, ReasoningEffort,
    ToolCallResult, ToolDefinition, ToolDispatcher,
};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{AgentUsage, ChatMessage, StreamPart, TenantId, ThreadId};
use agent_fw_eval::{
    ResolvedModelConfig, SampleExecutionError, SampleExecutor, SampleExecutorOutput, SampleInput,
    StreamCapture,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};

use crate::eval::runner::artifact::ModelInvocation;
use crate::eval::{
    build_judge_prompt, extract_planned_actions_from_sample, extract_resolved_actions_from_sample,
    judge_context_for_hash, planned_actions_extra, resolved_actions_extra, trajectory_events_extra,
    FinalResponseEvalSpec, FinalResponseScorerConfig, JudgeResponseErrorKind,
    JudgeResponseScoringData, JudgeResponseVerdict, JudgeRunMetadata, JudgeTrace,
    ResponseScorerMethod, ResponseScorerSpec, TrajectoryEventCapture,
};
use crate::runtime::providers::select_provider_key;
use crate::{AgentRole, ModelSpec, Runtime};

pub type RuntimeSampleStream = Pin<Box<dyn Stream<Item = StreamPart> + Send + 'static>>;

const JUDGE_AGENT_NAME: &str = "judge";
const RUNTIME_DEFAULT_EVAL_PROVIDER: &str = "runtime";
const RUNTIME_DEFAULT_EVAL_MODEL: &str = "runtime-default";
const JUDGE_MAX_TOKENS: u32 = 2_048;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSampleCapture {
    pub output: SampleExecutorOutput,
    pub model_invocations: Vec<ModelInvocation>,
    pub response_text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinalResponseJudgeCapture {
    pub results: BTreeMap<String, JudgeResponseScoringData>,
    pub model_invocations: Vec<ModelInvocation>,
}

pub struct RuntimeSampleExecutor {
    runtime: Arc<Runtime>,
}

#[async_trait]
pub trait CaptureSampleExecutor: Send + Sync {
    async fn execute_capture(
        &self,
        input: SampleInput,
        model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<RuntimeSampleCapture, SampleExecutionError>;

    async fn execute_final_response_judges(
        &self,
        _spec: &FinalResponseEvalSpec,
        _response_text: &str,
        _run_context: &serde_json::Value,
        _model_config: &ResolvedModelConfig,
        _final_response_config: &FinalResponseScorerConfig,
        _timeout: Option<Duration>,
    ) -> FinalResponseJudgeCapture {
        FinalResponseJudgeCapture::default()
    }
}

impl RuntimeSampleExecutor {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }

    pub async fn execute_capture(
        &self,
        input: SampleInput,
        _model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
        let thread_id = thread_id_for_sample(&input);
        let stream = match input.eval_mode {
            agent_fw_eval::EvalMode::Planner => {
                self.require_agent_for_mode(input.eval_mode, AgentRole::Planner)?;
                self.runtime.run_eval_role_stream(
                    AgentRole::Planner,
                    input.test_case.input.clone(),
                    thread_id.clone(),
                )
            }
            agent_fw_eval::EvalMode::Executor => {
                self.require_agent_for_mode(input.eval_mode, AgentRole::Executor)?;
                self.runtime.run_eval_role_stream(
                    AgentRole::Executor,
                    input.test_case.input.clone(),
                    thread_id.clone(),
                )
            }
            agent_fw_eval::EvalMode::Sequential => {
                self.require_agent_for_mode(input.eval_mode, AgentRole::Coordinator)?;
                self.runtime
                    .run_eval_query_stream(input.test_case.input.clone(), thread_id.clone())
            }
            agent_fw_eval::EvalMode::Specialist => {
                let specialist = input.target_agent_id.as_deref().ok_or_else(|| {
                    SampleExecutionError::AgentFailed(
                        "eval mode 'specialist' requires targetAgentId".to_string(),
                    )
                })?;
                self.require_named_agent_for_mode(
                    input.eval_mode,
                    specialist,
                    AgentRole::Specialist,
                )?;
                self.runtime.run_eval_specialist_stream(
                    specialist,
                    input.test_case.input.clone(),
                    thread_id.clone(),
                )
            }
            agent_fw_eval::EvalMode::TestCaseBuilder => {
                return Err(SampleExecutionError::Internal(
                    "testCaseBuilder runtime execution is not supported in eval runner".into(),
                ));
            }
        };

        capture_stream(
            stream,
            timeout,
            Some(thread_id.as_str().to_string()),
            |agent_name, model| provider_for_agent(&self.runtime, agent_name, model),
        )
        .await
    }

    pub async fn execute_final_response_judges(
        &self,
        spec: &FinalResponseEvalSpec,
        response_text: &str,
        run_context: &serde_json::Value,
        model_config: &ResolvedModelConfig,
        final_response_config: &FinalResponseScorerConfig,
        timeout: Option<Duration>,
    ) -> FinalResponseJudgeCapture {
        let judge_scorers: Vec<&ResponseScorerSpec> = spec
            .scorers
            .iter()
            .filter(|scorer| scorer.method == ResponseScorerMethod::Judge)
            .collect();
        if judge_scorers.is_empty() {
            return FinalResponseJudgeCapture::default();
        }

        let (provider, model, interpreter) = match self.resolve_judge_interpreter(model_config) {
            Ok(resolved) => resolved,
            Err(error) => return failed_judge_capture(judge_scorers, error),
        };

        let mut capture = FinalResponseJudgeCapture::default();
        for scorer in judge_scorers {
            let prompt = match build_judge_prompt(scorer, response_text, Some(run_context)) {
                Ok(prompt) => prompt,
                Err(error) => {
                    capture.results.insert(
                        scorer.id.clone(),
                        failed_judge_result(
                            format!("Judge prompt construction failed: {error}"),
                            None,
                            JudgeResponseErrorKind::JudgePromptFailed,
                        ),
                    );
                    continue;
                }
            };
            let judge_trace_prompt = if final_response_config.include_judge_trace {
                Some(prompt.clone())
            } else {
                None
            };
            let judge_run = JudgeRunMetadata::new(
                &provider,
                &model,
                &prompt,
                &judge_context_for_hash(scorer, response_text, Some(run_context)),
            );
            let program = match judge_chat_program(prompt, &model, self.runtime.tenant.clone()) {
                Ok(program) => program,
                Err(error) => {
                    capture.results.insert(
                        scorer.id.clone(),
                        failed_judge_result(
                            format!("Judge prompt construction failed: {error}"),
                            Some(judge_run),
                            JudgeResponseErrorKind::JudgePromptFailed,
                        ),
                    );
                    continue;
                }
            };
            let stream = interpreter.interpret(program, self.runtime.cancel_root.child());
            match capture_judge_stream(stream, timeout, &provider, &model).await {
                Ok(judge_output) => {
                    capture
                        .model_invocations
                        .extend(judge_output.model_invocations);
                    let judge_trace_response = judge_output.response_text;
                    let response_text = judge_trace_response.trim();
                    let mut result = if response_text.is_empty() {
                        JudgeResponseScoringData::new(failed_judge_verdict(
                            "Judge produced no text; cannot parse verdict JSON.",
                        ))
                        .with_error_kind(JudgeResponseErrorKind::JudgeNoText)
                    } else {
                        match parse_judge_response_verdict(response_text) {
                            Ok(verdict) => JudgeResponseScoringData::new(verdict),
                            Err((kind, reason)) => {
                                JudgeResponseScoringData::new(failed_judge_verdict(reason))
                                    .with_error_kind(kind)
                            }
                        }
                    };
                    if let Some(prompt) = judge_trace_prompt {
                        result = result.with_judge_trace(JudgeTrace {
                            prompt,
                            response: judge_trace_response,
                        });
                    }
                    capture
                        .results
                        .insert(scorer.id.clone(), result.with_judge_run(judge_run));
                }
                Err(error) => {
                    capture.results.insert(
                        scorer.id.clone(),
                        failed_judge_result(
                            format!("Judge execution failed: {error}"),
                            Some(judge_run),
                            JudgeResponseErrorKind::JudgeExecutionFailed,
                        ),
                    );
                }
            }
        }
        capture
    }

    fn require_agent_for_mode(
        &self,
        mode: agent_fw_eval::EvalMode,
        role: AgentRole,
    ) -> Result<(), SampleExecutionError> {
        if self.runtime.agent_name_by_role(role).is_some() {
            return Ok(());
        }

        Err(SampleExecutionError::AgentFailed(format!(
            "eval mode '{}' requires an agent with role '{role}', but the runtime spec does not register one; coordinator-driven evals should use mode 'sequential'",
            eval_mode_name(mode)
        )))
    }

    fn require_named_agent_for_mode(
        &self,
        mode: agent_fw_eval::EvalMode,
        agent_id: &str,
        role: AgentRole,
    ) -> Result<(), SampleExecutionError> {
        match self.runtime.agent_role(agent_id) {
            Some(actual) if actual == role => Ok(()),
            Some(actual) => Err(SampleExecutionError::AgentFailed(format!(
                "eval mode '{}' requires target agent '{agent_id}' to have role '{role}', but it has role '{actual}'",
                eval_mode_name(mode)
            ))),
            None => Err(SampleExecutionError::AgentFailed(format!(
                "eval mode '{}' target agent '{agent_id}' is not registered",
                eval_mode_name(mode)
            ))),
        }
    }

    fn resolve_judge_interpreter(
        &self,
        model_config: &ResolvedModelConfig,
    ) -> Result<(String, String, Arc<dyn ChatInterpreter>), JudgeInterpreterResolutionError> {
        let model = self.judge_model_spec(model_config).ok_or_else(|| {
            JudgeInterpreterResolutionError(
                "cannot choose judge model: eval config did not set a model and the runtime has no registered agents".to_string()
            )
        })?;
        let provider = select_provider_key(&model);
        if !self
            .runtime
            .judge_capable_interpreter_providers
            .contains(&provider)
        {
            return Err(JudgeInterpreterResolutionError(format!(
                "cannot execute judge scorer: provider '{provider}' is not configured as a judge-capable interpreter"
            )));
        }
        let interpreter = self
            .runtime
            .interpreter_providers
            .get(&provider)
            .cloned()
            .ok_or_else(|| {
                JudgeInterpreterResolutionError(format!(
                    "cannot execute judge scorer: provider '{provider}' has no registered interpreter"
                ))
            })?;
        Ok((provider, model.id, configure_judge_interpreter(interpreter)))
    }

    fn judge_model_spec(&self, model_config: &ResolvedModelConfig) -> Option<ModelSpec> {
        if model_config.model != RUNTIME_DEFAULT_EVAL_MODEL {
            return Some(ModelSpec {
                id: model_config.model.clone(),
                provider: (model_config.provider != RUNTIME_DEFAULT_EVAL_PROVIDER)
                    .then(|| model_config.provider.clone()),
            });
        }
        self.runtime
            .spec
            .agents
            .iter()
            .find(|agent| agent.role == AgentRole::Coordinator)
            .or_else(|| self.runtime.spec.agents.first())
            .map(|agent| agent.model.clone())
    }
}

struct JudgeInterpreterResolutionError(String);

impl std::fmt::Display for JudgeInterpreterResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct JudgeNoToolsDispatcher;

#[async_trait]
impl ToolDispatcher for JudgeNoToolsDispatcher {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    async fn dispatch(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        _input: serde_json::Value,
    ) -> ToolCallResult {
        ToolCallResult::error(
            tool_use_id,
            format!("judge eval does not allow tool calls; model requested '{tool_name}'"),
        )
    }
}

fn configure_judge_interpreter(interpreter: Arc<dyn ChatInterpreter>) -> Arc<dyn ChatInterpreter> {
    let no_tools = Arc::new(JudgeNoToolsDispatcher) as Arc<dyn ToolDispatcher>;
    let interpreter = interpreter
        .clone()
        .with_tool_dispatcher(no_tools)
        .unwrap_or(interpreter);
    let interpreter = interpreter.clone().with_max_turns(1).unwrap_or(interpreter);
    interpreter
        .clone()
        .with_model_settings(judge_model_settings())
        .unwrap_or(interpreter)
}

fn judge_model_settings() -> ModelSettings {
    ModelSettings::new(JUDGE_MAX_TOKENS, 0, ReasoningEffort::Low, true)
        .expect("valid judge model settings")
}

#[async_trait]
impl CaptureSampleExecutor for RuntimeSampleExecutor {
    async fn execute_capture(
        &self,
        input: SampleInput,
        model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<RuntimeSampleCapture, SampleExecutionError> {
        RuntimeSampleExecutor::execute_capture(self, input, model_config, timeout).await
    }

    async fn execute_final_response_judges(
        &self,
        spec: &FinalResponseEvalSpec,
        response_text: &str,
        run_context: &serde_json::Value,
        model_config: &ResolvedModelConfig,
        final_response_config: &FinalResponseScorerConfig,
        timeout: Option<Duration>,
    ) -> FinalResponseJudgeCapture {
        RuntimeSampleExecutor::execute_final_response_judges(
            self,
            spec,
            response_text,
            run_context,
            model_config,
            final_response_config,
            timeout,
        )
        .await
    }
}

#[async_trait]
impl SampleExecutor for RuntimeSampleExecutor {
    async fn execute(
        &self,
        input: SampleInput,
        model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<SampleExecutorOutput, SampleExecutionError> {
        // This trait method necessarily returns only the framework-generic
        // SampleOutput. The eval runner harness EvalRunner must call
        // execute_capture() directly so modelInvocations are not lost at this
        // boundary.
        self.execute_capture(input, model_config, timeout)
            .await
            .map(|capture| capture.output)
    }
}

async fn capture_stream(
    stream: RuntimeSampleStream,
    timeout: Option<Duration>,
    thread_id: Option<String>,
    provider_for: impl Fn(&str, &str) -> Option<String>,
) -> Result<RuntimeSampleCapture, SampleExecutionError> {
    match timeout {
        Some(timeout) => tokio::time::timeout(
            timeout,
            capture_stream_inner(stream, thread_id, provider_for),
        )
        .await
        .map_err(|_| SampleExecutionError::TimedOut { timeout })?,
        None => capture_stream_inner(stream, thread_id, provider_for).await,
    }
}

struct JudgeStreamCapture {
    response_text: String,
    model_invocations: Vec<ModelInvocation>,
}

async fn capture_judge_stream(
    stream: RuntimeSampleStream,
    timeout: Option<Duration>,
    provider: &str,
    model: &str,
) -> Result<JudgeStreamCapture, SampleExecutionError> {
    match timeout {
        Some(timeout) => {
            tokio::time::timeout(timeout, capture_judge_stream_inner(stream, provider, model))
                .await
                .map_err(|_| SampleExecutionError::TimedOut { timeout })?
        }
        None => capture_judge_stream_inner(stream, provider, model).await,
    }
}

async fn capture_judge_stream_inner(
    mut stream: RuntimeSampleStream,
    provider: &str,
    model: &str,
) -> Result<JudgeStreamCapture, SampleExecutionError> {
    let started = Instant::now();
    let mut capture = StreamCapture::new();
    let mut model_invocations = Vec::new();

    while let Some(part) = stream.next().await {
        if let StreamPart::ApprovalRequired { data } = &part {
            return Err(SampleExecutionError::Internal(format!(
                "judge eval unexpectedly blocked on approval-required for {} '{}'",
                data.kind, data.target
            )));
        }
        if let StreamPart::DataCostSummary { data } = &part {
            model_invocations.extend(
                data.agents
                    .iter()
                    .map(|usage| judge_model_invocation_from_agent_usage(usage, provider, model)),
            );
        }
        capture.step(&part);
    }

    let result = capture.finalize(started.elapsed().as_millis() as u64, None);
    if let Some(error) = result.sample_output.error.as_ref() {
        return Err(SampleExecutionError::AgentFailed(error.clone()));
    }

    if model_invocations.is_empty() {
        let usage = result.sample_output.token_usage;
        model_invocations.push(ModelInvocation {
            agent: JUDGE_AGENT_NAME.to_string(),
            provider: Some(provider.to_string()),
            model: model.to_string(),
            input_tokens: usage.input_tokens(),
            output_tokens: usage.output_tokens(),
            cached_tokens: usage.cached_tokens(),
            cache_creation_tokens: usage.cache_creation_tokens(),
            estimated_cost_usd: None,
        });
    }

    Ok(JudgeStreamCapture {
        response_text: result.response_text,
        model_invocations,
    })
}

async fn capture_stream_inner(
    mut stream: RuntimeSampleStream,
    thread_id: Option<String>,
    provider_for: impl Fn(&str, &str) -> Option<String>,
) -> Result<RuntimeSampleCapture, SampleExecutionError> {
    let started = Instant::now();
    let mut capture = StreamCapture::new();
    let mut trajectory_capture = TrajectoryEventCapture::new();
    let mut model_invocations = Vec::new();

    while let Some(part) = stream.next().await {
        if let StreamPart::ApprovalRequired { data } = &part {
            return Err(SampleExecutionError::Internal(format!(
                "eval sample unexpectedly blocked on approval-required for {} '{}'; eval execution should bypass approvals",
                data.kind, data.target
            )));
        }
        if let StreamPart::DataCostSummary { data } = &part {
            model_invocations.extend(
                data.agents
                    .iter()
                    .map(|usage| model_invocation_from_agent_usage(usage, &provider_for)),
            );
        }
        trajectory_capture.step(&part);
        capture.step(&part);
    }

    if model_invocations.is_empty() {
        model_invocations.push(ModelInvocation {
            agent: "unknown".to_string(),
            provider: None,
            model: "unknown".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            estimated_cost_usd: None,
        });
    }

    let result = capture.finalize(started.elapsed().as_millis() as u64, thread_id);
    if let Some(error) = result.sample_output.error.as_ref() {
        return Err(SampleExecutionError::AgentFailed(error.clone()));
    }

    let mut output = result.sample_output;
    let resolved_actions = extract_resolved_actions_from_sample(&output)
        .map_err(|e| SampleExecutionError::Internal(e.to_string()))?;
    let planned_actions = extract_planned_actions_from_sample(&output)
        .map_err(|e| SampleExecutionError::Internal(e.to_string()))?;
    let action_extra = merge_extra_objects(
        Some(resolved_actions_extra(&resolved_actions)),
        planned_actions_extra(&planned_actions),
    );
    let trajectory_events = trajectory_capture.into_events();
    let trajectory_extra = trajectory_events_extra(&trajectory_events);
    let extra = merge_extra_objects(output.extra.take(), action_extra);
    let extra = match trajectory_extra {
        Some(trajectory_extra) => merge_extra_objects(Some(extra), trajectory_extra),
        None => extra,
    };
    output.extra = Some(extra);

    Ok(RuntimeSampleCapture {
        output,
        model_invocations,
        response_text: result.response_text,
    })
}

fn judge_chat_program(
    prompt: String,
    model: &str,
    tenant: TenantId,
) -> Result<ChatProgram, SampleExecutionError> {
    let conversation = parse_conversation(vec![ChatMessage::user(prompt)]).map_err(|error| {
        SampleExecutionError::Internal(format!("invalid judge prompt: {error}"))
    })?;
    Ok(ChatProgram::new(
        conversation,
        ModelId::new(model.to_string()),
        TenantContext::new(tenant),
    ))
}

fn parse_judge_response_verdict(
    response_text: &str,
) -> Result<JudgeResponseVerdict, (JudgeResponseErrorKind, String)> {
    let value: serde_json::Value = serde_json::from_str(response_text).map_err(|error| {
        (
            JudgeResponseErrorKind::JudgeInvalidJson,
            format!("Judge returned invalid verdict JSON: {error}"),
        )
    })?;
    JudgeResponseVerdict::from_json_value(value).map_err(|error| {
        (
            JudgeResponseErrorKind::JudgeInvalidSchema,
            format!("Judge returned verdict JSON that did not match the expected schema: {error}"),
        )
    })
}

fn failed_judge_capture<'a>(
    scorers: impl IntoIterator<Item = &'a ResponseScorerSpec>,
    reason: JudgeInterpreterResolutionError,
) -> FinalResponseJudgeCapture {
    let reason = reason.to_string();
    FinalResponseJudgeCapture {
        results: scorers
            .into_iter()
            .map(|scorer| {
                (
                    scorer.id.clone(),
                    failed_judge_result(
                        reason.clone(),
                        None,
                        JudgeResponseErrorKind::JudgeProviderUnavailable,
                    ),
                )
            })
            .collect(),
        model_invocations: Vec::new(),
    }
}

fn failed_judge_result(
    reason: impl Into<String>,
    judge_run: Option<JudgeRunMetadata>,
    error_kind: JudgeResponseErrorKind,
) -> JudgeResponseScoringData {
    let mut result =
        JudgeResponseScoringData::new(failed_judge_verdict(reason)).with_error_kind(error_kind);
    result.judge_run = judge_run;
    result
}

fn failed_judge_verdict(reason: impl Into<String>) -> JudgeResponseVerdict {
    JudgeResponseVerdict {
        passed: false,
        selected_rubric_score: 0,
        reason: reason.into(),
    }
}

fn eval_mode_name(mode: agent_fw_eval::EvalMode) -> &'static str {
    match mode {
        agent_fw_eval::EvalMode::Planner => "planner",
        agent_fw_eval::EvalMode::Executor => "executor",
        agent_fw_eval::EvalMode::Sequential => "sequential",
        agent_fw_eval::EvalMode::Specialist => "specialist",
        agent_fw_eval::EvalMode::TestCaseBuilder => "testCaseBuilder",
    }
}

/// Merge `incoming` into `existing`, preserving existing keys not overwritten.
/// Used to fold projected action payloads (`resolvedActions`, `plannedActions`)
/// into the sample's `extra` object without clobbering other fields.
pub(crate) fn merge_extra_objects(
    existing: Option<serde_json::Value>,
    incoming: serde_json::Value,
) -> serde_json::Value {
    match (existing, incoming) {
        (Some(serde_json::Value::Object(mut existing)), serde_json::Value::Object(incoming)) => {
            existing.extend(incoming);
            serde_json::Value::Object(existing)
        }
        (_, incoming) => incoming,
    }
}

fn model_invocation_from_agent_usage(
    usage: &AgentUsage,
    provider_for: impl Fn(&str, &str) -> Option<String>,
) -> ModelInvocation {
    ModelInvocation {
        agent: if usage.agent_name.trim().is_empty() {
            "unknown".to_string()
        } else {
            usage.agent_name.clone()
        },
        provider: provider_for(&usage.agent_name, &usage.model),
        model: usage.model.clone(),
        input_tokens: usage.usage.prompt_tokens,
        output_tokens: usage.usage.completion_tokens,
        cached_tokens: usage.usage.cache_read_input_tokens,
        cache_creation_tokens: usage.usage.cache_creation_input_tokens,
        estimated_cost_usd: None,
    }
}

fn judge_model_invocation_from_agent_usage(
    usage: &AgentUsage,
    provider: &str,
    default_model: &str,
) -> ModelInvocation {
    ModelInvocation {
        agent: JUDGE_AGENT_NAME.to_string(),
        provider: Some(provider.to_string()),
        model: if usage.model.trim().is_empty() {
            default_model.to_string()
        } else {
            usage.model.clone()
        },
        input_tokens: usage.usage.prompt_tokens,
        output_tokens: usage.usage.completion_tokens,
        cached_tokens: usage.usage.cache_read_input_tokens,
        cache_creation_tokens: usage.usage.cache_creation_input_tokens,
        estimated_cost_usd: None,
    }
}

fn provider_for_agent(runtime: &Runtime, agent_name: &str, model: &str) -> Option<String> {
    runtime
        .spec
        .agents
        .iter()
        .find(|agent| agent.name == agent_name)
        .map(|agent| select_provider_key(&agent.model))
        .or_else(|| {
            runtime
                .spec
                .agents
                .iter()
                .find(|agent| agent.model.id == model)
                .map(|agent| select_provider_key(&agent.model))
        })
}

fn thread_id_for_sample(input: &SampleInput) -> ThreadId {
    // `sourceThreadId` is provenance for test-case authoring, not the eval
    // execution thread. Reusing it lets stateful agents answer from prior chat
    // context and skip tool calls, which corrupts trajectory scoring.
    ThreadId::new_unchecked(format!(
        "eval-{}-{}-{}",
        input.run_id,
        input.test_case.id.as_str(),
        input.sample_index
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::Mutex;

    use agent_fw_agent::{ChatInterpreter, ChatProgram};
    use agent_fw_algebra::CancellationToken;
    use agent_fw_core::approval::{ApprovalKind, ApprovalRequest};
    use agent_fw_core::tenant::TenantContext;
    use agent_fw_core::{
        AgentUsage, ApprovalId, CostSummary, FinishReason, TenantId, ThreadId, TokenUsage,
    };
    use futures::{stream, Stream};
    use serde_json::json;

    use crate::{
        AgentRole, AgentSpec, ModelSpec, ProviderConfig, RuntimeDeps, RuntimeSpec, TenantIdentity,
    };

    fn boxed(parts: Vec<StreamPart>) -> RuntimeSampleStream {
        Box::pin(stream::iter(parts))
    }

    struct NoopInterpreter;

    impl ChatInterpreter for NoopInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![StreamPart::finish(
                FinishReason::Stop,
                TokenUsage::ZERO,
            )]))
        }
    }

    struct JudgeVerdictInterpreter;

    impl ChatInterpreter for JudgeVerdictInterpreter {
        fn interpret(
            &self,
            program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            assert!(program
                .conversation()
                .prompt()
                .as_str()
                .contains("Return exactly this JSON shape"));
            Box::pin(stream::iter(vec![
                StreamPart::text(
                    r#"{"passed":true,"selected_rubric_score":1,"reason":"The response matches."}"#,
                ),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(12, 4, 0, 0)),
            ]))
        }
    }

    struct JudgeReasoningOnlyInterpreter;

    impl ChatInterpreter for JudgeReasoningOnlyInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::reasoning(
                    r#"{"passed":true,"selected_rubric_score":1,"reason":"Reasoning only."}"#,
                ),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(12, 172, 0, 0)),
            ]))
        }
    }

    struct JudgeInvalidJsonInterpreter;

    impl ChatInterpreter for JudgeInvalidJsonInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::text("not a json verdict"),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(12, 4, 0, 0)),
            ]))
        }
    }

    struct JudgeInvalidSchemaInterpreter;

    impl ChatInterpreter for JudgeInvalidSchemaInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::text(r#"{"passed":true,"reason":"Missing rubric score."}"#),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(12, 4, 0, 0)),
            ]))
        }
    }

    #[derive(Debug, Default)]
    struct JudgeInterpreterConfigProbe {
        dispatcher_tool_count: Option<usize>,
        max_turns: Option<usize>,
        model_settings: Option<ModelSettings>,
    }

    #[derive(Clone)]
    struct ConfigurableJudgeInterpreter {
        probe: Arc<Mutex<JudgeInterpreterConfigProbe>>,
    }

    impl ConfigurableJudgeInterpreter {
        fn new(probe: Arc<Mutex<JudgeInterpreterConfigProbe>>) -> Self {
            Self { probe }
        }
    }

    impl ChatInterpreter for ConfigurableJudgeInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::text(
                    r#"{"passed":true,"selected_rubric_score":1,"reason":"Configured."}"#,
                ),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(12, 4, 0, 0)),
            ]))
        }

        fn with_tool_dispatcher(
            self: Arc<Self>,
            dispatcher: Arc<dyn ToolDispatcher>,
        ) -> Option<Arc<dyn ChatInterpreter>> {
            self.probe.lock().expect("probe").dispatcher_tool_count =
                Some(dispatcher.tool_definitions().len());
            Some(Arc::new((*self).clone()))
        }

        fn with_max_turns(self: Arc<Self>, max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
            self.probe.lock().expect("probe").max_turns = Some(max_turns);
            Some(Arc::new((*self).clone()))
        }

        fn with_model_settings(
            self: Arc<Self>,
            settings: ModelSettings,
        ) -> Option<Arc<dyn ChatInterpreter>> {
            self.probe.lock().expect("probe").model_settings = Some(settings);
            Some(Arc::new((*self).clone()))
        }
    }

    fn runtime_with_interpreter(interpreter: Arc<dyn ChatInterpreter>) -> Runtime {
        runtime_with_interpreter_and_judge_capability(interpreter, true)
    }

    fn runtime_with_interpreter_and_judge_capability(
        interpreter: Arc<dyn ChatInterpreter>,
        judge_capable: bool,
    ) -> Runtime {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
        );

        Runtime::new(
            RuntimeSpec {
                tenant: TenantIdentity::new("tenant-1", "v1"),
                agents: vec![AgentSpec::new(
                    "planner",
                    AgentRole::Planner,
                    ModelSpec::new("claude-sonnet-4-6"),
                    "Plan.",
                )],
                references: vec![],
                plans: vec![],
                toolkits: vec![],
                approval_policies: Default::default(),
                approval_overrides: Default::default(),
                storage_factories: Default::default(),
                providers,
            },
            RuntimeDeps::new(
                interpreter,
                Arc::new(agent_fw_algebra::testing::NullEventSink),
                TenantContext::new(TenantId::new_unchecked("tenant-1")),
                Arc::new(agent_fw_interpreter::DashMapKVStore::new()),
            )
            .with_judge_capable_interpreter_provider("anthropic", judge_capable),
        )
        .expect("runtime should build")
    }

    fn runtime_with_anthropic_agent() -> Runtime {
        runtime_with_interpreter(Arc::new(NoopInterpreter))
    }

    fn sample_input_for_mode(eval_mode: agent_fw_eval::EvalMode) -> SampleInput {
        SampleInput {
            test_case: agent_fw_eval::EvalTestCase {
                id: agent_fw_core::TestCaseId::new_unchecked("tc-1"),
                tags: vec![],
                input: "run the sample".to_string(),
                expected_trajectory: vec![],
                trajectory_mode: agent_fw_eval::TrajectoryMode::Unordered,
                ground_truth: None,
                final_response: None,
                source_thread_id: None,
            },
            sample_index: 0,
            eval_mode,
            target_agent_id: None,
            run_id: "run-1".to_string(),
        }
    }

    fn resolved_model_config() -> ResolvedModelConfig {
        ResolvedModelConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
        }
    }

    #[tokio::test]
    async fn capture_stream_projects_resolved_actions_and_model_invocations() {
        let capture = capture_stream(
            boxed(vec![
                StreamPart::text("I updated the billing contact."),
                StreamPart::tool_call("tool-1", "executePlan", json!({ "plan": "p1" })),
                StreamPart::tool_result(
                    "tool-1",
                    "executePlan",
                    json!({ "plan": "p1" }),
                    json!({
                        "resolvedActions": [{
                            "type": "price_change",
                            "payload": {
                                "changeType": "absolute",
                                "value": 10.0
                            }
                        }]
                    }),
                ),
                StreamPart::cost_summary(CostSummary::new(vec![AgentUsage {
                    agent_name: "executor".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    usage: TokenUsage::new(100, 40, 10, 0),
                }])),
                StreamPart::finish(FinishReason::Stop, TokenUsage::new(100, 40, 10, 0)),
            ]),
            None,
            Some("thread-1".to_string()),
            |agent, _model| (agent == "executor").then(|| "anthropic".to_string()),
        )
        .await
        .expect("capture succeeds");

        assert_eq!(capture.output.actual_trajectory, vec!["executePlan"]);
        assert_eq!(capture.response_text, "I updated the billing contact.");
        assert_eq!(capture.output.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(
            capture.output.extra.as_ref().expect("extra")["resolvedActions"][0]["type"],
            "price_change"
        );
        assert_eq!(capture.model_invocations.len(), 1);
        assert_eq!(capture.model_invocations[0].agent, "executor");
        assert_eq!(
            capture.model_invocations[0].provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(capture.model_invocations[0].input_tokens, 100);
        assert_eq!(capture.model_invocations[0].cached_tokens, 10);
    }

    #[tokio::test]
    async fn capture_stream_emits_unknown_model_invocation_fallback() {
        let capture = capture_stream(
            boxed(vec![StreamPart::finish(
                FinishReason::Stop,
                TokenUsage::ZERO,
            )]),
            None,
            None,
            |_agent, _model| None,
        )
        .await
        .expect("capture succeeds");

        assert_eq!(capture.model_invocations.len(), 1);
        assert_eq!(capture.model_invocations[0].agent, "unknown");
        assert_eq!(capture.model_invocations[0].provider, None);
        assert_eq!(capture.model_invocations[0].model, "unknown");
        assert_eq!(
            capture.output.extra.as_ref().expect("extra")["resolvedActions"],
            json!([])
        );
    }

    #[tokio::test]
    async fn capture_stream_respects_timeout() {
        let err = capture_stream(
            Box::pin(stream::pending()),
            Some(Duration::from_millis(1)),
            None,
            |_agent, _model| None,
        )
        .await
        .expect_err("pending stream should time out");

        assert!(matches!(err, SampleExecutionError::TimedOut { .. }));
    }

    #[tokio::test]
    async fn capture_stream_fails_fast_on_unexpected_approval_required() {
        let request = ApprovalRequest {
            id: ApprovalId::new_unchecked("approval-1"),
            kind: ApprovalKind::Plan,
            target: "plan".to_string(),
            payload: json!({"planId": "plan-1"}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("tenant-1"),
            thread_id: ThreadId::new_unchecked("thread-1"),
            correlation_id: None,
        };
        let err = capture_stream(
            boxed(vec![StreamPart::approval_required(request)]),
            None,
            None,
            |_agent, _model| None,
        )
        .await
        .expect_err("approval-required should fail fast in eval capture");

        assert!(
            err.to_string()
                .contains("unexpectedly blocked on approval-required"),
            "unexpected error: {err}",
        );
    }

    #[tokio::test]
    async fn runtime_executor_mode_requires_executor_agent() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_anthropic_agent()));
        let err = executor
            .execute_capture(
                sample_input_for_mode(agent_fw_eval::EvalMode::Executor),
                &resolved_model_config(),
                None,
            )
            .await
            .expect_err("executor mode should require an executor agent");

        let message = err.to_string();
        assert!(
            message.contains("eval mode 'executor' requires an agent with role 'executor'"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("coordinator-driven evals should use mode 'sequential'"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn runtime_executor_executes_final_response_judge_with_interpreter() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            JudgeVerdictInterpreter,
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        let verdict = &result.verdict;
        assert!(verdict.passed);
        assert_eq!(verdict.selected_rubric_score, 1);
        assert_eq!(verdict.reason, "The response matches.");
        assert_eq!(result.error_kind, None);
        assert!(result.judge_trace.is_none());
        let judge_run = result.judge_run.as_ref().expect("judge run metadata");
        assert_eq!(judge_run.schema_version, 1);
        assert_eq!(judge_run.provider, "anthropic");
        assert_eq!(judge_run.model, "claude-sonnet-4-6");
        assert_eq!(judge_run.prompt_sha256.len(), 64);
        assert_eq!(judge_run.context_sha256.len(), 64);
        assert_eq!(capture.model_invocations.len(), 1);
        assert_eq!(capture.model_invocations[0].agent, "judge");
        assert_eq!(
            capture.model_invocations[0].provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(capture.model_invocations[0].model, "claude-sonnet-4-6");
        assert_eq!(capture.model_invocations[0].input_tokens, 12);
        assert_eq!(capture.model_invocations[0].output_tokens, 4);
    }

    #[tokio::test]
    async fn runtime_executor_includes_judge_trace_when_enabled() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            JudgeVerdictInterpreter,
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");
        let config = FinalResponseScorerConfig {
            include_judge_trace: true,
        };

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &config,
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        let trace = result.judge_trace.as_ref().expect("judge trace");
        assert!(trace.prompt.contains("Scorer id:\njudge_similarity"));
        assert!(trace
            .prompt
            .contains("Final response:\nBilling was updated for Acme."));
        assert_eq!(
            trace.response,
            r#"{"passed":true,"selected_rubric_score":1,"reason":"The response matches."}"#
        );
    }

    #[tokio::test]
    async fn runtime_executor_configures_judge_interpreter_for_strict_json() {
        let probe = Arc::new(Mutex::new(JudgeInterpreterConfigProbe::default()));
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            ConfigurableJudgeInterpreter::new(probe.clone()),
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        assert!(result.verdict.passed);

        let probe = probe.lock().expect("probe");
        assert_eq!(probe.dispatcher_tool_count, Some(0));
        assert_eq!(probe.max_turns, Some(1));
        assert_eq!(probe.model_settings, Some(judge_model_settings()));
    }

    #[tokio::test]
    async fn runtime_executor_reports_empty_judge_text_distinctly() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            JudgeReasoningOnlyInterpreter,
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        assert!(!result.verdict.passed);
        assert_eq!(result.verdict.selected_rubric_score, 0);
        assert!(
            result.verdict.reason.contains("Judge produced no text"),
            "unexpected reason: {}",
            result.verdict.reason
        );
        assert_eq!(result.error_kind, Some(JudgeResponseErrorKind::JudgeNoText));
        assert_eq!(capture.model_invocations.len(), 1);
        assert_eq!(capture.model_invocations[0].agent, "judge");
        assert_eq!(capture.model_invocations[0].output_tokens, 172);
    }

    #[tokio::test]
    async fn runtime_executor_reports_invalid_judge_json_distinctly() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            JudgeInvalidJsonInterpreter,
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        assert!(!result.verdict.passed);
        assert_eq!(
            result.error_kind,
            Some(JudgeResponseErrorKind::JudgeInvalidJson)
        );
        assert!(
            result.verdict.reason.contains("invalid verdict JSON"),
            "unexpected reason: {}",
            result.verdict.reason
        );
    }

    #[tokio::test]
    async fn runtime_executor_reports_invalid_judge_schema_distinctly() {
        let executor = RuntimeSampleExecutor::new(Arc::new(runtime_with_interpreter(Arc::new(
            JudgeInvalidSchemaInterpreter,
        ))));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        assert!(!result.verdict.passed);
        assert_eq!(
            result.error_kind,
            Some(JudgeResponseErrorKind::JudgeInvalidSchema)
        );
        assert!(
            result
                .verdict
                .reason
                .contains("did not match the expected schema"),
            "unexpected reason: {}",
            result.verdict.reason
        );
    }

    #[tokio::test]
    async fn runtime_executor_rejects_non_judge_capable_provider() {
        let executor = RuntimeSampleExecutor::new(Arc::new(
            runtime_with_interpreter_and_judge_capability(Arc::new(JudgeVerdictInterpreter), false),
        ));
        let spec = FinalResponseEvalSpec::from_value(&json!({
            "scorers": [
                {
                    "id": "judge_similarity",
                    "method": "judge",
                    "instructions": "Pass when the response is similar.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }))
        .expect("valid final response spec");

        let capture = executor
            .execute_final_response_judges(
                &spec,
                "Billing was updated for Acme.",
                &json!({"input": "Update billing."}),
                &resolved_model_config(),
                &FinalResponseScorerConfig::default(),
                None,
            )
            .await;

        let result = capture
            .results
            .get("judge_similarity")
            .expect("judge result");
        assert!(!result.verdict.passed);
        assert_eq!(
            result.error_kind,
            Some(JudgeResponseErrorKind::JudgeProviderUnavailable)
        );
        assert!(
            result
                .verdict
                .reason
                .contains("not configured as a judge-capable interpreter"),
            "unexpected reason: {}",
            result.verdict.reason
        );
        assert!(capture.model_invocations.is_empty());
    }

    #[tokio::test]
    async fn capture_stream_fails_on_runtime_error_part() {
        let err = capture_stream(
            boxed(vec![StreamPart::error(
                "no executor agent registered in the spec",
            )]),
            None,
            None,
            |_agent, _model| None,
        )
        .await
        .expect_err("runtime stream errors should fail the sample");

        assert!(
            err.to_string()
                .contains("no executor agent registered in the spec"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn merge_extra_objects_preserves_existing_fields() {
        let merged = merge_extra_objects(
            Some(json!({ "responseText": "done" })),
            merge_extra_objects(
                Some(resolved_actions_extra(&[])),
                crate::eval::planned_actions_extra(&[]),
            ),
        );

        assert_eq!(merged["responseText"], "done");
        assert_eq!(merged["resolvedActions"], json!([]));
        assert_eq!(merged["plannedActions"], json!([]));
    }

    #[test]
    fn provider_for_agent_falls_back_to_model_id() {
        let runtime = runtime_with_anthropic_agent();

        assert_eq!(
            provider_for_agent(&runtime, "unknown-agent", "claude-sonnet-4-6").as_deref(),
            Some("anthropic")
        );
    }

    #[test]
    fn thread_id_isolated_from_source_thread() {
        let mut test_case = agent_fw_eval::EvalTestCase {
            id: agent_fw_core::TestCaseId::new_unchecked("tc-1"),
            tags: vec![],
            input: "hello".to_string(),
            expected_trajectory: vec![],
            trajectory_mode: agent_fw_eval::TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: Some("source-thread".to_string()),
        };
        let input = SampleInput {
            test_case: test_case.clone(),
            sample_index: 0,
            eval_mode: agent_fw_eval::EvalMode::Sequential,
            target_agent_id: None,
            run_id: "run-1".to_string(),
        };
        assert_eq!(thread_id_for_sample(&input).as_str(), "eval-run-1-tc-1-0");

        test_case.source_thread_id = None;
        let input = SampleInput {
            test_case,
            sample_index: 2,
            eval_mode: agent_fw_eval::EvalMode::Sequential,
            target_agent_id: None,
            run_id: "run-1".to_string(),
        };
        assert_eq!(thread_id_for_sample(&input).as_str(), "eval-run-1-tc-1-2");
    }
}
