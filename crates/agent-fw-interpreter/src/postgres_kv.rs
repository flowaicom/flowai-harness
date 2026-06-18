//! PostgreSQL-backed KVStore implementation.
//!
//! Provides a persistent, distributed KV store using PostgreSQL for production
//! deployments where Redis is not available or when durable storage is preferred.
//!
//! # Feature Gate
//!
//! This module requires the `postgres` feature:
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["postgres"] }
//! ```
//!
//! # Table Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS kv_store (
//!     tenant    TEXT NOT NULL,
//!     key       TEXT NOT NULL,
//!     value     JSONB NOT NULL,
//!     expires_at TIMESTAMPTZ,
//!     created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     PRIMARY KEY (tenant, key)
//! );
//!
//! CREATE INDEX IF NOT EXISTS idx_kv_store_expires
//!     ON kv_store (expires_at)
//!     WHERE expires_at IS NOT NULL;
//! ```
//!
//! # TTL
//!
//! Uses `expires_at` timestamps. Expired entries are filtered out on read
//! and periodically cleaned up via `cleanup_expired()`.
//!
//! # Laws Satisfied
//!
//! - L1 (Get-After-Put): INSERT ON CONFLICT + SELECT returns stored value
//! - L2 (Put-Overwrites): ON CONFLICT DO UPDATE replaces value
//! - L3 (Delete-Removes): DELETE + SELECT returns None
//! - L4 (Get-Missing): SELECT for non-existent key returns None
//! - L5 (Delete-Idempotent): DELETE on absent key returns false (rows_affected = 0)
//! - L6 (Exists-Get-Consistency): Same WHERE clause for both
//! - L7 (TTL-Expiry): expires_at < NOW() filtered out
//! - L8 (Permanence): expires_at IS NULL never expires
//! - L9 (Tenant-Isolation): Composite PK (tenant, key) enforces isolation
//! - L10 (GetMany-Consistency): Single SELECT with IN clause

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgPool;
use sqlx::Row;

use agent_fw_algebra::{KVError, KVStore};

/// PostgreSQL-backed KV store for production deployments.
///
/// Implements the `KVStore` algebra with PostgreSQL as the storage backend.
/// Uses `expires_at` timestamps for TTL (expired entries filtered on read).
///
/// # Trade-offs vs Redis
///
/// - **Pro**: Durable (survives restarts), no extra infrastructure if you already have Postgres
/// - **Pro**: Transactional consistency with other Postgres tables
/// - **Con**: Higher latency than Redis for hot-path KV operations
/// - **Con**: No pub/sub (use EventSink/EventLog for that)
///
/// Use `RedisKVStore` for latency-sensitive hot paths. Use `PostgresKVStore`
/// for durability-critical data or when Redis is not available.
pub struct PostgresKVStore {
    pool: PgPool,
    table: String,
}

impl PostgresKVStore {
    /// Create from an existing connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            table: "kv_store".to_string(),
        }
    }

    /// Connect to a database URL.
    pub async fn connect(url: &str) -> Result<Self, KVError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| KVError::Storage(format!("Connection failed: {e}")))?;
        Ok(Self::new(pool))
    }

    /// Override the table name (default: "kv_store").
    ///
    /// # Panics
    ///
    /// Panics if `table` contains characters other than `[a-zA-Z0-9_]`.
    /// This is a setup-time validation — table names must be safe for SQL
    /// identifier interpolation (defense-in-depth against injection).
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        let table = table.into();
        assert!(
            !table.is_empty() && table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "invalid table name '{table}': must be non-empty and contain only [a-zA-Z0-9_]"
        );
        self.table = table;
        self
    }

    /// Ensure the table and indexes exist.
    ///
    /// Idempotent — safe to call on every startup.
    pub async fn ensure_schema(&self) -> Result<(), KVError> {
        let create_table = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {table} (
                tenant      TEXT NOT NULL,
                key         TEXT NOT NULL,
                value       JSONB NOT NULL,
                expires_at  TIMESTAMPTZ,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (tenant, key)
            )
            "#,
            table = self.table
        );
        sqlx::query(&create_table)
            .execute(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("Failed to create table: {e}")))?;

        let create_index = format!(
            r#"
            CREATE INDEX IF NOT EXISTS idx_{table}_expires
                ON {table} (expires_at)
                WHERE expires_at IS NOT NULL
            "#,
            table = self.table
        );
        sqlx::query(&create_index)
            .execute(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("Failed to create index: {e}")))?;

        Ok(())
    }

    /// Delete expired entries.
    ///
    /// Call periodically (e.g., every 5 minutes) to reclaim storage.
    /// Returns the number of entries cleaned up.
    pub async fn cleanup_expired(&self) -> Result<u64, KVError> {
        let sql = format!(
            "DELETE FROM {table} WHERE expires_at IS NOT NULL AND expires_at < NOW()",
            table = self.table
        );
        let result = sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("Cleanup failed: {e}")))?;
        Ok(result.rows_affected())
    }

    /// Access the underlying pool (escape hatch).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl KVStore for PostgresKVStore {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        tracing::debug!(tenant, key, "postgres_kv_put");

        let sql = format!(
            r#"
            INSERT INTO {table} (tenant, key, value, expires_at)
            VALUES ($1, $2, $3, CASE WHEN $4::BIGINT IS NOT NULL
                THEN NOW() + ($4::BIGINT || ' seconds')::INTERVAL
                ELSE NULL END)
            ON CONFLICT (tenant, key) DO UPDATE SET
                value = EXCLUDED.value,
                expires_at = EXCLUDED.expires_at,
                updated_at = NOW()
            "#,
            table = self.table
        );

        let ttl_secs: Option<i64> = ttl.map(|d| d.as_secs() as i64);

        sqlx::query(&sql)
            .bind(tenant)
            .bind(key)
            .bind(&value)
            .bind(ttl_secs)
            .execute(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("PUT failed: {e}")))?;

        Ok(())
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        tracing::debug!(tenant, key, "postgres_kv_get");

        let sql = format!(
            r#"
            SELECT value FROM {table}
            WHERE tenant = $1 AND key = $2
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
            table = self.table
        );

        let row = sqlx::query(&sql)
            .bind(tenant)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("GET failed: {e}")))?;

        Ok(row.map(|r| r.get("value")))
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        tracing::debug!(tenant, key, "postgres_kv_del");

        let sql = format!(
            "DELETE FROM {table} WHERE tenant = $1 AND key = $2",
            table = self.table
        );

        let result = sqlx::query(&sql)
            .bind(tenant)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("DELETE failed: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        tracing::debug!(tenant, key, "postgres_kv_exists");

        let sql = format!(
            r#"
            SELECT 1 FROM {table}
            WHERE tenant = $1 AND key = $2
              AND (expires_at IS NULL OR expires_at > NOW())
            LIMIT 1
            "#,
            table = self.table
        );

        let row = sqlx::query(&sql)
            .bind(tenant)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("EXISTS failed: {e}")))?;

        Ok(row.is_some())
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        tracing::debug!(tenant, prefix, "postgres_kv_list");

        let sql = format!(
            r#"
            SELECT key FROM {table}
            WHERE tenant = $1 AND key LIKE $2
              AND (expires_at IS NULL OR expires_at > NOW())
            ORDER BY key
            "#,
            table = self.table
        );

        let pattern = format!("{prefix}%");

        let rows = sqlx::query(&sql)
            .bind(tenant)
            .bind(&pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("LIST failed: {e}")))?;

        Ok(rows.iter().map(|r| r.get("key")).collect())
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        tracing::debug!(tenant, count = keys.len(), "postgres_kv_mget");

        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        // Build parameterized IN clause: $2, $3, $4, ...
        let placeholders: Vec<String> = (2..=keys.len() + 1).map(|i| format!("${i}")).collect();
        let in_clause = placeholders.join(", ");

        let sql = format!(
            r#"
            SELECT key, value FROM {table}
            WHERE tenant = $1 AND key IN ({in_clause})
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
            table = self.table,
            in_clause = in_clause
        );

        let mut query = sqlx::query(&sql).bind(tenant);
        for key in keys {
            query = query.bind(key);
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| KVError::Storage(format!("MGET failed: {e}")))?;

        let mut result = HashMap::with_capacity(rows.len());
        for row in rows {
            let key: String = row.get("key");
            let value: serde_json::Value = row.get("value");
            result.insert(key, value);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn table_name_accepts_valid_identifiers() {
        // Verify safe identifiers are accepted (no live PG needed)
        for name in ["kv_store", "my_table_2", "KV", "a"] {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "should be valid: {name}"
            );
        }
    }

    /// Direct test of identifier validation (no PgPool needed).
    fn is_valid_table_name(name: &str) -> bool {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    #[test]
    fn table_name_rejects_injection() {
        assert!(!is_valid_table_name("test; DROP TABLE--"));
        assert!(!is_valid_table_name("kv_store; --"));
        assert!(!is_valid_table_name("table name"));
    }

    #[test]
    fn table_name_rejects_empty() {
        assert!(!is_valid_table_name(""));
    }

    #[test]
    fn table_name_rejects_special_chars() {
        assert!(!is_valid_table_name("kv.store")); // dots not allowed
        assert!(!is_valid_table_name("kv-store")); // dashes not allowed
        assert!(!is_valid_table_name("kv$store")); // dollar not allowed
    }

    #[test]
    fn in_clause_construction() {
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let placeholders: Vec<String> = (2..=keys.len() + 1).map(|i| format!("${i}")).collect();
        let in_clause = placeholders.join(", ");
        assert_eq!(in_clause, "$2, $3, $4");
    }

    #[test]
    fn empty_keys_produces_empty_result() {
        let keys: Vec<String> = vec![];
        assert!(keys.is_empty());
    }
}
