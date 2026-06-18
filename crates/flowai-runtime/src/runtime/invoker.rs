//! Late-bound [`SubAgentInvoker`] for per-request orchestrator wiring.
//!
//! Per-request `ToolEnvironment`s need an `Arc<dyn SubAgentInvoker>` so that
//! a tool like
//! [`CallAgentHandler`](agent_fw_agent::CallAgentHandler) can delegate to
//! sibling agents. The canonical invoker is the request-scoped
//! [`AgentOrchestrator`](agent_fw_agent::AgentOrchestrator) — but the
//! orchestrator is constructed *after* the per-agent dispatcher map (which
//! in turn requires the env), so there's a chicken-and-egg.
//!
//! [`LateBoundInvoker`] resolves it: the harness builds an empty
//! `LateBoundInvoker`, installs it on every per-agent env, builds the
//! orchestrator, then calls [`LateBoundInvoker::set`] exactly once. By the
//! time the spawn task runs `orchestrator.invoke(...)` the weak ref
//! upgrades cleanly, so any `call_agent` tool the LLM emits reaches the
//! real orchestrator transparently.
//!
//! # Weak vs Arc — and why this matters
//!
//! The orchestrator's dispatcher map holds per-agent `ToolEnvironment`s,
//! and each env's `sub_agents` is **this** invoker. If the invoker held a
//! strong `Arc<AgentOrchestrator>`, the back-edge would close a reference
//! cycle (orchestrator → envs → invoker → orchestrator) and neither end
//! would ever drop — and crucially every `Arc<ChannelEventSink>` held
//! through those envs would survive forever, so the runtime's stream
//! receiver would never see end-of-stream. `Weak` breaks the cycle: the
//! invoker holds a non-owning ref, the orchestrator owns the only strong
//! refs (in the runtime's spawn task), and once the spawn ends and the
//! task drops its Arc the orchestrator drops, envs drop, sinks drop, the
//! mpsc receiver returns `None`, and the stream completes normally.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::Weak;

use agent_fw_algebra::sub_agent::{SubAgentRequest, SubAgentResult};
use agent_fw_algebra::{SubAgentError, SubAgentInvoker};
use agent_fw_core::stream_part::CostSummary;
use async_trait::async_trait;

/// A [`SubAgentInvoker`] whose inner implementation is set after
/// construction. See module docs for the chicken-and-egg this resolves
/// and why the inner ref is `Weak`, not `Arc`.
#[derive(Default)]
pub(crate) struct LateBoundInvoker {
    inner: OnceLock<Weak<dyn SubAgentInvoker>>,
}

impl LateBoundInvoker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Install the inner invoker as a [`Weak`] reference. The caller must
    /// keep at least one [`Arc`] to the same value alive (typically by
    /// holding the orchestrator handle inside the spawn task that drives
    /// the request); otherwise upgrades fail and `invoke` returns
    /// `SubAgentError::Internal`.
    ///
    /// Returns `Err(existing)` if `set` was already called — the harness
    /// invokes this exactly once per request.
    pub(crate) fn set(
        &self,
        inner: &Arc<dyn SubAgentInvoker>,
    ) -> Result<(), Weak<dyn SubAgentInvoker>> {
        self.inner.set(Arc::downgrade(inner))
    }

    fn resolved(&self) -> Option<Arc<dyn SubAgentInvoker>> {
        self.inner.get().and_then(|w| w.upgrade())
    }
}

#[async_trait]
impl SubAgentInvoker for LateBoundInvoker {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        match self.resolved() {
            Some(inner) => inner.invoke(request).await,
            None => Err(SubAgentError::Internal(
                "LateBoundInvoker has no live orchestrator (Weak upgrade failed — the runtime \
                 may have torn down before delegation completed)"
                    .into(),
            )),
        }
    }

    fn has_agent(&self, name: &str) -> bool {
        self.resolved().map(|i| i.has_agent(name)).unwrap_or(false)
    }

    fn available_agents(&self) -> Vec<String> {
        self.resolved()
            .map(|i| i.available_agents())
            .unwrap_or_default()
    }

    fn cost_summary(&self) -> Option<CostSummary> {
        self.resolved().and_then(|i| i.cost_summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::testing::NullSubAgentInvoker;

    #[tokio::test]
    async fn empty_late_bound_invoker_surfaces_internal_error() {
        let invoker = LateBoundInvoker::new();
        let err = invoker
            .invoke(SubAgentRequest::new("planner", "go"))
            .await
            .unwrap_err();
        assert!(matches!(err, SubAgentError::Internal(ref m) if m.contains("LateBound")));
        assert!(!invoker.has_agent("planner"));
        assert!(invoker.available_agents().is_empty());
    }

    #[tokio::test]
    async fn set_installs_inner_and_subsequent_calls_delegate() {
        let invoker = LateBoundInvoker::new();
        // Caller MUST keep a strong Arc alive for the duration of any
        // upgrade — `inner` here plays that role.
        let inner: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        assert!(invoker.set(&inner).is_ok(), "first set should succeed");

        // NullSubAgentInvoker has no agents — `has_agent` returns false but
        // delegates correctly (not the empty-fallback path).
        assert!(!invoker.has_agent("anything"));
        // Available agents now comes from inner.
        assert!(invoker.available_agents().is_empty());

        // Calling set again returns Err — second install is a programmer
        // error and OnceLock surfaces it explicitly.
        let second: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        assert!(invoker.set(&second).is_err());
    }

    #[tokio::test]
    async fn upgrade_fails_after_inner_arc_drops() {
        // Sanity: confirm the Weak indirection actually breaks the cycle.
        // If the only strong Arc to the inner invoker drops, subsequent
        // invokes return an internal error instead of silently keeping
        // the implementation alive.
        let invoker = LateBoundInvoker::new();
        let inner: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        invoker.set(&inner).expect("set");
        drop(inner);

        assert!(!invoker.has_agent("x"));
        assert!(invoker.available_agents().is_empty());
        let err = invoker
            .invoke(SubAgentRequest::new("x", "y"))
            .await
            .unwrap_err();
        assert!(matches!(err, SubAgentError::Internal(_)));
    }
}
