//! Plan execution lifecycle: load → validate → execute → update.
//!
//! # Two Abstraction Levels
//!
//! The plan module provides two ways to manage execution:
//!
//! ## `PlanExecutor` — Batch lifecycle (load → dispatch → store)
//!
//! Use when the entire execute cycle is a single operation: load the plan,
//! dispatch all actions, store the result. The executor owns the full
//! lifecycle and handles transitions internally. Good for batch jobs,
//! background workers, or simple tools without progress reporting.
//!
//! ## `persist::*_in_kv` — Interactive step-by-step
//!
//! Use when you need control between transitions: progress reporting,
//! cancellation checks, intermediate computation, or recovery logic
//! (see [`complete_or_recover_in_kv`](crate::persist::complete_or_recover_in_kv)).
//! Each function performs exactly one transition, giving the caller full
//! control of the lifecycle.
//!
//! ## When to Use Which
//!
//! - **`PlanExecutor`**: Your entire execution fits inside `ActionDispatcher::dispatch`.
//!   No need for progress reporting, card rendering, or cancellation between phases.
//!   Good for batch jobs, background workers, simple tool handlers.
//!
//! - **`persist::*_in_kv`**: You need control between transitions — progress events,
//!   cancellation checks, pure computation with card rendering, recovery logic.
//!   Good for interactive tool handlers with multi-phase execution.
//!
//! # ActionDispatcher
//!
//! Domain-specific trait for executing plan actions against the target system.
//! The dispatcher receives the FULL action sequence to allow compound execution
//! (batch operations).
//!
//! # Laws
//!
//! - Only Approved plans can be executed (Draft/Executing/terminal → error)
//! - Successful execution transitions to Executed
//! - Failed execution transitions to Failed
//! - Plan is stored in KV after every state transition

use agent_fw_algebra::approval::{ExpireReason, PendingApprovalStore};
use agent_fw_algebra::event_sink::EventSinkExt;
use agent_fw_algebra::{CancellationToken, EventSink, KVError, KVStore, KVStoreExt};
use agent_fw_core::approval::{ApprovalKind, ApprovalRequest};
use agent_fw_core::{ApprovalId, PlanId, TenantId, ThreadId};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::action::ActionSeq;
use crate::persist::plan_key;
use crate::plan::{ExecutionResult, Plan, PlanError, PlanStatus};

/// Action dispatcher: executes plan actions against the target system.
///
/// Laws:
/// - `dispatch()` is called exactly once per plan execution
/// - The dispatcher receives the FULL action sequence (not individual actions)
///   to allow compound execution (batch operations)
#[async_trait]
pub trait ActionDispatcher: Send + Sync {
    /// Domain-specific action type.
    type Action: Send + Sync;
    /// Execution context (domain-specific dependencies).
    type Context: Send + Sync;
    /// Domain-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Execute the full action sequence.
    async fn dispatch(
        &self,
        actions: &ActionSeq<Self::Action>,
        ctx: &Self::Context,
    ) -> Result<ExecutionResult, Self::Error>;
}

/// Errors from plan execution lifecycle.
#[derive(Debug, thiserror::Error)]
pub enum PlanExecutionError<E: std::error::Error> {
    #[error("Plan not found: {0}")]
    NotFound(PlanId),
    #[error("Invalid plan state for execution: {0}")]
    InvalidState(PlanStatus),
    #[error("KV error: {0}")]
    Kv(#[from] KVError),
    #[error("Dispatch failed: {0}")]
    Dispatch(E),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Plan transition error: {0}")]
    Transition(#[from] crate::plan::TransitionError),
}

/// Convert a `Rejected<A>` into a `PlanExecutionError` by extracting the error.
impl<A, E: std::error::Error> From<crate::plan::Rejected<A>> for PlanExecutionError<E> {
    fn from(r: crate::plan::Rejected<A>) -> Self {
        Self::Transition(r.error)
    }
}

/// Plan execution lifecycle manager.
///
/// Orchestrates: load → validate → start → dispatch → complete/fail → store.
pub struct PlanExecutor<'a, A, D: ActionDispatcher<Action = A>> {
    kv: &'a dyn KVStore,
    tenant: &'a TenantId,
    dispatcher: &'a D,
    kv_prefix: &'a str,
    _action: PhantomData<A>,
}

impl<'a, A, D> PlanExecutor<'a, A, D>
where
    A: Serialize + DeserializeOwned + Send + Sync + Clone,
    D: ActionDispatcher<Action = A>,
{
    /// Create a new plan executor.
    ///
    /// `kv_prefix` is the KV key prefix for plans (e.g., "plan").
    pub fn new(
        kv: &'a dyn KVStore,
        tenant: &'a TenantId,
        dispatcher: &'a D,
        kv_prefix: &'a str,
    ) -> Self {
        Self {
            kv,
            tenant,
            dispatcher,
            kv_prefix,
            _action: PhantomData,
        }
    }

    /// Execute a plan through its full lifecycle.
    ///
    /// 1. Load plan from KV
    /// 2. Validate it's in Approved state
    /// 3. Transition to Executing, store
    /// 4. Dispatch actions via ActionDispatcher
    /// 5. On success: transition to Executed, store
    /// 6. On failure: transition to Failed, store
    pub async fn execute(
        &self,
        plan_id: &PlanId,
        ctx: &D::Context,
    ) -> Result<ExecutionResult, PlanExecutionError<D::Error>> {
        let key = plan_key(self.kv_prefix, plan_id);

        // 1. Load plan
        let plan: Plan<A> = self
            .kv
            .get::<Plan<A>>(self.tenant.as_str(), &key)
            .await?
            .ok_or_else(|| PlanExecutionError::NotFound(plan_id.clone()))?;

        // 2. Validate state
        if plan.status != PlanStatus::Approved {
            return Err(PlanExecutionError::InvalidState(plan.status));
        }

        // 3. Start execution
        let plan = plan.start()?;
        self.kv.put(self.tenant.as_str(), &key, &plan, None).await?;

        // 4. Dispatch
        match self.dispatcher.dispatch(&plan.actions, ctx).await {
            Ok(result) => {
                // 5. Complete
                let plan = plan.complete(result.clone())?;
                self.kv.put(self.tenant.as_str(), &key, &plan, None).await?;
                Ok(result)
            }
            Err(e) => {
                // 6. Fail
                let plan = plan.fail(PlanError::new(e.to_string()))?;
                self.kv.put(self.tenant.as_str(), &key, &plan, None).await?;
                Err(PlanExecutionError::Dispatch(e))
            }
        }
    }
}

// ─── GatedPlanExecutor ────────────────────────────────────────────────

/// Errors specific to the approval gate around plan execution.
///
/// Disjoint from [`PlanExecutionError`] so callers can distinguish
/// "host rejected the plan" from "executor itself failed".
#[derive(Debug, thiserror::Error)]
pub enum GatedExecutionError<E: std::error::Error> {
    /// Host responded with `Reject`. The plan stays in `Draft`.
    #[error("plan approval rejected: {reason}")]
    Rejected { plan_id: PlanId, reason: String },
    /// Host responded with `Revise`. Carries the partial body the host
    /// wants the planner re-invoked with (alpha: fresh invocation).
    #[error("plan approval requested revision")]
    Revise {
        plan_id: PlanId,
        partial: serde_json::Value,
        feedback: Option<String>,
    },
    /// Cancellation fired while awaiting host decision (H7).
    #[error("plan approval cancelled while awaiting decision")]
    Cancelled { plan_id: PlanId },
    /// Event sink closed between `register` and pre-await emission, so
    /// the host would never see the `approval_required` event and the
    /// awaiter would block indefinitely (pre-dispatch approval review fix). The store
    /// entry is expired with `HostShutdown` before this error is returned.
    #[error("approval event sink closed before request was emitted")]
    EventSinkClosed { plan_id: PlanId },
    /// Approval store reported an error (registration, resolve, expire).
    #[error("approval store error: {0}")]
    Approval(#[from] agent_fw_algebra::approval::ApprovalError),
    /// Inner executor failed.
    #[error(transparent)]
    Execution(#[from] PlanExecutionError<E>),
}

/// Approval-gated wrapper around [`PlanExecutor`] (pre-dispatch approval).
///
/// Composes over the existing executor without modifying its signature.
/// The wrapper:
///
///   1. Loads the plan from KV.
///   2. If already `Approved` (or further), **skips the gate** (H6)
///      and delegates to the inner executor.
///   3. If `Draft`, registers an [`ApprovalRequest`] with the
///      [`PendingApprovalStore`], emits `approval_required` plus a
///      `plan-status-change` display alias (`draft → pending_approval`),
///      and awaits the host decision.
///   4. On `Approve`: emits `approval_decision` + `plan-status-change`
///      to `"approved"`, calls `Plan::approve(user_id)`, stores back to
///      KV, and delegates to the inner executor (the existing happy
///      path at `executor.rs:144-183`).
///   5. On `Reject`/`Revise`: emits `approval_decision`, leaves the
///      plan in `Draft`, and returns the corresponding error variant.
///
/// # Hazard mitigations
///
/// - **H6 (double-approval)**: state-checked before registering;
///   already-Approved plans bypass the gate.
/// - **H7 (cancellation)**: an explicit [`CancellationToken`] is
///   threaded through `execute` and raced against the awaiter via
///   `tokio::select!`. On cancel, the gate calls
///   `store.expire(id, Cancelled)` to release the entry.
///
/// # Display aliases
///
/// `pending_approval` is **not** a `PlanStatus` variant — the plan
/// stays in `Draft` while the gate is open. The host learns of the
/// display alias only through the emitted `plan-status-change` event.
pub struct GatedPlanExecutor<'a, A, D: ActionDispatcher<Action = A>> {
    inner: PlanExecutor<'a, A, D>,
    kv: &'a dyn KVStore,
    tenant: &'a TenantId,
    kv_prefix: &'a str,
    sink: Arc<dyn EventSink>,
    approval_store: Arc<dyn PendingApprovalStore>,
    approver: agent_fw_core::UserId,
    thread_id: ThreadId,
}

impl<'a, A, D> GatedPlanExecutor<'a, A, D>
where
    A: Serialize + DeserializeOwned + Send + Sync + Clone,
    D: ActionDispatcher<Action = A>,
{
    /// Construct a new approval-gated executor.
    ///
    /// `approver` is the [`UserId`] recorded on `Plan::approve(by)`
    /// after a host `Approve` decision (typically a synthetic "host"
    /// or "runtime" user; the actual end-user identity surfaces in the
    /// `approval_decision` event payload via `feedback`).
    pub fn new(
        kv: &'a dyn KVStore,
        tenant: &'a TenantId,
        dispatcher: &'a D,
        kv_prefix: &'a str,
        sink: Arc<dyn EventSink>,
        approval_store: Arc<dyn PendingApprovalStore>,
        approver: agent_fw_core::UserId,
        thread_id: ThreadId,
    ) -> Self {
        Self {
            inner: PlanExecutor::new(kv, tenant, dispatcher, kv_prefix),
            kv,
            tenant,
            kv_prefix,
            sink,
            approval_store,
            approver,
            thread_id,
        }
    }

    /// Execute a plan, gating the `Draft → Approved` transition.
    ///
    /// Takes an explicit [`CancellationToken`] so callers can cancel
    /// the await (H7).
    pub async fn execute(
        &self,
        plan_id: &PlanId,
        ctx: &D::Context,
        cancel: &CancellationToken,
    ) -> Result<crate::plan::ExecutionResult, GatedExecutionError<D::Error>>
    where
        A: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    {
        let key = crate::persist::plan_key(self.kv_prefix, plan_id);

        // 1. Load plan
        let plan: crate::plan::Plan<A> = self
            .kv
            .get::<crate::plan::Plan<A>>(self.tenant.as_str(), &key)
            .await
            .map_err(|e| PlanExecutionError::<D::Error>::Kv(e))?
            .ok_or_else(|| PlanExecutionError::<D::Error>::NotFound(plan_id.clone()))?;

        // 2. H6: skip gate if already approved (or further). Delegate
        //    straight to the inner executor — that's where the existing
        //    state-machine checks live (executor.rs:159).
        if plan.status != crate::plan::PlanStatus::Draft {
            return self
                .inner
                .execute(plan_id, ctx)
                .await
                .map_err(GatedExecutionError::from);
        }

        // 3. Register approval. Body uses a JSON-encoded snapshot of
        //    the plan; the host renders it as a card.
        //
        // The ApprovalId is a fresh UUID per attempt so a Rejected/Revised
        // plan that stays in `Draft` can be re-executed without colliding
        // with the store's permanent resolved-set (pre-dispatch approval review fix).
        // The `plan_id` rides on `correlation_id` for host-side lookup.
        let approval_id = ApprovalId::new_unchecked(uuid::Uuid::new_v4().to_string());
        let payload = serde_json::to_value(&plan)
            .map_err(|e| GatedExecutionError::Execution(PlanExecutionError::Serde(e)))?;
        let request = ApprovalRequest {
            id: approval_id.clone(),
            kind: ApprovalKind::Plan,
            target: plan_id.as_str().to_string(),
            payload,
            glimpse: None,
            resource_id: self.tenant.clone(),
            thread_id: self.thread_id.clone(),
            correlation_id: Some(plan_id.as_str().to_string()),
        };

        let awaiter = self.approval_store.register(request.clone()).await?;
        // pre-dispatch approval review fix: a closed sink between register and emit
        // would leave the awaiter pending forever. Both pre-await emits
        // are guarded — if either fails, we expire the store entry with
        // `HostShutdown` and return `EventSinkClosed`.
        if !self.sink.emit_approval_required(request) {
            let _ = self
                .approval_store
                .expire(&approval_id, ExpireReason::HostShutdown)
                .await;
            return Err(GatedExecutionError::EventSinkClosed {
                plan_id: plan_id.clone(),
            });
        }
        if !self
            .sink
            .emit_plan_status_change(plan_id.as_str(), "draft", "pending_approval")
        {
            let _ = self
                .approval_store
                .expire(&approval_id, ExpireReason::HostShutdown)
                .await;
            return Err(GatedExecutionError::EventSinkClosed {
                plan_id: plan_id.clone(),
            });
        }

        // 4. H7: race awaiter against cancellation
        let cancel_fut = cancel.cancelled();
        tokio::pin!(cancel_fut);
        let decision_result = tokio::select! {
            d = awaiter => d,
            _ = &mut cancel_fut => {
                let _ = self.approval_store.expire(&approval_id, ExpireReason::Cancelled).await;
                return Err(GatedExecutionError::Cancelled { plan_id: plan_id.clone() });
            }
        };
        let decision = decision_result?;

        // 5. Dispatch on decision
        //
        // Post-decision emits are best-effort: the awaiter has already
        // resolved so the gate has done its work. A closed sink at this
        // point loses host visibility but does not strand the gate —
        // log on false and continue (pre-dispatch approval review fix).
        let warn_if_dropped = |emitted: bool, label: &str| {
            if !emitted {
                tracing::warn!(
                    plan_id = %plan_id,
                    approval_id = %approval_id,
                    event = label,
                    "event sink closed; post-decision event lost to host"
                );
            }
        };
        match decision.outcome {
            agent_fw_core::approval::ApprovalOutcome::Approve => {
                warn_if_dropped(
                    self.sink.emit_approval_decision(decision),
                    "approval_decision(approve)",
                );
                warn_if_dropped(
                    self.sink.emit_plan_status_change(
                        plan_id.as_str(),
                        "pending_approval",
                        "approved",
                    ),
                    "plan_status_change(approved)",
                );
                // Move the plan to Approved and store back so the inner
                // executor sees the canonical state.
                let approved = plan
                    .approve(self.approver.clone())
                    .map_err(|r| GatedExecutionError::Execution(PlanExecutionError::from(r)))?;
                self.kv
                    .put(self.tenant.as_str(), &key, &approved, None)
                    .await
                    .map_err(|e| {
                        GatedExecutionError::Execution(PlanExecutionError::<D::Error>::Kv(e))
                    })?;
                // Delegate the rest of the lifecycle (start → dispatch →
                // complete/fail) to the inner executor's existing flow.
                self.inner
                    .execute(plan_id, ctx)
                    .await
                    .map_err(GatedExecutionError::from)
            }
            agent_fw_core::approval::ApprovalOutcome::Reject => {
                let reason = decision
                    .feedback
                    .clone()
                    .unwrap_or_else(|| "Plan rejected".into());
                warn_if_dropped(
                    self.sink.emit_approval_decision(decision),
                    "approval_decision(reject)",
                );
                warn_if_dropped(
                    self.sink.emit_plan_status_change(
                        plan_id.as_str(),
                        "pending_approval",
                        "draft",
                    ),
                    "plan_status_change(draft)",
                );
                Err(GatedExecutionError::Rejected {
                    plan_id: plan_id.clone(),
                    reason,
                })
            }
            agent_fw_core::approval::ApprovalOutcome::Revise { ref partial } => {
                let partial = partial.clone();
                let feedback = decision.feedback.clone();
                warn_if_dropped(
                    self.sink.emit_approval_decision(decision),
                    "approval_decision(revise)",
                );
                warn_if_dropped(
                    self.sink.emit_plan_status_change(
                        plan_id.as_str(),
                        "pending_approval",
                        "draft",
                    ),
                    "plan_status_change(draft)",
                );
                Err(GatedExecutionError::Revise {
                    plan_id: plan_id.clone(),
                    partial,
                    feedback,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_plan, single_action};
    use agent_fw_core::UserId;
    use agent_fw_test::fixtures::kv::InMemoryKVStore;

    type MemKV = InMemoryKVStore;

    // ─── Mock Dispatcher ──────────────────────────────────────────────

    struct MockDispatcher {
        should_fail: bool,
    }

    #[derive(Debug)]
    struct MockError(String);
    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for MockError {}

    #[async_trait]
    impl ActionDispatcher for MockDispatcher {
        type Action = String;
        type Context = ();
        type Error = MockError;

        async fn dispatch(
            &self,
            actions: &ActionSeq<String>,
            _ctx: &(),
        ) -> Result<ExecutionResult, MockError> {
            if self.should_fail {
                return Err(MockError("dispatch failed".into()));
            }
            Ok(ExecutionResult {
                entities_affected: actions.len(),
                summary: Some("Applied".into()),
                details: None,
            })
        }
    }

    // ─── Tests ────────────────────────────────────────────────────────

    async fn setup_approved_plan(kv: &MemKV, tenant: &TenantId) -> PlanId {
        let plan_id = PlanId::new_unchecked("plan-1");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("price_change".to_string()),
        );
        let plan = plan.approve(UserId::new_unchecked("admin")).unwrap();

        kv.put(tenant.as_str(), &plan_key("plan", &plan_id), &plan, None)
            .await
            .unwrap();

        plan_id
    }

    #[tokio::test]
    async fn execute_happy_path() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = setup_approved_plan(&kv, &tenant).await;

        let dispatcher = MockDispatcher { should_fail: false };
        let executor = PlanExecutor::new(&kv, &tenant, &dispatcher, "plan");

        let result = executor.execute(&plan_id, &()).await.unwrap();
        assert_eq!(result.entities_affected, 1);
        assert_eq!(result.summary.as_deref(), Some("Applied"));

        // Verify plan is now Executed in KV
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Executed);
    }

    #[tokio::test]
    async fn execute_dispatch_failure_transitions_to_failed() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = setup_approved_plan(&kv, &tenant).await;

        let dispatcher = MockDispatcher { should_fail: true };
        let executor = PlanExecutor::new(&kv, &tenant, &dispatcher, "plan");

        let err = executor.execute(&plan_id, &()).await.unwrap_err();
        assert!(matches!(err, PlanExecutionError::Dispatch(_)));

        // Verify plan is now Failed in KV
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Failed);
    }

    #[tokio::test]
    async fn execute_not_found() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let dispatcher = MockDispatcher { should_fail: false };
        let executor = PlanExecutor::new(&kv, &tenant, &dispatcher, "plan");

        let err = executor
            .execute(&PlanId::new_unchecked("nonexistent"), &())
            .await
            .unwrap_err();
        assert!(matches!(err, PlanExecutionError::NotFound(_)));
    }

    #[tokio::test]
    async fn execute_wrong_state() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-draft");

        // Store a Draft plan (not Approved)
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("action".to_string()),
        );
        kv.put(tenant.as_str(), &plan_key("plan", &plan_id), &plan, None)
            .await
            .unwrap();

        let dispatcher = MockDispatcher { should_fail: false };
        let executor = PlanExecutor::new(&kv, &tenant, &dispatcher, "plan");

        let err = executor.execute(&plan_id, &()).await.unwrap_err();
        assert!(matches!(
            err,
            PlanExecutionError::InvalidState(PlanStatus::Draft)
        ));
    }

    // ─── GatedPlanExecutor tests (pre-dispatch approval) ──────────────────────────

    use agent_fw_algebra::approval::{ApprovalError, InMemoryPendingApprovalStore};
    use agent_fw_algebra::{CancellationToken, EventSink};
    use agent_fw_core::approval::ApprovalDecision;
    use agent_fw_core::{StreamPart, ThreadId};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// Recording sink: captures every emitted StreamPart so tests can
    /// extract the UUID `ApprovalId` the gate generated.
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

    /// Poll the sink for an emitted `ApprovalRequired` event and return
    /// its `ApprovalId`. Used by tests that need to resolve the UUID-
    /// based approval the gate generates per attempt (pre-dispatch approval review fix).
    async fn wait_for_plan_approval_id(sink: &Arc<RecordingSink>, timeout_ms: u64) -> ApprovalId {
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

    async fn setup_draft_plan(kv: &MemKV, tenant: &TenantId, id: &str) -> PlanId {
        let plan_id = PlanId::new_unchecked(id);
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("price_change".to_string()),
        );
        kv.put(tenant.as_str(), &plan_key("plan", &plan_id), &plan, None)
            .await
            .unwrap();
        plan_id
    }

    fn build_gated_with_sink<'a>(
        kv: &'a MemKV,
        tenant: &'a TenantId,
        dispatcher: &'a MockDispatcher,
        store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore>,
        sink: Arc<dyn EventSink>,
    ) -> GatedPlanExecutor<'a, String, MockDispatcher> {
        GatedPlanExecutor::new(
            kv,
            tenant,
            dispatcher,
            "plan",
            sink,
            store,
            UserId::new_unchecked("runtime"),
            ThreadId::new_unchecked("th-1"),
        )
    }

    /// Convenience: build a gated executor with a fresh `RecordingSink`
    /// and return both so the test can extract the UUID `ApprovalId`.
    fn build_gated<'a>(
        kv: &'a MemKV,
        tenant: &'a TenantId,
        dispatcher: &'a MockDispatcher,
        store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore>,
    ) -> (
        GatedPlanExecutor<'a, String, MockDispatcher>,
        Arc<RecordingSink>,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let exec = build_gated_with_sink(kv, tenant, dispatcher, store, sink_dyn);
        (exec, sink)
    }

    /// Outcome the caller wants the gate to receive once the awaiter
    /// is registered.
    enum DriveOutcome {
        Approve,
        Reject(String),
        Revise(serde_json::Value),
    }

    /// Helper: drive an inflight executor future to completion while
    /// resolving the approval in parallel. Uses `select!` so we never
    /// need `tokio::spawn` (which would force `'static` bounds).
    ///
    /// The `ApprovalId` is a fresh UUID per call (pre-dispatch approval review fix);
    /// the helper polls the recording sink to recover it dynamically
    /// once the gate has emitted `approval_required`.
    async fn drive_with_decision<'a>(
        executor: &GatedPlanExecutor<'a, String, MockDispatcher>,
        plan_id: &PlanId,
        cancel: &CancellationToken,
        store: &Arc<dyn agent_fw_algebra::approval::PendingApprovalStore>,
        sink: &Arc<RecordingSink>,
        outcome: DriveOutcome,
    ) -> Result<crate::plan::ExecutionResult, GatedExecutionError<MockError>> {
        let exec_fut = executor.execute(plan_id, &(), cancel);
        tokio::pin!(exec_fut);

        // Race the executor against a sink poll; when the approval id
        // appears (= gate has registered + emitted), resolve, then
        // continue to await the executor.
        let resolver = async {
            let id = wait_for_plan_approval_id(sink, 500).await;
            let decision = match outcome {
                DriveOutcome::Approve => ApprovalDecision::approve(id),
                DriveOutcome::Reject(reason) => ApprovalDecision::reject(id, reason),
                DriveOutcome::Revise(partial) => ApprovalDecision::revise(id, partial),
            };
            store
                .resolve(decision)
                .await
                .map_err(GatedExecutionError::Approval)
        };
        tokio::pin!(resolver);

        tokio::select! {
            r = resolver.as_mut() => r?,
            // If the executor completes before the resolver, that's a
            // failure mode for this helper — the gate didn't block.
            r = &mut exec_fut => return r,
        }
        exec_fut.await
    }

    /// pre-dispatch approval: Draft plan execution blocks on the gate until Approve.
    #[tokio::test]
    async fn gated_executor_blocks_on_draft_until_approve() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-gate-1").await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let (executor, sink) = build_gated(&kv, &tenant, &dispatcher, store.clone());
        let cancel = CancellationToken::new();

        let result = drive_with_decision(
            &executor,
            &plan_id,
            &cancel,
            &store,
            &sink,
            DriveOutcome::Approve,
        )
        .await
        .expect("execute Ok after Approve");
        assert_eq!(result.entities_affected, 1);

        // Plan ended up Executed
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Executed);
    }

    /// pre-dispatch approval: Reject leaves the plan in Draft and never dispatches.
    #[tokio::test]
    async fn gated_executor_reject_leaves_plan_in_draft() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-gate-2").await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let (executor, sink) = build_gated(&kv, &tenant, &dispatcher, store.clone());
        let cancel = CancellationToken::new();

        let err = drive_with_decision(
            &executor,
            &plan_id,
            &cancel,
            &store,
            &sink,
            DriveOutcome::Reject("not safe".into()),
        )
        .await
        .expect_err("rejected execute must error");
        match err {
            GatedExecutionError::Rejected { reason, .. } => {
                assert_eq!(reason, "not safe");
            }
            other => panic!("expected Rejected, got {other:?}"),
        }

        // Plan stays in Draft
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Draft);
    }

    /// pre-dispatch approval H6: already-Approved plans skip the gate and go straight
    /// through the inner executor.
    #[tokio::test]
    async fn gated_executor_skips_gate_when_already_approved() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_approved_plan(&kv, &tenant).await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let (executor, _sink) = build_gated(&kv, &tenant, &dispatcher, store);
        let cancel = CancellationToken::new();

        // Should NOT block: pre-Approved plan delegates to inner.
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            executor.execute(&plan_id, &(), &cancel),
        )
        .await
        .expect("must not block")
        .expect("inner executor Ok");
        assert_eq!(result.entities_affected, 1);
    }

    /// pre-dispatch approval H7: cancellation during await releases the store entry
    /// and returns Cancelled error.
    #[tokio::test]
    async fn gated_executor_cancel_releases_store_entry() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-gate-3").await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store_inner = Arc::new(InMemoryPendingApprovalStore::new());
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> = store_inner.clone();
        let (executor, _sink) = build_gated(&kv, &tenant, &dispatcher, store);
        let cancel = CancellationToken::new();

        let exec_fut = executor.execute(&plan_id, &(), &cancel);
        tokio::pin!(exec_fut);
        let delay = tokio::time::sleep(std::time::Duration::from_millis(30));
        tokio::pin!(delay);
        tokio::select! {
            _ = &mut delay => {
                assert_eq!(store_inner.pending_count(), 1, "approval registered");
                cancel.cancel();
            }
            _ = &mut exec_fut => panic!("gate must block before cancel"),
        }
        let err = (&mut exec_fut).await.expect_err("cancelled execute errors");
        assert!(matches!(err, GatedExecutionError::Cancelled { .. }));
        assert_eq!(
            store_inner.pending_count(),
            0,
            "H7: store entry released on cancellation"
        );

        // Plan stays in Draft (not stored back as Approved).
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Draft);
    }

    /// Revise outcome surfaces as `GatedExecutionError::Revise` carrying
    /// the partial — host re-invokes planner with feedback (alpha
    /// "fresh invocation").
    #[tokio::test]
    async fn gated_executor_revise_surfaces_partial() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-gate-4").await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());
        let (executor, sink) = build_gated(&kv, &tenant, &dispatcher, store.clone());
        let cancel = CancellationToken::new();

        let err = drive_with_decision(
            &executor,
            &plan_id,
            &cancel,
            &store,
            &sink,
            DriveOutcome::Revise(serde_json::json!({"horizon_weeks": 4})),
        )
        .await
        .expect_err("revise execute errors");
        match err {
            GatedExecutionError::Revise { partial, .. } => {
                assert_eq!(partial["horizon_weeks"], 4);
            }
            other => panic!("expected Revise, got {other:?}"),
        }
    }

    /// Approval-store error propagates as a `GatedExecutionError::Approval`.
    /// The deterministic-ID collision path is no longer reachable in
    /// normal use (UUIDs per attempt — pre-dispatch approval review fix), so this test
    /// injects the collision manually to verify error propagation still
    /// works for any future store-side error.
    #[tokio::test]
    async fn gated_executor_propagates_store_error() {
        // Set up a draft plan and a store where the gate's freshly-
        // minted UUID is *guaranteed* to collide. We do that by wrapping
        // the in-memory store in a one-shot collider that pre-registers
        // any first ApprovalId it sees, then rejects the gate's actual
        // attempt with `AlreadyRegistered`.
        use agent_fw_algebra::approval::{ApprovalAwait, PendingApprovalStore};
        use agent_fw_core::approval::ApprovalRequest;

        struct CollidingStore {
            inner: InMemoryPendingApprovalStore,
            tripped: AtomicBool,
        }
        impl CollidingStore {
            fn new() -> Self {
                Self {
                    inner: InMemoryPendingApprovalStore::new(),
                    tripped: AtomicBool::new(false),
                }
            }
        }
        #[async_trait::async_trait]
        impl PendingApprovalStore for CollidingStore {
            async fn register(&self, req: ApprovalRequest) -> Result<ApprovalAwait, ApprovalError> {
                // First call: pre-register a colliding id to trip the
                // store's AlreadyRegistered branch, then forward the
                // real register so the gate sees the error.
                if !self.tripped.swap(true, Ordering::SeqCst) {
                    let dup = ApprovalRequest {
                        id: req.id.clone(),
                        ..req.clone()
                    };
                    let _ = self.inner.register(dup).await;
                }
                self.inner.register(req).await
            }
            async fn resolve(&self, decision: ApprovalDecision) -> Result<(), ApprovalError> {
                self.inner.resolve(decision).await
            }
            async fn expire(
                &self,
                id: &ApprovalId,
                reason: agent_fw_algebra::approval::ExpireReason,
            ) -> Result<(), ApprovalError> {
                self.inner.expire(id, reason).await
            }
            async fn get(&self, id: &ApprovalId) -> Result<Option<ApprovalRequest>, ApprovalError> {
                self.inner.get(id).await
            }
        }

        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-gate-5").await;
        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(CollidingStore::new());
        let (executor, _sink) = build_gated(&kv, &tenant, &dispatcher, store);
        let cancel = CancellationToken::new();
        let err = executor
            .execute(&plan_id, &(), &cancel)
            .await
            .expect_err("must error");
        match err {
            GatedExecutionError::Approval(ApprovalError::AlreadyRegistered(_)) => {}
            other => panic!("expected Approval(AlreadyRegistered), got {other:?}"),
        }
    }

    /// pre-dispatch approval review fix: a rejected plan that stays in Draft can be
    /// re-executed and reach a fresh approval gate without colliding
    /// with the store's permanent resolved-set. This is the bug the
    /// deterministic-ID fix addresses.
    #[tokio::test]
    async fn gated_executor_retry_after_reject_succeeds() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-retry").await;

        let dispatcher = MockDispatcher { should_fail: false };
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> =
            Arc::new(InMemoryPendingApprovalStore::new());

        // First attempt: reject.
        {
            let (executor, sink) = build_gated(&kv, &tenant, &dispatcher, store.clone());
            let cancel = CancellationToken::new();
            let err = drive_with_decision(
                &executor,
                &plan_id,
                &cancel,
                &store,
                &sink,
                DriveOutcome::Reject("first attempt rejected".into()),
            )
            .await
            .expect_err("first attempt must reject");
            assert!(matches!(err, GatedExecutionError::Rejected { .. }));
        }
        // Plan still Draft, so retry is meaningful.
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Draft);

        // Second attempt: approve. Must succeed even though the store's
        // resolved-set retains the first attempt's ApprovalId.
        let (executor, sink) = build_gated(&kv, &tenant, &dispatcher, store.clone());
        let cancel = CancellationToken::new();
        let result = drive_with_decision(
            &executor,
            &plan_id,
            &cancel,
            &store,
            &sink,
            DriveOutcome::Approve,
        )
        .await
        .expect("retry after reject must succeed");
        assert_eq!(result.entities_affected, 1);

        // Verify each attempt got a distinct ApprovalId (UUIDs).
        let approval_events: Vec<_> = sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StreamPart::ApprovalRequired { data } => Some(data.id),
                _ => None,
            })
            .collect();
        // The retry sink only has the *second* attempt's emit, so length
        // is 1; the asserting power is the lack of `AlreadyRegistered`
        // error from the second drive.
        assert_eq!(approval_events.len(), 1);
        assert_ne!(approval_events[0].as_str(), plan_id.as_str());
    }

    /// pre-dispatch approval review fix: a closed event sink between `register` and
    /// `emit_approval_required` must surface as
    /// `GatedExecutionError::EventSinkClosed` and expire the store entry
    /// — not hang forever.
    #[tokio::test]
    async fn gated_executor_emit_failure_expires_and_errors() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = setup_draft_plan(&kv, &tenant, "plan-emit-fail").await;
        let dispatcher = MockDispatcher { should_fail: false };
        let store_inner = Arc::new(InMemoryPendingApprovalStore::new());
        let store: Arc<dyn agent_fw_algebra::approval::PendingApprovalStore> = store_inner.clone();

        // Pre-closed sink: every emit returns false.
        let sink = Arc::new(RecordingSink::new());
        sink.close();
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let executor = build_gated_with_sink(&kv, &tenant, &dispatcher, store.clone(), sink_dyn);
        let cancel = CancellationToken::new();

        let err = executor
            .execute(&plan_id, &(), &cancel)
            .await
            .expect_err("must error on closed sink");
        assert!(matches!(err, GatedExecutionError::EventSinkClosed { .. }));

        // Store entry was expired — no leak.
        assert_eq!(store_inner.pending_count(), 0);
        // Plan stays in Draft.
        let stored: Plan<String> = kv
            .get(tenant.as_str(), &plan_key("plan", &plan_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PlanStatus::Draft);
    }
}
