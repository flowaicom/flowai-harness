//! Canonical Flow AI Harness runtime engine.
//!
//! `flowai-runtime` is the Harness implementation in Rust. The Python
//! and future TypeScript `flowai-harness` libraries validate and build
//! ergonomic user-facing specs, then bind to this engine. This crate owns
//! Harness-level composition: tenant identity, role wiring, approval policy
//! attachment, provider routing metadata, and runtime entrypoints.
//!
//! The generic framework primitives stay in `agent-fw-*` crates. This crate
//! describes how the Flow AI Harness composes those primitives.
//!
//! # Example
//!
//! ```
//! use flowai_runtime::{AgentSpec, AgentRole, ModelSpec, RuntimeSpec};
//!
//! let mut spec = RuntimeSpec::minimal("acme", "v1");
//! let mut coordinator = AgentSpec::new(
//!         "coordinator",
//!         AgentRole::Coordinator,
//!         ModelSpec::new("claude-sonnet-4-6"),
//!         "You coordinate analytical work.",
//! );
//! coordinator.routes = vec!["planner".to_string()];
//! spec.agents.push(coordinator);
//! spec.agents.push(AgentSpec::new(
//!         "planner",
//!         AgentRole::Planner,
//!         ModelSpec::new("claude-sonnet-4-6"),
//!         "You produce typed plans.",
//! ));
//!
//! assert_eq!(spec.agent_count(), 2);
//! assert!(spec.agent("coordinator").is_some());
//! ```

pub mod data;
pub mod eval;
pub mod mcp;
pub mod plans;
pub mod references;
pub mod runtime;
pub mod storage;
pub mod toolkits;
pub mod traces;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::pin::Pin;
use std::sync::Arc;

pub use agent_fw_eval::{
    TraceActor, TraceOmissionReason, TracePayload, TraceProvenance, TraceRecord, TraceRef,
    TraceScope, TraceStage, TraceStatus, TraceStep,
};
pub use agent_fw_reference::{ArtifactRef, StoredReference};
pub use eval::presets::{
    add_default_final_response_weight as eval_add_default_final_response_weight,
    default_score_weights_for_preset as eval_default_score_weights_for_preset,
    default_score_weights_for_preset_and_test_cases as eval_default_score_weights_for_preset_and_test_cases,
    materialize_score_weights as eval_materialize_score_weights,
    scorer_for_eval_test_case_with_config as eval_scorer_for_eval_test_case_with_config,
    scorer_for_mode as eval_scorer_for_mode, scorer_for_preset as eval_scorer_for_preset,
    scorer_for_test_case as eval_scorer_for_test_case,
    scorer_for_test_case_with_config as eval_scorer_for_test_case_with_config,
    validate_specialist_explicit_score_weights as eval_validate_specialist_explicit_score_weights,
    ActionMatchResult, ActionScorer, ActionSource, ActionStatus, ComparisonSummary,
    PresetScorerError, DEFAULT_EXECUTOR_ACTION_WEIGHT, DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT,
    DEFAULT_FINAL_RESPONSE_WEIGHT, DEFAULT_PLANNED_ACTION_WEIGHT, DEFAULT_SEQUENTIAL_ACTION_WEIGHT,
    DEFAULT_TRAJECTORY_WEIGHT, PRESET_EXECUTOR, PRESET_PLANNER, PRESET_SEQUENTIAL,
    PRESET_SPECIALIST, PRESET_TEST_CASE_BUILDER, PRESET_TRAJECTORY_ONLY, SCORER_EXECUTED_ACTIONS,
    SCORER_FINAL_RESPONSE, SCORER_PLANNED_ACTIONS, SCORER_TRAJECTORY,
};
pub use eval::runner::{EvalArtifact, EvalEventStream, EvalRequest, EvalRunner, EvalRunnerError};
pub use mcp::{RuntimeMcpConfig, RuntimeMcpError};
pub use plans::{
    HarnessActionContext, HydratingDispatcher, HydrationError, PlanProtocolError, PlanRegistry,
};
pub use references::{ReferenceError, ReferenceRegistry};
pub use toolkits::{ToolkitConfig, ToolkitError};
pub use traces::{
    NoopTraceSink, RecordingTraceSink, SharedTraceSink, TraceListFilter, TraceSink, TraceSinkError,
};

use agent_fw_agent::{
    AgentLabel, AgentOrchestrator, AgentRegistration, ChatInterpreter, ComposedDispatcher, ModelId,
    OrchestratorBuildError, ToolHandler,
};
use agent_fw_algebra::{CancellationToken, EventSink, KVStore, TargetDatabase};
use agent_fw_catalog::{CatalogSearchBackend, DataCatalog};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{PlanId, TenantId, ThreadId, WorkspaceContext};
use agent_fw_plan::{Plan, PlanStatus};
use agent_fw_tool::ToolEnvironment;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Canonical pure runtime specification consumed by the Flow AI Harness engine.
///
/// This is intentionally inspectable without constructing providers, opening
/// stores, or making LLM calls. Language facades validate their native schemas
/// and compile down to this value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSpec {
    /// Runtime tenant identity aligned with the framework [`TenantId`].
    pub tenant: TenantIdentity,
    /// Registered coordinator, planner, executor, and specialist agents.
    pub agents: Vec<AgentSpec>,
    /// Named typed reference declarations.
    pub references: Vec<ReferenceSpec>,
    /// Plan declarations compiled to the fixed `agent-fw-plan` lifecycle.
    pub plans: Vec<PlanSpec>,
    /// Default and vertical toolkits attached to the runtime.
    pub toolkits: Vec<ToolkitSpec>,
    /// Runtime-level approval policy floor.
    pub approval_policies: ApprovalPolicies,
    /// Agent- and tool-scoped approval policy overrides.
    #[serde(default, skip_serializing_if = "ApprovalOverrides::is_empty")]
    pub approval_overrides: ApprovalOverrides,
    /// Store factory descriptions supplied by the host language facade.
    pub storage_factories: StorageFactories,
    /// Provider transport configs keyed by provider kind.
    pub providers: ProviderRegistry,
}

impl RuntimeSpec {
    /// Build a minimal runtime spec with no agents or declarations.
    pub fn minimal(resource_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            tenant: TenantIdentity::new(resource_id, version),
            agents: vec![],
            references: vec![],
            plans: vec![],
            toolkits: vec![],
            approval_policies: ApprovalPolicies::default(),
            approval_overrides: ApprovalOverrides::default(),
            storage_factories: StorageFactories::default(),
            providers: ProviderRegistry::default(),
        }
    }

    /// Number of agent specifications in this runtime.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Look up an agent specification by name.
    pub fn agent(&self, name: &str) -> Option<&AgentSpec> {
        self.agents.iter().find(|agent| agent.name == name)
    }
}

/// Runtime tenant identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TenantIdentity {
    /// Tenant/resource identifier used for framework isolation.
    pub resource_id: TenantId,
    /// Tenant identity version for inspectable runtime specs.
    pub version: String,
}

impl TenantIdentity {
    /// Build tenant identity from raw strings.
    pub fn new(resource_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            resource_id: TenantId::new_unchecked(resource_id.into()),
            version: version.into(),
        }
    }
}

/// Harness-owned Flow AI role protocol.
///
/// ADR 0002 / runtime ownership keep these semantics in `flowai-runtime`; the
/// framework only receives an opaque [`AgentLabel`] for logs and metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Coordinator,
    Planner,
    Executor,
    Specialist,
}

impl AgentRole {
    /// Stable wire string for this Flow AI harness role.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Coordinator => "coordinator",
            Self::Planner => "planner",
            Self::Executor => "executor",
            Self::Specialist => "specialist",
        }
    }

    /// Harness-owned default turn budget.
    pub fn suggested_max_turns(&self) -> u32 {
        match self {
            Self::Coordinator => 12,
            Self::Planner => 8,
            Self::Executor => 4,
            Self::Specialist => 12,
        }
    }

    /// Harness default for whether an agent keeps conversation state.
    pub fn default_stateful(&self) -> bool {
        matches!(self, Self::Coordinator | Self::Planner)
    }

    /// Convert to framework metadata at the runtime assembly boundary.
    pub fn to_agent_label(self) -> AgentLabel {
        AgentLabel::new(self.as_str())
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Agent definition compiled by the runtime into `AgentRegistration`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpec {
    /// Stable agent name used for direct calls and delegation.
    pub name: String,
    /// Semantic role in the Harness protocol.
    pub role: AgentRole,
    /// Whether this role should retain conversation state across turns.
    pub stateful: bool,
    /// Per-agent model selection.
    pub model: ModelSpec,
    /// System prompt after facade-side prompt composition.
    pub system_prompt: String,
    /// Delegation route names for coordinator-like agents.
    #[serde(default)]
    pub routes: Vec<String>,
    /// Toolkit IDs attached to this agent.
    #[serde(default)]
    pub toolkits: Vec<String>,
    /// Optional per-agent maximum number of LLM/tool turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

impl AgentSpec {
    /// Construct a named agent spec.
    pub fn new(
        name: impl Into<String>,
        role: AgentRole,
        model: ModelSpec,
        system_prompt: impl Into<String>,
    ) -> Self {
        let role = role;
        Self {
            name: name.into(),
            role,
            stateful: role.default_stateful(),
            model,
            system_prompt: system_prompt.into(),
            routes: vec![],
            toolkits: vec![],
            max_turns: None,
        }
    }

    fn to_registration(&self) -> AgentRegistration {
        AgentRegistration {
            name: self.name.clone(),
            model: ModelId::new(self.model.id.clone()),
            system_prompt: self.system_prompt.clone(),
            role: Some(self.role.to_agent_label()),
            stateful: self.stateful,
        }
    }
}

impl<'de> Deserialize<'de> for AgentSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AgentSpecWire {
            name: String,
            role: AgentRole,
            #[serde(default)]
            stateful: Option<bool>,
            model: ModelSpec,
            system_prompt: String,
            #[serde(default)]
            routes: Vec<String>,
            #[serde(default)]
            toolkits: Vec<String>,
            #[serde(default)]
            max_turns: Option<u32>,
        }

        let wire = AgentSpecWire::deserialize(deserializer)?;
        if matches!(wire.max_turns, Some(0)) {
            return Err(serde::de::Error::custom(
                "agent maxTurns must be at least 1",
            ));
        }
        Ok(Self {
            name: wire.name,
            role: wire.role,
            stateful: wire
                .stateful
                .unwrap_or_else(|| wire.role.default_stateful()),
            model: wire.model,
            system_prompt: wire.system_prompt,
            routes: wire.routes,
            toolkits: wire.toolkits,
            max_turns: wire.max_turns,
        })
    }
}

/// Per-agent model selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    /// Provider-specific model identifier.
    pub id: String,
    /// Optional explicit provider key when model-family routing is ambiguous.
    #[serde(default)]
    pub provider: Option<String>,
}

impl ModelSpec {
    /// Create a model spec that uses provider auto-routing.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            provider: None,
        }
    }
}

/// Named typed memory pointer declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceSpec {
    /// Reference type name.
    pub name: String,
    /// JSON schema for the referenced payload.
    pub schema: JsonValue,
    /// Optional time-to-live in milliseconds.
    #[serde(default)]
    pub ttl_ms: Option<u64>,
}

/// Generic Harness action carried by `Plan<HarnessAction>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessAction {
    /// Discriminator for vertical-defined action payloads.
    pub kind: String,
    /// Vertical-defined action payload.
    #[serde(default)]
    pub payload: JsonValue,
    /// Artifact references used by the action.
    #[serde(default)]
    pub references: Vec<ArtifactRef>,
}

/// Plan declaration compiled to `agent-fw-plan::Plan<HarnessAction>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSpec {
    /// Plan type name.
    pub name: String,
    /// JSON schema for the plan body.
    pub schema: JsonValue,
    /// Display labels for fixed framework statuses.
    #[serde(default)]
    pub display_aliases: Vec<PlanDisplayAlias>,
}

/// Optional display alias for a fixed framework lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDisplayAlias {
    /// Framework lifecycle status.
    pub status: PlanStatus,
    /// User-facing alias for the status.
    pub alias: String,
}

/// Toolkit declaration by stable identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolkitSpec {
    /// Toolkit identifier, such as `catalog`.
    pub id: String,
    /// Toolkit-specific configuration.
    #[serde(default)]
    pub config: JsonValue,
}

/// Runtime-level approval policy floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPolicies {
    /// Plan execution approval floor.
    pub plans: ApprovalRule,
    /// Tool dispatch approval floor.
    pub tools: ApprovalRule,
}

/// Partial approval policy override.
///
/// Missing channels inherit from the higher-precedence parent policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPolicyPatch {
    /// Optional plan execution approval override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plans: Option<ApprovalRule>,
    /// Optional tool dispatch approval override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ApprovalRule>,
}

/// Hierarchical approval overrides scoped by agent and tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOverrides {
    /// Per-agent default policy overrides keyed by agent name.
    #[serde(default)]
    pub agents: BTreeMap<String, ApprovalPolicyPatch>,
    /// Per-tool policy overrides keyed by agent name, then tool name.
    #[serde(default)]
    pub tools: BTreeMap<String, BTreeMap<String, ApprovalRule>>,
}

impl ApprovalOverrides {
    /// Whether no hierarchical overrides are configured.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.tools.is_empty()
    }
}

impl Default for ApprovalPolicies {
    fn default() -> Self {
        Self {
            plans: ApprovalRule::Always,
            tools: ApprovalRule::Never,
        }
    }
}

/// Approval rule attached to plans or tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ApprovalRule {
    /// Never pause for approval.
    Never,
    /// Always pause for approval.
    Always,
    /// Defer to a facade-defined dynamic policy by identifier.
    Dynamic(String),
}

/// Store factory descriptions carried in the pure runtime spec.
///
/// Concrete interpreter construction is handled by [`storage`] descriptors at
/// runtime construction time, while this field remains a serialisable metadata
/// surface for SDKs and hosts that want to carry store intent in the spec.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageFactories {
    /// Key-value store factory.
    #[serde(default)]
    pub kv: Option<StorageFactorySpec>,
    /// Plan store factory.
    #[serde(default)]
    pub plans: Option<StorageFactorySpec>,
    /// Conversation memory store factory.
    #[serde(default)]
    pub memory: Option<StorageFactorySpec>,
}

/// One host-provided store factory descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageFactorySpec {
    /// Store backend kind.
    pub kind: String,
    /// Store backend configuration.
    #[serde(default)]
    pub config: JsonValue,
}

/// Provider registry keyed by provider kind, such as `anthropic` or `bedrock`.
///
/// The map key is the provider identity. The value is the provider-specific
/// transport configuration object passed by a language facade.
pub type ProviderRegistry = BTreeMap<String, ProviderConfig>;

/// Provider key used by the compatibility [`RuntimeDeps::new`] constructor.
pub const DEFAULT_PROVIDER_KEY: &str = "anthropic";

/// Provider-specific transport configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderConfig {
    /// Opaque provider-specific transport configuration.
    pub config: JsonValue,
}

impl ProviderConfig {
    /// Build a provider configuration from normalized JSON.
    pub fn new(config: JsonValue) -> Self {
        Self { config }
    }
}

/// Dependencies required to turn a pure spec into an effectful orchestrator.
#[derive(Clone)]
pub struct RuntimeDeps {
    /// Chat interpreters keyed by provider.
    ///
    /// [`Runtime::new`] resolves each [`AgentSpec::model`] to a provider key,
    /// validates that the provider is declared in [`RuntimeSpec::providers`],
    /// and then requires an interpreter under that same key here.
    pub interpreter_providers: BTreeMap<String, Arc<dyn ChatInterpreter>>,
    /// Provider keys whose interpreters are allowed to execute judge scorers.
    ///
    /// Deterministic testing interpreters may be valid for sample generation
    /// but invalid as LLM-as-judge providers. Keeping this capability explicit
    /// prevents mock text from being parsed as a judge verdict.
    pub judge_capable_interpreter_providers: BTreeSet<String>,
    /// Event sink used by the underlying orchestrator.
    pub event_sink: Arc<dyn EventSink>,
    /// Tenant context for this runtime handle.
    pub tenant: TenantContext,
    /// Cancellation root for orchestrated work.
    pub cancel: CancellationToken,
    /// Key-value store backing the reference registry (reference registry, C2) and
    /// future plan / memory storage. Tenant isolation is the KV
    /// interpreter's responsibility (KVStore L9).
    pub kv: Arc<dyn KVStore>,
    /// Optional customer-supplied `ActionDispatcher` invoked by the
    /// executor through [`HydratingDispatcher`] (plan registry) once a plan
    /// transitions `Draft → Approved`. Defaults to a noop dispatcher when
    /// `None`; Python adapter (PyO3) wires the real Python-backed adapter.
    pub action_dispatcher: Option<Arc<HarnessActionDispatcher>>,
    /// Host-provided tools keyed by agent name. These handlers are composed
    /// into the same per-request dispatcher as framework toolkits, then pass
    /// through the canonical guarded/approval/traced layer stack.
    pub host_tools: BTreeMap<String, Vec<HostToolBinding>>,
    /// Optional data catalog dependency attached to per-request tool
    /// environments for built-in catalog-backed toolkits.
    pub data_catalog: Option<Arc<dyn DataCatalog>>,
    /// Optional catalog search backend dependency attached to per-request
    /// environments for the replacement catalog toolkit.
    pub catalog_search_backend: Option<Arc<dyn CatalogSearchBackend>>,
    /// Optional data-environment workspace scope attached to per-request tool
    /// environments for workspace-local KV payload hydration.
    pub data_workspace_context: Option<WorkspaceContext>,
    /// Optional target database dependency attached to per-request tool
    /// environments for catalog tools that read warehouse data.
    pub target_database: Option<Arc<dyn TargetDatabase>>,
    /// Dynamic approval predicates keyed by serialisable runtime policy id.
    pub approval_predicates: runtime::approval::ApprovalPredicateRegistry,
    /// Canonical trace sink used by eval/runtime paths to persist trace records.
    pub trace_sink: SharedTraceSink,
}

impl RuntimeDeps {
    /// Create runtime dependencies with a fresh cancellation token.
    pub fn new(
        interpreter: Arc<dyn ChatInterpreter>,
        event_sink: Arc<dyn EventSink>,
        tenant: TenantContext,
        kv: Arc<dyn KVStore>,
    ) -> Self {
        let mut interpreter_providers = BTreeMap::new();
        interpreter_providers.insert(DEFAULT_PROVIDER_KEY.to_string(), interpreter);
        Self::from_interpreter_providers(interpreter_providers, event_sink, tenant, kv)
    }

    /// Create runtime dependencies from provider-keyed chat interpreters.
    pub fn from_interpreter_providers(
        interpreter_providers: BTreeMap<String, Arc<dyn ChatInterpreter>>,
        event_sink: Arc<dyn EventSink>,
        tenant: TenantContext,
        kv: Arc<dyn KVStore>,
    ) -> Self {
        let judge_capable_interpreter_providers = interpreter_providers.keys().cloned().collect();
        Self {
            interpreter_providers,
            judge_capable_interpreter_providers,
            event_sink,
            tenant,
            cancel: CancellationToken::new(),
            kv,
            action_dispatcher: None,
            host_tools: BTreeMap::new(),
            data_catalog: None,
            catalog_search_backend: None,
            data_workspace_context: None,
            target_database: None,
            approval_predicates: BTreeMap::new(),
            trace_sink: Arc::new(NoopTraceSink),
        }
    }

    /// Register or replace the interpreter for one provider key.
    pub fn with_interpreter_provider(
        mut self,
        provider: impl Into<String>,
        interpreter: Arc<dyn ChatInterpreter>,
    ) -> Self {
        let provider = provider.into();
        self.interpreter_providers
            .insert(provider.clone(), interpreter);
        self.judge_capable_interpreter_providers.insert(provider);
        self
    }

    /// Mark whether a provider-keyed interpreter may execute judge scorers.
    pub fn with_judge_capable_interpreter_provider(
        mut self,
        provider: impl Into<String>,
        judge_capable: bool,
    ) -> Self {
        let provider = provider.into();
        if judge_capable {
            self.judge_capable_interpreter_providers.insert(provider);
        } else {
            self.judge_capable_interpreter_providers.remove(&provider);
        }
        self
    }

    /// Override the cancellation token.
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Override the harness action dispatcher used for approved-plan
    /// execution. When unset the runtime falls back to
    /// [`NoopActionDispatcher`](crate::runtime::action::NoopActionDispatcher).
    pub fn with_action_dispatcher(mut self, dispatcher: Arc<HarnessActionDispatcher>) -> Self {
        self.action_dispatcher = Some(dispatcher);
        self
    }

    /// Register a host tool for a single agent.
    pub fn with_host_tool(mut self, agent: impl Into<String>, binding: HostToolBinding) -> Self {
        self.host_tools
            .entry(agent.into())
            .or_default()
            .push(binding);
        self
    }

    /// Register a data catalog dependency for the built-in `catalog` toolkit.
    pub fn with_data_catalog(mut self, catalog: Arc<dyn DataCatalog>) -> Self {
        self.data_catalog = Some(catalog);
        self
    }

    /// Register a catalog search backend dependency for the built-in
    /// replacement `catalog` toolkit.
    pub fn with_catalog_search_backend(mut self, backend: Arc<dyn CatalogSearchBackend>) -> Self {
        self.catalog_search_backend = Some(backend);
        self
    }

    /// Register the data-environment workspace scope used by tools that
    /// hydrate workspace-local KV payloads.
    pub fn with_data_workspace_context(mut self, context: WorkspaceContext) -> Self {
        self.data_workspace_context = Some(context);
        self
    }

    /// Register a target database dependency for built-in catalog tools such
    /// as `execute_query` and `sample_table_data`.
    pub fn with_target_database(mut self, target_database: Arc<dyn TargetDatabase>) -> Self {
        self.target_database = Some(target_database);
        self
    }

    /// Register a trace sink for canonical runtime/eval trace records.
    pub fn with_trace_sink(mut self, trace_sink: SharedTraceSink) -> Self {
        self.trace_sink = trace_sink;
        self
    }

    /// Register a dynamic approval predicate by id.
    pub fn with_approval_predicate(
        mut self,
        name: impl Into<String>,
        predicate: agent_fw_agent::approval::ApprovalPredicate,
    ) -> Self {
        self.approval_predicates.insert(name.into(), predicate);
        self
    }
}

/// Host-provided tool handler plus its optional approval override.
#[derive(Clone)]
pub struct HostToolBinding {
    /// Concrete handler supplied by a language binding or application host.
    pub handler: Arc<dyn ToolHandler>,
    /// Per-tool approval rule. When set, this is compiled into the runtime's
    /// framework [`ApprovalPolicy`](agent_fw_agent::approval::ApprovalPolicy)
    /// under `handler.definition().name`.
    pub approval: Option<ApprovalRule>,
}

impl HostToolBinding {
    /// Build a host binding with no per-tool approval override.
    pub fn new(handler: Arc<dyn ToolHandler>) -> Self {
        Self {
            handler,
            approval: None,
        }
    }

    /// Attach a per-tool approval rule.
    pub fn with_approval(mut self, approval: ApprovalRule) -> Self {
        self.approval = Some(approval);
        self
    }
}

/// Customer-supplied action dispatcher invoked by the executor once a
/// plan is approved.
///
/// Implementations execute the vertical-defined `HarnessAction` payloads
/// from the approved plan. The error type is fixed to [`HarnessActionError`]
/// so the trait is object-safe — bindings (Python via Python adapter, TypeScript via
/// napi-rs / WASM) wrap their own error into a string-backed
/// `HarnessActionError`.
pub type HarnessActionDispatcher = dyn agent_fw_plan::ActionDispatcher<
    Action = HarnessAction,
    Context = HarnessActionContext,
    Error = HarnessActionError,
>;

/// Action-dispatch error surface returned by [`HarnessActionDispatcher`].
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HarnessActionError {
    /// Free-form error message from the action dispatcher implementation.
    pub message: String,
}

impl HarnessActionError {
    /// Construct a new action error from any message-like value.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Convert a pure runtime description into an `AgentOrchestrator`.
pub trait IntoOrchestrator {
    /// Build an orchestrator from a pure spec and effectful dependencies.
    fn into_orchestrator(self, deps: RuntimeDeps) -> Result<AgentOrchestrator, RuntimeError>;
}

impl IntoOrchestrator for RuntimeSpec {
    fn into_orchestrator(self, deps: RuntimeDeps) -> Result<AgentOrchestrator, RuntimeError> {
        build_orchestrator(&self, deps)
    }
}

fn build_orchestrator(
    spec: &RuntimeSpec,
    deps: RuntimeDeps,
) -> Result<AgentOrchestrator, RuntimeError> {
    if &spec.tenant.resource_id != deps.tenant.resource_id() {
        return Err(RuntimeError::TenantMismatch {
            spec_resource_id: spec.tenant.resource_id.clone(),
            runtime_resource_id: deps.tenant.resource_id().clone(),
        });
    }

    let registrations = spec
        .agents
        .iter()
        .map(AgentSpec::to_registration)
        .collect::<Vec<_>>();
    let (default_interpreter, agent_interpreters) =
        resolve_orchestrator_interpreters(spec, &deps.interpreter_providers)?;

    AgentOrchestrator::builder()
        .agents(registrations)
        .interpreter(default_interpreter)
        .interpreters_per_agent(agent_interpreters)
        .tenant(deps.tenant)
        .event_sink(deps.event_sink)
        .memory_store(Arc::new(runtime::memory::KvAgentMemoryStore::new(
            deps.kv.clone(),
        )))
        .cancel(deps.cancel)
        .build()
        .map_err(RuntimeError::from)
}

fn validate_all_dispatchers(
    agents: &[AgentSpec],
    available: &[ToolkitSpec],
    references: &Arc<ReferenceRegistry>,
    plans: &Arc<PlanRegistry>,
    host_tools: &BTreeMap<String, Vec<HostToolBinding>>,
    env: &ToolEnvironment,
) -> Result<(), RuntimeError> {
    for agent in agents {
        compose_dispatcher_for_agent(agent, available, references, plans, host_tools, env.clone())?;
    }
    Ok(())
}

fn validate_runtime_spec(spec: &RuntimeSpec) -> Result<(), RuntimeError> {
    let mut names = BTreeSet::new();
    let mut coordinator_count = 0usize;

    for agent in &spec.agents {
        if !names.insert(agent.name.clone()) {
            return Err(RuntimeError::AgentSpecInvalid(format!(
                "duplicate agent name '{}'",
                agent.name
            )));
        }
        if agent.role == AgentRole::Coordinator {
            coordinator_count += 1;
        }
    }

    if coordinator_count > 1 {
        return Err(RuntimeError::AgentSpecInvalid(
            "multiple coordinator agents are not supported".to_string(),
        ));
    }

    for agent in &spec.agents {
        if agent.role == AgentRole::Coordinator && agent.routes.is_empty() {
            return Err(RuntimeError::AgentSpecInvalid(format!(
                "coordinator agent '{}' requires at least one route",
                agent.name
            )));
        }

        let mut seen_routes = BTreeSet::new();
        for route in &agent.routes {
            if route == &agent.name {
                return Err(RuntimeError::AgentSpecInvalid(format!(
                    "agent '{}' cannot route to itself",
                    agent.name
                )));
            }
            if !seen_routes.insert(route.clone()) {
                return Err(RuntimeError::AgentSpecInvalid(format!(
                    "agent '{}' declares duplicate route '{}'",
                    agent.name, route
                )));
            }
            if !names.contains(route) {
                return Err(RuntimeError::AgentSpecInvalid(format!(
                    "agent '{}' declares unknown route target '{}'",
                    agent.name, route
                )));
            }
        }
    }

    let unknown_agent_overrides: Vec<&String> = spec
        .approval_overrides
        .agents
        .keys()
        .filter(|name| !names.contains(*name))
        .collect();
    if !unknown_agent_overrides.is_empty() {
        return Err(RuntimeError::AgentSpecInvalid(format!(
            "approvalOverrides.agents references unknown agent(s): {:?}",
            unknown_agent_overrides
        )));
    }

    let unknown_tool_overrides: Vec<&String> = spec
        .approval_overrides
        .tools
        .keys()
        .filter(|name| !names.contains(*name))
        .collect();
    if !unknown_tool_overrides.is_empty() {
        return Err(RuntimeError::AgentSpecInvalid(format!(
            "approvalOverrides.tools references unknown agent(s): {:?}",
            unknown_tool_overrides
        )));
    }

    Ok(())
}

fn validate_catalog_search_backend_requirement(
    spec: &RuntimeSpec,
    deps: &RuntimeDeps,
) -> Result<(), RuntimeError> {
    if deps.catalog_search_backend.is_some() {
        return Ok(());
    }

    if let Some(agent) = spec
        .agents
        .iter()
        .find(|agent| agent.toolkits.iter().any(|toolkit| toolkit == "catalog"))
    {
        return Err(RuntimeError::CatalogSearchBackendMissing {
            agent: agent.name.clone(),
        });
    }

    Ok(())
}

fn compose_dispatcher_for_agent(
    agent: &AgentSpec,
    available: &[ToolkitSpec],
    references: &Arc<ReferenceRegistry>,
    plans: &Arc<PlanRegistry>,
    host_tools: &BTreeMap<String, Vec<HostToolBinding>>,
    env: ToolEnvironment,
) -> Result<Option<ComposedDispatcher>, RuntimeError> {
    let mut dispatcher =
        toolkits::compose_agent_dispatcher(agent, available, references, plans, env)?;

    if let Some(bindings) = host_tools.get(&agent.name) {
        for binding in bindings {
            dispatcher.add_handler(binding.handler.clone());
        }
    }

    dispatcher
        .validate_no_collisions()
        .map_err(|collisions| RuntimeError::Toolkit(ToolkitError::Collisions(collisions)))?;

    if dispatcher.is_empty() {
        Ok(None)
    } else {
        Ok(Some(dispatcher))
    }
}

fn resolve_orchestrator_interpreters(
    spec: &RuntimeSpec,
    interpreter_providers: &BTreeMap<String, Arc<dyn ChatInterpreter>>,
) -> Result<
    (
        Arc<dyn ChatInterpreter>,
        HashMap<String, Arc<dyn ChatInterpreter>>,
    ),
    RuntimeError,
> {
    let default_interpreter = interpreter_providers
        .get(DEFAULT_PROVIDER_KEY)
        .cloned()
        .or_else(|| interpreter_providers.values().next().cloned())
        .ok_or_else(|| RuntimeError::ProviderInterpreterMissing {
            agent: "<default>".to_string(),
            provider: DEFAULT_PROVIDER_KEY.to_string(),
            model: String::new(),
        })?;
    let mut agent_interpreters = HashMap::new();

    for agent in &spec.agents {
        let provider = runtime::providers::select_provider_key(&agent.model);
        if !spec.providers.contains_key(&provider) {
            return Err(RuntimeError::ProviderUnregistered {
                agent: agent.name.clone(),
                provider,
                model: agent.model.id.clone(),
            });
        }
        let interpreter = interpreter_providers
            .get(&provider)
            .cloned()
            .ok_or_else(|| RuntimeError::ProviderInterpreterMissing {
                agent: agent.name.clone(),
                provider: provider.clone(),
                model: agent.model.id.clone(),
            })?;
        let interpreter = match agent.max_turns {
            Some(max_turns) => interpreter
                .clone()
                .with_max_turns(max_turns as usize)
                .unwrap_or(interpreter),
            None => interpreter,
        };
        agent_interpreters.insert(agent.name.clone(), interpreter);
    }

    Ok((default_interpreter, agent_interpreters))
}

/// Stream type returned by runtime entrypoints.
///
/// Items are framework-standard [`StreamPart`]s — the same envelope every
/// `ChatInterpreter` and `AgentOrchestrator` emits — so language bindings
/// receive the full token-level event log. Construction-time failures are
/// returned as `Result<Self, RuntimeError>` on [`Runtime::new`]; stream-time
/// failures flow through as [`StreamPart::Error`] items.
pub type RuntimeEventStream =
    Pin<Box<dyn Stream<Item = agent_fw_core::StreamPart> + Send + 'static>>;

/// Runtime event stream paired with a request-scoped cancellation handle.
///
/// Cancelling this handle cancels the request token and every child token
/// derived for the coordinator, sub-agents, tools, approvals, and provider
/// streams in that request.
pub struct CancellableRuntimeEventStream {
    stream: RuntimeEventStream,
    cancel: CancellationToken,
}

impl CancellableRuntimeEventStream {
    /// Create a cancellable runtime stream from a stream and request token.
    pub fn new(stream: RuntimeEventStream, cancel: CancellationToken) -> Self {
        Self { stream, cancel }
    }

    /// Cancel the request and all downstream child tokens.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether the request token is already cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Clone the request-scoped cancellation token for host integrations.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Consume this wrapper and return only the event stream.
    pub fn into_stream(self) -> RuntimeEventStream {
        self.stream
    }

    /// Consume this wrapper and return both stream and request token.
    pub fn into_parts(self) -> (RuntimeEventStream, CancellationToken) {
        (self.stream, self.cancel)
    }
}

/// Opaque runtime handle exposed to language bindings.
pub struct Runtime {
    spec: RuntimeSpec,
    references: Arc<ReferenceRegistry>,
    plans: Arc<PlanRegistry>,
    /// Tenant id, cached from `deps.tenant` for registry lookups that
    /// the orchestrator doesn't carry.
    tenant: agent_fw_core::TenantId,
    /// Shared pending-approval store used by both the tool gate
    /// (`ApprovalLayer`) and the plan gate (`GatedPlanExecutor`).
    /// `respond_to_approval` resolves entries here.
    approval_store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore>,
    /// Compiled approval policy floor.
    approval_policy: Arc<agent_fw_agent::approval::ApprovalPolicy>,
    /// Effective approval policy per agent after hierarchy resolution.
    agent_approval_policies: BTreeMap<String, Arc<agent_fw_agent::approval::ApprovalPolicy>>,
    /// Provider-keyed chat interpreters used by per-request orchestrators.
    interpreter_providers: BTreeMap<String, Arc<dyn ChatInterpreter>>,
    /// Provider keys whose interpreters are allowed to execute judge scorers.
    judge_capable_interpreter_providers: BTreeSet<String>,
    /// Cancellation root for per-request orchestrators.
    cancel_root: CancellationToken,
    /// KV store shared with the reference / plan registries.
    kv: Arc<dyn KVStore>,
    /// Customer plan-action dispatcher (defaults to `NoopActionDispatcher`).
    action_dispatcher: Arc<HarnessActionDispatcher>,
    /// Host-provided callback tools keyed by agent name.
    host_tools: BTreeMap<String, Vec<HostToolBinding>>,
    /// Data catalog attached to per-request tool environments.
    data_catalog: Option<Arc<dyn DataCatalog>>,
    /// Catalog search backend attached to per-request tool environments.
    catalog_search_backend: Option<Arc<dyn CatalogSearchBackend>>,
    /// Data-environment workspace scope attached to per-request tool
    /// environments.
    data_workspace_context: Option<WorkspaceContext>,
    /// Target database attached to per-request tool environments.
    target_database: Option<Arc<dyn TargetDatabase>>,
    /// Synthetic approver id used when a host has not supplied one.
    approver: agent_fw_core::UserId,
    /// KV-backed conversation memory for stateful agents.
    agent_memory: Arc<dyn agent_fw_algebra::AgentMemoryStore>,
    /// Runtime/eval trace sink.
    trace_sink: SharedTraceSink,
}

impl Runtime {
    /// Build a runtime handle from a pure spec and effectful dependencies.
    ///
    /// Startup validation eagerly composes every agent's per-toolkit
    /// dispatcher once — solely to surface configuration errors
    /// (unknown toolkit IDs, malformed config, cross-toolkit name
    /// collisions) at startup rather than on the first user request.
    /// The validation dispatcher is discarded; the C4 runner produces
    /// the working dispatcher per request via [`Runtime::dispatcher_for`].
    pub fn new(spec: RuntimeSpec, deps: RuntimeDeps) -> Result<Self, RuntimeError> {
        validate_runtime_spec(&spec)?;
        if &spec.tenant.resource_id != deps.tenant.resource_id() {
            return Err(RuntimeError::TenantMismatch {
                spec_resource_id: spec.tenant.resource_id.clone(),
                runtime_resource_id: deps.tenant.resource_id().clone(),
            });
        }

        // Build the reference registry first (reference registry) — fails fast on a
        // malformed schema or duplicate spec name, before any LLM-side
        // wiring runs.
        let references = Arc::new(ReferenceRegistry::new(
            spec.references.clone(),
            deps.kv.clone(),
        )?);
        // Plan registry (plan registry) holds an Arc to the reference
        // registry so HydratingDispatcher can pre-resolve refs during
        // executor dispatch without re-threading the runtime.
        let plans = Arc::new(PlanRegistry::new(
            spec.plans.clone(),
            deps.kv.clone(),
            references.clone(),
        )?);
        let tenant = deps.tenant.resource_id().clone();

        // Default toolkit startup validation (default toolkit composition C5). Cross-toolkit
        // name collisions, unknown toolkit IDs, and malformed config
        // are caught here via a dry-run composition. The dispatcher is
        // discarded because its env intentionally lacks the per-request
        // extensions (`DataCatalog`, `TargetDatabase`) that the C4
        // runner supplies through `dispatcher_for`.
        let validation_env = ToolEnvironment::builder()
            .kv_arc(deps.kv.clone())
            .event_sink_arc(deps.event_sink.clone())
            .tenant_context(deps.tenant.clone())
            .cancel(deps.cancel.clone())
            .build();
        validate_all_dispatchers(
            &spec.agents,
            &spec.toolkits,
            &references,
            &plans,
            &deps.host_tools,
            &validation_env,
        )?;
        validate_catalog_search_backend_requirement(&spec, &deps)?;

        // Eager provider/interpreter validation. Each agent's `model`
        // resolves to a provider key owned by this runtime crate; the generic
        // framework only sees the resulting per-agent interpreter map.
        let _ = resolve_orchestrator_interpreters(&spec, &deps.interpreter_providers)?;

        let approval_store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> = Arc::new(
            agent_fw_interpreter::KvPendingApprovalStore::new(deps.kv.clone()),
        );
        let approval_policy =
            runtime::approval::compile_policy(&spec.approval_policies, &deps.approval_predicates)?;
        let agent_approval_policies = runtime::approval::compile_agent_policies(
            &spec,
            &deps.host_tools,
            &deps.approval_predicates,
        )?;
        let approval_policy = Arc::new(approval_policy);
        let action_dispatcher: Arc<HarnessActionDispatcher> = deps
            .action_dispatcher
            .unwrap_or_else(|| Arc::new(runtime::NoopActionDispatcher));
        let approver = agent_fw_core::UserId::new_unchecked("runtime");
        let agent_memory: Arc<dyn agent_fw_algebra::AgentMemoryStore> =
            Arc::new(runtime::memory::KvAgentMemoryStore::new(deps.kv.clone()));

        Ok(Self {
            spec,
            references,
            plans,
            tenant,
            approval_store,
            approval_policy,
            agent_approval_policies,
            interpreter_providers: deps.interpreter_providers,
            judge_capable_interpreter_providers: deps.judge_capable_interpreter_providers,
            cancel_root: deps.cancel,
            kv: deps.kv,
            action_dispatcher,
            host_tools: deps.host_tools,
            data_catalog: deps.data_catalog,
            catalog_search_backend: deps.catalog_search_backend,
            data_workspace_context: deps.data_workspace_context,
            target_database: deps.target_database,
            approver,
            agent_memory,
            trace_sink: deps.trace_sink,
        })
    }

    /// Access to the shared pending-approval store (runtime query assembly).
    ///
    /// Both the tool gate ([`agent_fw_agent::ApprovalLayer`]) and the plan
    /// gate ([`agent_fw_plan::executor::GatedPlanExecutor`]) take this same
    /// `Arc`, so a single [`Runtime::respond_to_approval`] call wakes
    /// whichever gate is awaiting the supplied id.
    pub fn approval_store(&self) -> &Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> {
        &self.approval_store
    }

    /// Access to the runtime trace sink.
    pub fn trace_sink(&self) -> &SharedTraceSink {
        &self.trace_sink
    }

    /// Access to the compiled approval policy floor (runtime query assembly).
    pub fn approval_policy(&self) -> &Arc<agent_fw_agent::approval::ApprovalPolicy> {
        &self.approval_policy
    }

    /// Effective approval policy for an agent after hierarchical overrides.
    pub fn approval_policy_for(
        &self,
        agent: &str,
    ) -> Arc<agent_fw_agent::approval::ApprovalPolicy> {
        self.agent_approval_policies
            .get(agent)
            .cloned()
            .unwrap_or_else(|| self.approval_policy.clone())
    }

    /// Access to the reference registry (reference registry). The runtime's
    /// `reference()` and `reference_glimpse()` methods delegate here.
    pub fn references(&self) -> &Arc<ReferenceRegistry> {
        &self.references
    }

    /// Access to the plan registry (plan registry). The runtime's
    /// `propose_plan()` and `plan()` methods delegate here.
    pub fn plans(&self) -> &Arc<PlanRegistry> {
        &self.plans
    }

    /// Compose the per-agent tool dispatcher with a caller-supplied
    /// per-request [`ToolEnvironment`] (default toolkit composition, C5).
    ///
    /// The returned dispatcher is the canonical per-request handle: it has
    /// the agent's toolkits composed (default toolkit composition C5) AND is wrapped with the
    /// framework's `guarded → approval → traced` layer stack (pre-dispatch approval B2),
    /// so every caller — including the runtime's own per-request
    /// orchestrator wiring — sees the same gated, traced surface. There is
    /// no other way to get an ungated dispatcher from the runtime.
    ///
    /// The `env` must carry every extension the selected toolkits
    /// require — typically `DataCatalog` and `CatalogSearchBackend` for
    /// `search_catalog`, `TargetDatabase` for `execute_query` and
    /// `sample_table_data`, plus the per-request
    /// `kv` / `event_sink` / `tenant`. Build the env with the
    /// `agent-fw-tool` builder and decorate it with
    /// `CatalogToolEnvironmentExt::with_catalog` /
    /// `ToolEnvironment::with_target_db` as required.
    ///
    /// Returns `Ok(None)` when `agent` is unknown or has no role-default,
    /// toolkit, or host-provided tools. Startup already validated
    /// composition, so the `Err` path here is reserved for env-shape
    /// mismatches the runtime cannot foresee.
    pub fn dispatcher_for(
        &self,
        agent: &str,
        env: ToolEnvironment,
    ) -> Result<Option<ComposedDispatcher>, RuntimeError> {
        self.dispatcher_for_with_policy(agent, env, self.approval_policy_for(agent))
    }

    pub(crate) fn dispatcher_for_with_policy(
        &self,
        agent: &str,
        env: ToolEnvironment,
        approval_policy: Arc<agent_fw_agent::approval::ApprovalPolicy>,
    ) -> Result<Option<ComposedDispatcher>, RuntimeError> {
        let Some(agent_spec) = self.spec.agents.iter().find(|a| a.name == agent) else {
            return Ok(None);
        };
        let env = env.with_ext::<ReferenceRegistry>(self.references.clone());
        let composed = compose_dispatcher_for_agent(
            agent_spec,
            &self.spec.toolkits,
            &self.references,
            &self.plans,
            &self.host_tools,
            env,
        )?;
        let Some(composed) = composed else {
            return Ok(None);
        };
        // Apply the canonical framework stack: `guarded` (cancellation
        // short-circuit) → `approval` (B2 pre-dispatch gate) → `traced`
        // (tool_call / tool_result event emission). Order matches the
        // recipe documented on `ComposedDispatcher::approval`.
        let gated = composed
            .guarded()
            .approval(approval_policy, self.approval_store.clone())
            .traced();
        Ok(Some(gated))
    }

    /// Return the first registered agent name for a harness role.
    ///
    /// Coordinator, planner, and executor roles are expected to be unique in
    /// normal runtime specs. Specialist roles may have multiple agents; this
    /// helper intentionally returns only the first match and should not be used
    /// for specialist discovery.
    ///
    /// Eval execution uses this to select planner/executor entrypoints by
    /// role without going through `run_specialist`, whose public contract is
    /// intentionally restricted to [`AgentRole::Specialist`].
    pub fn agent_name_by_role(&self, role: AgentRole) -> Option<&str> {
        self.spec
            .agents
            .iter()
            .find(|agent| agent.role == role)
            .map(|agent| agent.name.as_str())
    }

    /// Return the registered role for an agent name.
    pub fn agent_role(&self, agent_name: &str) -> Option<AgentRole> {
        self.spec
            .agents
            .iter()
            .find(|agent| agent.name == agent_name)
            .map(|agent| agent.role)
    }

    /// Run a user query through the coordinator and stream framework
    /// [`StreamPart`]s back to the caller (runtime query assembly C4).
    pub fn query(&self, request: QueryRequest) -> RuntimeEventStream {
        self.query_cancellable(request).into_stream()
    }

    /// Run a user query and retain a request-scoped cancellation handle.
    pub fn query_cancellable(&self, request: QueryRequest) -> CancellableRuntimeEventStream {
        self.query_cancellable_impl(request)
    }

    /// Run an eval against this runtime and return the eval runner artifact.
    ///
    /// This is the Rust entrypoint the Python facade will bridge in Python eval bridge.
    /// Streaming progress events are layered on top of the same runner path;
    /// this method keeps the artifact-producing path explicit and testable.
    pub async fn run_eval(
        self: &Arc<Self>,
        request: EvalRequest,
    ) -> Result<EvalArtifact, EvalRunnerError> {
        EvalRunner::new(self.clone()).run(request).await
    }

    /// Stream eval progress events for this runtime.
    ///
    /// Dropping the returned stream closes the event channel. The runner checks
    /// that closure between emitted events, so no additional samples are
    /// scheduled after cancellation is observed.
    pub fn stream_eval(self: &Arc<Self>, request: EvalRequest) -> EvalEventStream {
        EvalRunner::new(self.clone()).stream(request)
    }

    /// Resume an approval-gated operation by resolving the pending entry in
    /// the shared [`PendingApprovalStore`](agent_fw_algebra::approval::PendingApprovalStore).
    ///
    /// Wakes whichever gate is awaiting `decision.approval_id` — tool gate
    /// ([`agent_fw_agent::ApprovalLayer`]) or plan gate
    /// ([`agent_fw_plan::executor::GatedPlanExecutor`]) — and lets execution
    /// proceed. Cross-tenant defence-in-depth: the request body is looked up
    /// first and its tenant checked against the runtime tenant.
    pub async fn respond_to_approval(
        &self,
        decision: ApprovalDecision,
    ) -> Result<(), RuntimeError> {
        let core = runtime::approval::into_core_decision(&decision);

        // Defence-in-depth: when the body is still in the store, confirm
        // the pending request belongs to this tenant. After resolution
        // the body may be dropped (in-memory store drops on resolve);
        // pass straight to `resolve()` so the gate sees
        // `AlreadyResolved`/`NotFound` faithfully from the store.
        if let Some(body) = self
            .approval_store
            .get(&core.id)
            .await
            .map_err(runtime::approval::map_approval_error)?
        {
            if body.resource_id != self.tenant {
                return Err(RuntimeError::ApprovalNotFound(core.id.as_str().to_string()));
            }
        }

        self.approval_store
            .resolve(core)
            .await
            .map_err(runtime::approval::map_approval_error)
    }

    /// Look up a declared plan spec by name.
    pub fn plan_spec(&self, name: &str) -> Option<&PlanSpec> {
        self.spec.plans.iter().find(|plan| plan.name == name)
    }

    /// Look up a declared reference spec by name.
    pub fn reference_spec(&self, name: &str) -> Option<&ReferenceSpec> {
        self.spec
            .references
            .iter()
            .find(|reference| reference.name == name)
    }

    /// Create a typed artifact reference in the runtime-owned registry.
    ///
    /// Harness language facades compute the JSON-compatible glimpse before
    /// calling this method. The registry validates `value` against the declared
    /// [`ReferenceSpec`] schema, stores both `value` and `glimpse`, and returns
    /// the stable [`ArtifactRef`] handle.
    pub async fn create_reference(
        &self,
        kind: &str,
        value: JsonValue,
        glimpse: JsonValue,
    ) -> Result<ArtifactRef, RuntimeError> {
        Ok(self
            .references
            .create(kind, value, glimpse, &self.tenant)
            .await?)
    }

    /// Validate planner output against a registered `PlanSpec` schema,
    /// persist as `Draft`, and return the freshly-created plan
    /// (plan registry). The runtime caller picks the `plan_id` (typically a
    /// fresh UUID minted by the orchestrator).
    pub async fn propose_plan(
        &self,
        spec_name: &str,
        plan_id: PlanId,
        body: JsonValue,
    ) -> Result<Plan<HarnessAction>, RuntimeError> {
        Ok(self
            .plans
            .propose(spec_name, plan_id, body, &self.tenant)
            .await?)
    }

    /// Retrieve a persisted plan instance (plan registry). Returns
    /// `Ok(None)` for unknown ids and for plans owned by another
    /// tenant (the latter is the KV layer's L9 plus defence-in-depth
    /// in [`PlanRegistry::load`]).
    pub async fn plan(&self, id: &PlanId) -> Result<Option<Plan<HarnessAction>>, RuntimeError> {
        Ok(self.plans.load(id, &self.tenant).await?)
    }

    /// Display alias configured on a `PlanSpec` for a given lifecycle
    /// status. Useful for the host UI when surfacing `Draft` as
    /// `pending_approval` (and similar).
    pub fn plan_display_alias(&self, spec_name: &str, status: PlanStatus) -> Option<&str> {
        self.plans.display_alias(spec_name, status)
    }

    /// Retrieve the materialised value behind a typed artifact reference
    /// (reference registry). Returns `Err(ReferenceError::NotFound)` for unknown
    /// ids, expired TTLs, or cross-tenant resolves.
    pub async fn reference(&self, reference: &ArtifactRef) -> Result<JsonValue, RuntimeError> {
        let body = self.references.resolve(reference, &self.tenant).await?;
        Ok(body.value)
    }

    /// Retrieve only the cached glimpse for a typed artifact reference.
    /// Same underlying KV fetch as [`Runtime::reference`]; returns the
    /// host-precomputed glimpse the spec emitted on `create`.
    pub async fn reference_glimpse(
        &self,
        reference: &ArtifactRef,
    ) -> Result<JsonValue, RuntimeError> {
        Ok(self.references.glimpse(reference, &self.tenant).await?)
    }

    /// Directly invoke a specialist agent by name, skipping coordinator
    /// routing (runtime query assembly C4 — Abstractions §15.3).
    pub fn run_specialist(&self, request: SpecialistRequest) -> RuntimeEventStream {
        self.run_specialist_cancellable(request).into_stream()
    }

    /// Directly invoke a specialist agent and retain a request cancellation handle.
    pub fn run_specialist_cancellable(
        &self,
        request: SpecialistRequest,
    ) -> CancellableRuntimeEventStream {
        self.run_specialist_cancellable_impl(request)
    }
}

/// Query request supplied by a host application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    /// Natural language user prompt.
    pub prompt: String,
    /// Resource ID derived from auth context.
    pub resource_id: TenantId,
    /// Conversation thread ID.
    pub thread_id: ThreadId,
    /// Optional session resume token.
    #[serde(default)]
    pub resume: Option<String>,
}

/// Direct specialist request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistRequest {
    /// Specialist agent name.
    pub specialist: String,
    /// Natural language task prompt.
    pub prompt: String,
    /// Resource ID derived from auth context.
    pub resource_id: TenantId,
    /// Optional conversation thread ID.
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
}

/// Approval decision returned by host UI or backend code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    /// Approval request identifier.
    pub approval_id: String,
    /// Host decision.
    pub outcome: ApprovalOutcome,
    /// Optional rejection or revision feedback surfaced in the
    /// `approval_decision` event and (for tools) in the tool result.
    #[serde(default)]
    pub feedback: Option<String>,
    /// Partial body supplied with [`ApprovalOutcome::Revise`]. The runtime
    /// passes this verbatim into the planner's revise loop; absent values
    /// surface as `null` to the planner schema.
    #[serde(default)]
    pub partial: Option<JsonValue>,
}

/// Approval outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalOutcome {
    /// Allow the pending operation to continue.
    Approve,
    /// Reject the pending operation.
    Reject,
    /// Request a revised plan or operation.
    Revise,
}

/// Runtime-level errors.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The inspectable spec tenant resource does not match the effectful runtime tenant resource.
    #[error("tenant mismatch: spec resource `{spec_resource_id}` does not match runtime resource `{runtime_resource_id}`")]
    TenantMismatch {
        /// Resource id serialized in the runtime spec.
        spec_resource_id: TenantId,
        /// Resource id supplied by the runtime dependency context.
        runtime_resource_id: TenantId,
    },
    /// The pure runtime spec violates Flow AI harness agent invariants.
    #[error("agent spec invalid: {0}")]
    AgentSpecInvalid(String),
    /// The underlying orchestrator could not be built.
    #[error("orchestrator build failed: {0}")]
    Orchestrator(#[from] OrchestratorBuildError),
    /// Error from the reference registry (reference registry) — malformed schemas
    /// at construction, schema-validation failures on `create`,
    /// `NotFound` on `resolve`, etc.
    #[error("reference registry error: {0}")]
    Reference(#[from] ReferenceError),
    /// Error from the plan registry (plan registry) — malformed plan-spec
    /// schemas, planner-output validation failures, missing/empty
    /// actions, or state-machine transition errors.
    #[error("plan registry error: {0}")]
    Plan(#[from] PlanProtocolError),
    /// Error from default-toolkit composition (default toolkit composition) — unknown
    /// toolkit id, unknown tool name in a narrowing config, malformed
    /// config JSON, or dispatcher collision.
    #[error("toolkit composition error: {0}")]
    Toolkit(#[from] ToolkitError),
    /// An agent selected the `catalog` toolkit, but no catalog search backend
    /// was configured for the runtime.
    #[error("agent '{agent}' uses the catalog toolkit, but catalog search is not configured; pass data_environment.catalog_search.index_path or inject a CatalogSearchBackend")]
    CatalogSearchBackendMissing {
        /// Agent whose toolkit selection requires catalog search.
        agent: String,
    },
    /// Error compiling runtime approval policy data into framework
    /// predicates and rules.
    #[error("approval policy error: {0}")]
    ApprovalPolicy(#[from] runtime::approval::ApprovalPolicyError),
    /// The requested runtime feature is reserved for a later C-layer issue.
    #[error("runtime feature is not implemented yet: {0}")]
    Unimplemented(&'static str),
    /// `respond_to_approval` referenced an approval id the store doesn't know.
    #[error("approval not found: {0}")]
    ApprovalNotFound(String),
    /// `respond_to_approval` referenced an id already resolved or expired.
    #[error("approval already resolved: {0}")]
    ApprovalAlreadyResolved(String),
    /// Approval-store backend failure or transport error.
    #[error("approval store error: {0}")]
    Approval(String),
    /// The agent named in a request is not registered or has the wrong role.
    #[error("agent '{agent}' not found or not a {expected_role}")]
    AgentNotFound {
        /// Requested agent name.
        agent: String,
        /// Role the runtime expected for this entry-point.
        expected_role: String,
    },
    /// `query` was called against a spec with no coordinator agent.
    #[error("no coordinator agent registered in the spec")]
    NoCoordinator,
    /// An agent's `model` routed to a provider key absent from
    /// [`RuntimeSpec::providers`](crate::RuntimeSpec).
    #[error(
        "agent '{agent}' references provider '{provider}' for model '{model}', \
         but no such provider is declared in RuntimeSpec.providers"
    )]
    ProviderUnregistered {
        /// Agent whose model triggered the routing failure.
        agent: String,
        /// Provider key the auto-router or explicit override picked.
        provider: String,
        /// The model id that triggered the routing.
        model: String,
    },
    /// An agent's resolved provider is declared in [`RuntimeSpec::providers`]
    /// but has no matching chat interpreter in
    /// [`RuntimeDeps::interpreter_providers`](crate::RuntimeDeps::interpreter_providers).
    #[error(
        "agent '{agent}' references provider '{provider}' for model '{model}', \
         but no ChatInterpreter is registered for that provider"
    )]
    ProviderInterpreterMissing {
        /// Agent whose model triggered the routing failure.
        agent: String,
        /// Provider key selected for the agent.
        provider: String,
        /// The model id that triggered the routing.
        model: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::stream_part::FinishReason;
    use agent_fw_core::tenant::TenantContext;
    use agent_fw_core::usage::TokenUsage;
    use agent_fw_core::{StreamPart, TenantId};
    use futures::stream;
    use futures::StreamExt;
    use std::collections::HashSet;

    struct NoopInterpreter;

    impl ChatInterpreter for NoopInterpreter {
        fn interpret(
            &self,
            _program: agent_fw_agent::ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }
    }

    struct MaxTurnsProbeInterpreter {
        max_turns: Option<usize>,
    }

    impl ChatInterpreter for MaxTurnsProbeInterpreter {
        fn interpret(
            &self,
            _program: agent_fw_agent::ChatProgram,
            _cancel: CancellationToken,
        ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
            let text = format!("max_turns={:?}", self.max_turns);
            Box::pin(stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::text(text),
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }

        fn with_max_turns(self: Arc<Self>, max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
            Some(Arc::new(Self {
                max_turns: Some(max_turns),
            }))
        }
    }

    fn runtime_spec() -> RuntimeSpec {
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
                    "You coordinate analytical work.",
                ),
                AgentSpec::new(
                    "planner",
                    AgentRole::Planner,
                    ModelSpec::new("claude-sonnet-4-6"),
                    "You produce typed plans.",
                ),
                AgentSpec::new(
                    "executor",
                    AgentRole::Executor,
                    ModelSpec::new("claude-haiku-4-5"),
                    "You execute approved plans.",
                ),
                AgentSpec::new(
                    "product_insights",
                    AgentRole::Specialist,
                    ModelSpec::new("claude-sonnet-4-6"),
                    "You answer focused product questions.",
                ),
            ],
            references: vec![ReferenceSpec {
                name: "ProductSet".to_string(),
                schema: serde_json::json!({"type": "object"}),
                ttl_ms: Some(3_600_000),
            }],
            plans: vec![PlanSpec {
                name: "ScenarioPlan".to_string(),
                schema: serde_json::json!({"type": "object"}),
                display_aliases: vec![PlanDisplayAlias {
                    status: PlanStatus::Draft,
                    alias: "pending_approval".to_string(),
                }],
            }],
            toolkits: vec![ToolkitSpec {
                id: "catalog".to_string(),
                config: JsonValue::Null,
            }],
            approval_policies: ApprovalPolicies::default(),
            approval_overrides: Default::default(),
            storage_factories: StorageFactories::default(),
            providers,
        };
        spec.agents[0].routes = vec![
            "planner".to_string(),
            "executor".to_string(),
            "product_insights".to_string(),
        ];
        spec
    }

    fn deps() -> RuntimeDeps {
        RuntimeDeps::new(
            Arc::new(NoopInterpreter),
            Arc::new(agent_fw_algebra::testing::NullEventSink),
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            Arc::new(agent_fw_interpreter::DashMapKVStore::new()),
        )
    }

    #[test]
    fn runtime_spec_is_inspectable_without_effects() {
        let spec = runtime_spec();

        assert_eq!(spec.agent_count(), 4);
        assert_eq!(spec.references[0].name, "ProductSet");
        assert_eq!(spec.plans[0].display_aliases[0].alias, "pending_approval");
        assert!(spec.providers.contains_key("anthropic"));
        assert!(spec.agent("coordinator").is_some());
    }

    #[test]
    fn agent_spec_deserializes_optional_max_turns() {
        let agent: AgentSpec = serde_json::from_value(serde_json::json!({
            "name": "planner",
            "role": "planner",
            "model": {"id": "claude-sonnet-4-6"},
            "systemPrompt": "You plan.",
            "maxTurns": 24
        }))
        .expect("agent spec should deserialize");

        assert_eq!(agent.max_turns, Some(24));
    }

    #[test]
    fn agent_spec_rejects_zero_max_turns() {
        let error = serde_json::from_value::<AgentSpec>(serde_json::json!({
            "name": "planner",
            "role": "planner",
            "model": {"id": "claude-sonnet-4-6"},
            "systemPrompt": "You plan.",
            "maxTurns": 0
        }))
        .expect_err("maxTurns=0 should be rejected");

        assert!(
            error.to_string().contains("maxTurns must be at least 1"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn runtime_applies_agent_max_turns_to_selected_interpreter() {
        let mut spec = runtime_spec();
        spec.agents[0].max_turns = Some(24);
        let deps = RuntimeDeps::new(
            Arc::new(MaxTurnsProbeInterpreter { max_turns: None }),
            Arc::new(agent_fw_algebra::testing::NullEventSink),
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            Arc::new(agent_fw_interpreter::DashMapKVStore::new()),
        );
        let runtime = Runtime::new(spec, deps).expect("runtime should build");

        let parts: Vec<StreamPart> = runtime
            .query(QueryRequest {
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id: ThreadId::new_unchecked("thread-max-turns"),
                resume: None,
            })
            .collect()
            .await;

        assert!(
            parts.iter().any(
                |part| matches!(part, StreamPart::Text { text, .. } if text == "max_turns=Some(24)")
            ),
            "expected max-turn override in stream, got {parts:?}"
        );
    }

    #[test]
    fn agent_role_serializes_as_harness_wire_role() {
        assert_eq!(
            serde_json::to_string(&AgentRole::Coordinator).unwrap(),
            "\"coordinator\""
        );
        let parsed: AgentRole = serde_json::from_str("\"specialist\"").unwrap();
        assert_eq!(parsed, AgentRole::Specialist);
        assert_eq!(AgentRole::Executor.as_str(), "executor");
        assert_eq!(AgentRole::Planner.to_agent_label().as_str(), "planner");
    }

    #[test]
    fn agent_roles_have_stateful_defaults() {
        assert!(AgentRole::Coordinator.default_stateful());
        assert!(AgentRole::Planner.default_stateful());
        assert!(!AgentRole::Executor.default_stateful());
        assert!(!AgentRole::Specialist.default_stateful());
        assert!(
            AgentSpec::new(
                "planner",
                AgentRole::Planner,
                ModelSpec::new("claude-sonnet-4-6"),
                "plan"
            )
            .stateful
        );
        assert!(
            !AgentSpec::new(
                "executor",
                AgentRole::Executor,
                ModelSpec::new("claude-sonnet-4-6"),
                "execute"
            )
            .stateful
        );
    }

    #[test]
    fn runtime_resolves_agent_names_by_role() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");

        assert_eq!(
            runtime.agent_name_by_role(AgentRole::Coordinator),
            Some("coordinator")
        );
        assert_eq!(
            runtime.agent_name_by_role(AgentRole::Planner),
            Some("planner")
        );
        assert_eq!(
            runtime.agent_name_by_role(AgentRole::Executor),
            Some("executor")
        );
        assert_eq!(
            runtime.agent_name_by_role(AgentRole::Specialist),
            Some("product_insights")
        );
    }

    #[test]
    fn runtime_role_lookup_returns_none_when_role_is_absent() {
        let mut spec = runtime_spec();
        spec.agents
            .retain(|agent| agent.role != AgentRole::Executor);
        spec.agents[0].routes.retain(|route| route != "executor");
        let runtime = Runtime::new(spec, deps()).expect("runtime should build");

        assert_eq!(runtime.agent_name_by_role(AgentRole::Executor), None);
    }

    #[test]
    fn agent_spec_deserializes_missing_stateful_with_role_default() {
        let coordinator: AgentSpec = serde_json::from_value(serde_json::json!({
            "name": "coordinator",
            "role": "coordinator",
            "model": {"id": "claude-sonnet-4-6"},
            "systemPrompt": "coordinate",
            "routes": ["planner"]
        }))
        .expect("coordinator parses");
        let specialist: AgentSpec = serde_json::from_value(serde_json::json!({
            "name": "insights",
            "role": "specialist",
            "model": {"id": "claude-sonnet-4-6"},
            "systemPrompt": "answer"
        }))
        .expect("specialist parses");

        assert!(coordinator.stateful);
        assert!(!specialist.stateful);
    }

    #[test]
    fn minimal_runtime_spec_preserves_default_approval_floor() {
        let spec = RuntimeSpec::minimal("acme", "v1");

        assert_eq!(spec.tenant.resource_id.as_str(), "acme");
        assert_eq!(spec.approval_policies.plans, ApprovalRule::Always);
        assert_eq!(spec.approval_policies.tools, ApprovalRule::Never);
        assert!(spec.providers.is_empty());
    }

    #[test]
    fn runtime_rejects_spec_tenant_mismatch() {
        let mut spec = runtime_spec();
        spec.tenant = TenantIdentity::new("different-tenant", "v1");

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(matches!(err, RuntimeError::TenantMismatch { .. }));
        assert!(err.to_string().contains("tenant mismatch"));
    }

    #[test]
    fn into_orchestrator_round_trip_registers_agents() {
        let orchestrator = runtime_spec()
            .into_orchestrator(deps())
            .expect("orchestrator should build");

        assert!(orchestrator.has_agent("coordinator"));
        assert!(orchestrator.has_agent("planner"));
        assert!(orchestrator.has_agent("executor"));
        assert!(orchestrator.has_agent("product_insights"));

        let mut names = orchestrator.agent_names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "coordinator".to_string(),
                "executor".to_string(),
                "planner".to_string(),
                "product_insights".to_string(),
            ],
        );
    }

    #[test]
    fn runtime_new_rejects_duplicate_agent_names() {
        let mut spec = runtime_spec();
        spec.agents[1].name = "coordinator".to_string();

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(matches!(
            err,
            RuntimeError::AgentSpecInvalid(ref message)
                if message.contains("duplicate agent name 'coordinator'")
        ));
    }

    #[test]
    fn runtime_new_rejects_duplicate_coordinators() {
        let mut spec = runtime_spec();
        spec.agents[1].role = AgentRole::Coordinator;
        spec.agents[1].routes = vec!["executor".to_string()];

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(matches!(
            err,
            RuntimeError::AgentSpecInvalid(ref message)
                if message.contains("multiple coordinator agents")
        ));
    }

    #[test]
    fn runtime_new_rejects_coordinator_without_routes() {
        let mut spec = runtime_spec();
        spec.agents[0].routes.clear();

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(matches!(
            err,
            RuntimeError::AgentSpecInvalid(ref message)
                if message.contains("coordinator agent 'coordinator' requires at least one route")
        ));
    }

    #[test]
    fn runtime_new_rejects_invalid_route_graph() {
        for (routes, expected) in [
            (
                vec!["missing".to_string()],
                "unknown route target 'missing'",
            ),
            (
                vec!["planner".to_string(), "planner".to_string()],
                "duplicate route 'planner'",
            ),
            (vec!["coordinator".to_string()], "cannot route to itself"),
        ] {
            let mut spec = runtime_spec();
            spec.agents[0].routes = routes;

            let err = Runtime::new(spec, deps()).err().expect("expected error");
            assert!(
                matches!(err, RuntimeError::AgentSpecInvalid(ref message) if message.contains(expected)),
                "expected {expected:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn runtime_new_rejects_unknown_approval_override_agent() {
        let mut spec = runtime_spec();
        spec.approval_overrides.agents.insert(
            "missing".to_string(),
            ApprovalPolicyPatch {
                plans: Some(ApprovalRule::Never),
                tools: None,
            },
        );

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(
            matches!(err, RuntimeError::AgentSpecInvalid(ref message) if message.contains("approvalOverrides.agents")),
            "expected approval override validation error, got {err:?}",
        );
    }

    #[test]
    fn runtime_uses_agent_level_approval_policy_override() {
        let mut spec = runtime_spec();
        spec.approval_overrides.agents.insert(
            "executor".to_string(),
            ApprovalPolicyPatch {
                plans: Some(ApprovalRule::Never),
                tools: Some(ApprovalRule::Always),
            },
        );

        let runtime = Runtime::new(spec, deps()).expect("runtime should build");
        let coordinator_policy = runtime.approval_policy_for("coordinator");
        let executor_policy = runtime.approval_policy_for("executor");

        assert!(matches!(
            coordinator_policy.resolve_plan("plan"),
            agent_fw_agent::approval::ApprovalRule::Always
        ));
        assert!(matches!(
            coordinator_policy.resolve_tool("tool"),
            agent_fw_agent::approval::ApprovalRule::Never
        ));
        assert!(matches!(
            executor_policy.resolve_plan("plan"),
            agent_fw_agent::approval::ApprovalRule::Never
        ));
        assert!(matches!(
            executor_policy.resolve_tool("tool"),
            agent_fw_agent::approval::ApprovalRule::Always
        ));
    }

    #[test]
    fn runtime_handle_exposes_plan_and_reference_specs() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");

        assert_eq!(
            runtime.plan_spec("ScenarioPlan").unwrap().name,
            "ScenarioPlan"
        );
        assert_eq!(
            runtime.reference_spec("ProductSet").unwrap().name,
            "ProductSet"
        );
        assert!(runtime.plan_spec("MissingPlan").is_none());
        assert!(runtime.reference_spec("MissingReference").is_none());
    }

    #[test]
    fn artifact_ref_can_be_used_as_identity_key() {
        let artifact = ArtifactRef {
            kind: "ProductSet".to_string(),
            id: "ref-1".to_string(),
        };
        let mut seen = HashSet::new();

        seen.insert(artifact.clone());

        assert!(seen.contains(&artifact));
    }

    #[test]
    fn providers_serialize_as_direct_record_keyed_by_provider_kind() {
        let mut spec = runtime_spec();
        spec.providers.insert(
            "bedrock".to_string(),
            ProviderConfig::new(serde_json::json!({"region": "us-east-1"})),
        );

        let value = serde_json::to_value(&spec).unwrap();

        assert_eq!(
            value["providers"],
            serde_json::json!({
                "anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"},
                "bedrock": {"region": "us-east-1"},
            }),
        );
        assert!(value["providers"]["anthropic"].get("kind").is_none());
    }

    #[tokio::test]
    async fn respond_to_approval_unknown_id_returns_not_found() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");

        let err = runtime
            .respond_to_approval(ApprovalDecision {
                approval_id: "does-not-exist".to_string(),
                outcome: ApprovalOutcome::Approve,
                feedback: None,
                partial: None,
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            RuntimeError::ApprovalNotFound(ref s) if s == "does-not-exist"
        ));
    }

    #[tokio::test]
    async fn plan_lookup_unknown_id_returns_none() {
        // plan registry: plan() now delegates to the registry. An unknown id
        // returns Ok(None) rather than an error (mirrors NotFound
        // semantics on the reference path).
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let plan_id = PlanId::new_unchecked("did-not-exist");
        let loaded = runtime.plan(&plan_id).await.expect("ok shape");
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn plan_propose_and_load_round_trips() {
        // plan registry shape end-to-end at the Runtime level: propose a
        // plan via the registry using the canonical flat action shape
        // (§13.2 of the Harness abstractions doc), then load it back
        // via the runtime delegation and confirm the body matches.
        // Asserts P1 (flat action fields land in `payload`) and P2
        // (non-`actions` top-level body fields land in `context`) at
        // the public Runtime boundary.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let plan_id = PlanId::new_unchecked("scenario-1");
        let body = serde_json::json!({
            "actions": [
                {"kind": "price_change", "product_id": "p-1", "new_price": 9.99}
            ],
            "rationale": "test"
        });
        let proposed = runtime
            .propose_plan("ScenarioPlan", plan_id.clone(), body)
            .await
            .expect("propose");
        assert_eq!(proposed.status, agent_fw_plan::PlanStatus::Draft);
        assert_eq!(proposed.actions.first().kind, "price_change");
        assert_eq!(
            proposed.actions.first().payload,
            serde_json::json!({"product_id": "p-1", "new_price": 9.99})
        );
        assert_eq!(
            proposed.context.get("rationale"),
            Some(&serde_json::json!("test"))
        );

        let loaded = runtime
            .plan(&plan_id)
            .await
            .expect("ok shape")
            .expect("load Some");
        assert_eq!(loaded.id, plan_id);
        assert_eq!(loaded.status, agent_fw_plan::PlanStatus::Draft);
        assert_eq!(loaded.actions.first().kind, "price_change");
        assert_eq!(
            loaded.actions.first().payload,
            serde_json::json!({"product_id": "p-1", "new_price": 9.99})
        );
        assert_eq!(
            loaded.context.get("rationale"),
            Some(&serde_json::json!("test"))
        );
    }

    #[tokio::test]
    async fn plan_display_alias_surfaces_configured_alias() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        // runtime_spec()'s ScenarioPlan declares a Draft -> pending_approval alias.
        let alias = runtime.plan_display_alias("ScenarioPlan", agent_fw_plan::PlanStatus::Draft);
        assert_eq!(alias, Some("pending_approval"));
        let none_alias =
            runtime.plan_display_alias("ScenarioPlan", agent_fw_plan::PlanStatus::Approved);
        assert_eq!(none_alias, None);
    }

    #[tokio::test]
    async fn reference_lookup_unknown_id_returns_not_found() {
        // reference registry: reference() now delegates to the registry. An unknown
        // id under a known kind surfaces as a `ReferenceError::NotFound`
        // wrapped in `RuntimeError::Reference`.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let artifact = ArtifactRef {
            kind: "ProductSet".to_string(),
            id: "did-not-exist".to_string(),
        };
        let err = runtime.reference(&artifact).await.unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::Reference(ReferenceError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn reference_create_and_lookup_round_trip() {
        // reference registry acceptance shape end-to-end at the Runtime level:
        // create a reference through the registry, then resolve through
        // the Runtime's public delegation and confirm we get the same
        // value + glimpse back.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let value = serde_json::json!({"product_ids": ["a", "b"]});
        let glimpse = serde_json::json!({"n_products": 2});
        let artifact = runtime
            .references()
            .create(
                "ProductSet",
                value.clone(),
                glimpse.clone(),
                &TenantId::new_unchecked("tenant-1"),
            )
            .await
            .expect("registry create");

        let body = runtime.reference(&artifact).await.expect("runtime lookup");
        assert_eq!(body, value);

        let cached = runtime
            .reference_glimpse(&artifact)
            .await
            .expect("glimpse lookup");
        assert_eq!(cached, glimpse);
    }

    #[tokio::test]
    async fn query_streams_sub_agent_call_for_coordinator() {
        // runtime query assembly C4: `query` drives the orchestrator's
        // `SubAgentInvoker::invoke` for the coordinator, which emits
        // `sub_agent_call` then `sub_agent_result` even on a tool-less
        // run.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let thread_id = ThreadId::new_unchecked("thread-1");
        use futures::StreamExt;

        let parts: Vec<StreamPart> = runtime
            .query(QueryRequest {
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id,
                resume: None,
            })
            .collect()
            .await;

        let names: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolAgent(data) => Some(data.agent_name.clone()),
                _ => None,
            })
            .collect();
        assert!(
            names.iter().any(|n| n == "coordinator"),
            "expected a sub_agent_call/result for the coordinator, got {names:?}",
        );
    }

    #[tokio::test]
    async fn query_cancellable_exposes_request_cancel_handle() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");

        let stream = runtime.query_cancellable(QueryRequest {
            prompt: "hello".to_string(),
            resource_id: TenantId::new_unchecked("tenant-1"),
            thread_id: ThreadId::new_unchecked("thread-1"),
            resume: None,
        });

        assert!(!stream.is_cancelled());
        stream.cancel();
        assert!(stream.is_cancelled());
    }

    #[tokio::test]
    async fn query_with_missing_coordinator_emits_error() {
        let mut spec = runtime_spec();
        spec.agents.retain(|a| a.role != AgentRole::Coordinator);
        let runtime = Runtime::new(spec, deps()).expect("runtime should build");
        let thread_id = ThreadId::new_unchecked("thread-1");

        let part = runtime
            .query(QueryRequest {
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id,
                resume: None,
            })
            .next()
            .await
            .expect("first stream item");
        assert!(matches!(
            part,
            StreamPart::Error { ref error } if error.message.contains("no coordinator")
        ));
    }

    #[tokio::test]
    async fn run_role_stream_invokes_planner_role() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");

        let parts: Vec<StreamPart> = runtime
            .run_role_stream(
                AgentRole::Planner,
                "plan this".to_string(),
                ThreadId::new_unchecked("thread-plan"),
            )
            .collect()
            .await;

        let names: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolAgent(data) => Some(data.agent_name.clone()),
                _ => None,
            })
            .collect();
        assert!(
            names.iter().any(|n| n == "planner"),
            "expected planner role events, got {names:?}",
        );
    }

    #[tokio::test]
    async fn run_role_stream_missing_role_emits_error() {
        let mut spec = runtime_spec();
        spec.agents
            .retain(|agent| agent.role != AgentRole::Executor);
        spec.agents[0].routes.retain(|route| route != "executor");
        let runtime = Runtime::new(spec, deps()).expect("runtime should build");

        let part = runtime
            .run_role_stream(
                AgentRole::Executor,
                "execute this".to_string(),
                ThreadId::new_unchecked("thread-execute"),
            )
            .next()
            .await
            .expect("first stream item");
        assert!(matches!(
            part,
            StreamPart::Error { ref error } if error.message.contains("no executor agent")
        ));
    }

    fn eval_request(tenant: &str) -> EvalRequest {
        EvalRequest {
            tenant_id: TenantId::new_unchecked(tenant),
            workspace_id: agent_fw_core::WorkspaceId::new("workspace-main").expect("workspace id"),
            config: agent_fw_eval::EvalConfig {
                mode: agent_fw_eval::EvalMode::Sequential,
                test_case_source: agent_fw_eval::TestCaseSource::Set("inline".to_string()),
                samples_per_case: 1,
                pass_threshold: 0.5,
                concurrency: 1,
                k_values: vec![1],
                timeout_per_sample_secs: Some(5),
                ..Default::default()
            },
            test_cases: vec![agent_fw_eval::EvalTestCase {
                id: agent_fw_core::TestCaseId::new_unchecked("tc-1"),
                tags: vec![],
                input: "hello".to_string(),
                expected_trajectory: vec![],
                trajectory_mode: agent_fw_eval::TrajectoryMode::Unordered,
                ground_truth: None,
                final_response: None,
                source_thread_id: None,
            }],
            scorer_preset: None,
            score_weights: None,
        }
    }

    #[tokio::test]
    async fn runtime_run_eval_returns_artifact() {
        let runtime = Arc::new(Runtime::new(runtime_spec(), deps()).expect("runtime should build"));
        let artifact = runtime
            .run_eval(eval_request("tenant-1"))
            .await
            .expect("eval should run");

        assert_eq!(artifact.tenant_id.as_str(), "tenant-1");
        assert_eq!(artifact.workspace_id.as_str(), "workspace-main");
        assert_eq!(artifact.test_cases.len(), 1);
        assert_eq!(artifact.test_cases[0].samples.len(), 1);
        assert_eq!(
            artifact.test_cases[0].samples[0].resolved_actions,
            Vec::<eval::ResolvedAction>::new()
        );
        assert_eq!(
            artifact.test_cases[0].samples[0].model_invocations[0].agent,
            "unknown"
        );
    }

    #[tokio::test]
    async fn runtime_stream_eval_emits_contract_events() {
        let runtime = Arc::new(Runtime::new(runtime_spec(), deps()).expect("runtime should build"));
        let events: Vec<_> = runtime
            .stream_eval(eval_request("tenant-1"))
            .collect()
            .await;

        assert_eq!(events.len(), 5);
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert!(events.iter().all(|event| event.run_id == events[0].run_id));
        assert!(matches!(
            events[0].event,
            eval::HarnessEvalEvent::EvalStarted { .. }
        ));
        assert!(matches!(
            events[4].event,
            eval::HarnessEvalEvent::EvalCompleted { .. }
        ));
    }

    #[tokio::test]
    async fn query_with_wrong_resource_id_emits_error() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let part = runtime
            .query(QueryRequest {
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("different-tenant"),
                thread_id: ThreadId::new_unchecked("thread-1"),
                resume: None,
            })
            .next()
            .await
            .expect("first stream item");
        assert!(matches!(
            part,
            StreamPart::Error { ref error } if error.message.contains("resource_id mismatch")
        ));
    }

    #[tokio::test]
    async fn run_specialist_rejects_non_specialist_agent() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let part = runtime
            .run_specialist(SpecialistRequest {
                specialist: "planner".to_string(),
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id: None,
            })
            .next()
            .await
            .expect("first stream item");
        assert!(matches!(
            part,
            StreamPart::Error { ref error } if error.message.contains("not a specialist")
        ));
    }

    #[tokio::test]
    async fn run_specialist_emits_sub_agent_call_for_specialist() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        use futures::StreamExt;
        let parts: Vec<StreamPart> = runtime
            .run_specialist(SpecialistRequest {
                specialist: "product_insights".to_string(),
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id: None,
            })
            .collect()
            .await;
        let names: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolAgent(data) => Some(data.agent_name.clone()),
                _ => None,
            })
            .collect();
        assert!(
            names.iter().any(|n| n == "product_insights"),
            "expected sub_agent_call/result for product_insights, got {names:?}",
        );
        // No coordinator framing for direct specialist calls.
        assert!(
            !names.iter().any(|n| n == "coordinator"),
            "specialist direct call must not invoke the coordinator: {names:?}",
        );
    }

    #[tokio::test]
    async fn run_specialist_with_wrong_resource_id_emits_error() {
        // runtime query assembly review fix: `resource_id` is derived from auth context
        // and must not be accepted from the request body. Mirror the
        // `query` defence.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let part = runtime
            .run_specialist(SpecialistRequest {
                specialist: "product_insights".to_string(),
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("different-tenant"),
                thread_id: None,
            })
            .next()
            .await
            .expect("first stream item");
        assert!(matches!(
            part,
            StreamPart::Error { ref error } if error.message.contains("resource_id mismatch")
        ));
    }

    // ─── runtime query assembly review fix: eager provider/model validation ─────────────

    #[test]
    fn runtime_new_rejects_agent_whose_model_routes_to_undeclared_provider() {
        // Spec declares `bedrock` only, but the executor model is a
        // Claude family id that the id-family router maps to `anthropic`.
        // Construction must surface `ProviderUnregistered` with the agent
        // name + routed provider + model id.
        let mut providers = BTreeMap::new();
        providers.insert(
            "bedrock".to_string(),
            ProviderConfig::new(serde_json::json!({"region": "us-east-1"})),
        );
        let spec = RuntimeSpec {
            providers,
            ..runtime_spec_minimal_executor()
        };

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        match err {
            RuntimeError::ProviderUnregistered {
                agent,
                provider,
                model,
            } => {
                assert_eq!(agent, "executor");
                assert_eq!(provider, "anthropic");
                assert_eq!(model, "claude-sonnet-4-6");
            }
            other => panic!("expected ProviderUnregistered, got {other:?}"),
        }
    }

    #[test]
    fn runtime_new_rejects_explicit_provider_override_not_in_spec_providers() {
        // `model.provider` is an explicit override; the runtime should
        // accept it as the source of truth and reject when the spec
        // doesn't declare the named provider.
        let mut providers = BTreeMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new(serde_json::json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
        );
        let mut spec = runtime_spec_minimal_executor();
        spec.providers = providers;
        spec.agents[0].model.provider = Some("bedrock".to_string());

        let err = Runtime::new(spec, deps()).err().expect("expected error");
        assert!(matches!(
            err,
            RuntimeError::ProviderUnregistered { ref provider, .. } if provider == "bedrock"
        ));
    }

    fn runtime_spec_minimal_executor() -> RuntimeSpec {
        // Just enough to drive `Runtime::new` past references/plans/toolkit
        // validation and into the provider check.
        let mut providers = BTreeMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new(serde_json::json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
        );
        RuntimeSpec {
            tenant: TenantIdentity::new("tenant-1", "v1"),
            agents: vec![AgentSpec::new(
                "executor",
                AgentRole::Executor,
                ModelSpec::new("claude-sonnet-4-6"),
                "exec",
            )],
            references: vec![],
            plans: vec![],
            toolkits: vec![],
            approval_policies: ApprovalPolicies::default(),
            approval_overrides: Default::default(),
            storage_factories: StorageFactories::default(),
            providers,
        }
    }

    // ─── runtime query assembly review fix: per-agent tool env carries derived thread id ─

    #[test]
    fn tool_env_for_agent_thread_id_is_derived_sub_thread() {
        // runtime query assembly review fix: every per-agent `ToolEnvironment` must carry
        // the same `{parent}-{agent}` thread id the orchestrator's G2
        // derivation produces, so approval / plan events emitted from
        // tools attribute to the right sub-agent.
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let sink: Arc<dyn agent_fw_algebra::EventSink> =
            Arc::new(agent_fw_algebra::testing::NullEventSink);
        let parent = ThreadId::new_unchecked("thread-1");
        let executor = runtime
            .spec
            .agents
            .iter()
            .find(|a| a.name == "executor")
            .expect("executor in fixture")
            .clone();

        let invoker: Arc<dyn agent_fw_algebra::SubAgentInvoker> =
            Arc::new(agent_fw_algebra::testing::NullSubAgentInvoker);
        let env = runtime.tool_env_for_agent(&executor, sink, parent, invoker, false);
        let derived = env
            .tenant()
            .thread_id()
            .expect("env carries a thread id")
            .as_str()
            .to_string();
        assert_eq!(derived, "thread-1-executor");
    }

    #[test]
    fn tool_env_for_entry_agent_keeps_parent_thread_id() {
        let runtime = Runtime::new(runtime_spec(), deps()).expect("runtime should build");
        let sink: Arc<dyn agent_fw_algebra::EventSink> =
            Arc::new(agent_fw_algebra::testing::NullEventSink);
        let parent = ThreadId::new_unchecked("thread-1");
        let coordinator = runtime
            .spec
            .agents
            .iter()
            .find(|a| a.name == "coordinator")
            .expect("coordinator in fixture")
            .clone();

        let invoker: Arc<dyn agent_fw_algebra::SubAgentInvoker> =
            Arc::new(agent_fw_algebra::testing::NullSubAgentInvoker);
        let env = runtime.tool_env_for_agent(&coordinator, sink, parent, invoker, true);
        let thread_id = env
            .tenant()
            .thread_id()
            .expect("env carries a thread id")
            .as_str()
            .to_string();
        assert_eq!(thread_id, "thread-1");
    }
}
