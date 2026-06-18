//! Composable middleware layer for ToolHandler.
//!
//! # Design (cross-cutting via combinators, not annotations)
//!
//! [`ToolLayer`] wraps a [`ToolHandler`] to add cross-cutting concerns
//! (observability, caching, validation) without modifying inner behavior.
//! Layers compose via [`ToolLayer::then`] — stack as many as you like.
//!
//! [`TracedLayer`] is the concrete proof: it wraps any handler to emit
//! tool_call / tool_result events via the environment's EventSink.
//!
//! # Algebraic Laws
//!
//! ## ToolLayer Laws
//!
//! - **L1 (Transparency)**: `layer.wrap(h).definition() == h.definition()`
//!   The layer preserves the handler's schema identity.
//!
//! - **L2 (Composition)**: `a.then(b).wrap(h) == b.wrap(a.wrap(h))`
//!   Stacking layers is associative with predictable ordering.
//!
//! ## TracedLayer Laws (inherited from TracedHandler)
//!
//! - **L3 (Semantic preservation)**: The wrapped handler returns the same
//!   `ToolCallResult` as the unwrapped handler.
//!
//! - **L4 (Event ordering)**: tool_call event precedes tool_result event.

use std::sync::Arc;

use async_trait::async_trait;

use agent_fw_algebra::approval::{ExpireReason, PendingApprovalStore};
use agent_fw_algebra::event_sink::EventSinkExt;
use agent_fw_core::approval::{ApprovalKind, ApprovalRequest};
use agent_fw_core::{ApprovalId, ThreadId};

use crate::approval::{ApprovalContext, ApprovalPolicy};
use crate::{ToolCallResult, ToolHandler, TracedHandler};

// ─── ToolLayer ─────────────────────────────────────────────────────

/// Middleware layer for ToolHandler.
///
/// A layer wraps a handler to add cross-cutting concerns (observability,
/// caching, etc.) without modifying the handler's core behavior.
///
/// # Laws
///
/// - **L1 (Transparency)**: `layer.wrap(h).definition() == h.definition()`
/// - **L2 (Composition)**: `a.then(b).wrap(h) == b.wrap(a.wrap(h))`
pub trait ToolLayer: Send + Sync {
    /// Wrap a handler, returning a new handler with added behavior.
    fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler>;

    /// Stack two layers: applies `self` first, then `outer`.
    ///
    /// ```rust,ignore
    /// let both = tracing.then(metrics);
    /// // Equivalent to: metrics.wrap(tracing.wrap(handler))
    /// ```
    fn then<L: ToolLayer + 'static>(self, outer: L) -> ComposedLayer<Self, L>
    where
        Self: Sized + 'static,
    {
        ComposedLayer { inner: self, outer }
    }
}

// ─── ComposedLayer ─────────────────────────────────────────────────

/// Two layers stacked: applies `inner` first, then `outer`.
pub struct ComposedLayer<A, B> {
    inner: A,
    outer: B,
}

impl<A: ToolLayer, B: ToolLayer> ToolLayer for ComposedLayer<A, B> {
    fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler> {
        let wrapped_inner = self.inner.wrap(handler);
        self.outer.wrap(wrapped_inner)
    }
}

// ─── GuardedLayer ─────────────────────────────────────────────────

/// A layer that checks cancellation before delegating to the inner handler.
///
/// Pre-checks `env.is_cancelled()` before calling the inner handler.
/// If cancelled, returns `ToolCallResult::error` without invoking the handler.
/// Mid-handler cancellation checks (between phases) remain domain-specific.
///
/// # Laws
///
/// - **G1 (Pre-guard short-circuit)**: When `env.is_cancelled()` is true,
///   the inner handler is never called.
/// - **G2 (Transparency when active)**: When not cancelled,
///   `GuardedHandler<H>.handle(..) == H.handle(..)`.
/// - **G3 (Composes with TracedLayer)**: `.guarded().traced()` ordering
///   ensures cancellation is checked before tracing begins.
pub struct GuardedLayer;

impl ToolLayer for GuardedLayer {
    fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler> {
        Arc::new(GuardedHandler { inner: handler })
    }
}

struct GuardedHandler {
    inner: Arc<dyn ToolHandler>,
}

#[async_trait]
impl ToolHandler for GuardedHandler {
    fn definition(&self) -> crate::ToolDefinition {
        self.inner.definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &agent_fw_tool::ToolEnvironment,
    ) -> crate::ToolCallResult {
        if env.cancel().is_cancelled() {
            return crate::ToolCallResult::error(tool_use_id, "Cancelled");
        }
        self.inner.handle(tool_use_id, input, env).await
    }
}

// ─── ApprovalLayer ────────────────────────────────────────────────

/// Pre-dispatch approval gate (pre-dispatch approval).
///
/// Wraps every handler with an [`ApprovalHandler`] that:
///   1. Resolves the tool's [`crate::approval::ApprovalRule`] from the
///      injected [`ApprovalPolicy`].
///   2. If the rule says approval is required, allocates a fresh
///      [`ApprovalId`], registers an [`ApprovalRequest`] with the store,
///      emits `approval_required` on the event sink, and **awaits**
///      a host decision via the store's awaiter.
///   3. On `Approve`, emits `approval_decision` and delegates to the
///      inner handler. On `Reject` (or `Revise`, which alpha collapses
///      to `Reject`), emits `approval_decision` and returns a
///      `ToolCallResult` whose content carries `{ rejected: true,
///      reason, do_not_retry: true }` — the inner handler is NEVER
///      invoked (pre-dispatch approval non-negotiable invariant).
///
/// # Hazard mitigations (from the design plan)
///
/// - **H1 (Awaiter leak on cancel)**: races the awaiter against
///   `env.cancel().cancelled()` via `tokio::select!`. On cancel,
///   calls `store.expire(id, ExpireReason::Cancelled)` to release the
///   store-side entry — no DashMap leak.
/// - **H2 (Right-biased merge wraps the wrong handler)**: this layer
///   must be applied **post-merge** — i.e., on a dispatcher whose
///   handler set is final. The `.approval(...)` convenience on
///   `ComposedDispatcher` is the safe entry point.
/// - **H4 (Registration race)**: the store entry is created via
///   `register` **before** `emit_approval_required` is called, so a
///   fast `respond_to_approval` always finds the awaiter.
/// - **H8 (Anti-retry-loop)**: the `Reject` payload includes
///   `do_not_retry: true` so the LLM does not retry the same call.
///
/// # Composition order
///
/// `dispatcher.guarded().approval(...).traced()` — cancellation first
/// (no point asking for approval on a cancelled call), approval second,
/// tracing outermost so the SSE stream sees `tool_call →
/// approval_required → approval_decision → tool_result`.
pub struct ApprovalLayer {
    policy: Arc<ApprovalPolicy>,
    store: Arc<dyn PendingApprovalStore>,
}

impl ApprovalLayer {
    /// Create a new approval layer with the given policy registry and
    /// pending-approval store interpreter.
    pub fn new(policy: Arc<ApprovalPolicy>, store: Arc<dyn PendingApprovalStore>) -> Self {
        Self { policy, store }
    }
}

impl ToolLayer for ApprovalLayer {
    fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler> {
        Arc::new(ApprovalHandler {
            inner: handler,
            policy: self.policy.clone(),
            store: self.store.clone(),
        })
    }
}

struct ApprovalHandler {
    inner: Arc<dyn ToolHandler>,
    policy: Arc<ApprovalPolicy>,
    store: Arc<dyn PendingApprovalStore>,
}

#[async_trait]
impl ToolHandler for ApprovalHandler {
    fn definition(&self) -> crate::ToolDefinition {
        self.inner.definition()
    }

    fn extension_manifest(&self) -> agent_fw_tool::ToolExtensionManifest {
        self.inner.extension_manifest()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &agent_fw_tool::ToolEnvironment,
    ) -> ToolCallResult {
        let name = self.inner.definition().name;
        let tenant_id = env.resource_id().clone();
        let thread_id = env
            .tenant()
            .thread_id()
            .cloned()
            .unwrap_or_else(|| ThreadId::new_unchecked("default"));

        // ─── Resolve rule ─────────────────────────────────────────────
        let rule = self.policy.resolve_tool(&name);
        let required = {
            let ctx = ApprovalContext {
                kind: ApprovalKind::Tool,
                target: &name,
                input: &input,
                tenant: &tenant_id,
            };
            rule.is_required(&ctx)
        };

        if !required {
            return self.inner.handle(tool_use_id, input, env).await;
        }

        // ─── Approval required — build request ────────────────────────
        //
        // The ApprovalId is a fresh UUID per attempt. If the host rejects
        // a call and the LLM retries (or the planner is re-invoked), the
        // new attempt gets its own id and the store's permanent
        // resolved-set never collides (pre-dispatch approval review fix). The
        // originating `tool_use_id` rides on `correlation_id` so the host
        // can still correlate the request with the LLM's tool-use block.
        let approval_id = ApprovalId::new_unchecked(uuid::Uuid::new_v4().to_string());
        let request = ApprovalRequest {
            id: approval_id.clone(),
            kind: ApprovalKind::Tool,
            target: name.clone(),
            payload: input.clone(),
            glimpse: None,
            resource_id: tenant_id,
            thread_id,
            correlation_id: Some(tool_use_id.to_string()),
        };

        // ─── H4: register BEFORE emit ─────────────────────────────────
        let awaiter = match self.store.register(request.clone()).await {
            Ok(a) => a,
            Err(e) => {
                return ToolCallResult::error(
                    tool_use_id,
                    format!("Approval registration failed: {e}"),
                );
            }
        };
        // pre-dispatch approval review fix: a closed sink between register and emit
        // would leave the awaiter pending forever. Check the bool return
        // and tear down the registration if emission failed.
        if !env.event_sink().emit_approval_required(request) {
            let _ = self
                .store
                .expire(&approval_id, ExpireReason::HostShutdown)
                .await;
            return ToolCallResult::error(
                tool_use_id,
                "Approval event sink closed before request was emitted",
            );
        }

        // ─── H1: race awaiter against cancellation ────────────────────
        let cancel_fut = env.cancel().cancelled();
        tokio::pin!(cancel_fut);

        let decision_result = tokio::select! {
            decision = awaiter => decision,
            _ = &mut cancel_fut => {
                // Cancel branch: release the store-side entry. Ignore
                // the error — if expire fails because the host raced us
                // to resolve(), that's fine; the awaiter would have
                // resolved either way.
                let _ = self
                    .store
                    .expire(&approval_id, ExpireReason::Cancelled)
                    .await;
                return ToolCallResult::error(tool_use_id, "Cancelled while awaiting approval");
            }
        };

        // ─── Dispatch on decision ─────────────────────────────────────
        //
        // Post-decision emits are best-effort: the awaiter has already
        // resolved so the gate has done its work. A closed sink at this
        // point loses host visibility of the decision but does not
        // strand the gate — log on false and continue (pre-dispatch approval review fix).
        match decision_result {
            Ok(decision) if decision.outcome.is_approve() => {
                if !env.event_sink().emit_approval_decision(decision) {
                    tracing::warn!(
                        approval_id = %approval_id,
                        "event sink closed; approval_decision (approve) lost to host"
                    );
                }
                self.inner.handle(tool_use_id, input, env).await
            }
            Ok(decision) => {
                // Reject — or Revise, which alpha collapses to Reject for tools.
                let feedback = decision
                    .feedback
                    .clone()
                    .unwrap_or_else(|| "Approval rejected".into());
                if !env.event_sink().emit_approval_decision(decision) {
                    tracing::warn!(
                        approval_id = %approval_id,
                        "event sink closed; approval_decision (reject) lost to host"
                    );
                }

                // H8: anti-retry-loop payload — LLM should NOT retry.
                ToolCallResult {
                    tool_use_id: tool_use_id.into(),
                    content: serde_json::json!({
                        "error": "approval_rejected",
                        "rejected": true,
                        "reason": feedback,
                        "do_not_retry": true,
                    }),
                    is_error: true,
                    approval_dsl: None,
                    display_summary: None,
                }
            }
            Err(e) => ToolCallResult::error(tool_use_id, format!("Approval error: {e}")),
        }
    }
}

// ─── TracedLayer ───────────────────────────────────────────────────

/// A layer that adds observability events to every handler it wraps.
///
/// Emits `tool_call` before and `tool_result` after each invocation
/// via the environment's [`EventSink`]. The inner handler's behavior
/// is unchanged — this layer only adds effects.
///
/// # Usage
///
/// ```rust,ignore
/// let dispatcher = generic.into_dispatcher(env)
///     .tool(BuildPlanHandler::new(ctx, schema, table))
///     .tool(ExecutePlanHandler)
///     .traced();  // all tools get observability
/// ```
pub struct TracedLayer;

impl ToolLayer for TracedLayer {
    fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler> {
        Arc::new(TracedHandler::new(handler))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolCallResult, ToolDefinition};
    use agent_fw_algebra::event_sink::EventSink;
    use agent_fw_core::stream_part::ToolInvocationState;
    use agent_fw_core::StreamPart;
    use agent_fw_tool::ToolEnvironment;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ── Test infrastructure ──────────────────────────────────────────

    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".into(),
                description: "Echo input back".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, input)
        }
    }

    struct RecordingSink {
        events: Mutex<Vec<StreamPart>>,
        open: AtomicBool,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                open: AtomicBool::new(true),
            }
        }
        fn events(&self) -> Vec<StreamPart> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventSink for RecordingSink {
        fn emit(&self, part: StreamPart) -> bool {
            if !self.is_open() {
                return false;
            }
            self.events.lock().unwrap().push(part);
            true
        }
        fn close(&self) {
            self.open.store(false, Ordering::SeqCst);
        }
        fn is_open(&self) -> bool {
            self.open.load(Ordering::SeqCst)
        }
    }

    fn recording_env() -> (ToolEnvironment, Arc<RecordingSink>) {
        use agent_fw_algebra::testing::{NullKVStore, NullSubAgentInvoker};
        use agent_fw_algebra::CancellationToken;
        use agent_fw_core::id::TenantId;
        use agent_fw_core::tenant::TenantContext;

        let kv: Arc<dyn agent_fw_algebra::KVStore> = Arc::new(NullKVStore);
        let sink = Arc::new(RecordingSink::new());
        let sub_agents: Arc<dyn agent_fw_algebra::SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(TenantId::new_unchecked("test"));
        let env = ToolEnvironment::new(
            kv,
            sink.clone() as Arc<dyn EventSink>,
            sub_agents,
            tenant,
            cancel,
        );
        (env, sink)
    }

    // ── A counting layer (for composition tests) ──────────────────

    struct CountingLayer {
        count: Arc<AtomicUsize>,
    }

    impl CountingLayer {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    count: count.clone(),
                },
                count,
            )
        }
    }

    impl ToolLayer for CountingLayer {
        fn wrap(&self, handler: Arc<dyn ToolHandler>) -> Arc<dyn ToolHandler> {
            let count = self.count.clone();
            Arc::new(CountingHandler {
                inner: handler,
                count,
            })
        }
    }

    struct CountingHandler {
        inner: Arc<dyn ToolHandler>,
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ToolHandler for CountingHandler {
        fn definition(&self) -> ToolDefinition {
            self.inner.definition()
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            env: &ToolEnvironment,
        ) -> ToolCallResult {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.inner.handle(tool_use_id, input, env).await
        }
    }

    // ─── L1: Transparency ────────────────────────────────────────────

    #[test]
    fn traced_layer_preserves_definition() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let original_def = handler.definition();
        let wrapped = TracedLayer.wrap(handler);
        let wrapped_def = wrapped.definition();
        assert_eq!(original_def.name, wrapped_def.name);
        assert_eq!(original_def.description, wrapped_def.description);
        assert_eq!(original_def.input_schema, wrapped_def.input_schema);
    }

    #[test]
    fn counting_layer_preserves_definition() {
        let (layer, _count) = CountingLayer::new();
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let original_def = handler.definition();
        let wrapped = layer.wrap(handler);
        assert_eq!(original_def.name, wrapped.definition().name);
    }

    // ─── L2: Composition ─────────────────────────────────────────────

    #[tokio::test]
    async fn composed_layer_applies_both() {
        let (layer_a, count_a) = CountingLayer::new();
        let (layer_b, count_b) = CountingLayer::new();
        let composed = layer_a.then(layer_b);

        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let wrapped = composed.wrap(handler);

        let (env, _sink) = recording_env();
        wrapped
            .handle("id-1", serde_json::json!({"x": 1}), &env)
            .await;

        assert_eq!(count_a.load(Ordering::SeqCst), 1, "inner layer ran");
        assert_eq!(count_b.load(Ordering::SeqCst), 1, "outer layer ran");
    }

    #[test]
    fn composed_layer_preserves_definition() {
        let (layer_a, _) = CountingLayer::new();
        let (layer_b, _) = CountingLayer::new();
        let composed = layer_a.then(layer_b);

        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let original_def = handler.definition();
        let wrapped = composed.wrap(handler);
        assert_eq!(original_def.name, wrapped.definition().name);
    }

    // ─── TracedLayer: semantic preservation ───────────────────────────

    #[tokio::test]
    async fn traced_layer_semantic_preservation() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let wrapped = TracedLayer.wrap(handler.clone());

        let (env, _sink) = recording_env();
        let input = serde_json::json!({"key": "value"});

        let inner_result = handler.handle("id-1", input.clone(), &env).await;
        let traced_result = wrapped.handle("id-2", input, &env).await;

        assert_eq!(inner_result.is_error, traced_result.is_error);
        assert_eq!(inner_result.content, traced_result.content);
    }

    // ─── L1 (Single Emission): TracedHandler emits card exactly once ──

    /// Handler that returns approval_dsl — simulates build_plan / execute_plan.
    struct CardHandler;

    #[async_trait]
    impl ToolHandler for CardHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "card_tool".into(),
                description: "Returns approval card".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            _input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            let mut result =
                ToolCallResult::success(tool_use_id, serde_json::json!({"planId": "plan-1"}));
            result.approval_dsl = Some("{\"card\":true}".to_string());
            result.display_summary = Some("Plan created".to_string());
            result
        }
    }

    #[tokio::test]
    async fn traced_handler_emits_card_exactly_once() {
        let handler: Arc<dyn ToolHandler> = Arc::new(CardHandler);
        let wrapped = TracedLayer.wrap(handler);

        let (env, sink) = recording_env();
        wrapped.handle("card-1", serde_json::json!({}), &env).await;

        let events = sink.events();
        let data_flow_ui_count = events
            .iter()
            .filter(|e| matches!(e, StreamPart::DataFlowUI { .. }))
            .count();
        assert_eq!(
            data_flow_ui_count, 1,
            "TracedHandler must emit exactly one DataFlowUI event, got {data_flow_ui_count}"
        );
    }

    // ─── TracedLayer: event ordering ──────────────────────────────────

    #[tokio::test]
    async fn traced_layer_event_ordering() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let wrapped = TracedLayer.wrap(handler);

        let (env, sink) = recording_env();
        wrapped
            .handle("trace-1", serde_json::json!({"q": "test"}), &env)
            .await;

        let events = sink.events();
        assert_eq!(events.len(), 2, "TracedLayer emits exactly 2 events");

        assert!(
            matches!(&events[0], StreamPart::ToolInvocation(data)
                if matches!(data.state, ToolInvocationState::Call)),
            "First event must be tool_call"
        );
        assert!(
            matches!(&events[1], StreamPart::ToolInvocation(data)
                if matches!(data.state, ToolInvocationState::Result { .. })),
            "Second event must be tool_result"
        );
    }

    // ─── GuardedLayer tests ──────────────────────────────────────────

    #[test]
    fn guarded_layer_preserves_definition() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let original_def = handler.definition();
        let wrapped = GuardedLayer.wrap(handler);
        assert_eq!(original_def.name, wrapped.definition().name);
    }

    #[tokio::test]
    async fn g1_guarded_short_circuits_when_cancelled() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let wrapped = GuardedLayer.wrap(handler);

        // Create env with a pre-cancelled token
        let (env, _sink) = recording_env();
        let cancel = env.cancel();
        cancel.cancel();

        let result = wrapped
            .handle("guard-1", serde_json::json!({"x": 1}), &env)
            .await;
        assert!(
            result.is_error,
            "GuardedHandler must short-circuit when cancelled"
        );
    }

    #[tokio::test]
    async fn g2_guarded_transparent_when_active() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let wrapped = GuardedLayer.wrap(handler.clone());

        let (env, _sink) = recording_env();
        let inner_result = handler
            .handle("g2-inner", serde_json::json!({"x": 1}), &env)
            .await;
        let guarded_result = wrapped
            .handle("g2-guard", serde_json::json!({"x": 1}), &env)
            .await;

        assert_eq!(inner_result.is_error, guarded_result.is_error);
        assert_eq!(inner_result.content, guarded_result.content);
    }

    #[tokio::test]
    async fn g3_guarded_composes_with_traced() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        // Apply guarded first, then traced — same ordering as .guarded().traced()
        let wrapped = TracedLayer.wrap(GuardedLayer.wrap(handler));

        let (env, sink) = recording_env();
        let result = wrapped
            .handle("g3-1", serde_json::json!({"x": 1}), &env)
            .await;
        assert!(!result.is_error);
        // Should emit traced events (tool_call + tool_result)
        assert_eq!(sink.events().len(), 2);
    }

    // ─── ApprovalLayer tests — pre-dispatch approval acceptance ────────────────────

    use crate::approval::{ApprovalPolicy, ApprovalRule};
    use agent_fw_algebra::approval::{InMemoryPendingApprovalStore, PendingApprovalStore};
    use agent_fw_core::approval::ApprovalDecision;
    use agent_fw_core::ApprovalId;

    /// Counter tool: increments an `AtomicUsize` on each `handle()` call.
    /// The pre-dispatch approval acceptance invariant: this counter must stay at 0
    /// while approval is pending, and reach exactly 1 only after Approve.
    struct CounterHandler {
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ToolHandler for CounterHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "danger_op".into(),
                description: "Side-effecting tool gated by approval".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            _input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            self.count.fetch_add(1, Ordering::SeqCst);
            ToolCallResult::success(tool_use_id, serde_json::json!({"ran": true}))
        }
    }

    fn always_policy_for(tool: &str) -> Arc<ApprovalPolicy> {
        Arc::new(ApprovalPolicy::new().with_tool(tool, ApprovalRule::Always))
    }

    /// Build an `env` whose `tenant().resource_id()` is `"acme"` and
    /// `thread_id()` is `"th-1"`. The approval id is a fresh UUID per
    /// attempt (pre-dispatch approval review fix); tests that need to resolve must
    /// read the emitted `ApprovalRequired` event off the sink and use
    /// `data.id`.
    fn env_with_thread() -> (ToolEnvironment, Arc<RecordingSink>) {
        use agent_fw_algebra::testing::{NullKVStore, NullSubAgentInvoker};
        use agent_fw_algebra::CancellationToken;
        use agent_fw_core::id::{TenantId, ThreadId};
        use agent_fw_core::tenant::TenantContext;

        let kv: Arc<dyn agent_fw_algebra::KVStore> = Arc::new(NullKVStore);
        let sink = Arc::new(RecordingSink::new());
        let sub_agents: Arc<dyn agent_fw_algebra::SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(TenantId::new_unchecked("acme"))
            .with_thread(ThreadId::new_unchecked("th-1"));
        let env = ToolEnvironment::new(
            kv,
            sink.clone() as Arc<dyn EventSink>,
            sub_agents,
            tenant,
            cancel,
        );
        (env, sink)
    }

    /// Poll the sink for an emitted `ApprovalRequired` event and return
    /// its `ApprovalId`. Used by tests that need to resolve the UUID-
    /// based approval the layer generates per attempt (pre-dispatch approval review fix).
    async fn wait_for_approval_id(sink: &Arc<RecordingSink>, timeout_ms: u64) -> ApprovalId {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            for event in sink.events() {
                if let StreamPart::ApprovalRequired { data } = event {
                    return data.id;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!("approval_required event not emitted within {timeout_ms}ms");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// pre-dispatch approval L1: while approval is pending, the inner handler is NEVER invoked.
    #[tokio::test]
    async fn approval_pending_counter_stays_zero() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<dyn PendingApprovalStore> = Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        let wrapped = ApprovalLayer::new(policy, store).wrap(inner);

        let (env, _sink) = env_with_thread();

        // Spawn the gated call without resolving — should NEVER complete.
        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            wrapped
                .handle("call-1", serde_json::json!({}), &env_clone)
                .await
        });

        // Yield to let the spawned task hit the await.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            !handle.is_finished(),
            "Gated call must remain pending without a decision"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "pre-dispatch approval invariant: counter MUST stay at 0 while approval is pending"
        );

        // Cleanup: cancel so the spawned task can finish.
        env.cancel().cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), handle).await;
    }

    /// pre-dispatch approval L2: after Approve, inner handler runs EXACTLY ONCE.
    #[tokio::test]
    async fn approval_approve_runs_inner_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<dyn PendingApprovalStore> = Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        let wrapped = ApprovalLayer::new(policy, store.clone()).wrap(inner);

        let (env, sink) = env_with_thread();
        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            wrapped
                .handle("call-2", serde_json::json!({}), &env_clone)
                .await
        });

        // Wait for the approval_required event to be emitted before resolving
        // — that proves the layer registered before emitting (H4) and gives
        // us the UUID to resolve against (review fix).
        let approval_id = wait_for_approval_id(&sink, 500).await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "still pending");
        store
            .resolve(ApprovalDecision::approve(approval_id))
            .await
            .expect("resolve");

        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("handler completes within 500ms")
            .expect("no panic");

        assert!(!result.is_error, "approved call returns success");
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "pre-dispatch approval invariant: after Approve, counter MUST equal exactly 1"
        );

        // Event ordering: approval_required → approval_decision (TracedLayer not used here).
        let events = sink.events();
        let kinds: Vec<&'static str> = events
            .iter()
            .map(|e| match e {
                StreamPart::ApprovalRequired { .. } => "required",
                StreamPart::ApprovalDecision { .. } => "decision",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["required", "decision"],
            "ordering must be: approval_required first, then approval_decision"
        );
    }

    /// pre-dispatch approval L3: Reject NEVER invokes inner, emits decision, returns anti-retry payload.
    #[tokio::test]
    async fn approval_reject_never_invokes_inner() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<dyn PendingApprovalStore> = Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        let wrapped = ApprovalLayer::new(policy, store.clone()).wrap(inner);

        let (env, sink) = env_with_thread();
        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            wrapped
                .handle("call-3", serde_json::json!({}), &env_clone)
                .await
        });

        let approval_id = wait_for_approval_id(&sink, 500).await;
        store
            .resolve(ApprovalDecision::reject(approval_id, "unsafe args"))
            .await
            .expect("resolve");

        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("handler completes")
            .expect("no panic");

        assert!(result.is_error, "rejected call must be is_error=true");
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "pre-dispatch approval invariant: Reject MUST NOT invoke inner"
        );

        // H8: anti-retry-loop payload
        let content = &result.content;
        assert_eq!(content["rejected"], serde_json::json!(true));
        assert_eq!(content["do_not_retry"], serde_json::json!(true));
        assert_eq!(content["reason"], serde_json::json!("unsafe args"));
    }

    /// pre-dispatch approval L4 (hazard H1): cancellation during await releases the store entry.
    #[tokio::test]
    async fn approval_cancel_during_await_releases_store_entry() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<InMemoryPendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        let wrapped =
            ApprovalLayer::new(policy, store.clone() as Arc<dyn PendingApprovalStore>).wrap(inner);

        let (env, _sink) = env_with_thread();
        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            wrapped
                .handle("call-4", serde_json::json!({}), &env_clone)
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(store.pending_count(), 1, "approval registered");

        // Cancel — layer should call store.expire and clean up.
        env.cancel().cancel();
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("handler completes after cancel")
            .expect("no panic");

        assert!(result.is_error, "cancelled call returns error");
        assert_eq!(count.load(Ordering::SeqCst), 0, "inner never invoked");
        assert_eq!(
            store.pending_count(),
            0,
            "H1: store entry released on cancellation (no leak)"
        );
    }

    /// `ApprovalRule::Never` delegates straight through without a registration.
    #[tokio::test]
    async fn approval_never_passes_through() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<InMemoryPendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let policy = Arc::new(ApprovalPolicy::new()); // default Never
        let wrapped =
            ApprovalLayer::new(policy, store.clone() as Arc<dyn PendingApprovalStore>).wrap(inner);

        let (env, _sink) = env_with_thread();
        let result = wrapped.handle("call-5", serde_json::json!({}), &env).await;
        assert!(!result.is_error);
        assert_eq!(count.load(Ordering::SeqCst), 1, "Never bypasses the gate");
        assert_eq!(store.pending_count(), 0, "no registration for Never");
    }

    /// Composition with `TracedLayer`: ensure the outer trace sees a single
    /// tool_call → tool_result pair (gate doesn't multiply trace events).
    #[tokio::test]
    async fn approval_traced_composition_single_call_result() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<dyn PendingApprovalStore> = Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        // Order: approval first, traced outermost.
        let wrapped = TracedLayer.wrap(ApprovalLayer::new(policy, store.clone()).wrap(inner));

        let (env, sink) = env_with_thread();
        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            wrapped
                .handle("call-6", serde_json::json!({}), &env_clone)
                .await
        });

        let approval_id = wait_for_approval_id(&sink, 500).await;
        store
            .resolve(ApprovalDecision::approve(approval_id))
            .await
            .expect("resolve");
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("completes");

        let events = sink.events();
        let tool_calls = events
            .iter()
            .filter(|e| matches!(e, StreamPart::ToolInvocation(data) if matches!(data.state, ToolInvocationState::Call)))
            .count();
        let tool_results = events
            .iter()
            .filter(|e| matches!(e, StreamPart::ToolInvocation(data) if matches!(data.state, ToolInvocationState::Result { .. })))
            .count();
        assert_eq!(tool_calls, 1, "exactly one tool_call");
        assert_eq!(tool_results, 1, "exactly one tool_result");
    }

    /// pre-dispatch approval review fix: a tool call rejected by the host can be
    /// re-invoked with the same `tool_use_id` (LLM retry path) and
    /// reach a fresh approval gate without `AlreadyRegistered`.
    /// This is the bug the UUID-IDs fix addresses.
    #[tokio::test]
    async fn approval_retry_after_reject_succeeds() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store: Arc<dyn PendingApprovalStore> = Arc::new(InMemoryPendingApprovalStore::new());
        let policy = always_policy_for("danger_op");
        let wrapped = ApprovalLayer::new(policy, store.clone()).wrap(inner);

        let (env, sink) = env_with_thread();

        // First attempt: reject.
        let env_clone = env.clone();
        let wrapped_clone = wrapped.clone();
        let handle = tokio::spawn(async move {
            wrapped_clone
                .handle("call-retry", serde_json::json!({}), &env_clone)
                .await
        });
        let approval_id = wait_for_approval_id(&sink, 500).await;
        store
            .resolve(ApprovalDecision::reject(approval_id, "no thanks"))
            .await
            .expect("first resolve");
        let first = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("first attempt completes")
            .expect("no panic");
        assert!(first.is_error, "first attempt was rejected");
        assert_eq!(count.load(Ordering::SeqCst), 0, "inner never ran");

        // Second attempt: same tool_use_id, different ApprovalId (UUID).
        let env_clone = env.clone();
        let wrapped_clone = wrapped.clone();
        let handle = tokio::spawn(async move {
            wrapped_clone
                .handle("call-retry", serde_json::json!({}), &env_clone)
                .await
        });
        // Filter for the *second* ApprovalRequired in the cumulative sink log.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        let second_id = loop {
            let ids: Vec<ApprovalId> = sink
                .events()
                .into_iter()
                .filter_map(|e| match e {
                    StreamPart::ApprovalRequired { data } => Some(data.id),
                    _ => None,
                })
                .collect();
            if ids.len() >= 2 {
                break ids[1].clone();
            }
            if std::time::Instant::now() >= deadline {
                panic!("second approval_required never emitted");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        };
        store
            .resolve(ApprovalDecision::approve(second_id.clone()))
            .await
            .expect("retry resolve");
        let second = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("retry completes")
            .expect("no panic");
        assert!(!second.is_error, "retry succeeded");
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "pre-dispatch approval review fix: retry reaches inner exactly once"
        );

        // Sanity: the two ApprovalIds are distinct UUIDs.
        let ids: Vec<ApprovalId> = sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StreamPart::ApprovalRequired { data } => Some(data.id),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        // And both carry the correlation_id pointing at the LLM's tool_use_id.
        let correlations: Vec<Option<String>> = sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StreamPart::ApprovalRequired { data } => Some(data.correlation_id),
                _ => None,
            })
            .collect();
        assert_eq!(correlations[0].as_deref(), Some("call-retry"));
        assert_eq!(correlations[1].as_deref(), Some("call-retry"));
    }

    /// pre-dispatch approval review fix: a closed event sink between `register` and
    /// `emit_approval_required` must short-circuit the gate with an
    /// error and expire the store entry — not hang forever.
    #[tokio::test]
    async fn approval_emit_failure_expires_and_errors() {
        use agent_fw_algebra::testing::{NullKVStore, NullSubAgentInvoker};
        use agent_fw_algebra::{CancellationToken, KVStore, SubAgentInvoker};
        use agent_fw_core::id::TenantId;
        use agent_fw_core::tenant::TenantContext;

        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn ToolHandler> = Arc::new(CounterHandler {
            count: count.clone(),
        });

        let store_inner = Arc::new(InMemoryPendingApprovalStore::new());
        let store: Arc<dyn PendingApprovalStore> = store_inner.clone();
        let policy = always_policy_for("danger_op");
        let wrapped = ApprovalLayer::new(policy, store.clone()).wrap(inner);

        // Build env with a pre-closed sink — every emit returns false.
        let sink = Arc::new(RecordingSink::new());
        sink.close();
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(TenantId::new_unchecked("acme"))
            .with_thread(ThreadId::new_unchecked("th-1"));
        let env = ToolEnvironment::new(
            kv,
            sink.clone() as Arc<dyn EventSink>,
            sub_agents,
            tenant,
            cancel,
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            wrapped.handle("call-emit-fail", serde_json::json!({}), &env),
        )
        .await
        .expect("must not hang; closed-sink path returns immediately");

        assert!(result.is_error, "closed-sink path returns is_error=true");
        assert_eq!(count.load(Ordering::SeqCst), 0, "inner never invoked");
        assert_eq!(
            store_inner.pending_count(),
            0,
            "pre-dispatch approval review fix: store entry expired on emit failure (no leak)"
        );
    }
}
