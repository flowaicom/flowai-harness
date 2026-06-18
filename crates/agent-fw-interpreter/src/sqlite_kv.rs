//! SQLite-backed KVStore with TTL support.
//!
//! Provides persistent, zero-config KV storage with WAL mode for concurrent reads.
//!
//! # TTL Semantics
//!
//! - `None` TTL = permanent (never expires). Satisfies law L8.
//! - `Some(d)` TTL = expires `d` after insertion. Satisfies law L7.
//!
//! Expiry is checked lazily on reads (Rust-side check + DELETE) and proactively
//! via `cleanup_expired()`.

use agent_fw_algebra::kv_store::{KVError, KVStore};
use async_trait::async_trait;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// SQLite-backed KVStore.
///
/// Keys are scoped by tenant for isolation. Values are stored as JSON text.
/// Uses WAL mode for concurrent read access (file-backed) and a single
/// `Mutex<Connection>` for thread safety.
#[derive(Clone)]
pub struct SqliteKVStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteKVStore {
    /// Open (or create) a KV store backed by a file.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, KVError> {
        let conn = Connection::open(path)
            .map_err(|e| KVError::Storage(format!("Failed to open database: {e}")))?;
        Self::from_connection(conn, true)
    }

    /// Create an in-memory KV store (for testing).
    pub fn in_memory() -> Result<Self, KVError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| KVError::Storage(format!("Failed to open in-memory database: {e}")))?;
        Self::from_connection(conn, false)
    }

    fn from_connection(conn: Connection, use_wal: bool) -> Result<Self, KVError> {
        let pragmas = if use_wal {
            "PRAGMA journal_mode = WAL;\nPRAGMA busy_timeout = 5000;"
        } else {
            "PRAGMA busy_timeout = 5000;"
        };

        conn.execute_batch(&format!(
            "{pragmas}
             CREATE TABLE IF NOT EXISTS kv_store (
                 tenant TEXT NOT NULL,
                 key    TEXT NOT NULL,
                 value  TEXT NOT NULL,
                 expires_at INTEGER,
                 PRIMARY KEY (tenant, key)
             );"
        ))
        .map_err(|e| KVError::Storage(format!("Failed to initialize schema: {e}")))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Remove all expired entries.
    ///
    /// Call periodically for proactive garbage collection.
    /// Permanent entries (expires_at IS NULL) are never removed.
    pub fn cleanup_expired(&self) -> Result<usize, KVError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
        let now = now_epoch_millis()?;
        let count = conn
            .execute(
                "DELETE FROM kv_store WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                rusqlite::params![now],
            )
            .map_err(|e| KVError::Storage(format!("Cleanup failed: {e}")))?;
        Ok(count)
    }
}

/// Current epoch millis as a total function.
///
/// Returns `Err` if the system clock is before UNIX_EPOCH — a condition that
/// should never occur in practice, but we refuse to panic on it.
fn now_epoch_millis() -> Result<i64, KVError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|_| KVError::Storage("System clock is before UNIX_EPOCH".into()))
}

fn expires_at(ttl: Option<Duration>) -> Result<Option<i64>, KVError> {
    match ttl {
        None => Ok(None),
        Some(d) => Ok(Some(now_epoch_millis()? + d.as_millis() as i64)),
    }
}

#[async_trait]
impl KVStore for SqliteKVStore {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let key = key.to_string();
        let value_str =
            serde_json::to_string(&value).map_err(|e| KVError::Serialization(e.to_string()))?;
        let exp = expires_at(ttl)?;

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            conn.execute(
                "INSERT OR REPLACE INTO kv_store (tenant, key, value, expires_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![tenant, key, value_str, exp],
            )
            .map_err(|e| KVError::Storage(format!("Put failed: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            let now = now_epoch_millis()?;

            let result: Result<(String, Option<i64>), _> = conn.query_row(
                "SELECT value, expires_at FROM kv_store WHERE tenant = ?1 AND key = ?2",
                rusqlite::params![tenant, key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            );

            match result {
                Ok((value_str, exp)) => {
                    // Lazy expiry check
                    if let Some(exp_at) = exp {
                        if exp_at <= now {
                            // Expired — remove and return None
                            let _ = conn.execute(
                                "DELETE FROM kv_store WHERE tenant = ?1 AND key = ?2",
                                rusqlite::params![tenant, key],
                            );
                            return Ok(None);
                        }
                    }
                    let value: serde_json::Value = serde_json::from_str(&value_str)
                        .map_err(|e| KVError::Serialization(e.to_string()))?;
                    Ok(Some(value))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(KVError::Storage(format!("Get failed: {e}"))),
            }
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            let count = conn
                .execute(
                    "DELETE FROM kv_store WHERE tenant = ?1 AND key = ?2",
                    rusqlite::params![tenant, key],
                )
                .map_err(|e| KVError::Storage(format!("Delete failed: {e}")))?;
            Ok(count > 0)
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            let now = now_epoch_millis()?;

            let result: Result<(Option<i64>,), _> = conn.query_row(
                "SELECT expires_at FROM kv_store WHERE tenant = ?1 AND key = ?2",
                rusqlite::params![tenant, key],
                |row| Ok((row.get(0)?,)),
            );

            match result {
                Ok((exp,)) => {
                    if let Some(exp_at) = exp {
                        if exp_at <= now {
                            let _ = conn.execute(
                                "DELETE FROM kv_store WHERE tenant = ?1 AND key = ?2",
                                rusqlite::params![tenant, key],
                            );
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
                Err(e) => Err(KVError::Storage(format!("Exists failed: {e}"))),
            }
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let keys = keys.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            let now = now_epoch_millis()?;

            let mut result = HashMap::with_capacity(keys.len());
            let mut expired_keys = Vec::new();

            for key in &keys {
                let row: Result<(String, Option<i64>), _> = conn.query_row(
                    "SELECT value, expires_at FROM kv_store WHERE tenant = ?1 AND key = ?2",
                    rusqlite::params![tenant, key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                );

                match row {
                    Ok((value_str, exp)) => {
                        if let Some(exp_at) = exp {
                            if exp_at <= now {
                                expired_keys.push(key.clone());
                                continue;
                            }
                        }
                        let value: serde_json::Value = serde_json::from_str(&value_str)
                            .map_err(|e| KVError::Serialization(e.to_string()))?;
                        result.insert(key.clone(), value);
                    }
                    Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(e) => return Err(KVError::Storage(format!("GetMany failed: {e}"))),
                }
            }

            // Clean up expired keys
            for key in &expired_keys {
                let _ = conn.execute(
                    "DELETE FROM kv_store WHERE tenant = ?1 AND key = ?2",
                    rusqlite::params![tenant, key],
                );
            }

            Ok(result)
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        let conn = self.conn.clone();
        let tenant = tenant.to_string();
        let prefix = prefix.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| KVError::Storage(format!("Lock poisoned: {e}")))?;
            let now = now_epoch_millis()?;
            let like_pattern = format!("{}%", prefix);

            let mut stmt = conn
                .prepare(
                    "SELECT key FROM kv_store WHERE tenant = ?1 AND key LIKE ?2 AND (expires_at IS NULL OR expires_at > ?3)",
                )
                .map_err(|e| KVError::Storage(format!("Prepare failed: {e}")))?;

            let keys: Vec<String> = stmt
                .query_map(rusqlite::params![tenant, like_pattern, now], |row| {
                    row.get(0)
                })
                .map_err(|e| KVError::Storage(format!("ListKeys failed: {e}")))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| KVError::Storage(format!("ListKeys row error: {e}")))?;

            Ok(keys)
        })
        .await
        .map_err(|e| KVError::Storage(format!("Task join error: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::kv_store::KVStoreExt;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestValue {
        name: String,
        count: u32,
    }

    fn test_store() -> SqliteKVStore {
        SqliteKVStore::in_memory().unwrap()
    }

    #[tokio::test]
    async fn basic_put_get() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store.put("tenant1", "key1", &value, None).await.unwrap();
        let retrieved: Option<TestValue> = store.get("tenant1", "key1").await.unwrap();
        assert_eq!(retrieved, Some(value));
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store.put("tenant1", "key1", &value, None).await.unwrap();
        let retrieved: Option<TestValue> = store.get("tenant2", "key1").await.unwrap();
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store
            .put("tenant1", "key1", &value, Some(Duration::from_millis(1)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let retrieved: Option<TestValue> = store.get("tenant1", "key1").await.unwrap();
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store.put("tenant1", "key1", &value, None).await.unwrap();
        assert!(store.exists("tenant1", "key1").await.unwrap());

        let deleted = store.delete("tenant1", "key1").await.unwrap();
        assert!(deleted);
        assert!(!store.exists("tenant1", "key1").await.unwrap());
    }

    #[tokio::test]
    async fn list_keys_with_prefix() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store
            .put("tenant1", "plan:abc", &value, None)
            .await
            .unwrap();
        store
            .put("tenant1", "plan:def", &value, None)
            .await
            .unwrap();
        store
            .put("tenant1", "product:xyz", &value, None)
            .await
            .unwrap();

        let mut plan_keys = store.list_keys("tenant1", "plan:").await.unwrap();
        plan_keys.sort();
        assert_eq!(plan_keys.len(), 2);
        assert!(plan_keys.contains(&"plan:abc".to_string()));
        assert!(plan_keys.contains(&"plan:def".to_string()));
    }

    #[tokio::test]
    async fn cleanup_expired_removes_old_entries() {
        let store = test_store();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store
            .put("tenant1", "short", &value, Some(Duration::from_millis(1)))
            .await
            .unwrap();
        store
            .put("tenant1", "long", &value, Some(Duration::from_secs(3600)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let removed = store.cleanup_expired().unwrap();
        assert_eq!(removed, 1);

        // Long TTL entry remains
        let long: Option<TestValue> = store.get("tenant1", "long").await.unwrap();
        assert!(long.is_some());
    }

    #[tokio::test]
    async fn permanent_entry_survives_cleanup() {
        let store = test_store();
        let value = TestValue {
            name: "permanent".to_string(),
            count: 1,
        };

        store.put("tenant1", "forever", &value, None).await.unwrap();
        store
            .put(
                "tenant1",
                "ephemeral",
                &value,
                Some(Duration::from_millis(1)),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
        store.cleanup_expired().unwrap();

        let retrieved: Option<TestValue> = store.get("tenant1", "forever").await.unwrap();
        assert_eq!(retrieved, Some(value));
        let gone: Option<TestValue> = store.get("tenant1", "ephemeral").await.unwrap();
        assert_eq!(gone, None);
    }

    #[tokio::test]
    async fn get_many_returns_existing_keys() {
        let store = test_store();
        let v1 = TestValue {
            name: "a".into(),
            count: 1,
        };
        let v2 = TestValue {
            name: "b".into(),
            count: 2,
        };

        store.put("t", "k1", &v1, None).await.unwrap();
        store.put("t", "k2", &v2, None).await.unwrap();

        let keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let result = store.get_many_json("t", &keys).await.unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains_key("k1"));
        assert!(result.contains_key("k2"));
        assert!(!result.contains_key("k3"));
    }

    #[tokio::test]
    async fn put_overwrites() {
        let store = test_store();
        let v1 = TestValue {
            name: "first".into(),
            count: 1,
        };
        let v2 = TestValue {
            name: "second".into(),
            count: 2,
        };

        store.put("t", "k", &v1, None).await.unwrap();
        store.put("t", "k", &v2, None).await.unwrap();
        let retrieved: Option<TestValue> = store.get("t", "k").await.unwrap();
        assert_eq!(retrieved, Some(v2));
    }

    // =========================================================================
    // Property-Based Tests for KVStore Laws
    // =========================================================================

    use hegel::generators;

    fn draw_key(tc: &hegel::TestCase) -> String {
        tc.draw(generators::from_regex(r"[a-zA-Z0-9]{1,20}").fullmatch(true))
    }

    fn draw_tenant(tc: &hegel::TestCase) -> String {
        tc.draw(generators::from_regex(r"[a-zA-Z0-9]{1,10}").fullmatch(true))
    }

    fn draw_value(tc: &hegel::TestCase) -> TestValue {
        TestValue {
            name: tc.draw(generators::from_regex(r"[a-zA-Z]{1,10}").fullmatch(true)),
            count: tc.draw(generators::integers::<u32>().min_value(0).max_value(999)),
        }
    }

    fn draw_ttl(tc: &hegel::TestCase) -> Option<Duration> {
        if tc.draw(generators::booleans()) {
            None
        } else {
            Some(Duration::from_secs(tc.draw(
                generators::integers::<u64>().min_value(1).max_value(86400),
            )))
        }
    }

    /// L1: Get-After-Put
    #[hegel::test]
    fn law_get_after_set(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        let ttl = draw_ttl(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put(&tenant, &key, &value, ttl).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(value));
        });
    }

    /// L2: Put-Overwrites
    #[hegel::test]
    fn law_set_overwrites(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let v1 = draw_value(&tc);
        let v2 = draw_value(&tc);
        let ttl = draw_ttl(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put(&tenant, &key, &v1, ttl).await.unwrap();
            store.put(&tenant, &key, &v2, ttl).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(v2));
        });
    }

    /// L3: Delete-Removes
    #[hegel::test]
    fn law_delete_removes(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put(&tenant, &key, &value, None).await.unwrap();
            store.delete(&tenant, &key).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L4: Get-Missing
    #[hegel::test]
    fn law_get_missing(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L5: Delete-Idempotent
    #[hegel::test]
    fn law_delete_idempotent(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put(&tenant, &key, &value, None).await.unwrap();
            let first = store.delete(&tenant, &key).await.unwrap();
            let second = store.delete(&tenant, &key).await.unwrap();
            assert!(first);
            assert!(!second);
        });
    }

    /// L6: Exists-Get-Consistency
    #[hegel::test]
    fn law_exists_reflects_presence(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        let should_set: bool = tc.draw(generators::booleans());
        tokio_test::block_on(async {
            let store = test_store();
            if should_set {
                store.put(&tenant, &key, &value, None).await.unwrap();
            }
            let exists = store.exists(&tenant, &key).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(exists, retrieved.is_some());
        });
    }

    /// L8: Permanence
    #[hegel::test]
    fn law_permanence_survives_cleanup(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put(&tenant, &key, &value, None).await.unwrap();
            store.cleanup_expired().unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(value));
        });
    }

    /// L9: Tenant-Isolation
    #[hegel::test]
    fn law_tenant_isolation(tc: hegel::TestCase) {
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = test_store();
            store.put("tenant1", &key, &value, None).await.unwrap();
            let retrieved: Option<TestValue> = store.get("tenant2", &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L10: GetMany-Consistency
    #[hegel::test]
    fn law_get_many_consistency(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let k1 = draw_key(&tc);
        let k2 = draw_key(&tc);
        let v1 = draw_value(&tc);
        let v2 = draw_value(&tc);
        let set_k1: bool = tc.draw(generators::booleans());
        let set_k2: bool = tc.draw(generators::booleans());
        tokio_test::block_on(async {
            let store = test_store();
            if set_k1 {
                store.put(&tenant, &k1, &v1, None).await.unwrap();
            }
            if set_k2 {
                store.put(&tenant, &k2, &v2, None).await.unwrap();
            }

            let individual_1: Option<serde_json::Value> =
                store.get_json(&tenant, &k1).await.unwrap();
            let individual_2: Option<serde_json::Value> =
                store.get_json(&tenant, &k2).await.unwrap();

            let keys = vec![k1.clone(), k2.clone()];
            let batch = store.get_many_json(&tenant, &keys).await.unwrap();

            assert_eq!(batch.get(&k1).cloned(), individual_1);
            if k1 != k2 {
                assert_eq!(batch.get(&k2).cloned(), individual_2);
            }
        });
    }
}
