//! KV-backed pending approval store interpreter (pre-dispatch approval).
//!
//! Composes the in-memory [`InMemoryPendingApprovalStore`] (which holds
//! the per-request `oneshot::Sender` awaiters in a `DashMap`) with a
//! [`KVStore`] writethrough that records the request body and final
//! status for audit and host-visible inspection.
//!
//! # Design (the durability tier in pre-dispatch approval)
//!
//! The in-memory awaiter map is the **source of truth** for whether a
//! gate is suspended. The KV cache is a **write-through audit log** so
//! the host can:
//!   - Inspect a pending approval out-of-band (`runtime.respond_to_approval`
//!     can verify the request id is real before calling `resolve`).
//!   - Observe resolved decisions and expired tombstones after the fact.
//!
//! # H5 — Process restart durability
//!
//! In-process awaiters do not survive a restart. The KV bodies do, but
//! the original tool-call future that registered them is gone — the
//! conversation is bricked unless we expire the orphans. The
//! [`KvPendingApprovalStore::sweep_orphans_on_startup`] entry point is
//! the explicit hook for that, but **note**: enumerating all keys with
//! prefix `approval:` requires a KV listing operation the trait does
//! not expose. For alpha, hosts that need cross-restart sweeping pass
//! in the expected approval ids (recovered from their own session
//! ledger) — see the method docs. Out of scope: KV prefix scanning.
//!
//! # Algebraic compliance
//!
//! Verified by `agent-fw-test::approval_laws::test_all` against this
//! store as well as the reference in-memory store. The KV writethrough
//! is best-effort: if KV fails, the awaiter remains intact and the
//! gate still works — the writethrough error is logged at WARN level.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use agent_fw_algebra::approval::{
    ApprovalAwait, ApprovalError, ExpireReason, InMemoryPendingApprovalStore, PendingApprovalStore,
};
use agent_fw_algebra::{KVStore, KVStoreExt};
use agent_fw_core::approval::{ApprovalDecision, ApprovalRequest};
use agent_fw_core::{ApprovalId, TenantId};

/// Default KV namespace for approval bodies.
pub const DEFAULT_APPROVAL_NAMESPACE: &str = "approval";

// ─── StoredApproval — wire format in KV ───────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum StoredApprovalStatus {
    Pending,
    Resolved { decision: ApprovalDecision },
    Expired { reason: ExpireReason },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredApproval {
    request: ApprovalRequest,
    status: StoredApprovalStatus,
}

// ─── KvPendingApprovalStore ───────────────────────────────────────────

/// `PendingApprovalStore` with a KV writethrough for audit.
pub struct KvPendingApprovalStore {
    awaiters: InMemoryPendingApprovalStore,
    kv: Arc<dyn KVStore>,
    namespace: String,
}

impl KvPendingApprovalStore {
    /// Create a new store with the default namespace (`"approval"`).
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self::with_namespace(kv, DEFAULT_APPROVAL_NAMESPACE)
    }

    /// Create a new store with an explicit KV key namespace.
    pub fn with_namespace(kv: Arc<dyn KVStore>, namespace: impl Into<String>) -> Self {
        Self {
            awaiters: InMemoryPendingApprovalStore::new(),
            kv,
            namespace: namespace.into(),
        }
    }

    /// KV key for an approval body within this store's namespace.
    fn body_key(&self, id: &ApprovalId) -> String {
        format!("{}:{}", self.namespace, id.as_str())
    }

    /// Read the persisted body (status-tagged) for an id, if any.
    async fn read_stored(
        &self,
        tenant: &TenantId,
        id: &ApprovalId,
    ) -> Result<Option<StoredApproval>, ApprovalError> {
        self.kv
            .get::<StoredApproval>(tenant.as_str(), &self.body_key(id))
            .await
            .map_err(|e| ApprovalError::Storage(e.to_string()))
    }

    /// Best-effort writethrough. Logs at WARN on failure but does NOT
    /// propagate the error — the awaiter machinery is the source of
    /// truth, and a transient KV failure should not break the gate.
    async fn write_stored(&self, tenant: &TenantId, id: &ApprovalId, stored: &StoredApproval) {
        match self
            .kv
            .put(tenant.as_str(), &self.body_key(id), stored, None)
            .await
        {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(
                    approval_id = %id,
                    error = %e,
                    "KV writethrough for approval failed; awaiter remains intact",
                );
            }
        }
    }

    /// Sweep approvals that were left pending across a process restart
    /// (H5). The caller supplies the candidate ids — typically recovered
    /// from a host-side session ledger. Each id is expired with
    /// [`ExpireReason::Restart`] if its KV body is still in `Pending`
    /// status, or skipped if it has been resolved/expired.
    ///
    /// Returns the ids that were transitioned by this call.
    pub async fn sweep_orphans_on_startup(
        &self,
        tenant: &TenantId,
        candidates: impl IntoIterator<Item = ApprovalId>,
    ) -> Result<Vec<ApprovalId>, ApprovalError> {
        let mut swept = Vec::new();
        for id in candidates {
            let Some(stored) = self.read_stored(tenant, &id).await? else {
                continue;
            };
            if !matches!(stored.status, StoredApprovalStatus::Pending) {
                continue; // already resolved or expired
            }
            // Mark as Expired { Restart } in KV. There is no in-process
            // awaiter (we just started up), so no oneshot to wake.
            let updated = StoredApproval {
                request: stored.request.clone(),
                status: StoredApprovalStatus::Expired {
                    reason: ExpireReason::Restart,
                },
            };
            self.write_stored(tenant, &id, &updated).await;
            swept.push(id);
        }
        Ok(swept)
    }
}

#[async_trait]
impl PendingApprovalStore for KvPendingApprovalStore {
    async fn register(&self, req: ApprovalRequest) -> Result<ApprovalAwait, ApprovalError> {
        // H4: register the awaiter FIRST so a fast resolve finds the
        // sender. KV writethrough comes second; if it fails the gate
        // still works.
        let tenant = req.resource_id.clone();
        let id = req.id.clone();
        let awaiter = self.awaiters.register(req.clone()).await?;
        self.write_stored(
            &tenant,
            &id,
            &StoredApproval {
                request: req,
                status: StoredApprovalStatus::Pending,
            },
        )
        .await;
        Ok(awaiter)
    }

    async fn resolve(&self, decision: ApprovalDecision) -> Result<(), ApprovalError> {
        // Snapshot the body so we can update the KV status.
        let id = decision.id.clone();
        let prior = match self.awaiters.get(&id).await? {
            Some(req) => Some(req),
            None => self.read_stored_any_tenant(&id).await,
        };
        self.awaiters.resolve(decision.clone()).await?;
        if let Some(req) = prior {
            let tenant = req.resource_id.clone();
            self.write_stored(
                &tenant,
                &id,
                &StoredApproval {
                    request: req,
                    status: StoredApprovalStatus::Resolved { decision },
                },
            )
            .await;
        }
        Ok(())
    }

    async fn expire(&self, id: &ApprovalId, reason: ExpireReason) -> Result<(), ApprovalError> {
        let prior = match self.awaiters.get(id).await? {
            Some(req) => Some(req),
            None => self.read_stored_any_tenant(id).await,
        };
        self.awaiters.expire(id, reason).await?;
        if let Some(req) = prior {
            let tenant = req.resource_id.clone();
            self.write_stored(
                &tenant,
                id,
                &StoredApproval {
                    request: req,
                    status: StoredApprovalStatus::Expired { reason },
                },
            )
            .await;
        }
        Ok(())
    }

    async fn get(&self, id: &ApprovalId) -> Result<Option<ApprovalRequest>, ApprovalError> {
        // Prefer the in-memory copy (cheap, no KV roundtrip). Fall back
        // to KV for resolved bodies the awaiter store has already evicted.
        if let Some(req) = self.awaiters.get(id).await? {
            return Ok(Some(req));
        }
        Ok(self.read_stored_any_tenant(id).await)
    }
}

impl KvPendingApprovalStore {
    /// Internal helper: KV scan by id, no tenant context. Returns the
    /// request body whose `resource_id` then tells us the original
    /// tenant for the writethrough. Because the KV API requires a
    /// tenant string at lookup time and our caller `resolve`/`expire`
    /// doesn't carry one, we accept the limitation: in alpha the
    /// in-process awaiter map is the typical lookup path, and KV
    /// fallback returns `None`. Hosts that need cross-restart KV
    /// lookups should call `sweep_orphans_on_startup(&tenant, ids)`
    /// with an explicit tenant.
    async fn read_stored_any_tenant(&self, _id: &ApprovalId) -> Option<ApprovalRequest> {
        // Intentional alpha limitation: we don't enumerate tenants here.
        // The awaiter map is the primary path; KV is audit/inspection.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashmap_kv::DashMapKVStore;
    use agent_fw_core::approval::ApprovalKind;
    use agent_fw_core::ThreadId;
    use serde_json::json;

    fn req(id: &str, tenant: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: ApprovalId::new_unchecked(id),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: json!({"x": 1}),
            glimpse: None,
            resource_id: TenantId::new_unchecked(tenant),
            thread_id: ThreadId::new_unchecked("th-1"),
            correlation_id: None,
        }
    }

    fn build_store() -> KvPendingApprovalStore {
        KvPendingApprovalStore::new(Arc::new(DashMapKVStore::new()))
    }

    // ── Trait laws via the reusable harness ──────────────────────────

    #[tokio::test]
    async fn kv_store_satisfies_approval_laws() {
        agent_fw_test::approval_laws::test_all(build_store).await;
    }

    // ── KV writethrough specifics ────────────────────────────────────

    #[tokio::test]
    async fn register_writes_pending_body_to_kv() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = KvPendingApprovalStore::new(kv.clone());

        let _await = store.register(req("a1", "acme")).await.unwrap();
        let raw: Option<StoredApproval> = kv
            .get::<StoredApproval>("acme", "approval:a1")
            .await
            .unwrap();
        let stored = raw.expect("body written");
        assert!(matches!(stored.status, StoredApprovalStatus::Pending));
        assert_eq!(stored.request.target, "create_scenario");
    }

    #[tokio::test]
    async fn resolve_updates_kv_status_to_resolved() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = KvPendingApprovalStore::new(kv.clone());

        let awaiter = store.register(req("a2", "acme")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a2")))
            .await
            .unwrap();
        let _ = awaiter.await;

        let stored: StoredApproval = kv
            .get("acme", "approval:a2")
            .await
            .unwrap()
            .expect("body still present");
        assert!(matches!(
            stored.status,
            StoredApprovalStatus::Resolved { .. }
        ));
    }

    #[tokio::test]
    async fn expire_updates_kv_status_to_expired() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = KvPendingApprovalStore::new(kv.clone());

        let _ = store.register(req("a3", "acme")).await.unwrap();
        store
            .expire(&ApprovalId::new_unchecked("a3"), ExpireReason::Cancelled)
            .await
            .unwrap();

        let stored: StoredApproval = kv.get("acme", "approval:a3").await.unwrap().unwrap();
        match stored.status {
            StoredApprovalStatus::Expired { reason } => {
                assert_eq!(reason, ExpireReason::Cancelled);
            }
            other => panic!("expected Expired, got {other:?}"),
        }
    }

    // ── H5: orphan sweep on startup ──────────────────────────────────

    #[tokio::test]
    async fn sweep_orphans_marks_pre_restart_pending_as_expired() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());

        // Pre-restart store: register an approval.
        {
            let store = KvPendingApprovalStore::new(kv.clone());
            let _awaiter = store.register(req("orphan-1", "acme")).await.unwrap();
            // Simulate restart by dropping the store before any resolve/expire.
        }

        // Post-restart store: same KV, fresh awaiter map.
        let store = KvPendingApprovalStore::new(kv.clone());
        let tenant = TenantId::new_unchecked("acme");
        let swept = store
            .sweep_orphans_on_startup(&tenant, vec![ApprovalId::new_unchecked("orphan-1")])
            .await
            .unwrap();
        assert_eq!(swept.len(), 1);
        assert_eq!(swept[0].as_str(), "orphan-1");

        // KV body now reads Expired { Restart }
        let stored: StoredApproval = kv.get("acme", "approval:orphan-1").await.unwrap().unwrap();
        match stored.status {
            StoredApprovalStatus::Expired { reason } => {
                assert_eq!(reason, ExpireReason::Restart);
            }
            other => panic!("expected Expired Restart, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sweep_orphans_skips_already_resolved() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = KvPendingApprovalStore::new(kv.clone());

        let awaiter = store.register(req("a4", "acme")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a4")))
            .await
            .unwrap();
        let _ = awaiter.await;

        let tenant = TenantId::new_unchecked("acme");
        let swept = store
            .sweep_orphans_on_startup(&tenant, vec![ApprovalId::new_unchecked("a4")])
            .await
            .unwrap();
        assert!(
            swept.is_empty(),
            "Resolved approvals must not be re-expired"
        );
    }
}
