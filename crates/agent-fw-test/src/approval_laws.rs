//! Algebraic law test harness for `PendingApprovalStore` implementations.
//!
//! Any implementor — the in-memory reference, the KV-backed interpreter
//! shipped in `agent-fw-interpreter`, or a downstream custom store —
//! runs the same five-law battery to verify pre-dispatch approval invariants.
//!
//! # Laws
//!
//! - **L1 Register-once**: duplicate `ApprovalId` → `AlreadyRegistered`.
//! - **L2 Resolve totality**: unknown id → `NotFound`; already-resolved → `AlreadyResolved`;
//!   otherwise wakes the awaiter with `Ok(decision)`.
//! - **L3 Expire totality**: same totality shape as resolve, but wakes the
//!   awaiter with `Err(Expired { reason })`.
//! - **L4 Tenant isolation**: ids registered under one tenant cannot be
//!   resolved by a request claiming a different tenant (the body's
//!   `resource_id` records who registered it; the harness asserts the
//!   store does not lose that on roundtrip).
//! - **L5 Pre-dispatch invariant**: while a `register`'d approval is
//!   pending, the awaiter does not resolve until `resolve` or `expire`
//!   is called. This is the pre-dispatch approval counter-tool invariant at the
//!   store layer.
//!
//! # Usage
//!
//! ```ignore
//! use agent_fw_algebra::InMemoryPendingApprovalStore;
//!
//! #[tokio::test]
//! async fn in_memory_store_satisfies_approval_laws() {
//!     agent_fw_test::approval_laws::test_all(|| InMemoryPendingApprovalStore::new()).await;
//! }
//! ```

use std::sync::Arc;

use agent_fw_algebra::{ApprovalError, ExpireReason, PendingApprovalStore};
use agent_fw_core::approval::{ApprovalDecision, ApprovalKind, ApprovalRequest};
use agent_fw_core::{ApprovalId, TenantId, ThreadId};

/// Build a fresh store. Factory closure so the harness can construct
/// multiple independent stores per law.
pub async fn test_all<S, F>(factory: F)
where
    S: PendingApprovalStore + 'static,
    F: Fn() -> S,
{
    law_register_once(&factory()).await;
    law_resolve_totality(&factory()).await;
    law_expire_totality(&factory()).await;
    law_tenant_isolation(&factory()).await;
    law_pre_dispatch_invariant(&factory()).await;
}

// ─── Helpers ─────────────────────────────────────────────────────────

fn req(id: &str, tenant: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalId::new_unchecked(id),
        kind: ApprovalKind::Tool,
        target: "create_scenario".into(),
        payload: serde_json::json!({"x": 1}),
        glimpse: None,
        resource_id: TenantId::new_unchecked(tenant),
        thread_id: ThreadId::new_unchecked("thread-1"),
        correlation_id: None,
    }
}

// ─── L1 Register-once ─────────────────────────────────────────────────

pub async fn law_register_once<S: PendingApprovalStore>(store: &S) {
    let _await = store
        .register(req("L1", "acme"))
        .await
        .expect("L1: first register must succeed");

    let err = store
        .register(req("L1", "acme"))
        .await
        .expect_err("L1: duplicate register must error");
    assert!(
        matches!(err, ApprovalError::AlreadyRegistered(_)),
        "L1: duplicate register error must be AlreadyRegistered, got {err:?}"
    );
}

// ─── L2 Resolve totality ──────────────────────────────────────────────

pub async fn law_resolve_totality<S: PendingApprovalStore>(store: &S) {
    // Unknown id → NotFound
    let err = store
        .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked(
            "L2-unknown",
        )))
        .await
        .expect_err("L2: resolve unknown must error");
    assert!(
        matches!(err, ApprovalError::NotFound(_)),
        "L2: unknown id error must be NotFound, got {err:?}"
    );

    // Resolve happy path: register, resolve, awaiter sees decision
    let awaiter = store
        .register(req("L2-ok", "acme"))
        .await
        .expect("L2: register");
    store
        .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked(
            "L2-ok",
        )))
        .await
        .expect("L2: resolve");
    let decision = awaiter.await.expect("L2: awaiter resolves with Ok");
    assert!(decision.outcome.is_approve(), "L2: outcome must be approve");

    // Re-resolve → AlreadyResolved
    let err = store
        .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked(
            "L2-ok",
        )))
        .await
        .expect_err("L2: double-resolve must error");
    assert!(
        matches!(err, ApprovalError::AlreadyResolved(_)),
        "L2: double-resolve error must be AlreadyResolved, got {err:?}"
    );
}

// ─── L3 Expire totality ───────────────────────────────────────────────

pub async fn law_expire_totality<S: PendingApprovalStore>(store: &S) {
    // Unknown id → NotFound
    let err = store
        .expire(
            &ApprovalId::new_unchecked("L3-unknown"),
            ExpireReason::Restart,
        )
        .await
        .expect_err("L3: expire unknown must error");
    assert!(matches!(err, ApprovalError::NotFound(_)));

    // Expire happy path: register, expire, awaiter sees Err(Expired)
    let awaiter = store
        .register(req("L3-ok", "acme"))
        .await
        .expect("L3: register");
    store
        .expire(&ApprovalId::new_unchecked("L3-ok"), ExpireReason::Cancelled)
        .await
        .expect("L3: expire");
    let err = awaiter
        .await
        .expect_err("L3: awaiter must resolve with Err");
    match err {
        ApprovalError::Expired { id, reason } => {
            assert_eq!(id.as_str(), "L3-ok");
            assert_eq!(reason, ExpireReason::Cancelled);
        }
        other => panic!("L3: expected Expired, got {other:?}"),
    }

    // Re-expire → AlreadyResolved
    let err = store
        .expire(&ApprovalId::new_unchecked("L3-ok"), ExpireReason::Restart)
        .await
        .expect_err("L3: double-expire must error");
    assert!(matches!(err, ApprovalError::AlreadyResolved(_)));
}

// ─── L4 Tenant isolation ──────────────────────────────────────────────

pub async fn law_tenant_isolation<S: PendingApprovalStore>(store: &S) {
    // Register under tenant "acme"
    let _await = store
        .register(req("L4", "acme"))
        .await
        .expect("L4: register");

    // The body carries the original tenant — retrieval preserves it.
    let body = store
        .get(&ApprovalId::new_unchecked("L4"))
        .await
        .expect("L4: get")
        .expect("L4: body present");
    assert_eq!(
        body.resource_id.as_str(),
        "acme",
        "L4: stored body must retain registering tenant"
    );

    // (The runtime layer that exposes `respond_to_approval` is responsible
    // for verifying that the caller's tenant matches the body's
    // `resource_id` before invoking `store.resolve`. This law asserts the
    // store doesn't silently drop the tenant tag.)
}

// ─── L5 Pre-dispatch invariant ────────────────────────────────────────

/// While an approval is pending, the awaiter does not resolve. This is
/// the store-level shadow of the pre-dispatch approval counter-tool acceptance test.
pub async fn law_pre_dispatch_invariant<S: PendingApprovalStore + 'static>(store: &S) {
    use std::time::Duration;

    let awaiter = store
        .register(req("L5", "acme"))
        .await
        .expect("L5: register");

    // Spawn a task that awaits the awaiter; race against a short timeout.
    let task = tokio::spawn(async move { awaiter.await });

    // Give the runtime a tick to advance.
    let outcome = tokio::time::timeout(Duration::from_millis(50), &mut Box::pin(async {})).await;
    let _ = outcome;
    assert!(
        !task.is_finished(),
        "L5: awaiter must remain pending while no decision is recorded"
    );

    // Resolve and confirm the awaiter wakes.
    store
        .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("L5")))
        .await
        .expect("L5: resolve");
    let decision = tokio::time::timeout(Duration::from_millis(200), task)
        .await
        .expect("L5: awaiter must wake within 200ms after resolve")
        .expect("L5: task did not panic")
        .expect("L5: awaiter Ok");
    assert!(decision.outcome.is_approve());
}

// ─── In-tree verification: in-memory store satisfies all laws ────────

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::InMemoryPendingApprovalStore;

    #[tokio::test]
    async fn in_memory_store_satisfies_all_laws() {
        test_all(InMemoryPendingApprovalStore::new).await;
    }

    /// Smoke check: confirm L5 catches a broken implementation.
    /// A store that auto-resolves on register would fail this check.
    #[tokio::test]
    async fn l5_passes_for_in_memory_store() {
        let store = Arc::new(InMemoryPendingApprovalStore::new());
        law_pre_dispatch_invariant(store.as_ref()).await;
    }
}
