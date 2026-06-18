//! Tool framework with extensible ToolEnvironment.
//!
//! Provides the core building blocks for tool implementations:
//!
//! - [`ToolEnvironment`] — Reader-pattern dependency container with TypeMap extensions
//! - [`ToolError`] — Universal tool error wrapper
//! - [`ToolSchema`] — Trait for JSON schema generation
//! - [`HookChannel`] — Bidirectional state for hook integration
//!
//! # Design
//!
//! Tools follow the Reader pattern: they receive all dependencies via `ToolEnvironment`
//! rather than accessing global state. Framework-provided capabilities (KV, EventSink,
//! SubAgents, CancellationToken) are always available. Domain-specific capabilities
//! (e.g., TargetDatabase, DataCatalog) are injected via the TypeMap extension system.
//!
//! # Example
//!
//! ```ignore
//! // Framework capabilities are always available:
//! let kv = env.kv();
//! let sink = env.event_sink();
//!
//! // Domain-specific capabilities via TypeMap:
//! let catalog = env.try_ext::<dyn DataCatalog>()?;
//! let db = env.ext_or::<dyn TargetDatabase>(&default_db);
//! ```

// Allow the derive macro to reference `agent_fw_tool::ToolSchema` when used
// within this crate's own tests.
#[cfg(test)]
extern crate self as agent_fw_tool;

/// ToolEnvironment (Reader pattern), Has<T> compile-time witnesses,
/// ToolEnvironmentBuilder with progressive finesse.
mod catalog;
mod environment;

/// ToolError, ErrorKind — universal tool error wrapper.
mod error;

/// Pattern-matching error enrichment — StringPatternEnricher (monoid: identity + compose).
/// Tested in `agent-fw-test::error_enricher_laws`.
pub mod error_enrichment;

/// HookChannel — bidirectional state for approval-card / command-card integration.
mod hook;

/// KVNamespace — scoped key prefixing over any KVStore.
mod kv_bridge;

/// Startup extension validation — ToolExtensionManifest (fail fast, not fail late).
pub mod manifest;

/// ToolOutput trait — typed serialization with UI channel injection (approval_dsl, display_summary).
/// Tested in `agent-fw-test::tool_output_laws`.
pub mod output;

/// ProgressEmitter — auto-incrementing phase counter with SSE emission.
pub mod progress;

/// ToolSchema trait — JSON Schema generation for tool input types.
mod schema;

pub use catalog::{ToolExecutionError, ToolInfo, ToolRegistry, ToolResult, ToolTier};
pub use environment::{Has, ToolEnvironment, ToolEnvironmentBuilder};
pub use error::{ErrorKind, ToolError};
pub use error_enrichment::{
    build_plan_enricher, enrich_build_error, ComposedEnricher, ErrorEnricher, HasPattern,
    IdentityEnricher, NeedPattern, PatternEnricher, PatternEnricherBuilder, StringPatternEnricher,
};
pub use hook::{CommandCardPayload, HookChannel};
pub use kv_bridge::{KVNamespace, KvBridge};
pub use manifest::{
    manifest_for, CollisionKind, MissingExtension, ToolCollision, ToolExtensionManifest,
};
pub use output::ToolOutput;
pub use progress::ProgressEmitter;
pub use schema::ToolSchema;

// Re-export the derive macro so consumers just `use agent_fw_tool::ToolSchema;`
pub use agent_fw_tool_macro::ToolSchema as DeriveToolSchema;
