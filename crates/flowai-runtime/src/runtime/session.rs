//! Per-request scaffolding for the harness runtime (runtime query assembly C4).
//!
//! Two things live here:
//!
//! - [`PlanExecutionContext`], a `ToolEnvironment` extension that the
//!   `executePlan` handler reads to build the framework's
//!   [`GatedPlanExecutor`](agent_fw_plan::executor::GatedPlanExecutor) and
//!   [`HydratingDispatcher`](crate::plans::HydratingDispatcher).
//! - Per-request state shared between `Runtime::query` and its spawned
//!   driver (added in a later commit).

use std::sync::Arc;

use agent_fw_agent::approval::ApprovalPolicy;
use agent_fw_algebra::approval::PendingApprovalStore;

use crate::HarnessActionDispatcher;

/// Per-request plan-execution context surfaced through
/// [`ToolEnvironment`](agent_fw_tool::ToolEnvironment) extensions.
///
/// `executePlan` reads it via `env.try_ext::<PlanExecutionContext>()` and
/// uses it to construct the framework's
/// [`GatedPlanExecutor`](agent_fw_plan::executor::GatedPlanExecutor)
/// (approval gate) wrapped around a
/// [`HydratingDispatcher`](crate::plans::HydratingDispatcher) (reference
/// hydration) wrapped around the customer-supplied
/// [`HarnessActionDispatcher`]. Everything else (`kv`, `event_sink`,
/// `tenant`, `cancel`) is read from the standard `ToolEnvironment` API.
#[derive(Clone)]
pub struct PlanExecutionContext {
    /// Effective per-agent approval policy. `executePlan` uses the plan
    /// channel before constructing the framework gate, so agent-level
    /// `plans` overrides can skip the draft-plan pause.
    pub approval_policy: Arc<ApprovalPolicy>,
    /// Shared pending-approval store. Same `Arc` as
    /// [`Runtime::approval_store`](crate::Runtime::approval_store) so the
    /// host can resolve plan-gate decisions via
    /// [`Runtime::respond_to_approval`](crate::Runtime::respond_to_approval).
    pub approval_store: Arc<dyn PendingApprovalStore>,
    /// Customer plan-action dispatcher. Defaults to
    /// [`NoopActionDispatcher`](crate::runtime::action::NoopActionDispatcher)
    /// when [`RuntimeDeps::action_dispatcher`](crate::RuntimeDeps::action_dispatcher)
    /// is `None`.
    pub action_dispatcher: Arc<HarnessActionDispatcher>,
    /// Synthetic approver identity recorded on `Plan::approve(by)`. Bindings
    /// should override with the authenticated user id where available; the
    /// runtime falls back to `"runtime"` when unset.
    pub approver: agent_fw_core::UserId,
}
