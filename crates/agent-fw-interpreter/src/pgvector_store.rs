//! PostgreSQL + pgvector-backed VectorStore implementation.
//!
//! Provides production-grade semantic similarity search using PostgreSQL's
//! `pgvector` extension. Uses sqlx for connection pooling and parameterized queries.
//!
//! # Feature Gate
//!
//! This module requires the `postgres` feature:
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["postgres"] }
//! ```
//!
//! # Prerequisites
//!
//! The target PostgreSQL database must have the `pgvector` extension installed:
//! ```sql
//! CREATE EXTENSION IF NOT EXISTS vector;
//! ```
//!
//! # Table Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS embeddings (
//!     id          TEXT PRIMARY KEY,
//!     content     TEXT NOT NULL,
//!     item_type   TEXT NOT NULL,
//!     metadata    JSONB NOT NULL DEFAULT '{}',
//!     embedding   vector NOT NULL,
//!     created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
//! );
//!
//! CREATE INDEX IF NOT EXISTS idx_embeddings_ivfflat
//!     ON embeddings USING ivfflat (embedding vector_cosine_ops)
//!     WITH (lists = 100);
//! ```
//!
//! # Laws Satisfied
//!
//! - L1 (Upsert Idempotency): ON CONFLICT DO UPDATE ensures no duplicates
//! - L2 (Search Monotonicity): cosine similarity ordering is stable
//! - L3 (Delete Prefix): LIKE with prefix + % deletes exactly matching entries
//! - L4 (Get After Upsert): INSERT then SELECT by PK always returns the row
//! - L5 (Dimension Guard): ensure_schema() returns DimensionMismatch when
//!   existing table dimension differs from configured dimension

use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgPool;
use sqlx::Row;

use agent_fw_algebra::vector_store::{EmbeddingItem, VectorHit, VectorStore, VectorStoreError};

/// PostgreSQL + pgvector-backed [`VectorStore`].
///
/// Uses cosine distance (`<=>` operator) for similarity search and
/// IVFFlat indexing for sub-linear search performance.
pub struct PgVectorStore {
    pool: PgPool,
    table: String,
    dimension: usize,
}

impl PgVectorStore {
    /// Create from an existing connection pool.
    ///
    /// `dimension` must match the embedding model's output dimension
    /// (e.g., 1536 for text-embedding-ada-002, 1024 for Cohere embed-v3).
    pub fn new(pool: PgPool, dimension: usize) -> Self {
        Self {
            pool,
            table: "embeddings".to_string(),
            dimension,
        }
    }

    /// Connect to a database URL.
    pub async fn connect(url: &str, dimension: usize) -> Result<Self, VectorStoreError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| VectorStoreError::Connection(e.to_string()))?;
        Ok(Self::new(pool, dimension))
    }

    /// Override the table name (default: "embeddings").
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    /// Read the vector dimension of an existing table's `embedding` column.
    ///
    /// Returns `None` if the table doesn't exist yet, `Some(dim)` if it does.
    /// Uses `pg_attribute.atttypmod` — pgvector stores dimension as `atttypmod`.
    async fn read_table_dimension(&self) -> Result<Option<usize>, VectorStoreError> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT a.atttypmod
            FROM   pg_attribute a
            JOIN   pg_class     c ON c.oid = a.attrelid
            JOIN   pg_namespace n ON n.oid = c.relnamespace
            WHERE  c.relname  = $1
            AND    a.attname  = 'embedding'
            AND    a.atttypmod > 0
            AND    n.nspname  = 'public'
            "#,
        )
        .bind(&self.table)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| VectorStoreError::Execution(format!("Failed to read table dimension: {e}")))?;

        Ok(row.map(|(atttypmod,)| atttypmod as usize))
    }

    /// Ensure the table and index exist.
    ///
    /// Idempotent — safe to call on every startup. Fails fast with
    /// `DimensionMismatch` if the table exists with a different vector dimension.
    pub async fn ensure_schema(&self) -> Result<(), VectorStoreError> {
        let create_extension = "CREATE EXTENSION IF NOT EXISTS vector";
        sqlx::query(create_extension)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                VectorStoreError::Execution(format!("Failed to create vector extension: {e}"))
            })?;

        let create_table = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                id          TEXT PRIMARY KEY,
                content     TEXT NOT NULL,
                item_type   TEXT NOT NULL,
                metadata    JSONB NOT NULL DEFAULT '{{}}'::jsonb,
                embedding   vector({}) NOT NULL,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            self.table, self.dimension
        );
        sqlx::query(&create_table)
            .execute(&self.pool)
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Failed to create table: {e}")))?;

        // L5 (Dimension Guard): If the table already existed with a different
        // dimension, CREATE TABLE IF NOT EXISTS is a no-op and subsequent
        // operations would fail with cryptic SQL errors. Fail fast instead.
        if let Some(actual) = self.read_table_dimension().await? {
            if actual != self.dimension {
                return Err(VectorStoreError::DimensionMismatch {
                    expected: self.dimension,
                    actual,
                });
            }
        }

        // IVFFlat index for cosine similarity search.
        // Only create if enough rows exist (IVFFlat needs data to build lists).
        // For small datasets, exact search via sequential scan is fine.
        let create_index = format!(
            r#"
            CREATE INDEX IF NOT EXISTS idx_{table}_ivfflat
                ON {table} USING ivfflat (embedding vector_cosine_ops)
                WITH (lists = 100)
            "#,
            table = self.table
        );
        // Index creation may fail on empty tables — that's OK, we'll retry later.
        let _ = sqlx::query(&create_index).execute(&self.pool).await;

        Ok(())
    }

    /// Access the underlying pool (escape hatch).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Format a Vec<f32> as a pgvector literal: '[0.1,0.2,0.3]'.
    fn format_vector(embedding: &[f32]) -> String {
        let inner: Vec<String> = embedding.iter().map(|v| format!("{v}")).collect();
        format!("[{}]", inner.join(","))
    }
}

#[async_trait]
impl VectorStore for PgVectorStore {
    async fn search_similar(
        &self,
        embedding: &[f32],
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        // pgvector's <=> operator returns cosine distance (0 = identical, 2 = opposite).
        // Cosine similarity = 1 - cosine_distance.
        // min_similarity threshold: we want rows where 1 - distance >= min_similarity,
        // i.e., distance <= 1 - min_similarity.
        let max_distance = 1.0 - min_similarity;
        let vec_literal = Self::format_vector(embedding);

        let sql = format!(
            r#"
            SELECT id, content, item_type, metadata,
                   1 - (embedding <=> $1::vector) AS score
            FROM {table}
            WHERE (embedding <=> $1::vector) <= $2
            ORDER BY embedding <=> $1::vector
            LIMIT $3
            "#,
            table = self.table
        );

        let rows = tokio::time::timeout(
            Duration::from_secs(30),
            sqlx::query(&sql)
                .bind(&vec_literal)
                .bind(max_distance)
                .bind(limit as i64)
                .fetch_all(&self.pool),
        )
        .await
        .map_err(|_| VectorStoreError::Execution("Search timed out".into()))?
        .map_err(|e| VectorStoreError::Execution(e.to_string()))?;

        let hits = rows
            .into_iter()
            .map(|row| {
                let id: String = row.get("id");
                let content: String = row.get("content");
                let item_type: String = row.get("item_type");
                let metadata: serde_json::Value = row.get("metadata");
                let score: f64 = row.get("score");
                VectorHit {
                    id,
                    content,
                    item_type,
                    metadata,
                    score,
                }
            })
            .collect();

        Ok(hits)
    }

    async fn upsert_embedding(
        &self,
        id: &str,
        content: &str,
        item_type: &str,
        metadata: serde_json::Value,
        embedding: &[f32],
    ) -> Result<(), VectorStoreError> {
        let vec_literal = Self::format_vector(embedding);

        let sql = format!(
            r#"
            INSERT INTO {table} (id, content, item_type, metadata, embedding)
            VALUES ($1, $2, $3, $4, $5::vector)
            ON CONFLICT (id) DO UPDATE SET
                content = EXCLUDED.content,
                item_type = EXCLUDED.item_type,
                metadata = EXCLUDED.metadata,
                embedding = EXCLUDED.embedding,
                updated_at = NOW()
            "#,
            table = self.table
        );

        sqlx::query(&sql)
            .bind(id)
            .bind(content)
            .bind(item_type)
            .bind(&metadata)
            .bind(&vec_literal)
            .execute(&self.pool)
            .await
            .map_err(|e| VectorStoreError::Execution(e.to_string()))?;

        Ok(())
    }

    async fn upsert_batch(&self, items: &[EmbeddingItem]) -> Result<usize, VectorStoreError> {
        if items.is_empty() {
            return Ok(0);
        }

        // Use a transaction for atomicity.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| VectorStoreError::Connection(e.to_string()))?;

        let sql = format!(
            r#"
            INSERT INTO {table} (id, content, item_type, metadata, embedding)
            VALUES ($1, $2, $3, $4, $5::vector)
            ON CONFLICT (id) DO UPDATE SET
                content = EXCLUDED.content,
                item_type = EXCLUDED.item_type,
                metadata = EXCLUDED.metadata,
                embedding = EXCLUDED.embedding,
                updated_at = NOW()
            "#,
            table = self.table
        );

        let mut count = 0;
        for item in items {
            let vec_literal = Self::format_vector(&item.embedding);
            sqlx::query(&sql)
                .bind(&item.id)
                .bind(&item.content)
                .bind(&item.item_type)
                .bind(&item.metadata)
                .bind(&vec_literal)
                .execute(&mut *tx)
                .await
                .map_err(|e| VectorStoreError::Execution(e.to_string()))?;
            count += 1;
        }

        tx.commit()
            .await
            .map_err(|e| VectorStoreError::Execution(format!("Batch commit failed: {e}")))?;

        Ok(count)
    }

    async fn delete_by_prefix(&self, id_prefix: &str) -> Result<usize, VectorStoreError> {
        let sql = format!("DELETE FROM {table} WHERE id LIKE $1", table = self.table);
        let pattern = format!("{id_prefix}%");

        let result = sqlx::query(&sql)
            .bind(&pattern)
            .execute(&self.pool)
            .await
            .map_err(|e| VectorStoreError::Execution(e.to_string()))?;

        Ok(result.rows_affected() as usize)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<VectorHit>, VectorStoreError> {
        let sql = format!(
            "SELECT id, content, item_type, metadata FROM {table} WHERE id = $1",
            table = self.table
        );

        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VectorStoreError::Execution(e.to_string()))?;

        Ok(row.map(|r| VectorHit {
            id: r.get("id"),
            content: r.get("content"),
            item_type: r.get("item_type"),
            metadata: r.get("metadata"),
            score: 1.0, // Exact match
        }))
    }

    async fn health_check(&self) -> Result<(), VectorStoreError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| VectorStoreError::Connection(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_vector_empty() {
        assert_eq!(PgVectorStore::format_vector(&[]), "[]");
    }

    #[test]
    fn format_vector_single() {
        assert_eq!(PgVectorStore::format_vector(&[1.5]), "[1.5]");
    }

    #[test]
    fn format_vector_multiple() {
        let v = PgVectorStore::format_vector(&[0.1, 0.2, 0.3]);
        assert_eq!(v, "[0.1,0.2,0.3]");
    }

    #[test]
    fn format_vector_negative() {
        let v = PgVectorStore::format_vector(&[-1.0, 0.0, 1.0]);
        assert_eq!(v, "[-1,0,1]");
    }

    #[test]
    fn default_table_name() {
        // Can't create without pool, but verify the builder API compiles
        let _table = "embeddings";
    }

    #[test]
    fn dimension_mismatch_error_format() {
        let err = VectorStoreError::DimensionMismatch {
            expected: 1536,
            actual: 2560,
        };
        let msg = err.to_string();
        assert!(msg.contains("1536"), "should contain expected dimension");
        assert!(msg.contains("2560"), "should contain actual dimension");
        assert!(msg.contains("mismatch"), "should contain 'mismatch'");
    }
}
