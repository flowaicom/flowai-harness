//! Plan KV persistence helpers.
//!
//! Provides standalone functions for persisting and loading plans,
//! and for performing state transitions atomically through KV.
//!
//! The [`PlanExecutor`](crate::executor::PlanExecutor) handles the full
//! execute lifecycle. These helpers are for the other lifecycle phases:
//! creation, approval, and ad-hoc transitions.
//!
//! # Laws
//!
//! - **L1 (Round-trip)**: `persist_plan(plan); load_plan(id) == Some(plan)`
//! - **L2 (Tenant scoping)**: Plans are scoped by tenant — `persist(t1, plan); load(t2, id) == None`
//! - **L3 (Status monotonicity)**: Transitions only advance forward in the state machine.

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};
use agent_fw_core::{PlanId, TenantId, UserId};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

use crate::plan::{ExecutionResult, Plan, PlanError, PlanStatus, TransitionError};

// ─── Plan Transition Observer ─────────────────────────────────────────

/// Observer for plan state transitions.
///
/// Enables audit logging, notifications, and custom side-effects when
/// plans move through their lifecycle. The framework calls the appropriate
/// method after each successful transition and persistence.
///
/// # Laws
///
/// - **L1 (Non-blocking)**: Methods should not block the transition.
/// - **L2 (Infallibility)**: Observer errors are logged but do not fail the transition.
/// - **L3 (Post-commit)**: Called after the transition is persisted, not before.
pub trait PlanTransitionObserver: Send + Sync {
    /// Called after a successful state transition.
    fn on_transition(&self, plan_id: &PlanId, tenant: &TenantId, from: PlanStatus, to: PlanStatus);
}

/// No-op observer — monoid identity for plan observation.
pub struct NullPlanObserver;

impl PlanTransitionObserver for NullPlanObserver {
    fn on_transition(&self, _: &PlanId, _: &TenantId, _: PlanStatus, _: PlanStatus) {}
}

/// Logging observer — emits tracing events for each transition.
pub struct LoggingPlanObserver;

impl PlanTransitionObserver for LoggingPlanObserver {
    fn on_transition(&self, plan_id: &PlanId, tenant: &TenantId, from: PlanStatus, to: PlanStatus) {
        tracing::info!(
            plan_id = plan_id.as_str(),
            tenant = tenant.as_str(),
            from = %from,
            to = %to,
            "plan transition"
        );
    }
}

/// Composed observer — chains two observers (monoid append).
pub struct ComposedPlanObserver {
    first: Box<dyn PlanTransitionObserver>,
    second: Box<dyn PlanTransitionObserver>,
}

impl ComposedPlanObserver {
    pub fn new(
        first: Box<dyn PlanTransitionObserver>,
        second: Box<dyn PlanTransitionObserver>,
    ) -> Self {
        Self { first, second }
    }
}

impl PlanTransitionObserver for ComposedPlanObserver {
    fn on_transition(&self, plan_id: &PlanId, tenant: &TenantId, from: PlanStatus, to: PlanStatus) {
        self.first.on_transition(plan_id, tenant, from, to);
        self.second.on_transition(plan_id, tenant, from, to);
    }
}

/// Default plan TTL (7 days — matches `agent_fw_algebra::kv_store::SESSION_DATA_TTL`).
pub const PLAN_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Default KV prefix for plans.
pub const PLAN_PREFIX: &str = "plan";

/// Borrowed plan persistence context.
///
/// This packages the recurring plan-persistence policy pieces:
/// - KV backend
/// - tenant scope
/// - plan key prefix
/// - default TTL for persisted transitions
///
/// Framework callers that already own these inputs can depend on this single
/// value instead of repeatedly threading `(kv, tenant, prefix, ttl)` through
/// each lifecycle call.
#[derive(Clone, Copy)]
pub struct PlanStore<'a> {
    kv: &'a (dyn KVStore + 'a),
    tenant: &'a TenantId,
    prefix: &'a str,
    ttl: Duration,
}

/// Errors from plan persistence operations.
#[derive(Debug, thiserror::Error)]
pub enum PlanPersistError {
    #[error("Plan not found")]
    NotFound,
    #[error("Invalid transition: {0}")]
    Transition(#[from] TransitionError),
    #[error("KV error: {0}")]
    Kv(#[from] KVError),
}

/// Convert a `Rejected<A>` into a `PlanPersistError` by extracting the error.
///
/// This preserves `?` ergonomics at callsites: `plan.approve(user)?` works
/// when the surrounding function returns `Result<_, PlanPersistError>`.
impl<A> From<crate::plan::Rejected<A>> for PlanPersistError {
    fn from(r: crate::plan::Rejected<A>) -> Self {
        Self::Transition(r.error)
    }
}

/// Build the KV key for a plan.
pub fn plan_key(prefix: &str, plan_id: &PlanId) -> String {
    format!("{prefix}:{}", plan_id.as_str())
}

impl<'a> PlanStore<'a> {
    pub fn new(kv: &'a (dyn KVStore + 'a), tenant: &'a TenantId) -> Self {
        Self {
            kv,
            tenant,
            prefix: PLAN_PREFIX,
            ttl: PLAN_TTL,
        }
    }

    pub fn with_prefix(mut self, prefix: &'a str) -> Self {
        self.prefix = prefix;
        self
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn kv(&self) -> &'a (dyn KVStore + 'a) {
        self.kv
    }

    pub fn tenant(&self) -> &'a TenantId {
        self.tenant
    }

    pub fn prefix(&self) -> &'a str {
        self.prefix
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn key(&self, plan_id: &PlanId) -> String {
        plan_key(self.prefix, plan_id)
    }

    pub async fn persist<A: Serialize + Send + Sync>(
        &self,
        plan: &Plan<A>,
    ) -> Result<(), PlanPersistError> {
        persist_plan_with_prefix(self.kv, self.tenant, plan, self.prefix, self.ttl).await
    }

    pub async fn load<A: DeserializeOwned + Send>(
        &self,
        plan_id: &PlanId,
    ) -> Result<Option<Plan<A>>, PlanPersistError> {
        load_plan_with_prefix(self.kv, self.tenant, plan_id, self.prefix).await
    }

    pub async fn load_required<A: DeserializeOwned + Send>(
        &self,
        plan_id: &PlanId,
    ) -> Result<Plan<A>, PlanPersistError> {
        load_required(self.kv, self.tenant, plan_id, self.prefix).await
    }

    pub async fn approve<A: Serialize + DeserializeOwned + Send + Sync>(
        &self,
        plan_id: &PlanId,
        approved_by: UserId,
    ) -> Result<Plan<A>, PlanPersistError> {
        approve_plan_in_kv(self.kv, self.tenant, plan_id, approved_by, self.ttl).await
    }

    pub async fn start<A: Serialize + DeserializeOwned + Send + Sync>(
        &self,
        plan_id: &PlanId,
    ) -> Result<Plan<A>, PlanPersistError> {
        start_plan_in_kv(self.kv, self.tenant, plan_id, self.ttl).await
    }

    pub async fn complete<A: Serialize + DeserializeOwned + Send + Sync>(
        &self,
        plan_id: &PlanId,
        result: ExecutionResult,
    ) -> Result<Plan<A>, PlanPersistError> {
        complete_plan_in_kv(self.kv, self.tenant, plan_id, result, self.ttl).await
    }

    pub async fn fail<A: Serialize + DeserializeOwned + Send + Sync>(
        &self,
        plan_id: &PlanId,
        error: PlanError,
    ) -> Result<Plan<A>, PlanPersistError> {
        fail_plan_in_kv(self.kv, self.tenant, plan_id, error, self.ttl).await
    }

    pub async fn complete_or_recover<A: Serialize + DeserializeOwned + Send + Sync>(
        &self,
        plan_id: &PlanId,
        result: ExecutionResult,
    ) -> Result<Plan<A>, PlanPersistError> {
        complete_or_recover_in_kv(self.kv, self.tenant, plan_id, result, self.ttl).await
    }
}

/// Persist a plan to KV store.
///
/// Generic over `K: KVStore + ?Sized` so callers can pass either a concrete
/// KV implementation or `&dyn KVStore` (trait object). This removes the need
/// for sized newtype wrappers when bridging `ToolEnvironment::kv()`.
pub async fn persist_plan<A: Serialize + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan: &Plan<A>,
    ttl: Duration,
) -> Result<(), PlanPersistError> {
    persist_plan_with_prefix(kv, tenant, plan, PLAN_PREFIX, ttl).await
}

/// Persist a plan to KV store with a custom prefix.
pub async fn persist_plan_with_prefix<A: Serialize + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan: &Plan<A>,
    prefix: &str,
    ttl: Duration,
) -> Result<(), PlanPersistError> {
    let key = plan_key(prefix, &plan.id);
    kv.put(tenant.as_str(), &key, plan, Some(ttl)).await?;
    Ok(())
}

/// Load a plan from KV store.
pub async fn load_plan<A: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
) -> Result<Option<Plan<A>>, PlanPersistError> {
    load_plan_with_prefix(kv, tenant, plan_id, PLAN_PREFIX).await
}

/// Load a plan from KV store with a custom prefix.
pub async fn load_plan_with_prefix<A: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    prefix: &str,
) -> Result<Option<Plan<A>>, PlanPersistError> {
    let key = plan_key(prefix, plan_id);
    let plan = kv.get::<Plan<A>>(tenant.as_str(), &key).await?;
    Ok(plan)
}

/// Load a plan, returning `PlanPersistError::NotFound` if absent.
async fn load_required<A: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    prefix: &str,
) -> Result<Plan<A>, PlanPersistError> {
    load_plan_with_prefix(kv, tenant, plan_id, prefix)
        .await?
        .ok_or(PlanPersistError::NotFound)
}

/// Load → approve → persist.
///
/// Only advances Draft → Approved. Returns the updated plan.
pub async fn approve_plan_in_kv<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    approved_by: UserId,
    ttl: Duration,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = load_required::<A>(kv, tenant, plan_id, PLAN_PREFIX).await?;
    let plan = plan.approve(approved_by)?;
    persist_plan(kv, tenant, &plan, ttl).await?;
    Ok(plan)
}

/// Load → start → persist.
///
/// Only advances Approved → Executing. Returns the updated plan.
pub async fn start_plan_in_kv<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    ttl: Duration,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = load_required::<A>(kv, tenant, plan_id, PLAN_PREFIX).await?;
    let plan = plan.start()?;
    persist_plan(kv, tenant, &plan, ttl).await?;
    Ok(plan)
}

/// Load → complete → persist.
///
/// Only advances Executing → Executed. Returns the updated plan.
pub async fn complete_plan_in_kv<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    result: ExecutionResult,
    ttl: Duration,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = load_required::<A>(kv, tenant, plan_id, PLAN_PREFIX).await?;
    let plan = plan.complete(result)?;
    persist_plan(kv, tenant, &plan, ttl).await?;
    Ok(plan)
}

/// Load → fail → persist.
///
/// Only advances Executing → Failed. Returns the updated plan.
pub async fn fail_plan_in_kv<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    error: PlanError,
    ttl: Duration,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = load_required::<A>(kv, tenant, plan_id, PLAN_PREFIX).await?;
    let plan = plan.fail(error)?;
    persist_plan(kv, tenant, &plan, ttl).await?;
    Ok(plan)
}

// ─── Recovery Pattern ─────────────────────────────────────────────────

/// Try to complete a plan; on failure, transition to Failed instead.
///
/// This is the recovery pattern: a plan in Executing state should never
/// stay stuck. If `complete` fails (e.g. serialization, KV write), we
/// attempt `fail` so the plan reaches a terminal state.
///
/// Returns `Ok(plan)` on successful completion, or `Err` if both
/// complete AND fail transitions failed (the only truly unrecoverable case).
pub async fn complete_or_recover_in_kv<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    result: ExecutionResult,
    ttl: Duration,
) -> Result<Plan<A>, PlanPersistError> {
    match complete_plan_in_kv(kv, tenant, plan_id, result, ttl).await {
        Ok(plan) => Ok(plan),
        Err(complete_err) => {
            tracing::warn!(
                plan_id = plan_id.as_str(),
                error = %complete_err,
                "complete_plan failed, recovering to Failed state"
            );
            let plan_error = PlanError::new(format!("Completion failed: {complete_err}"));
            fail_plan_in_kv(kv, tenant, plan_id, plan_error, ttl)
                .await
                .map_err(|fail_err| {
                    tracing::error!(
                        plan_id = plan_id.as_str(),
                        complete_error = %complete_err,
                        fail_error = %fail_err,
                        "both complete and fail transitions failed"
                    );
                    fail_err
                })
        }
    }
}

// ─── Observed Transition Functions ─────────────────────────────────────

/// Like [`approve_plan_in_kv`] but notifies `observer` after successful transition.
pub async fn approve_plan_observed<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    approved_by: UserId,
    ttl: Duration,
    observer: &dyn PlanTransitionObserver,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = approve_plan_in_kv(kv, tenant, plan_id, approved_by, ttl).await?;
    observer.on_transition(plan_id, tenant, PlanStatus::Draft, PlanStatus::Approved);
    Ok(plan)
}

/// Like [`start_plan_in_kv`] but notifies `observer` after successful transition.
pub async fn start_plan_observed<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    ttl: Duration,
    observer: &dyn PlanTransitionObserver,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = start_plan_in_kv(kv, tenant, plan_id, ttl).await?;
    observer.on_transition(plan_id, tenant, PlanStatus::Approved, PlanStatus::Executing);
    Ok(plan)
}

/// Like [`complete_plan_in_kv`] but notifies `observer` after successful transition.
pub async fn complete_plan_observed<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    result: ExecutionResult,
    ttl: Duration,
    observer: &dyn PlanTransitionObserver,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = complete_plan_in_kv(kv, tenant, plan_id, result, ttl).await?;
    observer.on_transition(plan_id, tenant, PlanStatus::Executing, PlanStatus::Executed);
    Ok(plan)
}

/// Like [`fail_plan_in_kv`] but notifies `observer` after successful transition.
pub async fn fail_plan_observed<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    error: PlanError,
    ttl: Duration,
    observer: &dyn PlanTransitionObserver,
) -> Result<Plan<A>, PlanPersistError> {
    let plan = fail_plan_in_kv(kv, tenant, plan_id, error, ttl).await?;
    observer.on_transition(plan_id, tenant, PlanStatus::Executing, PlanStatus::Failed);
    Ok(plan)
}

/// Like [`complete_or_recover_in_kv`] but notifies `observer` after the
/// terminal transition (Executed on success, Failed on recovery).
pub async fn complete_or_recover_observed<A: Serialize + DeserializeOwned + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant: &TenantId,
    plan_id: &PlanId,
    result: ExecutionResult,
    ttl: Duration,
    observer: &dyn PlanTransitionObserver,
) -> Result<Plan<A>, PlanPersistError> {
    match complete_or_recover_in_kv(kv, tenant, plan_id, result, ttl).await {
        Ok(plan) => {
            observer.on_transition(plan_id, tenant, PlanStatus::Executing, plan.status);
            Ok(plan)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_plan, single_action, PlanStatus};
    use agent_fw_test::fixtures::kv::InMemoryKVStore;

    type MemKV = InMemoryKVStore;

    // ─── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn plan_round_trip() {
        // L1: persist → load returns the same plan
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-rt");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("action-a".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();
        let loaded: Plan<String> = load_plan(&kv, &tenant, &plan_id).await.unwrap().unwrap();

        assert_eq!(loaded.id, plan.id);
        assert_eq!(loaded.status, PlanStatus::Draft);
        assert_eq!(loaded.owner, tenant);
    }

    #[tokio::test]
    async fn plan_store_round_trip() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-store");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("action-a".to_string()),
        );

        let store = PlanStore::new(&kv, &tenant);
        store.persist(&plan).await.unwrap();

        let loaded: Plan<String> = store.load(&plan_id).await.unwrap().unwrap();
        assert_eq!(loaded.id, plan.id);
        assert_eq!(store.key(&plan_id), "plan:plan-store");
    }

    #[tokio::test]
    async fn plan_store_uses_configured_prefix_and_ttl() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-custom");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("action-a".to_string()),
        );

        let store = PlanStore::new(&kv, &tenant)
            .with_prefix("review-plan")
            .with_ttl(Duration::from_secs(60));

        store.persist(&plan).await.unwrap();
        let loaded: Plan<String> = store.load(&plan_id).await.unwrap().unwrap();

        assert_eq!(loaded.id, plan.id);
        assert_eq!(store.key(&plan_id), "review-plan:plan-custom");
        assert_eq!(store.ttl(), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn plan_tenant_scoping() {
        // L2: plans are scoped by tenant
        let kv = MemKV::new();
        let t1 = TenantId::new_unchecked("tenant-a");
        let t2 = TenantId::new_unchecked("tenant-b");
        let plan_id = PlanId::new_unchecked("plan-scope");
        let plan = create_plan(plan_id.clone(), t1.clone(), single_action("x".to_string()));

        persist_plan(&kv, &t1, &plan, PLAN_TTL).await.unwrap();

        // Loading from t2 should return None
        let loaded: Option<Plan<String>> = load_plan(&kv, &t2, &plan_id).await.unwrap();
        assert!(loaded.is_none());

        // Loading from t1 should succeed
        let loaded: Option<Plan<String>> = load_plan(&kv, &t1, &plan_id).await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn plan_approve_transition() {
        // L3: approve advances Draft → Approved
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-approve");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("act".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();

        let approved: Plan<String> = approve_plan_in_kv(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await
        .unwrap();

        assert_eq!(approved.status, PlanStatus::Approved);
        assert!(approved.approved_at.is_some());
        assert_eq!(approved.approved_by, Some(UserId::new_unchecked("admin")));

        // Verify persisted in KV
        let loaded: Plan<String> = load_plan(&kv, &tenant, &plan_id).await.unwrap().unwrap();
        assert_eq!(loaded.status, PlanStatus::Approved);
    }

    #[tokio::test]
    async fn plan_approve_not_found() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let result = approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &PlanId::new_unchecked("nonexistent"),
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await;
        assert!(matches!(result, Err(PlanPersistError::NotFound)));
    }

    #[tokio::test]
    async fn plan_approve_wrong_status() {
        // Cannot approve an already-approved plan
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-double");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("act".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();

        // First approve succeeds
        approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await
        .unwrap();

        // Second approve fails (already Approved, not Draft)
        let result = approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await;
        assert!(matches!(result, Err(PlanPersistError::Transition(_))));
    }

    #[tokio::test]
    async fn plan_full_lifecycle() {
        // Draft → Approved → Executing → Executed
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-lifecycle");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("lifecycle-action".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();

        // Approve
        approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await
        .unwrap();

        // Start
        let started: Plan<String> = start_plan_in_kv(&kv, &tenant, &plan_id, PLAN_TTL)
            .await
            .unwrap();
        assert_eq!(started.status, PlanStatus::Executing);
        assert!(started.started_at.is_some());

        // Complete
        let result = ExecutionResult {
            entities_affected: 42,
            summary: Some("Done".into()),
            details: None,
        };
        let completed: Plan<String> = complete_plan_in_kv(&kv, &tenant, &plan_id, result, PLAN_TTL)
            .await
            .unwrap();
        assert_eq!(completed.status, PlanStatus::Executed);
        assert!(completed.completed_at.is_some());
    }

    #[tokio::test]
    async fn plan_fail_lifecycle() {
        // Draft → Approved → Executing → Failed
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-fail");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("fail-action".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();

        approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await
        .unwrap();

        start_plan_in_kv::<String>(&kv, &tenant, &plan_id, PLAN_TTL)
            .await
            .unwrap();

        let error = PlanError::new("something went wrong");
        let failed: Plan<String> = fail_plan_in_kv(&kv, &tenant, &plan_id, error, PLAN_TTL)
            .await
            .unwrap();
        assert_eq!(failed.status, PlanStatus::Failed);
        assert!(failed.failed_at.is_some());
    }

    #[tokio::test]
    async fn start_requires_approved() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-start-fail");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("x".to_string()),
        );

        // Persist as Draft — starting should fail
        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();
        let result = start_plan_in_kv::<String>(&kv, &tenant, &plan_id, PLAN_TTL).await;
        assert!(matches!(result, Err(PlanPersistError::Transition(_))));
    }

    #[tokio::test]
    async fn complete_or_recover_happy_path() {
        // complete_or_recover succeeds → plan is Executed
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-recover-ok");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("act".to_string()),
        );

        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();
        approve_plan_in_kv::<String>(
            &kv,
            &tenant,
            &plan_id,
            UserId::new_unchecked("admin"),
            PLAN_TTL,
        )
        .await
        .unwrap();
        start_plan_in_kv::<String>(&kv, &tenant, &plan_id, PLAN_TTL)
            .await
            .unwrap();

        let result = ExecutionResult {
            entities_affected: 10,
            summary: Some("Done".into()),
            details: None,
        };
        let plan: Plan<String> =
            complete_or_recover_in_kv(&kv, &tenant, &plan_id, result, PLAN_TTL)
                .await
                .unwrap();
        assert_eq!(plan.status, PlanStatus::Executed);
    }

    #[tokio::test]
    async fn complete_or_recover_falls_back_to_failed() {
        // When complete fails (plan not in Executing state), recovery
        // also fails — because the plan is in a state that can't be failed.
        // This tests that the function propagates the error correctly.
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let plan_id = PlanId::new_unchecked("plan-recover-fail");
        let plan = create_plan(
            plan_id.clone(),
            tenant.clone(),
            single_action("act".to_string()),
        );

        // Persist as Draft — cannot complete OR fail from Draft
        persist_plan(&kv, &tenant, &plan, PLAN_TTL).await.unwrap();

        let result = ExecutionResult {
            entities_affected: 0,
            summary: None,
            details: None,
        };
        let err = complete_or_recover_in_kv::<String>(&kv, &tenant, &plan_id, result, PLAN_TTL)
            .await
            .unwrap_err();
        // Both complete and fail failed → returns the fail error
        assert!(matches!(err, PlanPersistError::Transition(_)));
    }
}
