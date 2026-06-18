//! Per-request runtime wiring (C4, runtime query assembly).
//!
//! This module assembles the framework primitives that the harness needs to
//! drive a `query` or `run_specialist` call end-to-end:
//!
//! - [`providers`]: maps an `AgentSpec.model` to one of the registered
//!   `Arc<dyn ChatInterpreter>` instances supplied via
//!   [`RuntimeDeps::interpreter_providers`](crate::RuntimeDeps::interpreter_providers).
//! - [`action`]: noop `ActionDispatcher` default used by the executor when
//!   the host hasn't supplied one (e.g. before Python adapter lands).
//! - [`approval`]: compile `ApprovalPolicies` from the spec into the
//!   framework's [`ApprovalPolicy`](agent_fw_agent::ApprovalPolicy);
//!   translate runtime↔core `ApprovalDecision`s; back
//!   [`Runtime::respond_to_approval`](crate::Runtime::respond_to_approval).
//! - [`session`]: per-request `Arc`-bundle (channel sink, dual sink,
//!   per-agent dispatcher composition, plan execution context).
//! - [`query`]: drives a per-request `AgentOrchestrator` for `query` and
//!   `run_specialist`.

pub mod action;
pub mod approval;
pub(crate) mod invoker;
pub mod memory;
pub mod providers;
pub mod query;
pub mod session;

pub use action::NoopActionDispatcher;
pub use session::PlanExecutionContext;
