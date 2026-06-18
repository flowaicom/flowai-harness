//! Composable tool handler algebra and monoidal dispatcher.
//!
//! # Design
//!
//! [`ToolHandler`] is the composable atom: a self-describing unit that pairs
//! a tool's schema ([`ToolDefinition`]) with its execution logic. Handlers
//! receive a [`ToolEnvironment`] per-call, following the Reader pattern.
//!
//! [`ComposedDispatcher`] is the monoidal combinator: it collects handlers
//! by name and implements [`ToolDispatcher`] for use with the interpreter.
//! It bridges the signature gap — `ToolDispatcher::dispatch` does not take
//! an environment, but `ToolHandler::handle` does. The dispatcher stores
//! the environment and passes it through.
//!
//! # Algebraic Laws
//!
//! ## ToolHandler Laws
//!
//! - **L1 (Schema-Dispatch Consistency)**: `handle()` succeeds only for input
//!   that is valid against `definition().input_schema`. Invalid input produces
//!   `ToolCallResult { is_error: true, .. }`.
//!
//! - **L2 (Deterministic Schema)**: `definition()` is pure — calling it twice
//!   on the same handler produces structurally equal `ToolDefinition` values.
//!
//! - **L3 (Error Totality)**: `handle()` never panics. All failure modes are
//!   represented as `ToolCallResult { is_error: true, .. }`.
//!
//! ## ComposedDispatcher Laws
//!
//! - **L1 (Dispatch Fidelity)**: `dispatch(name, id, input)` delegates to the
//!   handler registered under `name`, passing the stored `ToolEnvironment`.
//!
//! - **L2 (Unknown Tool Error)**: `dispatch(name, ..)` for an unregistered
//!   `name` returns `ToolCallResult::error(..)`.
//!
//! - **L3 (Monoidal Identity)**: `empty(env).merge(d) == d` and
//!   `d.merge(empty(env)) == d` (in terms of handler routing).
//!
//! - **L4 (Monoidal Associativity)**: `(a.merge(b)).merge(c)` routes the same
//!   as `a.merge(b.merge(c))` for all tool names.
//!
//! - **L5 (Definition Completeness)**: `tool_definitions()` returns exactly
//!   one definition per registered handler, matching `handler.definition()`.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use agent_fw_tool::ToolEnvironment;

use crate::{ToolCallResult, ToolDefinition, ToolDispatcher};

// ─── ToolHandler ────────────────────────────────────────────────────────

/// A composable unit connecting a tool's schema to its execution.
///
/// Each handler is self-describing: it carries both its definition (for
/// LLM registration) and its execution logic (for dispatch).
///
/// Handlers are the atoms of tool composition — combine them into a
/// [`ComposedDispatcher`] which implements [`ToolDispatcher`].
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// The tool schema: name, description, input_schema.
    ///
    /// Must be deterministic: calling twice yields structurally equal results.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given input and environment.
    ///
    /// Must never panic. All failures are represented as
    /// `ToolCallResult { is_error: true, .. }`.
    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult;

    /// Declare which TypeMap extensions this handler requires or optionally uses.
    ///
    /// Override this to enable startup-time validation via
    /// [`ComposedDispatcher::validate_extensions`]. The default returns
    /// an empty manifest (no requirements).
    fn extension_manifest(&self) -> agent_fw_tool::ToolExtensionManifest {
        agent_fw_tool::ToolExtensionManifest::new()
    }
}

// ─── ComposedDispatcher ─────────────────────────────────────────────────

/// Monoidal tool routing: collects [`ToolHandler`]s by name and implements
/// [`ToolDispatcher`] for use with the interpreter.
///
/// Stores a [`ToolEnvironment`] and passes it to each handler's `handle()`
/// call, bridging the gap between `ToolHandler` (which receives env per-call)
/// and `ToolDispatcher` (which does not take env in its signature).
///
/// # Construction
///
/// ```ignore
/// let dispatcher = ComposedDispatcher::new(env)
///     .with_handler(Arc::new(MyHandler))
///     .with_handler(Arc::new(AnotherHandler));
/// ```
///
/// # Monoidal Composition
///
/// ```ignore
/// let merged = dispatcher_a.merge(dispatcher_b);
/// // merged routes to all handlers from both dispatchers.
/// // Right-hand side wins on name collisions.
/// ```
#[derive(Clone)]
pub struct ComposedDispatcher {
    handlers: HashMap<String, Arc<dyn ToolHandler>>,
    latent_handlers: HashMap<String, Arc<dyn ToolHandler>>,
    env: ToolEnvironment,
    /// Every duplicate-name registration attempt observed during composition.
    ///
    /// `merge`, `add_handler`, `add_latent_handler`, and `with_handlers` keep
    /// their right-biased `HashMap::insert` semantics so the algebraic monoid
    /// laws still hold, but each time a name would be overwritten we record a
    /// `ToolCollision` here. `try_build` (and `try_merge`) surface these so
    /// startup fails loudly instead of a vertical tool silently shadowing a
    /// framework toolkit.
    collisions: Vec<agent_fw_tool::ToolCollision>,
}

impl ComposedDispatcher {
    /// Create an empty dispatcher (monoidal identity).
    ///
    /// Use this for incremental handler registration via `with_handler()`.
    /// After all handlers are registered, call [`validate_extensions`] to
    /// enforce the Seam 4 guarantee, or use [`try_build`] for automatic
    /// validation.
    pub fn new(env: ToolEnvironment) -> Self {
        Self {
            handlers: HashMap::new(),
            latent_handlers: HashMap::new(),
            env,
            collisions: Vec::new(),
        }
    }

    /// Finalize the dispatcher: surface duplicate-name collisions and missing
    /// extensions in one call.
    ///
    /// # Seam 4 guarantee
    ///
    /// After `try_build()` returns `Ok`, no handler's `try_ext()` call will
    /// fail for a declared extension, **and** no two registered handlers share
    /// a name. Collisions are reported in preference to missing extensions —
    /// shadowing is the more fundamental composition failure.
    ///
    /// `merge`, `with_handler`, `add_handler`, `with_handlers`, and
    /// `add_latent_handler` keep silent right-biased overwrite semantics for
    /// monoid-law compatibility, but every duplicate is recorded in the
    /// dispatcher and surfaced here. Use [`try_merge`] when you want the
    /// collision rejected at the merge call itself.
    ///
    /// Use [`new`] + manual [`validate_extensions`] /
    /// [`validate_no_collisions`] if you need to log warnings instead of
    /// failing.
    pub fn try_build(self) -> Result<Self, BuildError> {
        if !self.collisions.is_empty() {
            return Err(BuildError::Collisions(self.collisions));
        }
        if let Err(missing) = self.validate_extensions() {
            return Err(BuildError::MissingExtensions(missing));
        }
        Ok(self)
    }

    /// Register a handler (builder pattern). The handler's `definition().name` is used as the key.
    ///
    /// If a handler with the same name already exists, the new handler replaces
    /// it (right-biased) **and** a [`ToolCollision`](agent_fw_tool::ToolCollision)
    /// is recorded so `try_build` will reject the dispatcher.
    pub fn with_handler(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.add_handler(handler);
        self
    }

    /// Register a handler in-place. The handler's `definition().name` is used as the key.
    ///
    /// If a handler with the same name already exists, the new handler replaces
    /// it (right-biased) **and** a [`ToolCollision`](agent_fw_tool::ToolCollision)
    /// is recorded for surfacing by `try_build`. Prefer `with_handler` for
    /// builder-style chaining; use this when you have a `&mut` reference
    /// (e.g., from PyO3 bindings).
    pub fn add_handler(&mut self, handler: Arc<dyn ToolHandler>) {
        let name = handler.definition().name.clone();
        self.record_active_collision(&name);
        self.handlers.insert(name, handler);
    }

    /// Register a latent handler in-place.
    ///
    /// Latent handlers are executable but hidden from the default registry
    /// until request-scoped activation exposes them. Duplicates within the
    /// latent map, or against the active map, are recorded for `try_build`.
    pub fn add_latent_handler(&mut self, handler: Arc<dyn ToolHandler>) {
        let name = handler.definition().name.clone();
        self.record_latent_collision(&name);
        self.latent_handlers.insert(name, handler);
    }

    /// Register multiple handlers at once. Duplicate names are recorded as
    /// collisions and surfaced by `try_build`.
    pub fn with_handlers(
        mut self,
        handlers: impl IntoIterator<Item = Arc<dyn ToolHandler>>,
    ) -> Self {
        for handler in handlers {
            self.add_handler(handler);
        }
        self
    }

    /// Register a latent handler (builder pattern).
    pub fn with_latent_handler(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.add_latent_handler(handler);
        self
    }

    /// Merge another dispatcher's handlers into this one.
    ///
    /// On name collision, the other dispatcher's handler wins (right-biased),
    /// and every shadowed name is recorded as a
    /// [`ToolCollision`](agent_fw_tool::ToolCollision) so `try_build` rejects
    /// the dispatcher. The resulting dispatcher uses this dispatcher's
    /// environment.
    ///
    /// # Algebra (L6 — Dispatch Merge)
    ///
    /// `merge` forms a **monoid** over `ComposedDispatcher`:
    ///
    /// - **Associativity**: `(a.merge(b)).merge(c) ≡ a.merge(b.merge(c))`
    ///   — up to handler identity for the visible handler map.
    /// - **Identity**: merging with an empty dispatcher is a no-op.
    ///   `a.merge(empty) ≡ a` and `empty.merge(a) ≡ a`.
    /// - **Right-biased union**: on name collision, `other`'s handler wins.
    ///   This matches `HashMap::extend` semantics.
    ///
    /// Use [`try_merge`] when you want the collision rejected at the merge
    /// site rather than deferred to `try_build`.
    ///
    /// The monoid structure enables composable toolkit assembly:
    /// `generic_kit.merge(builder_kit).merge(custom_kit)`.
    pub fn merge(mut self, other: ComposedDispatcher) -> Self {
        for (name, handler) in other.handlers {
            self.record_active_collision(&name);
            self.handlers.insert(name, handler);
        }
        for (name, handler) in other.latent_handlers {
            self.record_latent_collision(&name);
            self.latent_handlers.insert(name, handler);
        }
        self.collisions.extend(other.collisions);
        self
    }

    /// Merge another dispatcher and fail if any tool names overlap.
    ///
    /// Strict counterpart to [`merge`]. Returns the merged dispatcher if the
    /// active and latent name sets are disjoint across the two inputs.
    /// Otherwise returns every duplicate as a
    /// [`ToolCollision`](agent_fw_tool::ToolCollision).
    ///
    /// Any collisions already recorded on either input are surfaced as well,
    /// so chaining `a.try_merge(b)?.try_merge(c)?` never hides an earlier
    /// duplicate.
    pub fn try_merge(
        self,
        other: ComposedDispatcher,
    ) -> Result<Self, Vec<agent_fw_tool::ToolCollision>> {
        let merged = self.merge(other);
        if merged.collisions.is_empty() {
            Ok(merged)
        } else {
            Err(merged.collisions)
        }
    }

    /// Record a collision when registering `name` into the active map.
    fn record_active_collision(&mut self, name: &str) {
        if self.handlers.contains_key(name) {
            self.collisions.push(agent_fw_tool::ToolCollision {
                tool_name: name.to_string(),
                kind: agent_fw_tool::CollisionKind::Active,
            });
        } else if self.latent_handlers.contains_key(name) {
            self.collisions.push(agent_fw_tool::ToolCollision {
                tool_name: name.to_string(),
                kind: agent_fw_tool::CollisionKind::ActiveVsLatent,
            });
        }
    }

    /// Record a collision when registering `name` into the latent map.
    fn record_latent_collision(&mut self, name: &str) {
        if self.latent_handlers.contains_key(name) {
            self.collisions.push(agent_fw_tool::ToolCollision {
                tool_name: name.to_string(),
                kind: agent_fw_tool::CollisionKind::Latent,
            });
        } else if self.handlers.contains_key(name) {
            self.collisions.push(agent_fw_tool::ToolCollision {
                tool_name: name.to_string(),
                kind: agent_fw_tool::CollisionKind::ActiveVsLatent,
            });
        }
    }

    /// Register a handler by value — no `Arc::new` needed.
    ///
    /// Reads as a sentence: `dispatcher.tool(BuildPlanHandler::new(..))`.
    /// The handler is wrapped in `Arc` internally.
    pub fn tool(self, handler: impl ToolHandler + 'static) -> Self {
        self.with_handler(Arc::new(handler))
    }

    /// Register a latent handler by value.
    pub fn latent_tool(self, handler: impl ToolHandler + 'static) -> Self {
        self.with_latent_handler(Arc::new(handler))
    }

    /// Wrap every registered handler with a [`ToolLayer`].
    ///
    /// Enables bulk cross-cutting: `dispatcher.layer(&TracedLayer)`
    /// instead of per-handler `traced(h)`.
    pub fn layer(mut self, layer: &dyn crate::layer::ToolLayer) -> Self {
        self.handlers = self
            .handlers
            .into_iter()
            .map(|(name, h)| (name, layer.wrap(h)))
            .collect();
        self
    }

    /// Wrap every registered handler with [`GuardedLayer`].
    ///
    /// Convenience for `.layer(&GuardedLayer)` — checks cancellation
    /// before every handler invocation. Place before `.traced()` so
    /// cancellation is checked before tracing begins.
    pub fn guarded(self) -> Self {
        self.layer(&crate::layer::GuardedLayer)
    }

    /// Wrap every registered handler with [`crate::layer::ApprovalLayer`].
    ///
    /// Pre-dispatch approval gate (pre-dispatch approval): handlers whose
    /// [`crate::approval::ApprovalPolicy`] entry resolves to `Always` or
    /// a `Dynamic` predicate that returns `true` pause before invoking
    /// the underlying logic — `approval_required` is emitted, and the
    /// inner handler does not run until `store.resolve(...)` is called
    /// with an `Approve` decision.
    ///
    /// # Composition order
    ///
    /// `.approval(...)` must be called **after** any `.merge(...)` —
    /// `ComposedDispatcher::merge` is right-biased (`tool_handler.rs:216-224`),
    /// so applying approval before merge would silently lose the gate
    /// when a downstream toolkit overrides a tool by name (H2 in the
    /// pre-dispatch approval design plan).
    ///
    /// Reads as a sentence:
    /// ```rust,ignore
    /// dispatcher
    ///     .tool(BuildPlanHandler::new(...))
    ///     .tool(ExecutePlanHandler)
    ///     .guarded()                       // cancellation first
    ///     .approval(policy, store)         // approval second
    ///     .traced()                        // tracing outermost
    /// ```
    pub fn approval(
        self,
        policy: std::sync::Arc<crate::approval::ApprovalPolicy>,
        store: std::sync::Arc<dyn agent_fw_algebra::approval::PendingApprovalStore>,
    ) -> Self {
        self.layer(&crate::layer::ApprovalLayer::new(policy, store))
    }

    /// Wrap every registered handler with [`TracedLayer`].
    ///
    /// Convenience for `.layer(&TracedLayer)` — reads as a sentence:
    /// ```rust,ignore
    /// generic.into_dispatcher(env)
    ///     .tool(BuildPlanHandler::new(ctx, schema, table))
    ///     .tool(ExecutePlanHandler)
    ///     .guarded()
    ///     .traced()
    /// ```
    pub fn traced(self) -> Self {
        self.layer(&crate::layer::TracedLayer)
    }

    /// Return every duplicate-name registration recorded during composition.
    ///
    /// Counterpart to [`validate_extensions`]. `try_build` calls this first —
    /// shadowing one tool with another is a more fundamental composition
    /// failure than a missing extension. Returns `Ok(())` if no collisions
    /// have been recorded, or `Err(collisions)` listing every culprit.
    pub fn validate_no_collisions(&self) -> Result<(), Vec<agent_fw_tool::ToolCollision>> {
        if self.collisions.is_empty() {
            Ok(())
        } else {
            Err(self.collisions.clone())
        }
    }

    /// Validate all handler extension manifests against the current environment.
    ///
    /// Call this at startup to surface missing extensions early — before any
    /// user interaction. Returns `Ok(())` if all required extensions are
    /// present, or `Err(missing)` with every unsatisfied requirement.
    pub fn validate_extensions(&self) -> Result<(), Vec<agent_fw_tool::MissingExtension>> {
        let mut all_missing = Vec::new();
        for handler in self.handlers.values() {
            let manifest = handler.extension_manifest();
            if let Err(missing) = manifest.validate(&handler.definition().name, &self.env) {
                all_missing.extend(missing);
            }
        }
        for handler in self.latent_handlers.values() {
            let manifest = handler.extension_manifest();
            if let Err(missing) = manifest.validate(&handler.definition().name, &self.env) {
                all_missing.extend(missing);
            }
        }
        if all_missing.is_empty() {
            Ok(())
        } else {
            Err(all_missing)
        }
    }

    /// Number of registered handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Whether the dispatcher has no handlers.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Check if a handler is registered for the given name.
    pub fn has_handler(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// Get a reference to the environment.
    pub fn env(&self) -> &ToolEnvironment {
        &self.env
    }

    /// Create a request-scoped clone with a different event sink.
    ///
    /// Shares the same handlers but routes events to a new sink.
    /// Use this to bind a per-request SSE channel so tool progress
    /// events flow to the correct client.
    pub fn with_sink(&self, sink: Arc<dyn agent_fw_algebra::EventSink>) -> Self {
        Self {
            handlers: self.handlers.clone(),
            latent_handlers: self.latent_handlers.clone(),
            env: self.env.with_sink(sink),
            collisions: self.collisions.clone(),
        }
    }
}

/// Why finalizing a [`ComposedDispatcher`] via `try_build` failed.
///
/// Collisions are reported in preference to missing extensions — shadowing
/// one tool with another is a more fundamental composition failure than a
/// declared extension being absent from the environment.
#[derive(Debug, Clone)]
pub enum BuildError {
    /// One or more tool names were registered more than once.
    Collisions(Vec<agent_fw_tool::ToolCollision>),
    /// One or more handlers declared an extension that the environment does
    /// not carry.
    MissingExtensions(Vec<agent_fw_tool::MissingExtension>),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Collisions(items) => {
                write!(f, "tool dispatcher has {} collision", items.len())?;
                if items.len() != 1 {
                    f.write_str("s")?;
                }
                for item in items {
                    write!(f, "\n  - {item}")?;
                }
                Ok(())
            }
            BuildError::MissingExtensions(items) => {
                write!(f, "tool dispatcher missing {} extension", items.len())?;
                if items.len() != 1 {
                    f.write_str("s")?;
                }
                for item in items {
                    write!(f, "\n  - {item}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BuildError {}

impl std::fmt::Display for ComposedDispatcher {
    /// Reads as: `"ComposedDispatcher [3 tools: draft_plan, approve_plan, query_data]"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.handlers.keys().map(String::as_str).collect();
        names.sort();
        write!(
            f,
            "ComposedDispatcher [{} tool{}: {}]",
            names.len(),
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        )
    }
}

#[async_trait]
impl ToolDispatcher for ComposedDispatcher {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.handlers.values().map(|h| h.definition()).collect()
    }

    fn latent_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.latent_handlers
            .values()
            .map(|h| h.definition())
            .collect()
    }

    async fn dispatch(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: serde_json::Value,
    ) -> ToolCallResult {
        match self.handlers.get(tool_name) {
            Some(handler) => {
                let scoped_env = self.env.with_tool_call_id(tool_use_id);
                handler.handle(tool_use_id, input, &scoped_env).await
            }
            None => match self.latent_handlers.get(tool_name) {
                Some(handler) => {
                    let scoped_env = self.env.with_tool_call_id(tool_use_id);
                    handler.handle(tool_use_id, input, &scoped_env).await
                }
                None => ToolCallResult::error(tool_use_id, format!("Unknown tool: {tool_name}")),
            },
        }
    }

    fn current_tool_call_id(&self) -> Option<String> {
        self.env.current_tool_call_id()
    }

    fn tool_call_id_cell(&self) -> Option<std::sync::Arc<std::sync::Mutex<Option<String>>>> {
        Some(self.env.tool_call_id_cell())
    }

    fn pending_card_cell(
        &self,
    ) -> Option<std::sync::Arc<std::sync::Mutex<Option<agent_fw_tool::CommandCardPayload>>>> {
        Some(self.env.pending_card_cell())
    }

    fn with_event_sink(
        self: Arc<Self>,
        sink: Arc<dyn agent_fw_algebra::EventSink>,
    ) -> Option<Arc<dyn ToolDispatcher>> {
        Some(Arc::new(Self {
            handlers: self.handlers.clone(),
            latent_handlers: self.latent_handlers.clone(),
            env: self.env.with_sink(sink),
            collisions: self.collisions.clone(),
        }))
    }
}

// ─── ToolHandler for Arc<dyn ToolHandler> ─────────────────────────────

#[async_trait]
impl ToolHandler for Arc<dyn ToolHandler> {
    fn definition(&self) -> ToolDefinition {
        (**self).definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        (**self).handle(tool_use_id, input, env).await
    }

    /// Forward `extension_manifest` to the inner handler.
    ///
    /// Without this, the default (empty) manifest hides every inner handler's
    /// extension requirements — silently breaking the Seam 4 guarantee
    /// enforced by `ComposedDispatcher::validate_extensions` (which always
    /// stores handlers as `Arc<dyn ToolHandler>`).
    fn extension_manifest(&self) -> agent_fw_tool::ToolExtensionManifest {
        (**self).extension_manifest()
    }
}

// ─── TracedHandler ────────────────────────────────────────────────────

/// A ToolHandler combinator that emits observability events via EventSink.
///
/// Wraps any `ToolHandler` and emits tool_call / tool_result stream parts
/// for every invocation. The inner handler's behavior is unchanged — this
/// combinator only adds effects, it never modifies the result.
///
/// # Laws
///
/// - **L1 (Transparency)**: `TracedHandler<H>.definition() == H.definition()`
/// - **L2 (Semantic preservation)**: `TracedHandler<H>.handle(id, input, env)`
///   returns exactly the same `ToolCallResult` as `H.handle(id, input, env)`
/// - **L3 (Event ordering)**: tool_call event precedes tool_result event
///
/// # Usage
///
/// ```rust,ignore
/// let handler = traced(my_handler);
/// dispatcher.with_handler(Arc::new(handler));
/// ```
pub struct TracedHandler<H> {
    inner: H,
}

impl<H> TracedHandler<H> {
    /// Wrap a handler with tracing.
    pub fn new(inner: H) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<H: ToolHandler> ToolHandler for TracedHandler<H> {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        use agent_fw_algebra::event_sink::EventSinkExt;

        let name = self.inner.definition().name.clone();
        // Clone strategy: 2 clones total — minimum for the owned-Value EventSink API.
        //   1. `input_args` = clone of `input` (used for both emit calls)
        //   2. `input_args.clone()` passed to emit_tool_call (consumed)
        // `input` itself is moved into `handle()`. Reducing further requires
        // changing EventSink to accept `&Value`, which is a trait-level change.
        let input_args = input.clone();

        // L3: tool_call event precedes execution
        env.event_sink()
            .emit_tool_call(tool_use_id, &name, input_args.clone());

        // L2: Delegate — result is identical to unwrapped handler
        let result = self.inner.handle(tool_use_id, input, env).await;

        // Emit UI channel events BEFORE tool_result (so frontend renders
        // the approval card before the tool result collapses).
        // These never reach the LLM context — they are frontend-only channels.
        if let Some(ref dsl) = result.approval_dsl {
            env.event_sink().emit_data_flow_ui(dsl);
        }
        if let Some(ref summary) = result.display_summary {
            env.event_sink().emit_text(summary);
        }

        // L3: tool_result event follows execution
        env.event_sink()
            .emit_tool_result(tool_use_id, &name, input_args, result.content.clone());

        result
    }
}

/// Wrap a handler in tracing.
///
/// Returns `TracedHandler<H>`, preserving the parametric type. The caller
/// decides when to erase via `Arc::new(traced(h))`.
pub fn traced<H: ToolHandler>(handler: H) -> TracedHandler<H> {
    TracedHandler::new(handler)
}

// ─── FnToolHandler ────────────────────────────────────────────────────

/// A [`ToolHandler`] built from a closure, eliminating struct+impl boilerplate.
///
/// The input type `I` must implement [`serde::de::DeserializeOwned`] for
/// automatic input parsing, and [`agent_fw_tool::ToolSchema`] for automatic
/// JSON schema derivation. This ensures the schema IS the type — one source
/// of truth, not two (make illegal states unrepresentable).
///
/// # Laws
///
/// - **L1 (Schema-Dispatch Consistency)**: Schema is derived from `I`, and
///   `handle()` deserializes into `I` — they agree by construction.
/// - **L2 (Deterministic Schema)**: `ToolSchema::json_schema()` is pure.
/// - **L3 (Error Totality)**: Deserialization errors and handler errors both
///   produce `ToolCallResult::error`. Never panics.
///
/// # Usage
///
/// ```rust,ignore
/// use agent_fw_agent::fn_handler;
///
/// let handler = fn_handler::<MyInput>(
///     "draft_plan",
///     "Create a pricing plan",
///     |env, input| Box::pin(async move {
///         let plan = build_plan(&input, env.kv().as_ref()).await?;
///         Ok(serde_json::to_value(plan).unwrap())
///     }),
/// );
///
/// let dispatcher = ComposedDispatcher::new(env)
///     .with_handler(Arc::new(handler));
/// ```
pub struct FnToolHandler<F, I> {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    handler: F,
    _phantom: std::marker::PhantomData<fn(I) -> I>,
}

/// Create a [`ToolHandler`] from a closure.
///
/// Schema is derived from the input type `I` via [`ToolSchema`]. The closure
/// receives `(&ToolEnvironment, I)` and returns a boxed future resolving to
/// `Result<serde_json::Value, ToolError>`.
///
/// # Example
///
/// ```rust,ignore
/// let handler = fn_handler::<MyInput>(
///     "myTool",
///     "My tool description",
///     |env, input| Box::pin(async move {
///         Ok(serde_json::json!({"status": "ok"}))
///     }),
/// );
/// ```
pub fn fn_handler<I, F>(
    name: impl Into<String>,
    description: impl Into<String>,
    handler: F,
) -> FnToolHandler<F, I>
where
    I: serde::de::DeserializeOwned + agent_fw_tool::ToolSchema + Send + 'static,
    F: for<'a> Fn(
            &'a ToolEnvironment,
            I,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<serde_json::Value, agent_fw_tool::ToolError>,
                    > + Send
                    + 'a,
            >,
        > + Send
        + Sync
        + 'static,
{
    FnToolHandler {
        name: name.into(),
        description: description.into(),
        input_schema: I::json_schema(),
        handler,
        _phantom: std::marker::PhantomData,
    }
}

#[async_trait]
impl<F, I> ToolHandler for FnToolHandler<F, I>
where
    I: serde::de::DeserializeOwned + agent_fw_tool::ToolSchema + Send + 'static,
    F: for<'a> Fn(
            &'a ToolEnvironment,
            I,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<serde_json::Value, agent_fw_tool::ToolError>,
                    > + Send
                    + 'a,
            >,
        > + Send
        + Sync
        + 'static,
{
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        // Deserialize input — errors are returned as ToolCallResult::error (L3)
        let parsed: I = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(e) => {
                return ToolCallResult::error(tool_use_id, format!("Invalid input: {e}"));
            }
        };

        // Invoke the closure
        match (self.handler)(env, parsed).await {
            Ok(mut content) => {
                // Extract typed UI channels from the returned value (same as #[tool_handler] macro).
                let mut approval_dsl = None;
                let mut display_summary = None;
                if let Some(obj) = content.as_object_mut() {
                    if let Some(v) = obj.remove("approvalDsl") {
                        approval_dsl = v
                            .as_str()
                            .map(String::from)
                            .or_else(|| serde_json::to_string(&v).ok());
                    }
                    if let Some(v) = obj.remove("displaySummary") {
                        display_summary = v.as_str().map(String::from);
                    }
                    obj.remove("_cardEmitted");
                }
                let mut result = ToolCallResult::success(tool_use_id, content);
                result.approval_dsl = approval_dsl;
                result.display_summary = display_summary;
                result
            }
            Err(e) => ToolCallResult::error(tool_use_id, e.to_string()),
        }
    }
}

// ─── tool! macro ──────────────────────────────────────────────────────

/// Create a [`ToolHandler`] from a typed closure — zero ceremony.
///
/// Eliminates the `Box::pin(async move { ... })` boilerplate that `fn_handler`
/// requires. The input type `I` must implement `Deserialize + ToolSchema`.
///
/// # Sync variant
///
/// ```rust,ignore
/// let handler = tool!("greet", "Greet someone", |_env, input: GreetInput| {
///     Ok(serde_json::json!({"greeting": format!("Hello, {}!", input.name)}))
/// });
/// ```
///
/// # Async variant
///
/// ```rust,ignore
/// let handler = tool!("draft_plan", "Create a plan", |env, input: BuildPlanSchema| async {
///     let db = env.try_ext::<dyn TargetDatabase>()?;
///     let plan = build_plan(db.as_ref(), &input).await?;
///     Ok(serde_json::to_value(plan).unwrap())
/// });
/// ```
///
/// # Composition
///
/// ```rust,ignore
/// let dispatcher = ComposedDispatcher::new(env)
///     .tool(tool!("greet", "Greet", |_e, i: GreetInput| {
///         Ok(serde_json::json!({"msg": format!("Hi {}", i.name)}))
///     }))
///     .tool(tool!("count", "Count", |_e, i: CountInput| {
///         Ok(serde_json::json!({"n": i.items.len()}))
///     }))
///     .traced();
/// ```
#[macro_export]
macro_rules! tool {
    // Async: tool!("name", "desc", |env, input: Type| async { body })
    ($name:expr, $desc:expr, |$env:ident, $input:ident : $input_ty:ty| async $body:block) => {
        $crate::fn_handler::<$input_ty, _>(
            $name,
            $desc,
            |$env, $input| ::std::boxed::Box::pin(async move $body),
        )
    };
    // Sync: tool!("name", "desc", |env, input: Type| { body })
    ($name:expr, $desc:expr, |$env:ident, $input:ident : $input_ty:ty| $body:block) => {
        $crate::fn_handler::<$input_ty, _>(
            $name,
            $desc,
            |$env, $input| ::std::boxed::Box::pin(async move $body),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::event_sink::EventSink;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_algebra::{CancellationToken, KVStore, SubAgentInvoker};
    use agent_fw_core::tenant::TenantContext;

    fn test_env() -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test"));
        ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel)
    }

    // ── Test handler: echoes input as output ────────────────────────────

    struct EchoHandler {
        name: String,
    }

    impl EchoHandler {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.clone(),
                description: format!("Echo handler: {}", self.name),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    }
                }),
            }
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            ToolCallResult::success(
                tool_use_id,
                serde_json::json!({
                    "handler": self.name,
                    "echo": input,
                }),
            )
        }
    }

    // ── ToolHandler unit tests ──────────────────────────────────────────

    #[test]
    fn deterministic_schema() {
        let handler = EchoHandler::new("test_tool");
        let d1 = handler.definition();
        let d2 = handler.definition();
        assert_eq!(d1.name, d2.name);
        assert_eq!(d1.description, d2.description);
        assert_eq!(d1.input_schema, d2.input_schema);
    }

    #[tokio::test]
    async fn handle_returns_result_not_panic() {
        let handler = EchoHandler::new("echo");
        let env = test_env();
        let result = handler
            .handle("id-1", serde_json::json!({"message": "hi"}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.tool_use_id, "id-1");
        assert_eq!(result.content["handler"], "echo");
    }

    // ── ComposedDispatcher unit tests ───────────────────────────────────

    #[test]
    fn empty_dispatcher() {
        let d = ComposedDispatcher::new(test_env());
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
        assert!(d.tool_definitions().is_empty());
    }

    #[test]
    fn with_handler_registers() {
        let d =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("alpha")));
        assert_eq!(d.len(), 1);
        assert!(d.has_handler("alpha"));
        assert!(!d.has_handler("beta"));
    }

    #[test]
    fn with_handlers_registers_multiple() {
        let d = ComposedDispatcher::new(test_env()).with_handlers(vec![
            Arc::new(EchoHandler::new("a")) as Arc<dyn ToolHandler>,
            Arc::new(EchoHandler::new("b")),
        ]);
        assert_eq!(d.len(), 2);
        assert!(d.has_handler("a"));
        assert!(d.has_handler("b"));
    }

    #[test]
    fn definition_completeness() {
        let d = ComposedDispatcher::new(test_env()).with_handlers(vec![
            Arc::new(EchoHandler::new("x")) as Arc<dyn ToolHandler>,
            Arc::new(EchoHandler::new("y")),
        ]);
        let defs = d.tool_definitions();
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
    }

    #[tokio::test]
    async fn dispatch_routes_to_correct_handler() {
        let d = ComposedDispatcher::new(test_env()).with_handlers(vec![
            Arc::new(EchoHandler::new("alpha")) as Arc<dyn ToolHandler>,
            Arc::new(EchoHandler::new("beta")),
        ]);

        let result = d
            .dispatch("alpha", "id-1", serde_json::json!({"msg": "hello"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["handler"], "alpha");

        let result = d
            .dispatch("beta", "id-2", serde_json::json!({"msg": "world"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["handler"], "beta");
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let d =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("known")));

        let result = d.dispatch("unknown", "id-3", serde_json::json!({})).await;
        assert!(result.is_error);
        assert_eq!(result.tool_use_id, "id-3");
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("Unknown tool"));
    }

    #[test]
    fn merge_combines_handlers() {
        let a =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("tool_a")));
        let b =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("tool_b")));

        let merged = a.merge(b);
        assert_eq!(merged.len(), 2);
        assert!(merged.has_handler("tool_a"));
        assert!(merged.has_handler("tool_b"));
    }

    /// Right-biased merge is the algebraic monoid op — `other`'s handler wins
    /// on collision. The collision is *recorded* (so `try_build` rejects the
    /// dispatcher), but the handler map itself still satisfies the documented
    /// monoid laws. Use [`ComposedDispatcher::try_merge`] when collisions
    /// should fail at the merge site instead.
    #[test]
    fn merge_is_right_biased_monoid() {
        let a =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("shared")));
        let b =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("shared")));

        let merged = a.merge(b);
        assert_eq!(merged.len(), 1);
        // Collision recorded for later surfacing via `try_build` / `validate_no_collisions`.
        assert!(merged.validate_no_collisions().is_err());
    }

    #[test]
    fn try_merge_rejects_collision() {
        let a =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("shared")));
        let b = ComposedDispatcher::new(test_env())
            .with_handler(Arc::new(EchoHandler::new("shared")))
            .with_handler(Arc::new(EchoHandler::new("other")));

        let err = a.try_merge(b).err().expect("collision expected");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].tool_name, "shared");
        assert!(matches!(err[0].kind, agent_fw_tool::CollisionKind::Active));
    }

    #[test]
    fn try_merge_ok_when_disjoint() {
        let a =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("alpha")));
        let b =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("beta")));

        let merged = a.try_merge(b).expect("disjoint merge should succeed");
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn try_build_rejects_collision_from_with_handler() {
        let d = ComposedDispatcher::new(test_env())
            .with_handler(Arc::new(EchoHandler::new("dupe")))
            .with_handler(Arc::new(EchoHandler::new("dupe")));

        let err = d.try_build().err().expect("collision expected");
        match err {
            BuildError::Collisions(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].tool_name, "dupe");
            }
            BuildError::MissingExtensions(_) => {
                panic!("expected Collisions, got MissingExtensions")
            }
        }
    }

    #[test]
    fn try_build_rejects_collision_from_merge() {
        let a =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("shared")));
        let b =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("shared")));

        let err = a
            .merge(b)
            .try_build()
            .err()
            .expect("try_build should reject silent merge collision");
        assert!(matches!(err, BuildError::Collisions(_)));
    }

    #[test]
    fn try_build_detects_active_vs_latent_collision() {
        let mut d = ComposedDispatcher::new(test_env());
        d.add_handler(Arc::new(EchoHandler::new("same")));
        d.add_latent_handler(Arc::new(EchoHandler::new("same")));

        let err = d.try_build().err().expect("collision expected");
        match err {
            BuildError::Collisions(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].tool_name, "same");
                assert!(matches!(
                    items[0].kind,
                    agent_fw_tool::CollisionKind::ActiveVsLatent
                ));
            }
            BuildError::MissingExtensions(_) => panic!("expected Collisions"),
        }
    }

    #[test]
    fn try_build_passes_for_clean_dispatcher() {
        let d = ComposedDispatcher::new(test_env())
            .with_handler(Arc::new(EchoHandler::new("alpha")))
            .with_handler(Arc::new(EchoHandler::new("beta")));

        let built = d.try_build().expect("clean dispatcher should build");
        assert_eq!(built.len(), 2);
    }

    #[test]
    fn merge_with_empty_is_identity() {
        let d =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("tool")));
        let empty = ComposedDispatcher::new(test_env());

        // d.merge(empty) should preserve d's handlers
        let merged = d.merge(empty);
        assert_eq!(merged.len(), 1);
        assert!(merged.has_handler("tool"));
    }

    #[test]
    fn empty_merge_with_d_is_identity() {
        let empty = ComposedDispatcher::new(test_env());
        let d =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("tool")));

        // empty.merge(d) should get d's handlers
        let merged = empty.merge(d);
        assert_eq!(merged.len(), 1);
        assert!(merged.has_handler("tool"));
    }

    // ── Merge Associativity (L6) ────────────────────────────────────────

    /// Verify the documented monoid associativity law:
    /// `(a.merge(b)).merge(c) ≡ a.merge(b.merge(c))`
    ///
    /// Observable equivalence: same set of handler names and, for each
    /// name, the same winning handler (by definition name, since
    /// `ToolHandler` is not `Eq`).
    #[test]
    fn merge_associativity() {
        // Helper to create the three dispatchers with overlapping "shared" name.
        let make_a = || {
            ComposedDispatcher::new(test_env())
                .with_handler(Arc::new(EchoHandler::new("tool_a")))
                .with_handler(Arc::new(EchoHandler::new("shared")))
        };
        let make_b = || {
            ComposedDispatcher::new(test_env())
                .with_handler(Arc::new(EchoHandler::new("tool_b")))
                .with_handler(Arc::new(EchoHandler::new("shared")))
        };
        let make_c = || {
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("tool_c")))
        };

        // (a.merge(b)).merge(c)
        let ab_c = make_a().merge(make_b()).merge(make_c());

        // a.merge(b.merge(c))
        let a_bc = make_a().merge(make_b().merge(make_c()));

        // Same tool names (observational equivalence via Display, which sorts names)
        assert_eq!(ab_c.to_string(), a_bc.to_string());
        assert_eq!(ab_c.len(), a_bc.len());

        // Both should contain all 4 unique names
        for name in &["tool_a", "tool_b", "tool_c", "shared"] {
            assert!(ab_c.has_handler(name), "ab_c missing {name}");
            assert!(a_bc.has_handler(name), "a_bc missing {name}");
        }
    }

    // ── Display tests ─────────────────────────────────────────────────

    #[test]
    fn composed_dispatcher_display_empty() {
        let d = ComposedDispatcher::new(test_env());
        assert_eq!(d.to_string(), "ComposedDispatcher [0 tools: ]");
    }

    #[test]
    fn composed_dispatcher_display_sorted() {
        let d = ComposedDispatcher::new(test_env())
            .with_handler(Arc::new(EchoHandler::new("beta")))
            .with_handler(Arc::new(EchoHandler::new("alpha")));
        assert_eq!(d.to_string(), "ComposedDispatcher [2 tools: alpha, beta]");
    }

    #[test]
    fn composed_dispatcher_display_singular() {
        let d =
            ComposedDispatcher::new(test_env()).with_handler(Arc::new(EchoHandler::new("only")));
        assert_eq!(d.to_string(), "ComposedDispatcher [1 tool: only]");
    }

    // ── FnToolHandler tests ────────────────────────────────────────────

    #[derive(serde::Deserialize, agent_fw_tool::DeriveToolSchema)]
    struct GreetInput {
        name: String,
    }

    #[test]
    fn fn_handler_deterministic_schema() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet someone", |_env, input| {
            Box::pin(async move {
                Ok(serde_json::json!({"greeting": format!("Hello, {}!", input.name)}))
            })
        });
        let d1 = handler.definition();
        let d2 = handler.definition();
        assert_eq!(d1.name, "greet");
        assert_eq!(d1.name, d2.name);
        assert_eq!(d1.input_schema, d2.input_schema);
    }

    #[test]
    fn fn_handler_schema_from_type() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet", |_env, _input| {
            Box::pin(async move { Ok(serde_json::json!({})) })
        });
        let def = handler.definition();
        // Schema should include "name" property derived from GreetInput
        let props = def.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("name"));
    }

    #[tokio::test]
    async fn fn_handler_success() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet someone", |_env, input| {
            Box::pin(async move {
                Ok(serde_json::json!({"greeting": format!("Hello, {}!", input.name)}))
            })
        });
        let env = test_env();
        let result = handler
            .handle("id-1", serde_json::json!({"name": "World"}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["greeting"], "Hello, World!");
    }

    #[tokio::test]
    async fn fn_handler_invalid_input_returns_error() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet", |_env, _input| {
            Box::pin(async move { Ok(serde_json::json!({})) })
        });
        let env = test_env();
        // Missing required "name" field
        let result = handler.handle("id-2", serde_json::json!({}), &env).await;
        assert!(result.is_error);
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("Invalid input"));
    }

    #[tokio::test]
    async fn fn_handler_error_propagation() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet", |_env, _input| {
            Box::pin(async move { Err(agent_fw_tool::ToolError::cancelled()) })
        });
        let env = test_env();
        let result = handler
            .handle("id-3", serde_json::json!({"name": "X"}), &env)
            .await;
        assert!(result.is_error);
    }

    // ── tool! macro tests ──────────────────────────────────────────────

    #[test]
    fn tool_macro_sync_schema() {
        let handler = tool!("greet", "Greet someone", |_env, input: GreetInput| {
            Ok(serde_json::json!({"greeting": format!("Hello, {}!", input.name)}))
        });
        let def = handler.definition();
        assert_eq!(def.name, "greet");
        assert_eq!(def.description, "Greet someone");
        // Schema derived from GreetInput
        let props = def.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("name"));
    }

    #[tokio::test]
    async fn tool_macro_sync_execution() {
        let handler = tool!("greet", "Greet", |_env, input: GreetInput| {
            Ok(serde_json::json!({"msg": format!("Hi {}", input.name)}))
        });
        let env = test_env();
        let result = handler
            .handle("id-m1", serde_json::json!({"name": "World"}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["msg"], "Hi World");
    }

    #[tokio::test]
    async fn tool_macro_async_execution() {
        let handler = tool!("greet", "Greet", |_env, input: GreetInput| async {
            // simulate async work
            let greeting = format!("Hello, {}!", input.name);
            Ok(serde_json::json!({"greeting": greeting}))
        });
        let env = test_env();
        let result = handler
            .handle("id-m2", serde_json::json!({"name": "Async"}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["greeting"], "Hello, Async!");
    }

    #[tokio::test]
    async fn tool_macro_composes_with_dispatcher() {
        let env = test_env();
        let dispatcher = ComposedDispatcher::new(env).tool(tool!(
            "greet",
            "Greet",
            |_env, input: GreetInput| {
                Ok(serde_json::json!({"msg": format!("Hi {}", input.name)}))
            }
        ));

        assert_eq!(dispatcher.len(), 1);
        assert!(dispatcher.has_handler("greet"));

        let result = dispatcher
            .dispatch("greet", "id-m3", serde_json::json!({"name": "Macro"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["msg"], "Hi Macro");
    }

    #[tokio::test]
    async fn tool_macro_error_on_invalid_input() {
        let handler = tool!("greet", "Greet", |_env, _input: GreetInput| {
            Ok(serde_json::json!({}))
        });
        let env = test_env();
        let result = handler.handle("id-m4", serde_json::json!({}), &env).await;
        assert!(result.is_error);
    }

    // ── fn_handler composes with dispatcher ───────────────────────────

    #[tokio::test]
    async fn fn_handler_composes_with_dispatcher() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet", |_env, input| {
            Box::pin(async move { Ok(serde_json::json!({"msg": format!("Hi {}", input.name)})) })
        });
        let env = test_env();
        let dispatcher = ComposedDispatcher::new(env).with_handler(Arc::new(handler));

        assert_eq!(dispatcher.len(), 1);
        assert!(dispatcher.has_handler("greet"));

        let result = dispatcher
            .dispatch("greet", "id-4", serde_json::json!({"name": "Alice"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["msg"], "Hi Alice");
    }

    // ── UI channel extraction tests ──────────────────────────────────

    #[tokio::test]
    async fn fn_handler_extracts_approval_dsl() {
        let handler = fn_handler::<GreetInput, _>("plan", "Build plan", |_env, _input| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "planId": "p-1",
                    "approvalDsl": "{\"card\":true}",
                    "displaySummary": "Created plan p-1",
                }))
            })
        });
        let env = test_env();
        let result = handler
            .handle("id-ui", serde_json::json!({"name": "X"}), &env)
            .await;
        assert!(!result.is_error);
        // UI channels extracted into typed fields
        assert_eq!(result.approval_dsl.as_deref(), Some("{\"card\":true}"));
        assert_eq!(result.display_summary.as_deref(), Some("Created plan p-1"));
        // Cleaned from content — LLM never sees them
        assert!(result.content.get("approvalDsl").is_none());
        assert!(result.content.get("displaySummary").is_none());
        // Domain payload preserved
        assert_eq!(result.content["planId"], "p-1");
    }

    #[tokio::test]
    async fn fn_handler_no_ui_channels_when_absent() {
        let handler = fn_handler::<GreetInput, _>("greet", "Greet", |_env, input| {
            Box::pin(async move { Ok(serde_json::json!({"msg": format!("Hi {}", input.name)})) })
        });
        let env = test_env();
        let result = handler
            .handle("id-no-ui", serde_json::json!({"name": "Y"}), &env)
            .await;
        assert!(!result.is_error);
        assert!(result.approval_dsl.is_none());
        assert!(result.display_summary.is_none());
        assert_eq!(result.content["msg"], "Hi Y");
    }

    #[tokio::test]
    async fn tool_macro_extracts_ui_channels() {
        let handler = tool!("plan", "Build", |_env, _input: GreetInput| {
            Ok(serde_json::json!({
                "planId": "p-2",
                "approvalDsl": "dsl-content",
                "_cardEmitted": true,
            }))
        });
        let env = test_env();
        let result = handler
            .handle("id-macro-ui", serde_json::json!({"name": "Z"}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.approval_dsl.as_deref(), Some("dsl-content"));
        // _cardEmitted also stripped
        assert!(result.content.get("_cardEmitted").is_none());
        assert_eq!(result.content["planId"], "p-2");
    }
}
