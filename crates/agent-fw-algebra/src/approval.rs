//! `PendingApprovalStore` algebra — the suspension primitive that lets
//! a tool dispatch or plan executor pause until a host responds with a
//! decision.
//!
//! The pure data types ([`agent_fw_core::ApprovalRequest`], etc.) live in
//! `agent-fw-core`. The closure-bearing rule/policy abstractions and the
//! `ApprovalLayer` consumer live in `agent-fw-agent`. This module owns the
//! single async trait that bridges them and the structured error type that
//! interpreters use.
//!
//! # Why not `PauseToken`?
//!
//! [`crate::pause::PauseToken`] is a singleton (global pause/resume) used
//! for human-in-the-loop eval. Approvals are **per-request**, with many
//! pending concurrently — each needs its own awaiter. The store maps
//! [`ApprovalId`] → `oneshot::Sender` and exposes a `Future` that the
//! gate `tokio::select!`s alongside the cancellation token.
//!
//! # Algebraic laws
//!
//! - **L1 (Register-once)**: `register(req)` with a duplicate
//!   [`ApprovalId`] returns [`ApprovalError::AlreadyRegistered`]. Callers
//!   allocate fresh ids per registration (typically via uuid).
//! - **L2 (Resolve totality)**: `resolve(decision)` for an unknown id
//!   returns [`ApprovalError::NotFound`]; for an already-resolved id
//!   returns [`ApprovalError::AlreadyResolved`]; otherwise wakes exactly
//!   one waiter and never panics.
//! - **L3 (Expire totality)**: `expire(id, reason)` for an unknown id
//!   returns `NotFound`; for an already-resolved id returns
//!   `AlreadyResolved`; otherwise wakes the waiter with
//!   `Err(Expired { reason })` and marks the id as resolved.
//! - **L4 (Sender-drop visibility)**: dropping the store while a waiter
//!   is pending wakes the waiter with `Err(SenderDropped)`.
//! - **L5 (Get totality)**: `get(id)` for an unknown id returns
//!   `Ok(None)`; never returns `NotFound`. Resolved-body retention is
//!   impl-defined.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;

use agent_fw_core::approval::{ApprovalDecision, ApprovalRequest};
use agent_fw_core::ApprovalId;

// ─── ExpireReason ─────────────────────────────────────────────────────

/// Why an approval was expired without a host decision.
///
/// Surfaces in the `Err(ApprovalError::Expired)` payload that the layer
/// returns to the gated call — so the LLM can see *why* the gate closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpireReason {
    /// Cancellation token fired while awaiting (H1 in the pre-dispatch approval plan).
    Cancelled,
    /// Approval-side timeout (not used in alpha but reserved).
    Timeout,
    /// Process restart found the body in KV with no awaiter (H5).
    Restart,
    /// Host explicitly closed the stream / shut down.
    HostShutdown,
}

// ─── ApprovalError ────────────────────────────────────────────────────

/// Structured error from any [`PendingApprovalStore`] method.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ApprovalError {
    #[error("approval not found: {0}")]
    NotFound(ApprovalId),
    #[error("approval already registered: {0}")]
    AlreadyRegistered(ApprovalId),
    #[error("approval already resolved: {0}")]
    AlreadyResolved(ApprovalId),
    #[error("approval expired: id={id} reason={reason:?}")]
    Expired {
        id: ApprovalId,
        reason: ExpireReason,
    },
    #[error("approval sender dropped before resolution")]
    SenderDropped,
    #[error("storage error: {0}")]
    Storage(String),
}

// ─── ApprovalAwait ────────────────────────────────────────────────────

/// The future a gate awaits while the host decides.
///
/// Wraps a `tokio::sync::oneshot::Receiver` whose payload is the resolved
/// decision or an [`ApprovalError`] (expired / sender-dropped). The gate
/// `tokio::select!`s this against the cancellation token to satisfy H1.
pub struct ApprovalAwait {
    rx: oneshot::Receiver<Result<ApprovalDecision, ApprovalError>>,
}

impl ApprovalAwait {
    /// Wrap an existing oneshot receiver. Interpreters construct an
    /// `ApprovalAwait` from inside their `register` implementation.
    pub fn new(rx: oneshot::Receiver<Result<ApprovalDecision, ApprovalError>>) -> Self {
        Self { rx }
    }
}

impl std::fmt::Debug for ApprovalAwait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalAwait").finish_non_exhaustive()
    }
}

impl Future for ApprovalAwait {
    type Output = Result<ApprovalDecision, ApprovalError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(Ok(inner)) => Poll::Ready(inner),
            // Sender dropped without sending → L4 (sender-drop visibility)
            Poll::Ready(Err(_recv_err)) => Poll::Ready(Err(ApprovalError::SenderDropped)),
            Poll::Pending => Poll::Pending,
        }
    }
}

// ─── PendingApprovalStore ─────────────────────────────────────────────

/// The async algebra a gate uses to suspend itself until the host decides.
///
/// **Storage semantics.** Implementations choose whether to retain the
/// [`ApprovalRequest`] body after resolution (useful for audit trail) or
/// drop it (lower memory). All implementations must keep an in-process
/// awaiter table for as long as the request is pending so `resolve` can
/// wake the right caller.
#[async_trait]
pub trait PendingApprovalStore: Send + Sync {
    /// Register a pending approval and return its awaiter.
    ///
    /// Returns [`ApprovalError::AlreadyRegistered`] if the id is already
    /// pending. Callers should allocate fresh ids per registration.
    async fn register(&self, req: ApprovalRequest) -> Result<ApprovalAwait, ApprovalError>;

    /// Resolve a pending approval with a host decision.
    ///
    /// Wakes the awaiter exactly once with `Ok(decision)`. Returns
    /// [`ApprovalError::NotFound`] for unknown ids and
    /// [`ApprovalError::AlreadyResolved`] for ids that have already been
    /// resolved or expired.
    async fn resolve(&self, decision: ApprovalDecision) -> Result<(), ApprovalError>;

    /// Expire a pending approval (cancellation, timeout, restart).
    ///
    /// Wakes the awaiter with `Err(Expired { reason })`. Same totality
    /// semantics as [`resolve`].
    async fn expire(&self, id: &ApprovalId, reason: ExpireReason) -> Result<(), ApprovalError>;

    /// Read the registered request body.
    ///
    /// Returns `Ok(None)` for unknown ids. Retention of resolved bodies
    /// is implementation-defined.
    async fn get(&self, id: &ApprovalId) -> Result<Option<ApprovalRequest>, ApprovalError>;
}

// ─── InMemoryPendingApprovalStore ─────────────────────────────────────

/// Reference in-memory implementation suitable for tests and for use as
/// the awaiter half of a composite store (durable body in KV + awaiter
/// here).
///
/// Behavior:
/// - Bodies are kept until `resolve` or `expire` is called, then dropped.
/// - One waiter per id (oneshot, not broadcast). Duplicate `register`
///   returns `AlreadyRegistered`.
#[derive(Default)]
pub struct InMemoryPendingApprovalStore {
    inner: Mutex<InMemoryStoreInner>,
}

#[derive(Default)]
struct InMemoryStoreInner {
    /// Active senders + bodies, keyed by ApprovalId.
    pending: HashMap<ApprovalId, PendingEntry>,
    /// Track ids that have been resolved or expired so `resolve`/`expire`
    /// can return `AlreadyResolved` instead of `NotFound`.
    resolved: HashMap<ApprovalId, ()>,
}

struct PendingEntry {
    body: ApprovalRequest,
    sender: oneshot::Sender<Result<ApprovalDecision, ApprovalError>>,
}

impl InMemoryPendingApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of currently-pending approval ids. Test helper.
    pub fn pending_count(&self) -> usize {
        self.inner.lock().expect("poisoned").pending.len()
    }

    /// Lock recovery: poison-resilient lock for L1 (never panic).
    fn lock(&self) -> std::sync::MutexGuard<'_, InMemoryStoreInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[async_trait]
impl PendingApprovalStore for InMemoryPendingApprovalStore {
    async fn register(&self, req: ApprovalRequest) -> Result<ApprovalAwait, ApprovalError> {
        let mut guard = self.lock();
        if guard.pending.contains_key(&req.id) || guard.resolved.contains_key(&req.id) {
            return Err(ApprovalError::AlreadyRegistered(req.id.clone()));
        }
        let (tx, rx) = oneshot::channel();
        guard.pending.insert(
            req.id.clone(),
            PendingEntry {
                body: req,
                sender: tx,
            },
        );
        Ok(ApprovalAwait::new(rx))
    }

    async fn resolve(&self, decision: ApprovalDecision) -> Result<(), ApprovalError> {
        let mut guard = self.lock();
        if guard.resolved.contains_key(&decision.id) {
            return Err(ApprovalError::AlreadyResolved(decision.id.clone()));
        }
        let entry = guard
            .pending
            .remove(&decision.id)
            .ok_or_else(|| ApprovalError::NotFound(decision.id.clone()))?;
        guard.resolved.insert(decision.id.clone(), ());
        drop(guard);
        // Best-effort send. Receiver may have been dropped (e.g., gate
        // future cancelled before resolve completed) — that's fine; we
        // still record the resolution.
        let _ = entry.sender.send(Ok(decision));
        Ok(())
    }

    async fn expire(&self, id: &ApprovalId, reason: ExpireReason) -> Result<(), ApprovalError> {
        let mut guard = self.lock();
        if guard.resolved.contains_key(id) {
            return Err(ApprovalError::AlreadyResolved(id.clone()));
        }
        let entry = guard
            .pending
            .remove(id)
            .ok_or_else(|| ApprovalError::NotFound(id.clone()))?;
        guard.resolved.insert(id.clone(), ());
        drop(guard);
        let _ = entry.sender.send(Err(ApprovalError::Expired {
            id: id.clone(),
            reason,
        }));
        Ok(())
    }

    async fn get(&self, id: &ApprovalId) -> Result<Option<ApprovalRequest>, ApprovalError> {
        let guard = self.lock();
        Ok(guard.pending.get(id).map(|e| e.body.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::approval::ApprovalKind;
    use agent_fw_core::{TenantId, ThreadId};
    use serde_json::json;

    fn req(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: ApprovalId::new_unchecked(id),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: json!({"x": 1}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("acme"),
            thread_id: ThreadId::new_unchecked("thread-1"),
            correlation_id: None,
        }
    }

    // ── L1: Register-once ─────────────────────────────────────────────

    #[tokio::test]
    async fn register_returns_awaiter() {
        let store = InMemoryPendingApprovalStore::new();
        let _await = store.register(req("a1")).await.unwrap();
        assert_eq!(store.pending_count(), 1);
    }

    #[tokio::test]
    async fn duplicate_register_errors() {
        let store = InMemoryPendingApprovalStore::new();
        store.register(req("a1")).await.unwrap();
        let err = store.register(req("a1")).await.unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadyRegistered(_)));
    }

    #[tokio::test]
    async fn resolve_after_register_returns_already_registered() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap();
        let _ = awaiter.await;
        // Re-registering after resolution should still error (anti-replay)
        let err = store.register(req("a1")).await.unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadyRegistered(_)));
    }

    // ── L2: Resolve totality ─────────────────────────────────────────

    #[tokio::test]
    async fn resolve_wakes_awaiter_with_decision() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap();
        let decision = awaiter.await.unwrap();
        assert!(decision.outcome.is_approve());
        assert_eq!(decision.id.as_str(), "a1");
        assert_eq!(store.pending_count(), 0);
    }

    #[tokio::test]
    async fn resolve_unknown_returns_not_found() {
        let store = InMemoryPendingApprovalStore::new();
        let err = store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked(
                "missing",
            )))
            .await
            .unwrap_err();
        assert!(matches!(err, ApprovalError::NotFound(_)));
    }

    #[tokio::test]
    async fn double_resolve_returns_already_resolved() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap();
        let _ = awaiter.await;
        let err = store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadyResolved(_)));
    }

    // ── L3: Expire totality ──────────────────────────────────────────

    #[tokio::test]
    async fn expire_wakes_awaiter_with_expired_error() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .expire(&ApprovalId::new_unchecked("a1"), ExpireReason::Cancelled)
            .await
            .unwrap();
        let err = awaiter.await.unwrap_err();
        match err {
            ApprovalError::Expired { id, reason } => {
                assert_eq!(id.as_str(), "a1");
                assert_eq!(reason, ExpireReason::Cancelled);
            }
            other => panic!("expected Expired, got {other:?}"),
        }
        assert_eq!(store.pending_count(), 0);
    }

    #[tokio::test]
    async fn expire_unknown_returns_not_found() {
        let store = InMemoryPendingApprovalStore::new();
        let err = store
            .expire(&ApprovalId::new_unchecked("missing"), ExpireReason::Restart)
            .await
            .unwrap_err();
        assert!(matches!(err, ApprovalError::NotFound(_)));
    }

    #[tokio::test]
    async fn expire_after_resolve_returns_already_resolved() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap();
        let _ = awaiter.await;
        let err = store
            .expire(&ApprovalId::new_unchecked("a1"), ExpireReason::Cancelled)
            .await
            .unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadyResolved(_)));
    }

    // ── L4: Sender-drop visibility ──────────────────────────────────

    #[tokio::test]
    async fn dropping_store_wakes_awaiter_with_sender_dropped() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        drop(store);
        let err = awaiter.await.unwrap_err();
        assert!(matches!(err, ApprovalError::SenderDropped));
    }

    // ── L5: Get totality ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_body_for_pending() {
        let store = InMemoryPendingApprovalStore::new();
        store.register(req("a1")).await.unwrap();
        let body = store.get(&ApprovalId::new_unchecked("a1")).await.unwrap();
        assert!(body.is_some());
        assert_eq!(body.unwrap().target, "create_scenario");
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown() {
        let store = InMemoryPendingApprovalStore::new();
        let body = store
            .get(&ApprovalId::new_unchecked("missing"))
            .await
            .unwrap();
        assert!(body.is_none());
    }

    #[tokio::test]
    async fn get_returns_none_after_resolve() {
        let store = InMemoryPendingApprovalStore::new();
        let awaiter = store.register(req("a1")).await.unwrap();
        store
            .resolve(ApprovalDecision::approve(ApprovalId::new_unchecked("a1")))
            .await
            .unwrap();
        let _ = awaiter.await;
        let body = store.get(&ApprovalId::new_unchecked("a1")).await.unwrap();
        assert!(body.is_none());
    }

    // ── Cancel-during-await: gate-side select! pattern ──────────────

    /// Models the layer's `tokio::select!` between cancellation and
    /// awaiter. After cancel, the gate calls `expire` to release the
    /// store-side entry; the awaiter then resolves with `Expired`.
    #[tokio::test]
    async fn cancel_then_expire_releases_entry() {
        let store = std::sync::Arc::new(InMemoryPendingApprovalStore::new());
        let awaiter = store.register(req("a1")).await.unwrap();

        // Simulate gate-side: drop the awaiter (the future was cancelled).
        drop(awaiter);

        // Layer's drop handler / cancellation branch calls expire.
        store
            .expire(&ApprovalId::new_unchecked("a1"), ExpireReason::Cancelled)
            .await
            .unwrap();

        // No leaks.
        assert_eq!(store.pending_count(), 0);
    }
}
