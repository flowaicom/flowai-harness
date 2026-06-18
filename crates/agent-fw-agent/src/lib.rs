//! Agent orchestration: programs-as-values, multi-agent coordination.
//!
//! # Design Principles
//!
//! 1. **Programs as Values**: `ChatProgram` describes WHAT to do without doing it.
//!    The interpreter (`ChatInterpreter`) decides HOW and WHEN to execute.
//!
//! 2. **Make Illegal States Unrepresentable**: Domain types use smart constructors
//!    to ensure valid data at compile time where possible, runtime where necessary.
//!
//! 3. **Pure Core, Effectful Shell**: Pure transformation functions at the center,
//!    effects pushed to the edges (interpreter).
//!
//! # Algebraic Laws
//!
//! ## ChatProgram Laws
//!
//! - **L1 Purity**: `ChatProgram` is a value that describes a computation without
//!   performing it. Constructing a `ChatProgram` has no side effects. Two programs
//!   built from the same inputs are structurally equal.
//!
//! - **L2 Referential Transparency**: Given the same `Conversation`, `ModelId`, and
//!   `TenantContext`, `ChatProgram::new` always produces an equivalent program.
//!   The program can be freely substituted, stored, serialized, or passed around
//!   without altering observable behavior.
//!
//! - **L3 Composition Associativity**: Sequential composition of programs (running
//!   one program, feeding its output as context to the next) is associative:
//!   `(a >> b) >> c` is equivalent to `a >> (b >> c)` in terms of final result.
//!   The interpreter may optimize execution order but must preserve this semantic.
//!
//! ## ChatInterpreter Laws
//!
//! - **Termination**: Every stream from `interpret` must eventually emit a
//!   `StreamPart::Finish` event and complete.
//!
//! - **Ordering**: `StepStart` precedes all other events in a turn.
//!
//! - **Idempotence**: The same `ChatProgram`, given the same external conditions,
//!   produces a structurally equivalent stream of events.
//!
//! # Key Types
//!
//! - [`ChatProgram`] / [`ChatInterpreter`] — Programs-as-values algebra
//! - [`Conversation`], [`Prompt`], [`ChatMessage`] — Validated domain types
//! - [`ModelId`], [`ModelRouter`] — Multi-provider model resolution
//! - [`AgentOrchestrator`] — Multi-agent coordination with usage tracking
//! - [`ToolLayer`] / [`TracedLayer`] — Composable handler middleware
//! - [`PromptComposer`] / [`PromptSection`] — Pure prompt construction (structured data → Markdown)

/// Pre-dispatch approval primitives (pre-dispatch approval): `ApprovalRule`, `ApprovalRequest`,
/// `ApprovalDecision`, `ApprovalPolicy`. Consumed by the `ApprovalLayer` in
/// [`layer`] and by the `GatedPlanExecutor` in `agent-fw-plan`.
pub mod approval;

#[cfg(feature = "rig-hooks")]
mod basic_tools;
mod conversation;
#[cfg(feature = "rig-hooks")]
mod dispatcher_rig;
mod hook_bridge;
mod interpreter;
pub mod interpreter_contract;
pub mod layer;
pub mod metrics_accumulator;
mod model;
mod orchestrator;
mod prompt_composer;
mod request_scoped_tools;
#[cfg(feature = "rig-hooks")]
mod rig_factory;
#[cfg(feature = "rig-history")]
mod rig_history;
#[cfg(feature = "rig-hooks")]
mod rig_hook;
mod run_artifacts;
mod runtime_contract;
mod runtime_protocol;
mod stream_runtime;
mod tool_dispatch;
mod tool_handler;
mod tool_middleware;
mod tool_suite;

mod subagent_handlers;

pub use agent_fw_algebra::{AgentMemoryError, AgentMemoryStore};
pub use agent_fw_core::{ToolCompositionOverride, ToolDispatchOverrides};
pub use agent_fw_tool::{CollisionKind, ToolCollision};
pub use approval::{
    ApprovalContext, ApprovalDecision, ApprovalKind, ApprovalOutcome, ApprovalPolicy,
    ApprovalPredicate, ApprovalRequest, ApprovalRule,
};
#[cfg(feature = "rig-hooks")]
pub use basic_tools::{CalculatorTool, GetCurrentTimeTool};
pub use conversation::{
    parse_conversation, ChatMessage, ChatProgram, ChatRole, Conversation, ConversationError,
    Prompt, SystemPrompt,
};
#[cfg(feature = "rig-hooks")]
pub use dispatcher_rig::{dispatcher_rig_tools, DispatcherRigTool, ToolDispatcherRigExt};
pub use hook_bridge::{HookBridge, MetricsSummary, ToolOutcome};
pub use interpreter::ChatInterpreter;
pub use interpreter_contract::{assert_chat_interpreter_contract, assert_chat_interpreter_events};
pub use layer::{ApprovalLayer, ComposedLayer, ToolLayer, TracedLayer};
pub use metrics_accumulator::{MetricsAccumulator, MetricsSnapshot};
pub use model::{
    anthropic_model_supports_adaptive_thinking, anthropic_model_supports_effort,
    anthropic_reasoning_params, AgentBlueprint, AgentLabel, ModelId, ModelRouter, ModelSettings,
    ModelSettingsError, ReasoningEffort, ResolvedModel,
};
pub use orchestrator::{
    AgentOrchestrator, AgentOrchestratorBuilder, AgentRegistration, OrchestratorBuildError,
};
pub use prompt_composer::{PromptComposer, PromptSection};
pub use request_scoped_tools::{
    apply_request_scoped_tool_overrides, RequestScopedToolOverrideError,
};
#[cfg(feature = "rig-hooks")]
pub use rig_factory::{
    DynRigAgentFactory, DynRigCompletionProvider, MissingDefaultProvider, RigAgent,
    RigAgentFactory, RigBuilder, RigCompletionProvider, RigToolBuilder,
};
#[cfg(feature = "rig-history")]
pub use rig_history::{chat_message_to_rig_messages, conversation_to_rig_history};
#[cfg(feature = "rig-hooks")]
pub use rig_hook::{
    RigHookBridge, RigRequestResult, RigStreamOutcome, COMMAND_CARD_TERMINATE_REASON,
};
pub use run_artifacts::{
    load_eval_result_artifact, load_trace_artifact, load_trace_artifacts,
    write_eval_result_artifact, write_trace_artifact, write_trace_artifacts, AgentEvalResultV1,
    AgentManifestV1, AgentTraceArtifactV1, ArtifactRefV1, EvalCaseResultV1, EvalStatusV1,
    RunArtifactIoError, TraceRunContextV1, TraceStepKindV1, TraceStepStatusV1, TraceStepV1,
    AGENT_EVAL_RESULT_SCHEMA_V1, AGENT_MANIFEST_SCHEMA_V1, AGENT_TRACE_ARTIFACT_SCHEMA_V1,
    DEFAULT_EVAL_RESULT_ARTIFACT_PATH, DEFAULT_TRACE_ARTIFACTS_PATH, DEFAULT_TRACE_ARTIFACT_PATH,
};
pub use runtime_contract::{
    AgentRuntimeContractV1, CpuRequirementV1, EnvRequirementV1, ExecutionHttpEndpointV1,
    ExecutionModeKindV1, ExecutionModeV1, ExecutionOutputModeV1, GpuBackendV1, GpuRequirementV1,
    MemoryRequirementV1, MountRequirementV1, NetworkModeV1, NetworkRequirementsV1,
    ProviderCapabilityV1, ProviderKindV1, ProviderPurposeV1, ProviderRequirementV1,
    ProviderSelectionPolicyV1, ResourceRequirementsV1, SandboxRequirementsV1,
    AGENT_RUNTIME_CONTRACT_SCHEMA_V1,
};
pub use runtime_protocol::{AgentChatMessage, AgentChatRequest, AgentEndpointOverride};
pub use stream_runtime::{collect_sub_agent_stream, CollectedStreamResult};
pub use tool_dispatch::{ToolCallResult, ToolDefinition, ToolDispatcher};
pub use tool_handler::{
    fn_handler, traced, BuildError, ComposedDispatcher, FnToolHandler, ToolHandler, TracedHandler,
};
pub use tool_middleware::{with_tool_retry, with_tool_timeout, RetryHandler, TimeoutHandler};

pub use subagent_handlers::CallAgentHandler;
