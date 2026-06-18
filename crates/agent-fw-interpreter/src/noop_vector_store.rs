//! No-op VectorStore — used when vector search is disabled (e.g., SQLite mode).
//!
//! All write operations silently succeed (returning 0 upserted/deleted).
//! Search/get return empty results. `health_check` returns `NotConfigured`.
//!
//! This is the "disabled automatically for SQLite" implementation — the system
//! continues to function but vector search is a no-op. Callers that check
//! `health_check` can detect that vectors are unavailable and adjust UX.

use async_trait::async_trait;

use agent_fw_algebra::{EmbeddingItem, VectorHit, VectorStore, VectorStoreError};

/// A VectorStore that does nothing.
///
/// Write operations silently succeed so that callers don't need to branch
/// on "vector store available?" at every call site. Search/get return
/// empty results, which callers already handle as "no relevant results."
pub struct NoOpVectorStore;

#[async_trait]
impl VectorStore for NoOpVectorStore {
    async fn search_similar(
        &self,
        _embedding: &[f32],
        _limit: usize,
        _min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        Ok(vec![])
    }

    async fn upsert_embedding(
        &self,
        _id: &str,
        _content: &str,
        _item_type: &str,
        _metadata: serde_json::Value,
        _embedding: &[f32],
    ) -> Result<(), VectorStoreError> {
        Ok(())
    }

    async fn upsert_batch(&self, _items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        Ok(0)
    }

    async fn delete_by_prefix(&self, _id_prefix: &str) -> Result<usize, VectorStoreError> {
        Ok(0)
    }

    async fn get_by_id(&self, _id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        Ok(None)
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        Err(VectorStoreError::NotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_search_returns_empty() {
        let store = NoOpVectorStore;
        let hits = store.search_similar(&[0.1, 0.2], 10, 0.5).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn noop_upsert_succeeds_silently() {
        let store = NoOpVectorStore;
        let count = store.upsert_batch(&[]).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn noop_health_check_returns_not_configured() {
        let store = NoOpVectorStore;
        let err = store.health_check().await.unwrap_err();
        assert!(matches!(err, VectorStoreError::NotConfigured));
    }

    #[tokio::test]
    async fn noop_get_by_id_returns_none() {
        let store = NoOpVectorStore;
        assert!(store.get_by_id("anything").await.unwrap().is_none());
    }
}
