//! Stage-based pipeline orchestration — factors out cancel-check + KV-update + event-emit.
//!
//! Provides a reusable `PipelineCtx` that standardizes the repetitive
//! cancel → status → emit → execute → re-check pattern found in every
//! multi-stage pipeline (ingestion, eval, ETL).
//!
//! # Placement
//!
//! `PipelineCtx` is a combinator (like `retry`, `timeout`, `with_fallback`) that
//! composes `KVStore` + `CancellationToken` into a reusable stage pattern. It is
//! generic over any `KVStore` implementation. It lives here alongside other
//! combinators, not in `agent-fw-interpreter`.
//!
//! # Laws
//!
//! - **L1 (Cancellation)**: If token is cancelled before stage, stage is skipped (`None`)
//! - **L2 (Progress)**: Every stage transition emits exactly one progress event
//! - **L3 (Completion)**: Pipeline completes iff all stages complete or one fails
//! - **L4 (Accumulation)**: Summary is the monoid fold of per-stage summaries
//! - **L5 (Mid-cancel discard)**: If cancelled during op, op completes but result is `None`

use std::future::Future;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::mpsc;

use crate::CancellationToken;
use crate::KVStore;

/// Standard TTL for ephemeral KV entries (24 hours).
const DEFAULT_EPHEMERAL_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Pipeline context — factors cancel + KV-status + event-emit into a single struct.
///
/// Generic over the status/event type `S`.
///
/// # Usage
///
/// ```ignore
/// let ctx = PipelineCtx::new(tenant_id, status_key, kv, cancel, tx);
///
/// // Each stage checks cancellation, updates KV, emits progress, runs the op
/// let result = ctx.run_stage(MyStatus::Discovering, || async {
///     discover_tables().await
/// }).await;
/// ```
pub struct PipelineCtx<S> {
    tenant_id: String,
    status_key: String,
    kv: std::sync::Arc<dyn KVStore>,
    cancel: CancellationToken,
    tx: mpsc::Sender<S>,
    ttl: Duration,
}

impl<S: Serialize + Clone + Send + 'static> PipelineCtx<S> {
    /// Create a new pipeline context.
    pub fn new(
        tenant_id: impl Into<String>,
        status_key: impl Into<String>,
        kv: std::sync::Arc<dyn KVStore>,
        cancel: CancellationToken,
        tx: mpsc::Sender<S>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            status_key: status_key.into(),
            kv,
            cancel,
            tx,
            ttl: DEFAULT_EPHEMERAL_TTL,
        }
    }

    /// Override the default KV TTL for status entries.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Run a single stage with cancel-check → KV-update → event-emit → execute → re-check.
    ///
    /// # Returns
    ///
    /// - `Some(Ok(value))` — stage completed successfully
    /// - `Some(Err(error))` — stage failed (caller decides how to handle)
    /// - `None` — cancelled or client disconnected (pipeline should abort)
    ///
    /// # Cancellation Semantics
    ///
    /// - Pre-cancel (before op): `None`, op never runs.
    /// - Mid-cancel (during op): op runs to completion, result discarded, returns `None`.
    ///   Side effects from the op ARE committed. Callers relying on non-idempotent
    ///   operations should check cancellation within the op itself.
    /// - Post-cancel (after op, before next stage): next `run_stage` returns `None`.
    ///
    /// Mid-cancel discard is deliberate: the alternative (`select!` against cancel token)
    /// would require the op to be cancel-safe, which most database operations are not.
    pub async fn run_stage<T, F, Fut>(&self, status: S, op: F) -> Option<Result<T, String>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        // Pre-check cancellation
        if self.cancel.is_cancelled() {
            return None;
        }

        // Update KV status
        let status_value = match serde_json::to_value(&status) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    key = %self.status_key,
                    "failed to serialize pipeline status, storing null"
                );
                serde_json::Value::Null
            }
        };
        if let Err(e) = self
            .kv
            .put_json(
                &self.tenant_id,
                &self.status_key,
                status_value,
                Some(self.ttl),
            )
            .await
        {
            tracing::warn!(error = %e, key = %self.status_key, "failed to update pipeline status in KV");
        }

        // Emit progress event (Law L2)
        if self.tx.send(status).await.is_err() {
            return None; // client disconnected
        }

        // Execute the operation
        let result = op().await;

        // Post-check cancellation
        if self.cancel.is_cancelled() {
            return None;
        }

        Some(result)
    }

    /// Check if the pipeline should continue (not cancelled, client connected).
    pub fn is_alive(&self) -> bool {
        !self.cancel.is_cancelled() && !self.tx.is_closed()
    }

    /// Emit a progress event without running an operation.
    ///
    /// Useful for intermediate status updates between stages.
    pub async fn emit_progress(&self, status: S) -> bool {
        let status_value = match serde_json::to_value(&status) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    key = %self.status_key,
                    "failed to serialize pipeline progress, storing null"
                );
                serde_json::Value::Null
            }
        };
        if let Err(e) = self
            .kv
            .put_json(
                &self.tenant_id,
                &self.status_key,
                status_value,
                Some(self.ttl),
            )
            .await
        {
            tracing::warn!(error = %e, key = %self.status_key, "failed to update pipeline progress in KV");
        }
        self.tx.send(status).await.is_ok()
    }

    /// Access the cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Access the KV store.
    pub fn kv(&self) -> &dyn KVStore {
        self.kv.as_ref()
    }

    /// Access the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Access the status key.
    pub fn status_key(&self) -> &str {
        &self.status_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CancellationToken;

    // We need a KV store for tests — use a minimal mock
    struct NullKVStore;

    #[async_trait::async_trait]
    impl KVStore for NullKVStore {
        async fn get_json(
            &self,
            _tenant: &str,
            _key: &str,
        ) -> Result<Option<serde_json::Value>, crate::KVError> {
            Ok(None)
        }

        async fn put_json(
            &self,
            _tenant: &str,
            _key: &str,
            _value: serde_json::Value,
            _ttl: Option<Duration>,
        ) -> Result<(), crate::KVError> {
            Ok(())
        }

        async fn delete(&self, _tenant: &str, _key: &str) -> Result<bool, crate::KVError> {
            Ok(false)
        }

        async fn exists(&self, _tenant: &str, _key: &str) -> Result<bool, crate::KVError> {
            Ok(false)
        }

        async fn list_keys(
            &self,
            _tenant: &str,
            _prefix: &str,
        ) -> Result<Vec<String>, crate::KVError> {
            Ok(vec![])
        }

        async fn get_many_json(
            &self,
            _tenant: &str,
            _keys: &[String],
        ) -> Result<std::collections::HashMap<String, serde_json::Value>, crate::KVError> {
            Ok(std::collections::HashMap::new())
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    enum TestStatus {
        Step1,
        Step2,
        Done,
    }

    #[tokio::test]
    async fn l1_cancelled_before_stage_returns_none() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (tx, _rx) = mpsc::channel(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel, tx);

        let result = ctx
            .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
            .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn l2_stage_emits_progress() {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel, tx);

        let result = ctx
            .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
            .await;

        assert_eq!(result, Some(Ok(42)));

        // Should have received exactly one progress event
        let event = rx.try_recv().unwrap();
        assert_eq!(event, TestStatus::Step1);
    }

    #[tokio::test]
    async fn stage_propagates_error() {
        let cancel = CancellationToken::new();
        let (tx, _rx) = mpsc::channel(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel, tx);

        let result = ctx
            .run_stage(TestStatus::Step1, || async {
                Err::<i32, _>("stage failed".to_string())
            })
            .await;

        assert_eq!(result, Some(Err("stage failed".to_string())));
    }

    #[tokio::test]
    async fn client_disconnect_returns_none() {
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::channel(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel, tx);

        // Drop the receiver to simulate client disconnect
        drop(rx);

        let result = ctx
            .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
            .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn is_alive_reflects_state() {
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::channel::<TestStatus>(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel.clone(), tx);

        assert!(ctx.is_alive());

        cancel.cancel();
        assert!(!ctx.is_alive());

        // Also test receiver drop
        let cancel2 = CancellationToken::new();
        let (tx2, rx2) = mpsc::channel::<TestStatus>(16);
        let kv2 = std::sync::Arc::new(NullKVStore);
        let ctx2 = PipelineCtx::new("tenant", "status-key", kv2, cancel2, tx2);
        drop(rx2);
        assert!(!ctx2.is_alive());

        drop(rx); // suppress warning
    }

    #[tokio::test]
    async fn l3_cancel_mid_pipeline_skips_remaining() {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel::<TestStatus>(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "key", kv, cancel.clone(), tx);

        // Stage 1 succeeds
        let r1 = ctx
            .run_stage(TestStatus::Step1, || async { Ok::<_, String>(1) })
            .await;
        assert_eq!(r1, Some(Ok(1)));

        // Cancel before stage 2
        cancel.cancel();

        // Stage 2 should return None (skipped)
        let r2 = ctx
            .run_stage(TestStatus::Done, || async { Ok::<_, String>(2) })
            .await;
        assert!(r2.is_none(), "L3: cancelled stage must return None");

        // Only Step1 event should have been emitted
        let event = rx.try_recv().unwrap();
        assert_eq!(event, TestStatus::Step1);
        // No Done event
        assert!(
            rx.try_recv().is_err(),
            "L3: cancelled stage must not emit progress event"
        );
    }

    #[tokio::test]
    async fn emit_progress_sends_event() {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(16);
        let kv = std::sync::Arc::new(NullKVStore);
        let ctx = PipelineCtx::new("tenant", "status-key", kv, cancel, tx);

        assert!(ctx.emit_progress(TestStatus::Step2).await);

        let event = rx.try_recv().unwrap();
        assert_eq!(event, TestStatus::Step2);
    }
}
