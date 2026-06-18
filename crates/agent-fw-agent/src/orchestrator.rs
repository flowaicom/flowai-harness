//! Multi-agent orchestrator with usage tracking and event forwarding.
//!
//! # Design
//!
//! The orchestrator manages a registry of named agents and coordinates
//! their execution. It implements `SubAgentInvoker` so tools can
//! invoke sub-agents through the standard algebra trait.
//!
//! # Algebraic Laws
//!
//! The orchestrator upholds the programs-as-values discipline:
//!
//! - **L1 Purity**: `ChatProgram` construction within `invoke` is pure — the
//!   program is built from the registration's system prompt and the request's
//!   prompt without any side effects. Only the interpreter execution is effectful.
//!
//! - **L2 Referential Transparency**: Given the same `SubAgentRequest` and
//!   agent registration, the orchestrator constructs the same `ChatProgram`.
//!   The interpreter may produce different outputs (LLM non-determinism), but
//!   the program description is deterministic.
//!
//! - **L3 Composition Associativity**: Sub-agent invocations compose
//!   associatively — the result of chaining agent A into agent B into agent C
//!   is independent of grouping. Usage tracking (a monoid) accumulates
//!   associatively via `TokenUsage::combine`.

use agent_fw_algebra::resource::bracket;
use agent_fw_algebra::sub_agent::{SubAgentRequest, SubAgentResult, ThreadScope};
use agent_fw_algebra::{
    AgentMemoryStore, CancellationToken, EventSink, SubAgentError, SubAgentInvoker,
};
use agent_fw_core::stream_part::{AgentUsage, CostSummary, ToolInvocationState};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::usage::TokenUsage;
use agent_fw_core::StreamPart;
use agent_fw_core::{LatencySummary, PhaseBreakdown};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::interpreter::ChatInterpreter;
use crate::model::{AgentLabel, ModelId};
use crate::tool_dispatch::ToolDispatcher;

/// Configuration for a registered agent.
#[derive(Debug, Clone)]
pub struct AgentRegistration {
    /// Agent name (used for invocation).
    pub name: String,
    /// Model to use.
    pub model: ModelId,
    /// System prompt.
    pub system_prompt: String,
    /// Opaque agent label (for logging, metrics, and consumer-side matching).
    pub role: Option<AgentLabel>,
    /// Whether this agent should load and append conversation memory.
    pub stateful: bool,
}

impl AgentRegistration {
    /// Create a named registration and use the name as an opaque label.
    pub fn new(
        name: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            role: Some(AgentLabel::new(name.as_str())),
            name,
            model: ModelId::new(model),
            system_prompt: prompt.into(),
            stateful: false,
        }
    }

    /// Create a custom-named agent registration.
    pub fn custom(
        name: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            role: Some(AgentLabel::new(name.clone())),
            name,
            model: ModelId::new(model),
            system_prompt: prompt.into(),
            stateful: false,
        }
    }

    /// Mark this registration as stateful or stateless.
    pub fn with_stateful(mut self, stateful: bool) -> Self {
        self.stateful = stateful;
        self
    }
}

/// Multi-agent coordinator with event forwarding and usage tracking.
///
/// All sub-agent events flow through a shared EventSink for SSE streaming.
/// Token usage is accumulated per-agent via monoid combination.
pub struct AgentOrchestrator {
    /// Registered agents.
    agents: HashMap<String, AgentRegistration>,
    /// Chat interpreters, either shared or overridden per agent.
    ///
    /// [`SubAgentInvoker::invoke`] resolves a base interpreter from this
    /// registry, then attaches the selected agent's dispatcher. That keeps
    /// provider/interpreter routing and tool routing independently composable.
    interpreters: AgentInterpreterRegistry,
    /// Optional per-agent tool dispatcher (runtime query assembly G1).
    ///
    /// When present, [`SubAgentInvoker::invoke`] calls
    /// [`ChatInterpreter::with_tool_dispatcher`] on the selected interpreter
    /// before interpreting so each sub-agent sees its own dispatcher.
    dispatchers: HashMap<String, Arc<dyn ToolDispatcher>>,
    /// Shared event sink.
    event_sink: Arc<dyn EventSink>,
    /// Tenant context.
    tenant: TenantContext,
    /// Cancellation token.
    cancel: CancellationToken,
    /// Per-agent accumulated usage (monoid combination).
    usage: Mutex<HashMap<String, AccumulatedUsage>>,
    /// Optional conversation memory store for stateful agents.
    memory: Arc<dyn AgentMemoryStore>,
    /// Request start time for wall-clock latency.
    request_start: Instant,
}

struct AgentInterpreterRegistry {
    default: Arc<dyn ChatInterpreter>,
    per_agent: HashMap<String, Arc<dyn ChatInterpreter>>,
}

impl AgentInterpreterRegistry {
    fn for_agent(&self, agent: &str) -> Arc<dyn ChatInterpreter> {
        self.per_agent
            .get(agent)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }
}

#[derive(Default)]
struct AgentInterpreterBuilder {
    default: Option<Arc<dyn ChatInterpreter>>,
    per_agent: HashMap<String, Arc<dyn ChatInterpreter>>,
}

impl AgentInterpreterBuilder {
    fn build(self) -> Result<AgentInterpreterRegistry, OrchestratorBuildError> {
        let default = self
            .default
            .ok_or(OrchestratorBuildError::MissingInterpreter)?;
        Ok(AgentInterpreterRegistry {
            default,
            per_agent: self.per_agent,
        })
    }
}

#[derive(Debug, Clone)]
struct AccumulatedUsage {
    model: String,
    usage: TokenUsage,
}

impl AgentOrchestrator {
    /// Create a builder for the orchestrator.
    pub fn builder() -> AgentOrchestratorBuilder {
        AgentOrchestratorBuilder::default()
    }

    /// Check if a named agent is registered.
    pub fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    /// List registered agent names.
    pub fn agent_names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    /// Record token usage for an agent (monoid combine).
    pub fn record_usage(&self, agent_name: &str, model: &str, usage: TokenUsage) {
        let mut map = self.usage.lock().unwrap_or_else(|e| {
            tracing::warn!("usage mutex was poisoned, recovering");
            e.into_inner()
        });
        let entry = map
            .entry(agent_name.to_string())
            .or_insert_with(|| AccumulatedUsage {
                model: model.to_string(),
                usage: TokenUsage::ZERO,
            });
        entry.usage = entry.usage.combine(&usage);
    }

    /// Generate a cost summary from accumulated usage.
    pub fn cost_summary(&self) -> CostSummary {
        let map = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let agents: Vec<AgentUsage> = map
            .iter()
            .map(|(name, acc)| AgentUsage {
                agent_name: name.clone(),
                model: acc.model.clone(),
                usage: acc.usage.clone(),
            })
            .collect();
        CostSummary::new(agents)
    }

    /// Emit cost and latency summary events.
    pub fn emit_summaries(&self) -> bool {
        let cost = self.cost_summary();
        self.event_sink.emit(StreamPart::cost_summary(cost))
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Cancel the orchestrator.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Get the cancellation token.
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Get the event sink.
    pub fn event_sink(&self) -> &Arc<dyn EventSink> {
        &self.event_sink
    }

    /// Get the tenant context.
    pub fn tenant(&self) -> &TenantContext {
        &self.tenant
    }

    /// Wall-clock elapsed time since orchestrator creation.
    pub fn elapsed(&self) -> std::time::Duration {
        self.request_start.elapsed()
    }
}

#[async_trait]
impl SubAgentInvoker for AgentOrchestrator {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        let registration = self
            .agents
            .get(&request.agent_name)
            .ok_or_else(|| SubAgentError::NotFound(request.agent_name.clone()))?;

        let invocation_id = request.resolved_invocation_id();

        // Emit sub-agent call event
        self.event_sink.emit(StreamPart::sub_agent_call(
            &registration.name,
            &invocation_id,
        ));

        let start = Instant::now();

        let agent_tenant = match request.thread_scope {
            ThreadScope::Current => self.tenant.clone(),
            ThreadScope::Derived => self.tenant.with_derived_thread(&registration.name),
        };

        // Build a ChatProgram for the agent (pure — L1). Stateful agents
        // receive the persisted non-system message history for their resolved
        // thread before the current prompt.
        let mut messages = Vec::new();
        messages.push(crate::conversation::ChatMessage::system(
            &registration.system_prompt,
        ));
        if registration.stateful {
            let history = self
                .memory
                .load(&agent_tenant, &registration.name)
                .await
                .map_err(|err| SubAgentError::Internal(err.to_string()))?;
            messages.extend(history);
        }
        messages.push(crate::conversation::ChatMessage::user(&request.prompt));

        let conversation = crate::conversation::parse_conversation(messages)
            .map_err(|e| SubAgentError::AgentFailed(e.to_string()))?;

        let program = crate::conversation::ChatProgram::new(
            conversation,
            registration.model.clone(),
            agent_tenant.clone(),
        );

        // Execute via interpreter within an explicit bracket:
        //   acquire: create child cancellation scope
        //   use:     consume stream, accumulate usage + text
        //   release: cancel child scope (LIFO cleanup)
        //
        // This makes the cleanup contract explicit and composable.
        let child_cancel = self.cancel.child();
        let base_interpreter = self.interpreters.for_agent(&registration.name);
        let interpreter = match self.dispatchers.get(&registration.name) {
            Some(dispatcher) => base_interpreter
                .clone()
                .with_tool_dispatcher(dispatcher.clone())
                .unwrap_or(base_interpreter),
            None => base_interpreter,
        };
        let event_sink = self.event_sink.clone();
        let agent_name_for_scope = request.agent_name.clone();

        let (response_text, total_usage, tool_interactions) = bracket(
            async { Ok::<_, SubAgentError>(child_cancel) },
            |child| {
                let interpreter = interpreter.clone();
                let event_sink = event_sink.clone();
                let agent_name = agent_name_for_scope.clone();
                Box::pin(async move {
                    use tokio_stream::StreamExt;
                    let mut stream = interpreter.interpret(program, child.clone());

                    let mut response_text = String::new();
                    let mut total_usage = TokenUsage::ZERO;
                    let mut pending_tool_calls: HashMap<String, (String, serde_json::Value)> =
                        HashMap::new();
                    let mut tool_interactions = Vec::new();

                    while let Some(part) = stream.next().await {
                        if let StreamPart::Error { error } = &part {
                            let message = error.message.clone();
                            event_sink.emit(part);
                            return Err(SubAgentError::AgentFailed(message));
                        }
                        match &part {
                            StreamPart::Text { text } => {
                                response_text.push_str(text);
                            }
                            StreamPart::Finish { usage, .. } => {
                                total_usage = total_usage.combine(usage);
                            }
                            StreamPart::ToolInvocation(data) => match &data.state {
                                ToolInvocationState::Call => {
                                    pending_tool_calls.insert(
                                        data.id.clone(),
                                        (data.name.clone(), data.args.clone()),
                                    );
                                }
                                ToolInvocationState::Result { result } => {
                                    if let Some((tool_name, arguments)) =
                                        pending_tool_calls.remove(&data.id)
                                    {
                                        tool_interactions.push(serde_json::json!({
                                            "callId": data.id,
                                            "toolName": tool_name,
                                            "arguments": arguments,
                                            "result": result,
                                        }));
                                    }
                                }
                            },
                            _ => {}
                        }
                        // Forward events through the sink, scoping sub-agent
                        // tool progress with agent name prefix for frontend
                        // disambiguation (mirrors Python TeeEventSink pattern).
                        let forwarded = match part {
                            StreamPart::ToolProgress(mut data) => {
                                data.tool_name = format!("{}/{}", agent_name, data.tool_name);
                                StreamPart::ToolProgress(data)
                            }
                            other => other,
                        };
                        event_sink.emit(forwarded);
                    }

                    Ok((response_text, total_usage, tool_interactions))
                })
            },
            |child| {
                Box::pin(async move {
                    // Explicit cleanup: cancel the child scope.
                    // Ensures any in-flight interpreter work is cancelled
                    // even if the stream was not fully consumed (e.g., on error).
                    child.cancel();
                })
            },
        )
        .await?;

        let duration = start.elapsed();
        let latency = LatencySummary {
            total_duration_ms: duration.as_millis() as u64,
            phases: PhaseBreakdown::ZERO.with_sub_agent_time(duration.as_millis() as u64),
            ..Default::default()
        };
        let model = registration.model.as_str().to_string();

        // Record usage
        self.record_usage(&request.agent_name, &model, total_usage.clone());

        if registration.stateful {
            let user = crate::conversation::ChatMessage::user(request.prompt.clone());
            let assistant = if tool_interactions.is_empty() {
                crate::conversation::ChatMessage::assistant(response_text.clone())
            } else {
                crate::conversation::ChatMessage::assistant(response_text.clone())
                    .with_tool_interactions(tool_interactions)
            };
            self.memory
                .append_turn(&agent_tenant, &registration.name, user, assistant)
                .await
                .map_err(|err| SubAgentError::Internal(err.to_string()))?;
        }

        // Emit sub-agent result event
        self.event_sink.emit(StreamPart::sub_agent_result(
            &registration.name,
            &invocation_id,
        ));

        Ok(SubAgentResult::new(
            request.agent_name,
            invocation_id,
            response_text,
            total_usage,
            model,
        )
        .with_latency(Some(latency)))
    }

    fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    fn available_agents(&self) -> Vec<String> {
        self.agent_names()
    }

    fn cost_summary(&self) -> Option<CostSummary> {
        Some(AgentOrchestrator::cost_summary(self))
    }
}

/// Error building an [`AgentOrchestrator`].
///
/// Each variant names exactly one missing required field, avoiding the
/// stringly-typed `Result<_, String>` anti-pattern.
#[derive(Debug, Clone)]
pub enum OrchestratorBuildError {
    /// `with_interpreter` was never called.
    MissingInterpreter,
    /// `with_tenant` was never called.
    MissingTenant,
    /// At least one stateful agent was registered, but no memory store was
    /// provided.
    MissingMemoryStore { stateful_agents: Vec<String> },
}

impl std::fmt::Display for OrchestratorBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingInterpreter => write!(f, "interpreter is required"),
            Self::MissingTenant => write!(f, "tenant is required"),
            Self::MissingMemoryStore { stateful_agents } => write!(
                f,
                "memory_store is required for stateful agents: {}",
                stateful_agents.join(", ")
            ),
        }
    }
}

impl std::error::Error for OrchestratorBuildError {}

/// Builder for AgentOrchestrator.
#[derive(Default)]
pub struct AgentOrchestratorBuilder {
    agents: Vec<AgentRegistration>,
    interpreters: AgentInterpreterBuilder,
    tenant: Option<TenantContext>,
    event_sink: Option<Arc<dyn EventSink>>,
    cancel: Option<CancellationToken>,
    dispatchers: HashMap<String, Arc<dyn ToolDispatcher>>,
    memory: Option<Arc<dyn AgentMemoryStore>>,
}

/// Error building agents from configuration.
#[derive(Debug, Clone)]
pub enum AgentBuildError {
    /// A system prompt file could not be read.
    PromptReadError {
        role: String,
        path: String,
        reason: String,
    },
}

impl std::fmt::Display for AgentBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PromptReadError { role, path, reason } => {
                write!(
                    f,
                    "failed to read system prompt for role '{role}' at {path}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for AgentBuildError {}

/// Configuration for a single agent role used by framework-native builders.
#[derive(Debug, Clone)]
pub struct RoleConfigInput {
    /// Agent name used for invocation and metadata labeling.
    pub name: String,
    /// Path to system prompt file, relative to project root. If `None`, uses an empty prompt.
    pub system_prompt_path: Option<String>,
    /// Inline system prompt content. Takes precedence over `system_prompt_path`.
    pub system_prompt_inline: Option<String>,
    /// Tool names (informational — not used for registration, but available for wiring).
    pub tools: Vec<String>,
    /// Delegation targets (informational).
    pub delegations: Vec<String>,
}

impl AgentOrchestratorBuilder {
    /// Build agent registrations from a list of named agent configs.
    ///
    /// For each agent, reads the system prompt (inline or from file) and
    /// creates an `AgentRegistration` with the given model.
    ///
    /// # Arguments
    ///
    /// * `roles` — Named agent configurations (name, prompt, tools, delegations).
    /// * `model` — Model to use for all agents (can be overridden later).
    /// * `project_root` — Root directory for resolving relative prompt file paths.
    pub fn from_roles(
        mut self,
        roles: &[RoleConfigInput],
        model: &ModelId,
        project_root: &std::path::Path,
    ) -> Result<Self, AgentBuildError> {
        for role in roles {
            let system_prompt = if let Some(ref inline) = role.system_prompt_inline {
                inline.clone()
            } else if let Some(ref path) = role.system_prompt_path {
                let full_path = project_root.join(path);
                std::fs::read_to_string(&full_path).map_err(|e| {
                    AgentBuildError::PromptReadError {
                        role: role.name.clone(),
                        path: full_path.display().to_string(),
                        reason: e.to_string(),
                    }
                })?
            } else {
                String::new()
            };

            self.agents.push(AgentRegistration {
                name: role.name.clone(),
                model: model.clone(),
                system_prompt,
                role: Some(AgentLabel::new(role.name.clone())),
                stateful: false,
            });
        }
        Ok(self)
    }

    // ── Fluent builder methods (read as sentences) ────────────────────

    /// `.agent(registration)` — add an agent to the network.
    pub fn agent(mut self, registration: AgentRegistration) -> Self {
        self.agents.push(registration);
        self
    }

    /// `.agents(registrations)` — add multiple agents.
    pub fn agents(mut self, registrations: impl IntoIterator<Item = AgentRegistration>) -> Self {
        self.agents.extend(registrations);
        self
    }

    /// `.interpreter(arc)` — set the chat interpreter (required).
    pub fn interpreter(mut self, interpreter: Arc<dyn ChatInterpreter>) -> Self {
        self.interpreters.default = Some(interpreter);
        self
    }

    /// `.agent_interpreter(name, interpreter)` — register a per-agent
    /// [`ChatInterpreter`]. The orchestrator selects this interpreter for the
    /// named agent before attaching that agent's optional dispatcher.
    pub fn agent_interpreter(
        mut self,
        agent: impl Into<String>,
        interpreter: Arc<dyn ChatInterpreter>,
    ) -> Self {
        self.interpreters
            .per_agent
            .insert(agent.into(), interpreter);
        self
    }

    /// `.interpreters_per_agent(map)` — bulk-register per-agent interpreters.
    ///
    /// Equivalent to repeated [`agent_interpreter`](Self::agent_interpreter)
    /// calls. Replaces any existing entries with the same agent name.
    pub fn interpreters_per_agent(
        mut self,
        interpreters: HashMap<String, Arc<dyn ChatInterpreter>>,
    ) -> Self {
        self.interpreters.per_agent.extend(interpreters);
        self
    }

    /// `.tenant(ctx)` — set the tenant context (required).
    pub fn tenant(mut self, tenant: TenantContext) -> Self {
        self.tenant = Some(tenant);
        self
    }

    /// `.event_sink(arc)` — set the event sink (defaults to null).
    pub fn event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// `.cancel(token)` — set the cancellation token (defaults to fresh).
    pub fn cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// `.agent_dispatcher(name, dispatcher)` — register a per-agent
    /// [`ToolDispatcher`] (runtime query assembly G1). When the orchestrator invokes the
    /// named agent it asks the interpreter for a clone that has this
    /// dispatcher attached (see
    /// [`ChatInterpreter::with_tool_dispatcher`]). Agents without an entry
    /// keep using the orchestrator-level interpreter unchanged.
    pub fn agent_dispatcher(
        mut self,
        agent: impl Into<String>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Self {
        self.dispatchers.insert(agent.into(), dispatcher);
        self
    }

    /// `.dispatchers_per_agent(map)` — bulk-register per-agent dispatchers.
    ///
    /// Equivalent to repeated [`agent_dispatcher`](Self::agent_dispatcher)
    /// calls. Replaces any existing entries with the same agent name.
    pub fn dispatchers_per_agent(
        mut self,
        dispatchers: HashMap<String, Arc<dyn ToolDispatcher>>,
    ) -> Self {
        self.dispatchers.extend(dispatchers);
        self
    }

    /// `.memory_store(arc)` — install conversation memory for stateful agents.
    pub fn memory_store(mut self, memory: Arc<dyn AgentMemoryStore>) -> Self {
        self.memory = Some(memory);
        self
    }

    // ── Backward-compat aliases (with_ prefix) ─────────────────────

    #[doc(hidden)]
    pub fn with_agent(self, registration: AgentRegistration) -> Self {
        self.agent(registration)
    }

    #[doc(hidden)]
    pub fn with_agents(self, registrations: impl IntoIterator<Item = AgentRegistration>) -> Self {
        self.agents(registrations)
    }

    #[doc(hidden)]
    pub fn with_interpreter(self, interpreter: Arc<dyn ChatInterpreter>) -> Self {
        self.interpreter(interpreter)
    }

    #[doc(hidden)]
    pub fn with_agent_interpreter(
        self,
        agent: impl Into<String>,
        interpreter: Arc<dyn ChatInterpreter>,
    ) -> Self {
        self.agent_interpreter(agent, interpreter)
    }

    #[doc(hidden)]
    pub fn with_tenant(self, tenant: TenantContext) -> Self {
        self.tenant(tenant)
    }

    #[doc(hidden)]
    pub fn with_event_sink(self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink(sink)
    }

    #[doc(hidden)]
    pub fn with_cancel(self, cancel: CancellationToken) -> Self {
        self.cancel(cancel)
    }

    // ── Build ───────────────────────────────────────────────────────

    /// Build the orchestrator.
    ///
    /// Returns `Err` if required fields (`interpreter`, `tenant`) were not set.
    /// Event sink defaults to a null sink; cancel defaults to a fresh token.
    ///
    /// Reads as:
    /// ```ignore
    /// AgentOrchestrator::builder()
    ///     .agent(coordinator)
    ///     .agent(planner)
    ///     .interpreter(interp)
    ///     .tenant(tenant)
    ///     .event_sink(sink)
    ///     .cancel(cancel)
    ///     .build()?;
    /// ```
    pub fn build(self) -> Result<AgentOrchestrator, OrchestratorBuildError> {
        let interpreters = self.interpreters.build()?;
        let tenant = self.tenant.ok_or(OrchestratorBuildError::MissingTenant)?;
        let stateful_agents = self
            .agents
            .iter()
            .filter(|agent| agent.stateful)
            .map(|agent| agent.name.clone())
            .collect::<Vec<_>>();
        let memory = match self.memory {
            Some(memory) => memory,
            None if stateful_agents.is_empty() => {
                Arc::new(agent_fw_algebra::testing::NullAgentMemoryStore)
            }
            None => {
                return Err(OrchestratorBuildError::MissingMemoryStore { stateful_agents });
            }
        };

        let mut agents = HashMap::new();
        for reg in self.agents {
            agents.insert(reg.name.clone(), reg);
        }

        Ok(AgentOrchestrator {
            agents,
            interpreters,
            dispatchers: self.dispatchers,
            event_sink: self
                .event_sink
                .unwrap_or_else(|| Arc::new(agent_fw_algebra::testing::NullEventSink)),
            tenant,
            cancel: self.cancel.unwrap_or_default(),
            usage: Mutex::new(HashMap::new()),
            memory,
            request_start: Instant::now(),
        })
    }

    /// Build with explicit event_sink and cancel overrides.
    ///
    /// Backward-compatible entry point. Prefer `.event_sink(s).cancel(c).build()`.
    #[doc(hidden)]
    pub fn build_with(
        self,
        event_sink: Arc<dyn EventSink>,
        cancel: CancellationToken,
    ) -> Result<AgentOrchestrator, OrchestratorBuildError> {
        self.event_sink(event_sink).cancel(cancel).build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentMemoryError, ChatMessage, ChatRole};

    #[test]
    fn agent_registration() {
        let reg = AgentRegistration::new("planner", "claude-sonnet", "You plan things");
        assert_eq!(reg.name, "planner");
        assert_eq!(reg.model.as_str(), "claude-sonnet");
        assert_eq!(reg.role.as_ref().map(AgentLabel::as_str), Some("planner"));
    }

    #[test]
    fn usage_accumulation() {
        // Test the monoid combination of usage
        let u1 = TokenUsage::simple(100, 50);
        let u2 = TokenUsage::simple(200, 75);
        let combined = u1.combine(&u2);
        assert_eq!(combined.prompt_tokens, 300);
        assert_eq!(combined.completion_tokens, 125);
    }

    // ── AgentRegistration labels ─────────────────────────────────────

    #[test]
    fn custom_constructor() {
        let reg = AgentRegistration::custom("analyst", "claude-sonnet", "You analyze");
        assert_eq!(reg.name, "analyst");
        assert_eq!(reg.role.as_ref().map(AgentLabel::as_str), Some("analyst"));
    }

    // ─── runtime query assembly G1 / G2: per-agent dispatcher + sub-agent thread-id ─────

    use crate::conversation::ChatProgram;
    use crate::interpreter::ChatInterpreter;
    use crate::tool_dispatch::{ToolCallResult, ToolDefinition};
    use agent_fw_algebra::sub_agent::SubAgentRequest;
    use agent_fw_core::id::{TenantId, ThreadId};
    use agent_fw_core::stream_part::FinishReason;
    use agent_fw_core::StreamPart;
    use async_trait::async_trait;
    use futures::stream;
    use futures::Stream;
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;

    /// Records the `ChatProgram` it sees and replies with a canned finish.
    struct ProbeInterpreter {
        seen_thread_id: Arc<StdMutex<Option<String>>>,
        /// Optional dispatcher field — `with_tool_dispatcher` returns a fresh
        /// `ProbeInterpreter` carrying the supplied dispatcher so the
        /// orchestrator can be observed picking the right per-agent one.
        dispatcher: Option<Arc<dyn ToolDispatcher>>,
        /// When set, every `interpret(...)` call records the agent name that
        /// the attached dispatcher reports via `tool_definitions().first()`.
        seen_dispatcher_name: Arc<StdMutex<Option<String>>>,
    }

    impl ProbeInterpreter {
        fn new(
            seen_thread_id: Arc<StdMutex<Option<String>>>,
            seen_dispatcher_name: Arc<StdMutex<Option<String>>>,
        ) -> Self {
            Self {
                seen_thread_id,
                dispatcher: None,
                seen_dispatcher_name,
            }
        }
    }

    impl ChatInterpreter for ProbeInterpreter {
        fn interpret(
            &self,
            program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            // G2 probe: capture the sub-agent's derived thread id.
            *self.seen_thread_id.lock().unwrap() =
                program.tenant().thread_id().map(|t| t.as_str().to_string());
            // G1 probe: capture the per-agent dispatcher's "name" via the
            // tag stored in its first ToolDefinition.
            if let Some(d) = self.dispatcher.as_ref() {
                let tag = d
                    .tool_definitions()
                    .first()
                    .map(|td| td.name.clone())
                    .unwrap_or_default();
                *self.seen_dispatcher_name.lock().unwrap() = Some(tag);
            }
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }

        fn with_tool_dispatcher(
            self: Arc<Self>,
            dispatcher: Arc<dyn ToolDispatcher>,
        ) -> Option<Arc<dyn ChatInterpreter>> {
            Some(Arc::new(ProbeInterpreter {
                seen_thread_id: self.seen_thread_id.clone(),
                dispatcher: Some(dispatcher),
                seen_dispatcher_name: self.seen_dispatcher_name.clone(),
            }))
        }
    }

    /// A `ToolDispatcher` whose only purpose is to carry an identifying
    /// `name` so a probe interpreter can observe which dispatcher the
    /// orchestrator picked for a given agent.
    struct NamedDispatcher {
        name: String,
    }

    #[async_trait]
    impl ToolDispatcher for NamedDispatcher {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: self.name.clone(),
                description: String::new(),
                input_schema: serde_json::json!({}),
            }]
        }
        async fn dispatch(
            &self,
            _tool_name: &str,
            tool_use_id: &str,
            _input: serde_json::Value,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, serde_json::json!({}))
        }
    }

    #[tokio::test]
    async fn g1_orchestrator_attaches_per_agent_dispatcher() {
        let thread_probe = Arc::new(StdMutex::new(None));
        let dispatcher_probe = Arc::new(StdMutex::new(None));
        let interp = Arc::new(ProbeInterpreter::new(
            thread_probe.clone(),
            dispatcher_probe.clone(),
        ));
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let planner_dispatcher: Arc<dyn ToolDispatcher> = Arc::new(NamedDispatcher {
            name: "planner-dispatcher".to_string(),
        });
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan"))
            .interpreter(interp)
            .tenant(tenant)
            .agent_dispatcher("planner", planner_dispatcher)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("planner", "do the thing"))
            .await
            .expect("invoke");

        assert_eq!(
            dispatcher_probe.lock().unwrap().as_deref(),
            Some("planner-dispatcher"),
            "orchestrator should pass the planner's dispatcher to its interpreter",
        );
    }

    #[tokio::test]
    async fn g1_no_dispatcher_registered_keeps_base_interpreter() {
        let thread_probe = Arc::new(StdMutex::new(None));
        let dispatcher_probe = Arc::new(StdMutex::new(None));
        let interp = Arc::new(ProbeInterpreter::new(
            thread_probe.clone(),
            dispatcher_probe.clone(),
        ));
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("executor", "claude-sonnet", "run"))
            .interpreter(interp)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("executor", "go"))
            .await
            .expect("invoke");

        // No dispatcher attached → ProbeInterpreter's dispatcher field is None.
        assert_eq!(dispatcher_probe.lock().unwrap().as_deref(), None);
    }

    struct RoutingProbeInterpreter {
        name: String,
        dispatcher: Option<Arc<dyn ToolDispatcher>>,
        seen: Arc<StdMutex<Vec<(String, Option<String>)>>>,
    }

    impl RoutingProbeInterpreter {
        fn new(
            name: impl Into<String>,
            seen: Arc<StdMutex<Vec<(String, Option<String>)>>>,
        ) -> Self {
            Self {
                name: name.into(),
                dispatcher: None,
                seen,
            }
        }
    }

    impl ChatInterpreter for RoutingProbeInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            let dispatcher_name = self.dispatcher.as_ref().and_then(|d| {
                d.tool_definitions()
                    .first()
                    .map(|definition| definition.name.clone())
            });
            self.seen
                .lock()
                .unwrap()
                .push((self.name.clone(), dispatcher_name));
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }

        fn with_tool_dispatcher(
            self: Arc<Self>,
            dispatcher: Arc<dyn ToolDispatcher>,
        ) -> Option<Arc<dyn ChatInterpreter>> {
            Some(Arc::new(Self {
                name: self.name.clone(),
                dispatcher: Some(dispatcher),
                seen: self.seen.clone(),
            }))
        }
    }

    #[tokio::test]
    async fn c4_agent_specific_interpreter_is_selected_before_dispatcher_attachment() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let base_interpreter = Arc::new(RoutingProbeInterpreter::new("base", seen.clone()));
        let planner_interpreter = Arc::new(RoutingProbeInterpreter::new(
            "planner-interpreter",
            seen.clone(),
        ));
        let planner_dispatcher: Arc<dyn ToolDispatcher> = Arc::new(NamedDispatcher {
            name: "planner-dispatcher".to_string(),
        });
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));

        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan"))
            .interpreter(base_interpreter)
            .agent_interpreter("planner", planner_interpreter)
            .agent_dispatcher("planner", planner_dispatcher)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("planner", "do the thing"))
            .await
            .expect("invoke");

        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[(
                "planner-interpreter".to_string(),
                Some("planner-dispatcher".to_string())
            )],
            "orchestrator should pick the per-agent interpreter, then attach that agent's dispatcher",
        );
    }

    #[tokio::test]
    async fn g2_sub_agent_chat_program_carries_derived_thread_id() {
        let thread_probe = Arc::new(StdMutex::new(None));
        let dispatcher_probe = Arc::new(StdMutex::new(None));
        let interp = Arc::new(ProbeInterpreter::new(
            thread_probe.clone(),
            dispatcher_probe,
        ));
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan"))
            .interpreter(interp)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("planner", "do"))
            .await
            .expect("invoke");

        // runtime query assembly G2: framework helper format `{parent}-{agent}`.
        assert_eq!(
            thread_probe.lock().unwrap().as_deref(),
            Some("thread-A-planner"),
        );
    }

    #[tokio::test]
    async fn entry_agent_can_keep_current_thread_scope() {
        let thread_probe = Arc::new(StdMutex::new(None));
        let dispatcher_probe = Arc::new(StdMutex::new(None));
        let interp = Arc::new(ProbeInterpreter::new(
            thread_probe.clone(),
            dispatcher_probe,
        ));
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new(
                "coordinator",
                "claude-sonnet",
                "coordinate",
            ))
            .interpreter(interp)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("coordinator", "do").with_current_thread())
            .await
            .expect("invoke");

        assert_eq!(thread_probe.lock().unwrap().as_deref(), Some("thread-A"));
    }

    struct ConversationProbeInterpreter {
        seen: Arc<StdMutex<Vec<Vec<(ChatRole, String)>>>>,
    }

    impl ChatInterpreter for ConversationProbeInterpreter {
        fn interpret(
            &self,
            program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            self.seen.lock().unwrap().push(
                program
                    .conversation()
                    .messages()
                    .iter()
                    .map(|message| (message.role, message.content.clone()))
                    .collect(),
            );
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::text("fresh response"),
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }
    }

    struct ErrorAfterTextInterpreter;

    impl ChatInterpreter for ErrorAfterTextInterpreter {
        fn interpret(
            &self,
            _program: ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::text("partial response"),
                StreamPart::error("terminal failure"),
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }
    }

    #[derive(Default)]
    struct RecordingMemoryStore {
        loaded: Vec<ChatMessage>,
        appended: Arc<StdMutex<Vec<(String, String, String, String)>>>,
    }

    #[async_trait]
    impl AgentMemoryStore for RecordingMemoryStore {
        async fn load(
            &self,
            tenant: &TenantContext,
            agent: &str,
        ) -> Result<Vec<ChatMessage>, AgentMemoryError> {
            self.appended.lock().unwrap().push((
                "load".to_string(),
                tenant
                    .thread_id()
                    .map(|thread| thread.as_str().to_string())
                    .unwrap_or_default(),
                agent.to_string(),
                String::new(),
            ));
            Ok(self.loaded.clone())
        }

        async fn append_turn(
            &self,
            tenant: &TenantContext,
            agent: &str,
            user: ChatMessage,
            assistant: ChatMessage,
        ) -> Result<(), AgentMemoryError> {
            self.appended.lock().unwrap().push((
                "append".to_string(),
                tenant
                    .thread_id()
                    .map(|thread| thread.as_str().to_string())
                    .unwrap_or_default(),
                agent.to_string(),
                format!("{} -> {}", user.content, assistant.content),
            ));
            Ok(())
        }
    }

    #[tokio::test]
    async fn stateful_agent_loads_history_and_appends_successful_turn() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let appended = Arc::new(StdMutex::new(Vec::new()));
        let memory = Arc::new(RecordingMemoryStore {
            loaded: vec![
                ChatMessage::user("prior user"),
                ChatMessage::assistant("prior assistant"),
            ],
            appended: appended.clone(),
        });
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan").with_stateful(true))
            .interpreter(Arc::new(ConversationProbeInterpreter {
                seen: seen.clone(),
            }))
            .memory_store(memory)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("planner", "current").with_current_thread())
            .await
            .expect("invoke");

        let messages = seen.lock().unwrap().first().cloned().unwrap();
        assert_eq!(
            messages,
            vec![
                (ChatRole::System, "plan".to_string()),
                (ChatRole::User, "prior user".to_string()),
                (ChatRole::Assistant, "prior assistant".to_string()),
                (ChatRole::User, "current".to_string()),
            ]
        );
        assert_eq!(
            appended.lock().unwrap().as_slice(),
            &[
                (
                    "load".to_string(),
                    "thread-A".to_string(),
                    "planner".to_string(),
                    String::new()
                ),
                (
                    "append".to_string(),
                    "thread-A".to_string(),
                    "planner".to_string(),
                    "current -> fresh response".to_string()
                ),
            ]
        );
    }

    #[test]
    fn stateful_agent_requires_explicit_memory_store() {
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let err = match AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan").with_stateful(true))
            .interpreter(Arc::new(ConversationProbeInterpreter {
                seen: Arc::new(StdMutex::new(Vec::new())),
            }))
            .tenant(tenant)
            .build()
        {
            Ok(_) => panic!("stateful registration without memory should fail build"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            OrchestratorBuildError::MissingMemoryStore { ref stateful_agents }
                if stateful_agents == &vec!["planner".to_string()]
        ));
    }

    #[tokio::test]
    async fn stateful_agent_does_not_append_or_emit_result_after_stream_error() {
        let appended = Arc::new(StdMutex::new(Vec::new()));
        let memory = Arc::new(RecordingMemoryStore {
            loaded: vec![],
            appended: appended.clone(),
        });
        let sink = Arc::new(agent_fw_algebra::testing::RecordingEventSink::new());
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("planner", "claude-sonnet", "plan").with_stateful(true))
            .interpreter(Arc::new(ErrorAfterTextInterpreter))
            .memory_store(memory)
            .event_sink(sink.clone())
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let err = orchestrator
            .invoke(SubAgentRequest::new("planner", "current").with_current_thread())
            .await
            .expect_err("terminal stream error should fail invocation");
        assert!(
            matches!(err, SubAgentError::AgentFailed(message) if message == "terminal failure")
        );

        assert_eq!(
            appended.lock().unwrap().as_slice(),
            &[(
                "load".to_string(),
                "thread-A".to_string(),
                "planner".to_string(),
                String::new()
            )],
            "memory should load but never append after terminal stream error"
        );
        let events = sink.events();
        assert!(
            events
                .iter()
                .any(|part| matches!(part, StreamPart::Error { .. })),
            "interpreter error should still be forwarded"
        );
        assert!(
            !events.iter().any(|part| matches!(
                part,
                StreamPart::ToolAgent(data)
                    if matches!(data.state, agent_fw_core::stream_part::ToolAgentState::Result)
            )),
            "sub-agent result must not be emitted after terminal stream error"
        );
    }

    #[tokio::test]
    async fn stateless_agent_does_not_load_or_append_memory() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let appended = Arc::new(StdMutex::new(Vec::new()));
        let memory = Arc::new(RecordingMemoryStore {
            loaded: vec![ChatMessage::user("prior user")],
            appended: appended.clone(),
        });
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-A"));
        let orchestrator = AgentOrchestrator::builder()
            .agent(AgentRegistration::new("executor", "claude-sonnet", "run"))
            .interpreter(Arc::new(ConversationProbeInterpreter {
                seen: seen.clone(),
            }))
            .memory_store(memory)
            .tenant(tenant)
            .build()
            .expect("orchestrator builds");

        let _ = orchestrator
            .invoke(SubAgentRequest::new("executor", "current").with_current_thread())
            .await
            .expect("invoke");

        let messages = seen.lock().unwrap().first().cloned().unwrap();
        assert_eq!(
            messages,
            vec![
                (ChatRole::System, "run".to_string()),
                (ChatRole::User, "current".to_string()),
            ]
        );
        assert!(appended.lock().unwrap().is_empty());
    }
}
