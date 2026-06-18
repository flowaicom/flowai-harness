//! Pure domain types for the agent framework.
//!
//! This crate contains no async code and no IO. All types are pure data
//! with algebraic properties (monoid instances, serde roundtrip laws).
//!
//! # Modules
//!
//! - [`algebra`] — Semigroup, Monoid, Validated (core algebraic abstractions)
//! - [`usage`] — Token usage with monoid instance
//! - [`chat`] — Pure chat message roles and payloads
//! - [`cost`] — Model pricing and cost estimation
//! - [`latency`] — Latency metrics with monoid instance
//! - [`datasource`] — Shared datasource kernel types (DatabaseType)
//! - [`id`] — Typed identifiers (TenantId, ThreadId, UserId)
//! - [`tenant`] — Multi-tenancy context
//! - [`stream_part`] — AI SDK Data Stream Protocol types

/// Semigroup, Monoid, Validated — core algebraic typeclasses.
pub mod algebra;

/// Pure data types for pre-dispatch approval (pre-dispatch approval):
/// `ApprovalRequest`, `ApprovalDecision`, `ApprovalOutcome`, `ApprovalKind`,
/// `PlanStatusChange`. The async `PendingApprovalStore` algebra lives in
/// `agent-fw-algebra`; the `ApprovalRule`/`ApprovalPolicy`/`ApprovalLayer`
/// consumers live in `agent-fw-agent`.
pub mod approval;

/// Pure chat message data shared by agent and interpreter layers.
pub mod chat;

/// Model pricing and cost estimation (pure arithmetic, no IO).
pub mod cost;

/// Shared datasource kernel types (DatabaseType).
pub mod datasource;

/// Provider/model catalog metadata shared across Studio and applications.
pub mod model_catalog;

/// Generic provider/endpoint connection probe contracts.
pub mod provider_connection;

/// Typed identifiers: TenantId, ThreadId, PlanId, etc. (newtype wrappers).
pub mod id;

/// Latency distribution with monoid instance (combine timing samples).
pub mod latency;

/// Generic provider settings maps and model-config request contracts.
pub mod provider_settings;

/// Shared text utilities.
pub mod text;

/// Request-scoped tool sequencing/composition guidance.
pub mod composition_overrides;
/// Request-scoped tool description and allow/deny overrides.
pub mod tool_overrides;
/// Structured tool-registry mutations.
pub mod tool_registry_overrides;

/// Non-empty collection guarantee — law: len ≥ 1 always.
/// Tested in `agent-fw-test::non_empty_laws`.
pub mod non_empty;

/// Ordered event sequence with monotonic IDs.
pub mod sequenced;

/// Composable async event streams (programs-as-values for streaming).
/// Tested in `agent-fw-test::stream_builder_laws`.
pub mod stream_builder;

/// AI SDK Data Stream Protocol atoms — pure enum, structural types.
pub mod stream_part;

/// Multi-tenancy context (TenantContext, UserRef).
pub mod tenant;

/// Token usage accounting — commutative monoid (identity: zero, combine: fieldwise add).
pub mod usage;

/// Pure workspace context description for tenant/workspace boundaries.
pub mod workspace_context;

// Re-export key types at crate root for convenience
pub use algebra::{
    combine, fold, fold_ref, sequence_validated, validate_that, All, Any, Max, Min, Monoid,
    Semigroup, Validated, ValidationBuilder,
};
pub use approval::{
    ApprovalDecision, ApprovalKind, ApprovalOutcome, ApprovalRequest, PlanStatusChange,
};
pub use chat::{ChatMessage, ChatRole};
pub use composition_overrides::ToolCompositionOverride;
pub use cost::{
    estimate_cost, estimate_cost_simple, try_pricing_for_model, CacheTokens, ModelFamily,
    ModelPricing, RateCard, HAIKU_4_RATES, OPUS_4_RATES, SONNET_4_RATES,
};
pub use datasource::DatabaseType;
pub use id::{
    ApprovalId, EntitySetId, EvalRunId, FilterHash, PlanId, TenantId, TestCaseId, ThreadId, UserId,
    WorkspaceId,
};
pub use latency::{
    fold_latency, DistributionSummary, KVMetrics, KVTimingEvent, LatencyDistribution,
    LatencySummary, PhaseBreakdown, RetryEvent, RetryReason, TokenMetrics, ToolDistributionSummary,
    ToolDistributions, ToolStatus, ToolTiming,
};
pub use model_catalog::{
    EndpointTransportInfo, ModelCapabilities, ModelCatalog, ModelCatalogConfig, ModelInfo,
    PricingSource, ProviderCatalogSummary, ProviderCredentialMode, ProviderInfo, ProviderRegion,
    ProviderSetting, ProviderSettingKind, ProviderSettingOption,
};
pub use non_empty::NonEmpty;
pub use provider_connection::ConnectionProbeResult;
pub use provider_settings::{
    all_provider_model_views, find_provider_model_view, provider_model_views, AgentInfoView,
    AgentModelSelectionView, ListProviderModelsRequest, ListProviderModelsResponse,
    ModelConfigResponse, ModelPricingView, ProviderConfigView, ProviderModelCapabilitiesView,
    ProviderModelView, ProviderSettingsMap, VerifyConnectionRequest,
};
pub use sequenced::Sequenced;
pub use stream_builder::{
    concat_all_streams, concat_streams, empty_stream, EventStream, StreamBuilder,
    StreamBuilderError, SubAgentCall, SubAgentResult, Termination, ToolCall, ToolResult,
    ValidatedStream,
};
pub use stream_part::{
    command_card_ui, extract_dsl, AgentUsage, CommandCardPayload, CostSummary, ErrorInfo,
    FileRegistration, FinishReason, MessagePart, MessagePartAccumulator, MessagePartState,
    StreamPart, ToolAgentData, ToolAgentState, ToolInvocationData, ToolInvocationState,
    ToolProgressData,
};

// Approval typed payloads are re-exported via the top-level approval pub use
// above; the StreamPart variants `ApprovalRequired`/`ApprovalDecision`/
// `PlanStatusChange` carry them directly.
#[cfg(feature = "axum")]
pub use tenant::MissingTenantContext;
pub use tenant::{TenantContext, UserRef};
pub use text::{truncate_utf8_bytes, truncate_utf8_chars};
pub use tool_overrides::ToolDispatchOverrides;
pub use tool_registry_overrides::{
    ToolRegistryAddSource, ToolRegistryAddSpec, ToolRegistryOverride, ToolRegistryRenameSpec,
};
pub use usage::TokenUsage;
pub use workspace_context::{normalize_workspace_id, WorkspaceContext};
