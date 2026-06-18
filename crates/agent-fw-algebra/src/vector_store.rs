//! VectorStore — async semantic similarity search.
//!
//! This trait abstracts vector embedding storage and similarity search,
//! used for hybrid search pipelines (fuzzy + vector).
//!
//! # Laws
//!
//! L1 (Upsert Idempotency): Upserting the same id overwrites without duplicates
//! L2 (Search Monotonicity): Adding higher-similarity embedding maintains/increases rank
//! L3 (Delete Prefix): Exactly removes entries with specified prefix
//! L4 (Get After Upsert): `upsert(id, ..); get_by_id(id)` returns `Some`
//! L5 (Threshold Filtering): `search_similar(.., min_similarity)` only returns hits with `score >= min_similarity`
//! L6 (Count Consistency): After N upserts with distinct ids, search returns at most N matching items

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

/// Vector store error.
#[derive(Debug, Error)]
pub enum VectorStoreError {
    #[error("Vector store connection error: {0}")]
    Connection(String),

    #[error("Vector store execution error: {0}")]
    Execution(String),

    #[error("Vector store not configured")]
    NotConfigured,

    #[error("Vector dimension mismatch: table has {actual}, configured for {expected}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// A similarity search result from the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorHit {
    pub id: String,
    pub content: String,
    pub item_type: String,
    pub metadata: serde_json::Value,
    pub score: f64,
}

/// An item to upsert into the vector store (content + embedding).
#[derive(Debug, Clone)]
pub struct EmbeddingItem {
    pub id: String,
    pub content: String,
    pub item_type: String,
    pub metadata: serde_json::Value,
    pub embedding: Vec<f32>,
}

/// Async vector embedding storage and similarity search.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Search for items similar to the given embedding.
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError>;

    /// Upsert a single embedding.
    async fn upsert_embedding(
        &self,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: serde_json::Value,
        embedding: &[f32],
    ) -> Result<(), VectorStoreError>;

    /// Upsert a batch of embeddings. Returns the number of items upserted.
    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError>;

    /// Delete all entries whose id starts with the given prefix.
    /// Returns the number of items deleted.
    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError>;

    /// Get a single item by its id.
    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError>;

    /// Check that the vector store is healthy.
    async fn health_check(&self) -> Result<(), VectorStoreError>;
}

#[async_trait]
impl<T: VectorStore + ?Sized> VectorStore for &T {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        (**self)
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
        (**self)
            .upsert_embedding(id, content, item_type, metadata, embedding)
            .await
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        (**self).upsert_batch(items).await
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        (**self).delete_by_prefix(id_prefix).await
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        (**self).get_by_id(id).await
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        (**self).health_check().await
    }
}

#[async_trait]
impl<T: VectorStore + ?Sized> VectorStore for Arc<T> {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        self.as_ref()
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
        self.as_ref()
            .upsert_embedding(id, content, item_type, metadata, embedding)
            .await
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        self.as_ref().upsert_batch(items).await
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        self.as_ref().delete_by_prefix(id_prefix).await
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        self.as_ref().get_by_id(id).await
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        self.as_ref().health_check().await
    }
}

// =============================================================================
// EmbeddingService — async embedding generation
// =============================================================================

/// Errors from the embedding service.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("Embedding API error: {0}")]
    Api(String),
    #[error("Embedding service not configured")]
    NotConfigured,
}

/// Async embedding generation service.
///
/// Converts text content into dense vector representations for semantic
/// similarity search. Decoupled from `VectorStore` — the store manages
/// persistence and search, while this trait manages embedding computation.
///
/// # Laws
///
/// L1 (Determinism): `embed_one(text)` returns the same vector for the same text
///     (within floating-point tolerance; model inference is deterministic with temperature=0)
/// L2 (Dimension Consistency): All returned vectors have length `== self.dimension()`
/// L3 (Batch Consistency): `embed_batch([a, b])` returns the same vectors as
///     `[embed_one(a), embed_one(b)]` (within floating-point tolerance)
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Embed a batch of texts into dense vectors.
    ///
    /// Implementations should batch API calls for efficiency.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Embed a single text. Default implementation delegates to `embed_batch`.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let results = self.embed_batch(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Api("empty response from embed_batch".into()))
    }

    /// The dimensionality of returned embedding vectors.
    fn dimension(&self) -> usize;

    /// The model identifier used for embedding generation.
    fn model_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyVectorStore;

    #[async_trait]
    impl VectorStore for DummyVectorStore {
        async fn search_similar(
            &self,
            _embedding: &[f32],
            _limit: usize,
            _min_similarity: f64,
        ) -> Result<Vec<VectorHit>, VectorStoreError> {
            Ok(vec![VectorHit {
                id: "vec:1".into(),
                content: "content".into(),
                item_type: "table".into(),
                metadata: serde_json::json!({}),
                score: 0.9,
            }])
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

        async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
            Ok(items.len())
        }

        async fn delete_by_prefix(&self, _id_prefix: &str) -> Result<usize, VectorStoreError> {
            Ok(1)
        }

        async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
            Ok(Some(VectorHit {
                id: id.to_string(),
                content: "content".into(),
                item_type: "table".into(),
                metadata: serde_json::json!({}),
                score: 1.0,
            }))
        }

        async fn health_check(&self) -> Result<(), VectorStoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn borrowed_vector_store_forwards() {
        let store = DummyVectorStore;
        let hit = (&store).get_by_id("vec:1").await.unwrap().unwrap();
        assert_eq!(hit.id, "vec:1");
    }

    #[tokio::test]
    async fn arc_vector_store_forwards() {
        let store: Arc<dyn VectorStore> = Arc::new(DummyVectorStore);
        let hits = store.search_similar(&[0.0, 1.0], 3, 0.1).await.unwrap();
        assert_eq!(hits.len(), 1);
    }
}
