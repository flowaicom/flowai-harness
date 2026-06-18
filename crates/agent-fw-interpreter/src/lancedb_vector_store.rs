//! LanceDB-backed VectorStore implementation.
//!
//! Provides embedded vector similarity search using LanceDB, a columnar
//! database built on Apache Arrow and the Lance format. Ideal for
//! SQLite-only deployments where PostgreSQL + pgvector is unavailable.
//!
//! # Feature Gate
//!
//! This module requires the `lancedb` feature:
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["lancedb"] }
//! ```
//!
//! # Table Schema (Arrow)
//!
//! | Column    | Type                          | Nullable |
//! |-----------|-------------------------------|----------|
//! | id        | Utf8                          | false    |
//! | content   | Utf8                          | false    |
//! | item_type | Utf8                          | false    |
//! | metadata  | Utf8 (JSON string)            | false    |
//! | vector    | FixedSizeList\<Float32\>(dim)  | false    |
//!
//! # Laws Satisfied
//!
//! - L1 (Upsert Idempotency): `merge_insert` on `id` column ensures no duplicates
//! - L2 (Search Monotonicity): cosine similarity ordering is stable
//! - L3 (Delete Prefix): SQL LIKE with prefix + % deletes exactly matching entries
//! - L4 (Get After Upsert): merge_insert then query by id always returns the row
//! - L5 (Dimension Guard): `connect()` returns `DimensionMismatch` when existing
//!   table vector dimension differs from configured dimension

use std::sync::Arc;

use arrow_array::types::Float32Type;
use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

use agent_fw_algebra::vector_store::{EmbeddingItem, VectorHit, VectorStore, VectorStoreError};

/// LanceDB-backed [`VectorStore`].
///
/// Uses cosine distance for similarity search. LanceDB is embedded (no server
/// process) and stores data in the Lance columnar format, optimized for
/// vector operations.
pub struct LanceDbVectorStore {
    #[allow(dead_code)]
    db: lancedb::Connection,
    table: lancedb::Table,
    dimension: usize,
}

impl std::fmt::Debug for LanceDbVectorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanceDbVectorStore")
            .field("dimension", &self.dimension)
            .finish_non_exhaustive()
    }
}

impl LanceDbVectorStore {
    fn normalize_embedding(
        embedding: &[f32],
        dimension: usize,
    ) -> Result<Vec<Option<f32>>, VectorStoreError> {
        if embedding.is_empty() {
            return Ok(vec![Some(0.0); dimension]);
        }

        if embedding.len() != dimension {
            return Err(VectorStoreError::DimensionMismatch {
                expected: dimension,
                actual: embedding.len(),
            });
        }

        Ok(embedding.iter().map(|&value| Some(value)).collect())
    }

    /// Connect to (or create) a LanceDB database at the given path and
    /// open (or create) the embeddings table.
    ///
    /// `dimension` must match the embedding model's output dimension.
    /// Returns `DimensionMismatch` if the table already exists with a
    /// different vector dimension.
    pub async fn connect(path: &str, dimension: usize) -> Result<Self, VectorStoreError> {
        Self::connect_with_table(path, "embeddings", dimension).await
    }

    /// Connect with a custom table name.
    pub async fn connect_with_table(
        path: &str,
        table_name: &str,
        dimension: usize,
    ) -> Result<Self, VectorStoreError> {
        let db = lancedb::connect(path)
            .execute()
            .await
            .map_err(|e| VectorStoreError::Connection(format!("Failed to open LanceDB: {e}")))?;

        let table_names = db
            .table_names()
            .execute()
            .await
            .map_err(|e| VectorStoreError::Connection(format!("Failed to list tables: {e}")))?;

        let table = if table_names.iter().any(|n| n == table_name) {
            let table =
                db.open_table(table_name).execute().await.map_err(|e| {
                    VectorStoreError::Connection(format!("Failed to open table: {e}"))
                })?;

            // L5 (Dimension Guard): verify dimension matches existing schema
            let existing_schema = table
                .schema()
                .await
                .map_err(|e| VectorStoreError::Execution(format!("Failed to read schema: {e}")))?;

            if let Ok(field) = existing_schema.field_with_name("vector") {
                if let DataType::FixedSizeList(_, existing_dim) = field.data_type() {
                    let existing = *existing_dim as usize;
                    if existing != dimension {
                        return Err(VectorStoreError::DimensionMismatch {
                            expected: dimension,
                            actual: existing,
                        });
                    }
                }
            }

            table
        } else {
            let schema = Self::build_schema(dimension);
            db.create_empty_table(table_name, schema)
                .execute()
                .await
                .map_err(|e| VectorStoreError::Connection(format!("Failed to create table: {e}")))?
        };

        Ok(Self {
            db,
            table,
            dimension,
        })
    }

    /// Build the Arrow schema for the embeddings table.
    fn build_schema(dimension: usize) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("item_type", DataType::Utf8, false),
            Field::new("metadata", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dimension as i32,
                ),
                false,
            ),
        ]))
    }

    /// Build a RecordBatch from a single embedding item.
    fn build_single_batch(
        schema: &Arc<Schema>,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: &serde_json::Value,
        embedding: &[f32],
        dimension: usize,
    ) -> Result<RecordBatch, VectorStoreError> {
        let metadata_str = serde_json::to_string(metadata).map_err(|e| {
            VectorStoreError::Execution(format!("Failed to serialize metadata: {e}"))
        })?;
        let normalized = Self::normalize_embedding(embedding, dimension)?;

        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id])),
                Arc::new(StringArray::from(vec![content])),
                Arc::new(StringArray::from(vec![item_type])),
                Arc::new(StringArray::from(vec![metadata_str.as_str()])),
                Arc::new(arrow_array::FixedSizeListArray::from_iter_primitive::<
                    Float32Type,
                    _,
                    _,
                >(vec![Some(normalized)], dimension as i32)),
            ],
        )
        .map_err(|e| VectorStoreError::Execution(format!("Failed to build RecordBatch: {e}")))
    }

    /// Build a RecordBatch from multiple embedding items.
    fn build_batch(
        schema: &Arc<Schema>,
        items: &[EmbeddingItem],
        dimension: usize,
    ) -> Result<RecordBatch, VectorStoreError> {
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let contents: Vec<&str> = items.iter().map(|i| i.content.as_str()).collect();
        let item_types: Vec<&str> = items.iter().map(|i| i.item_type.as_str()).collect();
        let metadatas: Vec<String> = items
            .iter()
            .map(|i| serde_json::to_string(&i.metadata).unwrap_or_else(|_| "{}".to_string()))
            .collect();
        let metadata_refs: Vec<&str> = metadatas.iter().map(|s| s.as_str()).collect();

        let embeddings: Vec<Option<Vec<Option<f32>>>> = items
            .iter()
            .map(|item| Self::normalize_embedding(&item.embedding, dimension).map(Some))
            .collect::<Result<_, _>>()?;

        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(contents)),
                Arc::new(StringArray::from(item_types)),
                Arc::new(StringArray::from(metadata_refs)),
                Arc::new(arrow_array::FixedSizeListArray::from_iter_primitive::<
                    Float32Type,
                    _,
                    _,
                >(embeddings, dimension as i32)),
            ],
        )
        .map_err(|e| VectorStoreError::Execution(format!("Failed to build RecordBatch: {e}")))
    }

    /// Extract VectorHits from vector search result RecordBatches.
    ///
    /// LanceDB adds a `_distance` column (cosine distance in \[0, 2\]).
    /// We convert to similarity: `score = 1.0 - distance`.
    fn extract_hits(batches: &[RecordBatch], min_similarity: f64) -> Vec<VectorHit> {
        let mut hits = Vec::new();

        for batch in batches {
            let num_rows = batch.num_rows();
            if num_rows == 0 {
                continue;
            }

            let ids = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let contents = batch
                .column_by_name("content")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let item_types = batch
                .column_by_name("item_type")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let metadatas = batch
                .column_by_name("metadata")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            let (Some(ids), Some(contents), Some(item_types), Some(metadatas), Some(distances)) =
                (ids, contents, item_types, metadatas, distances)
            else {
                continue;
            };

            for i in 0..num_rows {
                let distance = distances.value(i) as f64;
                let score = 1.0 - distance;

                if score < min_similarity {
                    continue;
                }

                let metadata: serde_json::Value =
                    serde_json::from_str(metadatas.value(i)).unwrap_or(serde_json::json!({}));

                hits.push(VectorHit {
                    id: ids.value(i).to_string(),
                    content: contents.value(i).to_string(),
                    item_type: item_types.value(i).to_string(),
                    metadata,
                    score,
                });
            }
        }

        hits
    }

    /// Extract a single VectorHit from query result RecordBatches (no _distance column).
    fn extract_single(batches: &[RecordBatch]) -> Option<VectorHit> {
        for batch in batches {
            if batch.num_rows() == 0 {
                continue;
            }

            let ids = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
            let contents = batch
                .column_by_name("content")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
            let item_types = batch
                .column_by_name("item_type")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
            let metadatas = batch
                .column_by_name("metadata")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;

            let metadata: serde_json::Value =
                serde_json::from_str(metadatas.value(0)).unwrap_or(serde_json::json!({}));

            return Some(VectorHit {
                id: ids.value(0).to_string(),
                content: contents.value(0).to_string(),
                item_type: item_types.value(0).to_string(),
                metadata,
                score: 1.0, // Exact lookup
            });
        }

        None
    }

    /// Escape a string value for use in LanceDB SQL predicates.
    fn escape_sql(s: &str) -> String {
        s.replace('\'', "''")
    }
}

#[async_trait]
impl VectorStore for LanceDbVectorStore {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        // Empty table: vector_search may fail, return empty results
        let count = self
            .table
            .count_rows(None)
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Failed to count rows: {e}")))?;

        if count == 0 {
            return Ok(vec![]);
        }

        let batches: Vec<RecordBatch> = self
            .table
            .vector_search(embedding)
            .map_err(|e| {
                VectorStoreError::Execution(format!("Failed to build vector search: {e}"))
            })?
            .column("vector")
            .distance_type(lancedb::DistanceType::Cosine)
            .limit(limit)
            .execute()
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Vector search failed: {e}")))?
            .try_collect()
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Failed to collect results: {e}")))?;

        Ok(Self::extract_hits(&batches, min_similarity))
    }

    async fn upsert_embedding(
        &self,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: serde_json::Value,
        embedding: &[f32],
    ) -> Result<(), VectorStoreError> {
        let schema = Self::build_schema(self.dimension);
        let batch = Self::build_single_batch(
            &schema,
            id,
            content,
            item_type,
            &metadata,
            embedding,
            self.dimension,
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);

        let mut merge = self.table.merge_insert(&["id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge
            .execute(Box::new(batches))
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Upsert failed: {e}")))?;

        Ok(())
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        if items.is_empty() {
            return Ok(0);
        }

        let schema = Self::build_schema(self.dimension);
        let batch = Self::build_batch(&schema, items, self.dimension)?;
        let count = batch.num_rows();

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);

        let mut merge = self.table.merge_insert(&["id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge
            .execute(Box::new(batches))
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Batch upsert failed: {e}")))?;

        Ok(count)
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        let escaped = Self::escape_sql(id_prefix);
        let predicate = format!("id LIKE '{escaped}%'");

        let count = self
            .table
            .count_rows(Some(predicate.clone()))
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Failed to count rows: {e}")))?;

        if count > 0 {
            self.table
                .delete(&predicate)
                .await
                .map_err(|e| VectorStoreError::Execution(format!("Delete failed: {e}")))?;
        }

        Ok(count)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        let escaped = Self::escape_sql(id);
        let predicate = format!("id = '{escaped}'");

        let batches: Vec<RecordBatch> = self
            .table
            .query()
            .only_if(predicate)
            .limit(1)
            .execute()
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Get by id failed: {e}")))?
            .try_collect()
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Failed to collect results: {e}")))?;

        Ok(Self::extract_single(&batches))
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        self.table
            .count_rows(None)
            .await
            .map_err(|e| VectorStoreError::Connection(format!("Health check failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_store(dimension: usize) -> LanceDbVectorStore {
        let dir = tempfile::tempdir().unwrap();
        LanceDbVectorStore::connect(dir.path().to_str().unwrap(), dimension)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn upsert_and_get() {
        let store = temp_store(3).await;
        store
            .upsert_embedding(
                "item-1",
                "hello world",
                "doc",
                serde_json::json!({"key": "value"}),
                &[1.0, 0.0, 0.0],
            )
            .await
            .unwrap();

        let hit = store.get_by_id("item-1").await.unwrap();
        assert!(hit.is_some());
        let hit = hit.unwrap();
        assert_eq!(hit.content, "hello world");
        assert_eq!(hit.item_type, "doc");
        assert_eq!(hit.metadata["key"], "value");

        assert!(store.get_by_id("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_empty_embedding_uses_zero_vector_sentinel() {
        let store = temp_store(2).await;
        store
            .upsert_embedding("meta", "hash", "_meta", serde_json::json!({}), &[])
            .await
            .unwrap();

        let hit = store.get_by_id("meta").await.unwrap().unwrap();
        assert_eq!(hit.content, "hash");
        assert_eq!(hit.item_type, "_meta");
    }

    #[tokio::test]
    async fn upsert_idempotent() {
        let store = temp_store(3).await;

        store
            .upsert_embedding("a", "v1", "t", serde_json::json!({}), &[1.0, 0.0, 0.0])
            .await
            .unwrap();
        store
            .upsert_embedding("a", "v2", "t", serde_json::json!({}), &[1.0, 0.0, 0.0])
            .await
            .unwrap();

        let hit = store.get_by_id("a").await.unwrap().unwrap();
        assert_eq!(hit.content, "v2"); // Overwritten
    }

    #[tokio::test]
    async fn search_returns_similar() {
        let store = temp_store(3).await;

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
    async fn search_empty_table() {
        let store = temp_store(3).await;
        let hits = store
            .search_similar(&[1.0, 0.0, 0.0], 10, 0.0)
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_matching() {
        let store = temp_store(2).await;

        store
            .upsert_embedding("tbl-1", "t1", "table", serde_json::json!({}), &[1.0, 0.0])
            .await
            .unwrap();
        store
            .upsert_embedding("tbl-2", "t2", "table", serde_json::json!({}), &[1.0, 0.0])
            .await
            .unwrap();
        store
            .upsert_embedding("col-1", "c1", "column", serde_json::json!({}), &[0.0, 1.0])
            .await
            .unwrap();

        let deleted = store.delete_by_prefix("tbl-").await.unwrap();
        assert_eq!(deleted, 2);

        let total = store.table.count_rows(None).await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn upsert_batch_works() {
        let store = temp_store(2).await;

        let items = vec![
            EmbeddingItem {
                id: "x".into(),
                content: "xx".into(),
                item_type: "t".into(),
                metadata: serde_json::json!({}),
                embedding: vec![1.0, 0.0],
            },
            EmbeddingItem {
                id: "y".into(),
                content: "yy".into(),
                item_type: "t".into(),
                metadata: serde_json::json!({}),
                embedding: vec![0.0, 1.0],
            },
        ];

        let count = store.upsert_batch(&items).await.unwrap();
        assert_eq!(count, 2);

        let total = store.table.count_rows(None).await.unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn health_check_works() {
        let store = temp_store(3).await;
        store.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn dimension_mismatch_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        // Create with dimension 3
        let _store = LanceDbVectorStore::connect(path, 3).await.unwrap();

        // Reconnect with different dimension → error
        let err = LanceDbVectorStore::connect(path, 5).await.unwrap_err();
        assert!(matches!(
            err,
            VectorStoreError::DimensionMismatch {
                expected: 5,
                actual: 3
            }
        ));
    }
}
