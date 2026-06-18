//! Tool environment providing all dependencies for tool execution.
//!
//! This follows the Reader pattern: tools receive dependencies explicitly
//! rather than accessing global state. This makes testing trivial
//! and dependencies clear.
//!
//! # Five Canonical Access Paths
//!
//! | Need | Method | Returns | Story |
//! |------|--------|---------|-------|
//! | Framework capability (KV, EventSink, ...) | [`Has<T>::get_capability()`] | `&Arc<T>` | Compile-time proof. Always present. Infallible. |
//! | Required common extension (DB) | [`ToolEnvironment::try_target_db()`] | `Result<&Arc<T>, ToolError>` | Runtime check with first-class ergonomics for the most common query capability. |
//! | Required domain extension | [`ToolEnvironment::try_ext()`] | `Result<&Arc<T>, ToolError>` | Runtime check. Fails with actionable ToolError. |
//! | Required setup-validated extension | [`ToolEnvironment::expect_ext()`] | `&Arc<T>` | Runtime check. Panics only on wiring bugs after startup validation. |
//! | Optional domain extension | [`ToolEnvironment::maybe_ext()`] | `Option<&Arc<T>>` | Runtime check. No fallback ceremony. |
//! | Optional domain extension with default | [`ToolEnvironment::ext_or()`] | `&Arc<T>` | Runtime check. Never fails. |
//!
//! # Framework Capabilities
//!
//! Always available via typed accessors and the [`Has<T>`] trait bound:
//!
//! ```ignore
//! fn my_tool(env: &(impl Has<dyn KVStore> + Has<dyn EventSink>)) { ... }
//! ```
//!
//! # Domain Extensions
//!
//! Injected via the TypeMap extension system. Since these are dynamic
//! (runtime downcasts), they cannot be expressed in function signatures.
//! Instead, document dependencies and use `try_ext`, `maybe_ext`, or `ext_or`:
//!
//! ```ignore
//! /// Extensions: TargetDatabase (required), Bounds (optional, defaults to NONE)
//! async fn execute_plan(env: &ToolEnvironment, spec: ExecuteSpec) -> Result<...> {
//!     let db = env.try_target_db()?;
//!     let bounds = env.ext_or::<dyn Bounds>(&Bounds::NONE_ARC);
//!     // ...
//! }
//! ```
//!
//! Rust's type system can't express "this TypeMap must contain T" without
//! making `ToolEnvironment` generic (which would destroy object safety).
//! We document what we can't encode, and validate at startup via
//! [`ToolExtensionManifest`](crate::ToolExtensionManifest).

use agent_fw_algebra::{
    CancellationToken, EmbeddingService, EventSink, KVStore, SubAgentInvoker, TargetDatabase,
    VectorStore,
};
use agent_fw_core::tenant::TenantContext;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::hook::{CommandCardPayload, HookChannel};
use crate::ToolError;

/// Type-erased extension storage (like `http::Extensions`).
#[derive(Clone, Default)]
struct Extensions {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    fn insert<T: Send + Sync + 'static + ?Sized>(&mut self, val: Arc<T>) {
        self.map.insert(TypeId::of::<Arc<T>>(), Arc::new(val));
    }

    fn get<T: Send + Sync + 'static + ?Sized>(&self) -> Option<&Arc<T>> {
        self.map
            .get(&TypeId::of::<Arc<T>>())
            .and_then(|v| v.downcast_ref::<Arc<T>>())
    }
}

/// Tool environment with dynamic dispatch and TypeMap extensions.
///
/// This is the preferred environment for production use. Framework capabilities
/// are held as `Arc<dyn Trait>`, enabling runtime polymorphism. Domain-specific
/// capabilities are injected via the TypeMap extension system.
///
/// # Thread Safety
///
/// All fields are `Arc`-wrapped, making `ToolEnvironment` cheaply cloneable
/// and shareable across async tasks.
///
/// # Cancellation Support
///
/// All async operations should check `is_cancelled()` periodically and abort
/// early when true. Use `with_child_cancel()` for sub-operations that should
/// be cancelled when the parent is cancelled.
#[derive(Clone)]
pub struct ToolEnvironment {
    /// KV store for temporary state.
    kv: Arc<dyn KVStore>,
    /// Event sink for emitting stream parts.
    event_sink: Arc<dyn EventSink>,
    /// Sub-agent invoker for multi-agent orchestration.
    sub_agents: Arc<dyn SubAgentInvoker>,
    /// Tenant context (immutable, extracted from auth).
    tenant: TenantContext,
    /// Cancellation token for cooperative termination.
    cancel: CancellationToken,
    /// Hook machinery (tools don't touch this directly).
    hook_state: HookChannel,
    /// Per-dispatch tool call ID for progress correlation.
    tool_call_id: Option<String>,
    /// Domain-specific extensions via TypeMap.
    extensions: Extensions,
}

impl ToolEnvironment {
    /// Create a new tool environment with framework capabilities.
    pub fn new(
        kv: Arc<dyn KVStore>,
        event_sink: Arc<dyn EventSink>,
        sub_agents: Arc<dyn SubAgentInvoker>,
        tenant: TenantContext,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            kv,
            event_sink,
            sub_agents,
            tenant,
            cancel,
            hook_state: HookChannel::new(),
            tool_call_id: None,
            extensions: Extensions::default(),
        }
    }

    /// Start building an environment with smart defaults.
    ///
    /// Only `kv` and `tenant` are required. Everything else defaults to
    /// null/no-op implementations — override what you need:
    ///
    /// ```ignore
    /// // Production: explicit everything
    /// ToolEnvironment::builder()
    ///     .kv(my_kv)
    ///     .event_sink(my_sink)
    ///     .sub_agents(orchestrator)
    ///     .tenant("prod-tenant-123")
    ///     .build()
    ///
    /// // Test: just what you need
    /// ToolEnvironment::builder()
    ///     .kv(DashMapKVStore::new())
    ///     .tenant("test")
    ///     .build()
    /// ```
    pub fn builder() -> ToolEnvironmentBuilder {
        ToolEnvironmentBuilder::default()
    }

    // =========================================================================
    // Framework Capability Accessors
    // =========================================================================

    /// Get a reference to the KV store.
    #[inline]
    pub fn kv(&self) -> &Arc<dyn KVStore> {
        &self.kv
    }

    /// Get a reference to the event sink.
    #[inline]
    pub fn event_sink(&self) -> &Arc<dyn EventSink> {
        &self.event_sink
    }

    /// Get a reference to the sub-agent invoker.
    #[inline]
    pub fn sub_agents(&self) -> &Arc<dyn SubAgentInvoker> {
        &self.sub_agents
    }

    /// Get a reference to the tenant context.
    #[inline]
    pub fn tenant(&self) -> &TenantContext {
        &self.tenant
    }

    /// Get a reference to the cancellation token.
    #[inline]
    pub fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Get the tenant resource ID for KV key scoping.
    pub fn resource_id(&self) -> &agent_fw_core::id::TenantId {
        self.tenant.resource_id()
    }

    /// Get a reference to the hook state (for wiring to AgentEventBridge).
    pub fn hook_state(&self) -> &HookChannel {
        &self.hook_state
    }

    /// Read the current tool call ID (set by the hook before tool execution).
    pub fn current_tool_call_id(&self) -> Option<String> {
        self.hook_state.current_tool_call_id()
    }

    /// Set the current tool call ID (called by hook middleware).
    pub fn set_current_tool_call_id(&self, id: Option<String>) {
        self.hook_state.set_current_tool_call_id(id);
    }

    /// Get a shared reference to the tool-call-id cell for hook wiring.
    pub fn tool_call_id_cell(&self) -> Arc<Mutex<Option<String>>> {
        self.hook_state.tool_call_id_cell()
    }

    /// Buffer a card + summary for post-tool-result emission by the hook.
    pub fn buffer_card(&self, display_summary: Option<String>, approval_dsl: Option<String>) {
        self.hook_state.buffer_card(display_summary, approval_dsl);
    }

    /// Take the pending card buffered during tool execution, if any.
    pub fn take_pending_card(&self) -> Option<CommandCardPayload> {
        self.hook_state.take_pending_card()
    }

    /// Get a shared reference to the pending-card cell for hook wiring.
    pub fn pending_card_cell(&self) -> Arc<Mutex<Option<CommandCardPayload>>> {
        self.hook_state.pending_card_cell()
    }

    // =========================================================================
    // Extension (TypeMap) Methods
    // =========================================================================

    /// Insert a domain-specific capability into the TypeMap.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let env = ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel)
    ///     .with_ext::<dyn TargetDatabase>(target_db)
    ///     .with_ext::<dyn DataCatalog>(catalog);
    /// ```
    pub fn with_ext<T: Send + Sync + 'static + ?Sized>(mut self, val: Arc<T>) -> Self {
        self.extensions.insert::<T>(val);
        self
    }

    /// Register a target database as a first-class common extension.
    pub fn with_target_db(self, db: Arc<dyn TargetDatabase>) -> Self {
        self.with_ext::<dyn TargetDatabase>(db)
    }

    /// Register a vector store as a first-class common extension.
    pub fn with_vector_store(self, vector_store: Arc<dyn VectorStore>) -> Self {
        self.with_ext::<dyn VectorStore>(vector_store)
    }

    /// Register an embedding service as a first-class common extension.
    pub fn with_embedder(self, embedder: Arc<dyn EmbeddingService>) -> Self {
        self.with_ext::<dyn EmbeddingService>(embedder)
    }

    /// Retrieve a domain-specific capability from the TypeMap.
    fn ext<T: Send + Sync + 'static + ?Sized>(&self) -> Option<&Arc<T>> {
        self.extensions.get::<T>()
    }

    /// Retrieve an optional domain-specific capability.
    ///
    /// Returns `None` if the capability has not been registered.
    /// Use this when absence is expected and there is no natural default value.
    ///
    /// # Law
    ///
    /// - **Consistency**: `maybe_ext::<T>() == ext::<T>()` for all `T`.
    pub fn maybe_ext<T: Send + Sync + 'static + ?Sized>(&self) -> Option<&Arc<T>> {
        self.ext::<T>()
    }

    /// Retrieve an optional target database capability.
    pub fn maybe_target_db(&self) -> Option<&Arc<dyn TargetDatabase>> {
        self.maybe_ext::<dyn TargetDatabase>()
    }

    /// Retrieve an optional vector store capability.
    pub fn vector_store(&self) -> Option<&Arc<dyn VectorStore>> {
        self.maybe_ext::<dyn VectorStore>()
    }

    /// Retrieve an optional embedding service capability.
    pub fn embedder(&self) -> Option<&Arc<dyn EmbeddingService>> {
        self.maybe_ext::<dyn EmbeddingService>()
    }

    /// Retrieve a required domain-specific capability, panicking if missing.
    ///
    /// Use this only for setup-validated extensions where absence indicates an
    /// application wiring bug rather than a recoverable tool error.
    pub fn expect_ext<T: Send + Sync + 'static + ?Sized>(&self) -> &Arc<T> {
        self.ext::<T>().unwrap_or_else(|| {
            panic!(
                "ToolEnvironment missing required extension: {}",
                std::any::type_name::<T>()
            )
        })
    }

    /// Retrieve a required target database capability, panicking if missing.
    pub fn target_db(&self) -> &Arc<dyn TargetDatabase> {
        self.expect_ext::<dyn TargetDatabase>()
    }

    /// Retrieve a domain-specific capability, returning a default if missing.
    ///
    /// Total: never panics, never errors. If the extension is not present,
    /// returns the provided default.
    ///
    /// # Law
    ///
    /// - **Consistency**: `ext_or::<T>(d) == ext::<T>().unwrap_or(d)` for all `T`, `d`.
    pub fn ext_or<'a, T: Send + Sync + 'static + ?Sized>(
        &'a self,
        default: &'a Arc<T>,
    ) -> &'a Arc<T> {
        self.ext::<T>().unwrap_or(default)
    }

    /// Check whether an extension is present by its `TypeId`.
    ///
    /// Used internally by [`ToolExtensionManifest`](crate::manifest::ToolExtensionManifest)
    /// for startup-time validation. Most code should use `ext::<T>()` instead.
    pub fn has_ext_by_type_id(&self, type_id: std::any::TypeId) -> bool {
        self.extensions.map.contains_key(&type_id)
    }

    /// Retrieve a domain-specific capability, returning `Err(ToolError)` if missing.
    ///
    /// Use this in tool handlers to maintain error totality (L3: never panic).
    ///
    /// ```ignore
    /// let db = env.try_ext::<dyn TargetDatabase>()?;
    /// ```
    pub fn try_ext<T: Send + Sync + 'static + ?Sized>(&self) -> Result<&Arc<T>, crate::ToolError> {
        self.ext::<T>()
            .ok_or_else(|| crate::ToolError::missing_ext(std::any::type_name::<T>()))
    }

    /// Retrieve a required target database capability as a tool error.
    pub fn try_target_db(&self) -> Result<&Arc<dyn TargetDatabase>, crate::ToolError> {
        self.try_ext::<dyn TargetDatabase>()
    }

    // =========================================================================
    // Cancellation Helpers
    // =========================================================================

    /// Check if the operation should abort.
    ///
    /// Tools should check this periodically during long-running operations
    /// and abort early when true.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Guard: returns `Err` if cancelled, `Ok(())` otherwise.
    ///
    /// Use at the top of every tool `call()`:
    /// ```ignore
    /// self.env.ensure_active()?;
    /// ```
    #[inline]
    pub fn ensure_active(&self) -> Result<(), ToolError> {
        if self.cancel.is_cancelled() {
            Err(ToolError::cancelled())
        } else {
            Ok(())
        }
    }

    // =========================================================================
    // Progress Tracking
    // =========================================================================

    /// Create a [`ProgressEmitter`](crate::ProgressEmitter) wired to this
    /// environment's event sink.
    ///
    /// ```ignore
    /// let progress = env.progress_tracker("draft_plan", Some(call_id), 3);
    /// progress.advance("Resolving entities", None);
    /// ```
    pub fn progress_tracker(
        &self,
        tool_name: &str,
        tool_call_id: Option<&str>,
        total_phases: u8,
    ) -> crate::ProgressEmitter {
        let effective_id = tool_call_id
            .map(String::from)
            .or_else(|| self.tool_call_id.clone());
        crate::ProgressEmitter::new(
            self.event_sink.clone(),
            tool_name,
            effective_id,
            total_phases,
        )
    }

    /// Create a lazy [`ProgressEmitter`](crate::ProgressEmitter) where
    /// `total_phases` is set later via
    /// [`set_total_phases()`](crate::ProgressEmitter::set_total_phases).
    ///
    /// Use this for dynamic phase counts (e.g., constraint expansion where
    /// the number of groups varies per invocation):
    ///
    /// ```ignore
    /// let progress = env.lazy_progress_tracker("draft_plan", Some(call_id));
    /// progress.set_total_phases(groups.len() as u8 * 2);
    /// for group in &groups {
    ///     progress.advance(&format!("Resolving {}", group.name), None);
    ///     progress.advance(&format!("Building {}", group.name), None);
    /// }
    /// ```
    pub fn lazy_progress_tracker(
        &self,
        tool_name: &str,
        tool_call_id: Option<&str>,
    ) -> crate::ProgressEmitter {
        let effective_id = tool_call_id
            .map(String::from)
            .or_else(|| self.tool_call_id.clone());
        crate::ProgressEmitter::lazy(self.event_sink.clone(), tool_name, effective_id)
    }

    // =========================================================================
    // Builder Methods (derive modified environments)
    // =========================================================================

    /// Create a derived environment with a different tenant context.
    ///
    /// Shares the cancellation token — cancelling the parent cancels the child.
    pub fn with_tenant(&self, tenant: TenantContext) -> Self {
        let mut ctx = self.clone();
        ctx.tenant = tenant;
        ctx
    }

    /// Create a derived environment with a different KV store.
    ///
    /// Used to wrap the KV store with instrumentation (e.g., `InstrumentedKVStore`).
    pub fn with_kv(&self, kv: Arc<dyn KVStore>) -> Self {
        let mut ctx = self.clone();
        ctx.kv = kv;
        ctx
    }

    /// Create a derived environment with a different event sink.
    ///
    /// Used for sub-agent isolation. Shares the cancellation token.
    pub fn with_sink(&self, event_sink: Arc<dyn EventSink>) -> Self {
        let mut ctx = self.clone();
        ctx.event_sink = event_sink;
        ctx
    }

    /// Create a derived environment with a per-dispatch tool call ID.
    ///
    /// The ID is used as the default for `progress_tracker()` and
    /// `lazy_progress_tracker()` when no explicit ID is passed.
    /// This enables automatic progress→tool correlation without
    /// changes to tool handler code.
    pub fn with_tool_call_id(&self, id: &str) -> Self {
        let mut ctx = self.clone();
        ctx.tool_call_id = Some(id.to_string());
        ctx
    }

    /// Create a derived environment with a child cancellation token.
    ///
    /// The child token is cancelled when the parent is cancelled.
    /// Use this for sub-operations that should be independently cancellable
    /// but also cancelled when the parent is cancelled.
    pub fn with_child_cancel(&self) -> Self {
        let mut ctx = self.clone();
        ctx.cancel = self.cancel.child();
        ctx
    }
}

// =============================================================================
// ToolEnvironmentBuilder — fluent construction with smart defaults
// =============================================================================

/// Builder for [`ToolEnvironment`] with smart defaults.
///
/// Only `kv` and `tenant` are required. All other capabilities default to
/// null/no-op implementations:
///
/// - `event_sink` → `NullEventSink` (events discarded)
/// - `sub_agents` → `NullSubAgentInvoker` (no sub-agents available)
/// - `cancel` → fresh `CancellationToken`
///
/// # Design (you specify what matters, the rest just works)
///
/// The builder eliminates the 5-`Arc` ceremony that clutters every test and
/// setup function. You name the capabilities you care about; the builder
/// provides safe defaults for everything else.
///
/// # Example
///
/// ```ignore
/// let env = ToolEnvironment::builder()
///     .kv(DashMapKVStore::new())
///     .tenant("test")
///     .build();
/// ```
#[derive(Default)]
pub struct ToolEnvironmentBuilder {
    kv: Option<Arc<dyn KVStore>>,
    event_sink: Option<Arc<dyn EventSink>>,
    sub_agents: Option<Arc<dyn SubAgentInvoker>>,
    tenant: Option<TenantContext>,
    cancel: Option<CancellationToken>,
}

impl ToolEnvironmentBuilder {
    /// Set the KV store (required).
    ///
    /// Accepts any `impl KVStore` — wraps in `Arc` for you.
    pub fn kv(mut self, kv: impl KVStore + 'static) -> Self {
        self.kv = Some(Arc::new(kv));
        self
    }

    /// Set the KV store from an existing `Arc`.
    pub fn kv_arc(mut self, kv: Arc<dyn KVStore>) -> Self {
        self.kv = Some(kv);
        self
    }

    /// Set the event sink.
    ///
    /// Default: `NullEventSink` (events discarded silently).
    pub fn event_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.event_sink = Some(Arc::new(sink));
        self
    }

    /// Set the event sink from an existing `Arc`.
    pub fn event_sink_arc(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// Set the sub-agent invoker.
    ///
    /// Default: `NullSubAgentInvoker` (no sub-agents available).
    pub fn sub_agents(mut self, sub_agents: impl SubAgentInvoker + 'static) -> Self {
        self.sub_agents = Some(Arc::new(sub_agents));
        self
    }

    /// Set the sub-agent invoker from an existing `Arc`.
    pub fn sub_agents_arc(mut self, sub_agents: Arc<dyn SubAgentInvoker>) -> Self {
        self.sub_agents = Some(sub_agents);
        self
    }

    /// Set the tenant by ID string (required).
    ///
    /// Creates a `TenantContext` with an unchecked `TenantId`.
    pub fn tenant(mut self, tenant_id: &str) -> Self {
        self.tenant = Some(TenantContext::new(
            agent_fw_core::id::TenantId::new_unchecked(tenant_id),
        ));
        self
    }

    /// Set the tenant from a `TenantContext`.
    pub fn tenant_context(mut self, tenant: TenantContext) -> Self {
        self.tenant = Some(tenant);
        self
    }

    /// Set the cancellation token.
    ///
    /// Default: a fresh `CancellationToken`.
    pub fn cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Build the environment.
    ///
    /// # Panics
    ///
    /// Panics if `kv` or `tenant` were not set.
    pub fn build(self) -> ToolEnvironment {
        use agent_fw_algebra::testing::{NullEventSink, NullSubAgentInvoker};

        let kv = self
            .kv
            .expect("ToolEnvironmentBuilder: kv is required — call .kv(store)");
        let tenant = self
            .tenant
            .expect("ToolEnvironmentBuilder: tenant is required — call .tenant(\"id\")");

        ToolEnvironment::new(
            kv,
            self.event_sink.unwrap_or_else(|| Arc::new(NullEventSink)),
            self.sub_agents
                .unwrap_or_else(|| Arc::new(NullSubAgentInvoker)),
            tenant,
            self.cancel.unwrap_or_else(CancellationToken::new),
        )
    }
}

// =============================================================================
// Has<T> — compile-time capability witness
// =============================================================================

/// Compile-time witness that an environment provides capability `T`.
///
/// Use this as a trait bound on tools to express their dependencies at the
/// type level rather than relying on runtime TypeMap lookups:
///
/// ```ignore
/// fn my_tool(env: &impl Has<dyn TargetDatabase>) -> Result<(), ToolError> {
///     let db = env.get_capability();
///     // ...
/// }
/// ```
///
/// # Law
///
/// - **L1 (Totality)**: `get_capability()` never panics.
///
/// # Proof obligation
///
/// Totality is enforced *structurally*: each `impl Has<T> for Env` must
/// return a reference to a non-optional field of `Env`. Because the field
/// is always present after construction, the call is infallible by the
/// type system — no `Option`, no `expect()`, no runtime check.
///
/// If you add a new `Has<T>` impl backed by an `Option<Arc<T>>` field,
/// you must instead use a builder / typestate pattern to guarantee the
/// field is populated before `build()`, or the totality law is violated.
pub trait Has<T: Send + Sync + 'static + ?Sized> {
    /// Retrieve the capability.
    ///
    /// # Contract
    ///
    /// Implementations **must** return a reference to a non-optional field.
    /// This makes the call infallible by construction (not by convention).
    fn get_capability(&self) -> &Arc<T>;
}

// Framework capabilities are always present:

impl Has<dyn KVStore> for ToolEnvironment {
    #[inline]
    fn get_capability(&self) -> &Arc<dyn KVStore> {
        &self.kv
    }
}

impl Has<dyn EventSink> for ToolEnvironment {
    #[inline]
    fn get_capability(&self) -> &Arc<dyn EventSink> {
        &self.event_sink
    }
}

impl Has<dyn SubAgentInvoker> for ToolEnvironment {
    #[inline]
    fn get_capability(&self) -> &Arc<dyn SubAgentInvoker> {
        &self.sub_agents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::event_sink::EventSink;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_algebra::{
        DbError, DbRow, EmbeddingItem, QueryParam, ReadOnlyQuery, VectorHit, VectorStoreError,
    };
    use async_trait::async_trait;

    fn test_tenant() -> TenantContext {
        TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test-tenant"))
    }

    fn test_env() -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        ToolEnvironment::new(kv, sink, sub_agents, test_tenant(), cancel)
    }

    #[test]
    fn environment_creation() {
        let env = test_env();
        assert_eq!(env.resource_id().as_str(), "test-tenant");
        assert!(!env.is_cancelled());
    }

    #[test]
    fn environment_is_cloneable() {
        let env1 = test_env();
        let env2 = env1.clone();
        assert_eq!(env1.resource_id().as_str(), env2.resource_id().as_str());
    }

    #[test]
    fn cancellation_shared() {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();

        let env1 = ToolEnvironment::new(kv, sink, sub_agents, test_tenant(), cancel.clone());
        let env2 = env1.clone();

        assert!(!env1.is_cancelled());
        assert!(!env2.is_cancelled());

        cancel.cancel();

        assert!(env1.is_cancelled());
        assert!(env2.is_cancelled());
    }

    #[test]
    fn child_cancel() {
        let env = test_env();
        let child = env.with_child_cancel();

        assert!(!env.is_cancelled());
        assert!(!child.is_cancelled());

        // Cancelling child should not affect parent
        child.cancel().cancel();
        assert!(!env.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn parent_cancel_propagates_to_child() {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();

        let env = ToolEnvironment::new(kv, sink, sub_agents, test_tenant(), cancel.clone());
        let child = env.with_child_cancel();

        cancel.cancel();
        assert!(env.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn ensure_active_ok_when_not_cancelled() {
        let env = test_env();
        assert!(env.ensure_active().is_ok());
    }

    #[test]
    fn ensure_active_err_when_cancelled() {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();

        let env = ToolEnvironment::new(kv, sink, sub_agents, test_tenant(), cancel.clone());
        cancel.cancel();
        assert!(env.ensure_active().is_err());
    }

    #[test]
    fn with_tenant_preserves_cancel() {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();

        let env1 = ToolEnvironment::new(kv, sink, sub_agents, test_tenant(), cancel.clone());
        let new_tenant =
            TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("other-tenant"));
        let env2 = env1.with_tenant(new_tenant);

        assert_eq!(env2.resource_id().as_str(), "other-tenant");

        cancel.cancel();
        assert!(env1.is_cancelled());
        assert!(env2.is_cancelled());
    }

    // =========================================================================
    // TypeMap Extension Tests
    // =========================================================================

    // A domain-specific trait for testing extensions
    trait DomainCapability: Send + Sync {
        fn name(&self) -> &str;
    }

    struct TestCapability;
    impl DomainCapability for TestCapability {
        fn name(&self) -> &str {
            "test"
        }
    }

    #[test]
    fn with_ext_stores_and_retrieves() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));

        let cap = env.ext::<dyn DomainCapability>();
        assert!(cap.is_some());
        assert_eq!(cap.unwrap().name(), "test");
    }

    #[test]
    fn maybe_ext_returns_none_for_missing() {
        let env = test_env();
        let cap = env.maybe_ext::<dyn DomainCapability>();
        assert!(cap.is_none());
    }

    #[test]
    fn maybe_ext_returns_extension_when_present() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));
        let cap = env.maybe_ext::<dyn DomainCapability>();
        assert!(cap.is_some());
        assert_eq!(cap.unwrap().name(), "test");
    }

    #[test]
    fn expect_ext_returns_extension_when_present() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));
        let cap = env.expect_ext::<dyn DomainCapability>();
        assert_eq!(cap.name(), "test");
    }

    #[test]
    fn hook_helpers_proxy_to_hook_state() {
        let env = test_env();

        assert!(env.current_tool_call_id().is_none());
        env.set_current_tool_call_id(Some("call-123".to_string()));
        assert_eq!(env.current_tool_call_id(), Some("call-123".to_string()));
        assert_eq!(
            *env.tool_call_id_cell().lock().expect("tool call id mutex"),
            Some("call-123".to_string())
        );

        env.buffer_card(Some("summary".to_string()), Some("dsl".to_string()));
        assert_eq!(
            env.take_pending_card(),
            Some(CommandCardPayload {
                display_summary: Some("summary".to_string()),
                approval_dsl: Some("dsl".to_string()),
            })
        );
        assert!(env.take_pending_card().is_none());
        assert!(env
            .pending_card_cell()
            .lock()
            .expect("pending card mutex")
            .is_none());
    }

    // =========================================================================
    // ext_or tests
    // =========================================================================

    #[test]
    fn ext_or_returns_extension_when_present() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));
        let fallback: Arc<dyn DomainCapability> = Arc::new(TestCapability);
        let cap = env.ext_or::<dyn DomainCapability>(&fallback);
        assert_eq!(cap.name(), "test");
    }

    #[test]
    fn ext_or_returns_default_when_missing() {
        struct FallbackCapability;
        impl DomainCapability for FallbackCapability {
            fn name(&self) -> &str {
                "fallback"
            }
        }

        let env = test_env();
        let fallback: Arc<dyn DomainCapability> = Arc::new(FallbackCapability);
        let cap = env.ext_or::<dyn DomainCapability>(&fallback);
        assert_eq!(cap.name(), "fallback");
    }

    #[test]
    fn ext_or_consistent_with_ext() {
        // Law: ext_or(d) == ext().unwrap_or(d)
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));
        let fallback: Arc<dyn DomainCapability> = Arc::new(TestCapability);

        let via_ext_or = env.ext_or::<dyn DomainCapability>(&fallback).name();
        let via_ext = env
            .ext::<dyn DomainCapability>()
            .unwrap_or(&fallback)
            .name();
        assert_eq!(via_ext_or, via_ext);
    }

    #[test]
    fn extensions_survive_clone() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));

        let cloned = env.clone();
        let cap = cloned.ext::<dyn DomainCapability>();
        assert!(cap.is_some());
        assert_eq!(cap.unwrap().name(), "test");
    }

    // =========================================================================
    // try_ext tests
    // =========================================================================

    #[test]
    fn try_ext_returns_ok_when_present() {
        let env = test_env().with_ext::<dyn DomainCapability>(Arc::new(TestCapability));
        let cap = env.try_ext::<dyn DomainCapability>();
        assert!(cap.is_ok());
        assert_eq!(cap.unwrap().name(), "test");
    }

    #[test]
    fn try_ext_returns_err_when_missing() {
        let env = test_env();
        let result = env.try_ext::<dyn DomainCapability>();
        match result {
            Ok(_) => panic!("expected Err"),
            Err(err) => {
                assert!(err.message().contains("DomainCapability"));
                assert!(err.message().contains("with_ext"));
            }
        }
    }

    #[test]
    fn common_capability_accessors_round_trip() {
        struct StubTargetDb;
        #[async_trait]
        impl TargetDatabase for StubTargetDb {
            async fn query(
                &self,
                _query: &ReadOnlyQuery,
                _params: &[QueryParam],
            ) -> Result<Vec<DbRow>, DbError> {
                Ok(vec![])
            }

            async fn health_check(&self) -> Result<(), DbError> {
                Ok(())
            }

            async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
                Ok(vec![])
            }

            async fn get_table_columns(&self, _table_name: &str) -> Result<Vec<DbRow>, DbError> {
                Ok(vec![])
            }

            async fn sample_table(
                &self,
                _table_name: &str,
                _limit: usize,
            ) -> Result<Vec<serde_json::Value>, DbError> {
                Ok(vec![])
            }
        }

        struct StubVectorStore;
        #[async_trait]
        impl VectorStore for StubVectorStore {
            async fn search_similar(
                &self,
                _embedding: &[f32],
                _limit: usize,
                _min_similarity: f64,
            ) -> Result<Vec<VectorHit>, VectorStoreError> {
                Ok(vec![])
            }

            async fn upsert_embedding(
                &self,
                _id: &str,
                _content: &str,
                _item_type: &str,
                _metadata: serde_json::Value,
                _embedding: &[f32],
            ) -> Result<(), VectorStoreError> {
                Ok(())
            }

            async fn upsert_batch(
                &self,
                _items: &[EmbeddingItem],
            ) -> Result<usize, VectorStoreError> {
                Ok(0)
            }

            async fn delete_by_prefix(&self, _id_prefix: &str) -> Result<usize, VectorStoreError> {
                Ok(0)
            }

            async fn get_by_id(&self, _id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
                Ok(None)
            }

            async fn health_check(&self) -> Result<(), VectorStoreError> {
                Ok(())
            }
        }

        let db: Arc<dyn TargetDatabase> = Arc::new(StubTargetDb);
        let vector_store: Arc<dyn VectorStore> = Arc::new(StubVectorStore);

        struct StubEmbedder;
        #[async_trait]
        impl EmbeddingService for StubEmbedder {
            async fn embed_batch(
                &self,
                texts: &[&str],
            ) -> Result<Vec<Vec<f32>>, agent_fw_algebra::EmbeddingError> {
                Ok(texts.iter().map(|_| vec![0.0, 1.0]).collect())
            }

            fn dimension(&self) -> usize {
                2
            }

            fn model_name(&self) -> &str {
                "stub"
            }
        }

        let embedder: Arc<dyn EmbeddingService> = Arc::new(StubEmbedder);

        let env = test_env()
            .with_target_db(Arc::clone(&db))
            .with_vector_store(Arc::clone(&vector_store))
            .with_embedder(Arc::clone(&embedder));

        assert!(Arc::ptr_eq(env.target_db(), &db));
        assert!(Arc::ptr_eq(env.try_target_db().unwrap(), &db));
        assert!(Arc::ptr_eq(env.vector_store().unwrap(), &vector_store));
        assert!(Arc::ptr_eq(env.embedder().unwrap(), &embedder));
        assert!(env.maybe_target_db().is_some());
    }

    // =========================================================================
    // Has<T> Compile-Time Capability Tests
    // =========================================================================

    #[test]
    fn has_kv_store() {
        let env = test_env();
        let kv: &Arc<dyn KVStore> = Has::<dyn KVStore>::get_capability(&env);
        let _ = kv; // Compile-time proof it exists
    }

    #[test]
    fn has_event_sink() {
        let env = test_env();
        let sink: &Arc<dyn EventSink> = Has::<dyn EventSink>::get_capability(&env);
        assert!(sink.is_open());
    }

    #[test]
    fn has_sub_agent_invoker() {
        let env = test_env();
        let agents: &Arc<dyn SubAgentInvoker> = Has::<dyn SubAgentInvoker>::get_capability(&env);
        assert!(!agents.has_agent("anything"));
    }

    /// Demonstrate compile-time bounds for tools.
    fn _tool_with_bounds(env: &(impl Has<dyn KVStore> + Has<dyn EventSink>)) -> bool {
        let _kv = env.get_capability() as &Arc<dyn KVStore>;
        let sink: &Arc<dyn EventSink> = env.get_capability();
        sink.is_open()
    }

    #[test]
    fn tool_with_has_bounds_compiles() {
        let env = test_env();
        assert!(_tool_with_bounds(&env));
    }

    // =========================================================================
    // ToolEnvironmentBuilder Tests
    // =========================================================================

    #[test]
    fn builder_with_required_fields() {
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .tenant("test-builder")
            .build();
        assert_eq!(env.resource_id().as_str(), "test-builder");
        assert!(!env.is_cancelled());
    }

    #[test]
    fn builder_with_all_fields() {
        let cancel = CancellationToken::new();
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant("full")
            .cancel(cancel.clone())
            .build();
        assert_eq!(env.resource_id().as_str(), "full");
        cancel.cancel();
        assert!(env.is_cancelled());
    }

    #[test]
    fn builder_defaults_event_sink() {
        // NullEventSink: emit returns true (no-op), is_open returns true
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .tenant("t")
            .build();
        assert!(env.event_sink().is_open());
    }

    #[test]
    fn builder_defaults_sub_agents() {
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .tenant("t")
            .build();
        assert!(!env.sub_agents().has_agent("anything"));
    }

    #[test]
    fn builder_defaults_cancel_token() {
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .tenant("t")
            .build();
        assert!(!env.is_cancelled());
    }

    #[test]
    fn builder_kv_arc() {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let env = ToolEnvironment::builder().kv_arc(kv).tenant("t").build();
        assert_eq!(env.resource_id().as_str(), "t");
    }

    #[test]
    fn builder_tenant_context() {
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("ctx-tenant"));
        let env = ToolEnvironment::builder()
            .kv(NullKVStore)
            .tenant_context(tenant)
            .build();
        assert_eq!(env.resource_id().as_str(), "ctx-tenant");
    }

    #[test]
    #[should_panic(expected = "kv is required")]
    fn builder_panics_without_kv() {
        let _ = ToolEnvironment::builder().tenant("t").build();
    }

    #[test]
    #[should_panic(expected = "tenant is required")]
    fn builder_panics_without_tenant() {
        let _ = ToolEnvironment::builder().kv(NullKVStore).build();
    }

    #[test]
    fn multiple_extensions() {
        trait AnotherCapability: Send + Sync {
            fn value(&self) -> i32;
        }

        struct AnotherImpl;
        impl AnotherCapability for AnotherImpl {
            fn value(&self) -> i32 {
                42
            }
        }

        let env = test_env()
            .with_ext::<dyn DomainCapability>(Arc::new(TestCapability))
            .with_ext::<dyn AnotherCapability>(Arc::new(AnotherImpl));

        assert_eq!(env.ext::<dyn DomainCapability>().unwrap().name(), "test");
        assert_eq!(env.ext::<dyn AnotherCapability>().unwrap().value(), 42);
    }
}
