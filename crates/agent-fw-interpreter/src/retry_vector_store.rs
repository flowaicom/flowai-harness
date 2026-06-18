//! RetryVectorStore — algebra-level retry wrapper for any VectorStore.
//!
//! # Design
//!
//! Retry is a **combinator**, not an implementation detail. Compose it
//! at the algebra level:
//!
//! ```text
//! let store = RetryVectorStore::new(PgVectorStore::connect(url).await?, policy);
//! ```

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_algebra::vector_store::{EmbeddingItem, VectorHit, VectorStore, VectorStoreError};
use async_trait::async_trait;
use std::time::Duration;

/// A VectorStore wrapper that retries failed operations according to a policy.
///
/// Retries on `Connection` and `Execution` errors (transient).
/// `NotConfigured` and `DimensionMismatch` are not retried (deterministic).
pub struct RetryVectorStore<V: VectorStore> {
    inner: V,
    policy: RetryPolicy,
}

impl<V: VectorStore> RetryVectorStore<V> {
    /// Wrap a VectorStore with retry behavior.
    pub fn new(inner: V, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Create with a reasonable default: 3 retries, 100ms initial delay, exponential backoff.
    pub fn with_defaults(inner: V) -> Self {
        Self::new(
            inner,
            RetryPolicy::exponential_backoff(3, Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(2)),
        )
    }

    /// Access the inner store.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Access the retry policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

fn is_retryable(e: &VectorStoreError) -> bool {
    use super::retry_defaults::TransientError;
    e.is_transient()
}

#[async_trait]
impl<V: VectorStore> VectorStore for RetryVectorStore<V> {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async {
                self.inner
                    .search_similar(embedding, limit, min_similarity)
                    .await
            },
            is_retryable,
        )
        .await
    }

    async fn upsert_embedding(
        &self,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: serde_json::Value,
        embedding: &[f32],
    ) -> Result<(), VectorStoreError> {
        // Clone metadata on each retry attempt (same pattern as RetryKVStore::put_json)
        let metadata = std::sync::Arc::new(metadata);
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || {
                let md = metadata.clone();
                async move {
                    self.inner
                        .upsert_embedding(id, content, item_type, (*md).clone(), embedding)
                        .await
                }
            },
            is_retryable,
        )
        .await
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.upsert_batch(items).await },
            is_retryable,
        )
        .await
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.delete_by_prefix(id_prefix).await },
            is_retryable,
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_by_id(id).await },
            is_retryable,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.health_check().await },
            is_retryable,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// A VectorStore that fails N times then delegates.
    struct FailNTimes<V: VectorStore> {
        inner: V,
        remaining_failures: AtomicI32,
    }

    impl<V: VectorStore> FailNTimes<V> {
        fn new(inner: V, failures: i32) -> Self {
            Self {
                inner,
                remaining_failures: AtomicI32::new(failures),
            }
        }
    }

    impl<V: VectorStore> FailNTimes<V> {
        fn maybe_fail(&self) -> Result<(), VectorStoreError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(VectorStoreError::Connection("connection reset".into()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl<V: VectorStore> VectorStore for FailNTimes<V> {
        async fn search_similar(
            &self,
            embedding: &[f32],
            limit: usize,
            min_similarity: f64,
        ) -> Result<Vec<VectorHit>, VectorStoreError> {
            self.maybe_fail()?;
            self.inner
                .search_similar(embedding, limit, min_similarity)
                .await
        }

        async fn upsert_embedding(
            &self,
            id: &str,
            content: &str,
            item_type: &str,
            metadata: serde_json::Value,
            embedding: &[f32],
        ) -> Result<(), VectorStoreError> {
            self.maybe_fail()?;
            self.inner
                .upsert_embedding(id, content, item_type, metadata, embedding)
                .await
        }

        async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
            self.maybe_fail()?;
            self.inner.upsert_batch(items).await
        }

        async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
            self.maybe_fail()?;
            self.inner.delete_by_prefix(id_prefix).await
        }

        async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
            self.maybe_fail()?;
            self.inner.get_by_id(id).await
        }

        async fn health_check(&self) -> Result<(), VectorStoreError> {
            self.maybe_fail()?;
            self.inner.health_check().await
        }
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_failure() {
        let inner = FailNTimes::new(crate::MockVectorStore::new(), 2);
        let store = RetryVectorStore::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        // health_check should succeed after 2 failures + 1 success
        store.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn retry_exhausts_on_persistent_failure() {
        let inner = FailNTimes::new(crate::MockVectorStore::new(), 100);
        let store = RetryVectorStore::new(inner, RetryPolicy::fixed(2, Duration::from_millis(1)));

        let result = store.health_check().await;
        assert!(result.is_err(), "Should fail after exhausting retries");
    }

    #[test]
    fn retryable_classification() {
        assert!(is_retryable(&VectorStoreError::Connection(
            "connection reset".into()
        )));
        assert!(is_retryable(&VectorStoreError::Execution("timeout".into())));
        assert!(!is_retryable(&VectorStoreError::NotConfigured));
        assert!(!is_retryable(&VectorStoreError::DimensionMismatch {
            expected: 768,
            actual: 1536
        }));
    }
}
