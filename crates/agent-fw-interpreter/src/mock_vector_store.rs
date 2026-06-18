//! Mock VectorStore — in-memory implementation for testing.
//!
//! Stores embeddings in a DashMap and performs brute-force cosine similarity
//! for search.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use agent_fw_algebra::vector_store::{EmbeddingItem, VectorHit, VectorStore, VectorStoreError};

#[derive(Clone)]
struct StoredItem {
    id: String,
    content: String,
    item_type: String,
    metadata: serde_json::Value,
    embedding: Vec<f32>,
}

/// In-memory vector store using brute-force cosine similarity.
pub struct MockVectorStore {
    items: Arc<DashMap<String, StoredItem>>,
}

impl MockVectorStore {
    pub fn new() -> Self {
        Self {
            items: Arc::new(DashMap::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Default for MockVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum();
    let mag_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

#[async_trait]
impl VectorStore for MockVectorStore {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        let mut hits: Vec<(f64, VectorHit)> = self
            .items
            .iter()
            .map(|entry| {
                let item = entry.value();
                let score = cosine_similarity(embedding, &item.embedding);
                (
                    score,
                    VectorHit {
                        id: item.id.clone(),
                        content: item.content.clone(),
                        item_type: item.item_type.clone(),
                        metadata: item.metadata.clone(),
                        score,
                    },
                )
            })
            .filter(|(score, _)| *score >= min_similarity)
            .collect();

        hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);

        Ok(hits.into_iter().map(|(_, hit)| hit).collect())
    }

    async fn upsert_embedding(
        &self,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: serde_json::Value,
        embedding: &[f32],
    ) -> Result<(), VectorStoreError> {
        self.items.insert(
            id.to_string(),
            StoredItem {
                id: id.to_string(),
                content: content.to_string(),
                item_type: item_type.to_string(),
                metadata,
                embedding: embedding.to_vec(),
            },
        );
        Ok(())
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        let count = items.len();
        for item in items {
            self.items.insert(
                item.id.clone(),
                StoredItem {
                    id: item.id.clone(),
                    content: item.content.clone(),
                    item_type: item.item_type.clone(),
                    metadata: item.metadata.clone(),
                    embedding: item.embedding.clone(),
                },
            );
        }
        Ok(count)
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        let keys: Vec<String> = self
            .items
            .iter()
            .filter(|e| e.key().starts_with(id_prefix))
            .map(|e| e.key().clone())
            .collect();
        let count = keys.len();
        for key in keys {
            self.items.remove(&key);
        }
        Ok(count)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        Ok(self.items.get(id).map(|entry| {
            let item = entry.value();
            VectorHit {
                id: item.id.clone(),
                content: item.content.clone(),
                item_type: item.item_type.clone(),
                metadata: item.metadata.clone(),
                score: 1.0,
            }
        }))
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_get() {
        let store = MockVectorStore::new();
        store
            .upsert_embedding(
                "item-1",
                "hello world",
                "doc",
                serde_json::json!({}),
                &[1.0, 0.0],
            )
            .await
            .unwrap();

        let hit = store.get_by_id("item-1").await.unwrap();
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().content, "hello world");

        assert!(store.get_by_id("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn search_returns_similar() {
        let store = MockVectorStore::new();
        store
            .upsert_embedding(
                "a",
                "apple",
                "fruit",
                serde_json::json!({}),
                &[1.0, 0.0, 0.0],
            )
            .await
            .unwrap();
        store
            .upsert_embedding(
                "b",
                "banana",
                "fruit",
                serde_json::json!({}),
                &[0.9, 0.1, 0.0],
            )
            .await
            .unwrap();
        store
            .upsert_embedding(
                "c",
                "car",
                "vehicle",
                serde_json::json!({}),
                &[0.0, 0.0, 1.0],
            )
            .await
            .unwrap();

        let hits = store
            .search_similar(&[1.0, 0.0, 0.0], 2, 0.5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "a"); // Exact match
        assert_eq!(hits[1].id, "b"); // Close
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_matching() {
        let store = MockVectorStore::new();
        store
            .upsert_embedding("tbl-1", "t1", "table", serde_json::json!({}), &[1.0])
            .await
            .unwrap();
        store
            .upsert_embedding("tbl-2", "t2", "table", serde_json::json!({}), &[1.0])
            .await
            .unwrap();
        store
            .upsert_embedding("col-1", "c1", "column", serde_json::json!({}), &[1.0])
            .await
            .unwrap();

        let deleted = store.delete_by_prefix("tbl-").await.unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn upsert_batch_works() {
        let store = MockVectorStore::new();
        let items = vec![
            EmbeddingItem {
                id: "x".into(),
                content: "xx".into(),
                item_type: "t".into(),
                metadata: serde_json::json!({}),
                embedding: vec![1.0],
            },
            EmbeddingItem {
                id: "y".into(),
                content: "yy".into(),
                item_type: "t".into(),
                metadata: serde_json::json!({}),
                embedding: vec![0.0],
            },
        ];
        let count = store.upsert_batch(&items).await.unwrap();
        assert_eq!(count, 2);
        assert_eq!(store.len(), 2);
    }
}
