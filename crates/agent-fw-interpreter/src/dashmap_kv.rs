//! DashMap-backed KVStore implementation with TTL support.
//!
//! This provides a high-performance, concurrent in-memory KV store.
//! Suitable for single-instance deployments or testing.
//!
//! # TTL Semantics
//!
//! - `None` TTL = permanent (never expires). Satisfies law L8.
//! - `Some(d)` TTL = expires after `d`. Satisfies law L7.
//!
//! Expired entries are lazily removed on access. A background cleanup task
//! can be spawned via `cleanup_expired()` for proactive garbage collection.
//! Permanent entries survive cleanup.

use agent_fw_algebra::kv_store::{KVError, KVStore};
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Entry in the KV store with TTL metadata.
#[derive(Clone)]
struct Entry {
    value: serde_json::Value,
    created: Instant,
    /// `None` = permanent (never expires), `Some(d)` = expires after `d`.
    ttl: Option<Duration>,
}

impl Entry {
    fn is_expired(&self) -> bool {
        self.ttl.map_or(false, |d| self.created.elapsed() > d)
    }
}

/// In-memory KV store using DashMap for concurrent access.
///
/// Keys are scoped by tenant to ensure isolation.
/// Values are stored as JSON for flexibility.
#[derive(Clone, Default)]
pub struct DashMapKVStore {
    data: Arc<DashMap<String, Entry>>,
}

impl DashMapKVStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
        }
    }

    /// Create a store with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Arc::new(DashMap::with_capacity(capacity)),
        }
    }

    /// Get the number of entries (including expired).
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Remove all expired entries.
    ///
    /// Call this periodically in a background task if proactive cleanup is needed.
    pub fn cleanup_expired(&self) {
        self.data.retain(|_, entry| !entry.is_expired());
    }

    /// Build the scoped key from tenant and key.
    fn scoped_key(tenant: &str, key: &str) -> String {
        format!("{}:{}", tenant, key)
    }
}

#[async_trait]
impl KVStore for DashMapKVStore {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        let scoped_key = Self::scoped_key(tenant, key);
        let entry = Entry {
            value,
            created: Instant::now(),
            ttl,
        };
        self.data.insert(scoped_key, entry);
        Ok(())
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        let scoped_key = Self::scoped_key(tenant, key);

        let entry = self.data.get(&scoped_key);
        match entry {
            Some(entry) => {
                if entry.is_expired() {
                    // Lazily remove expired entry
                    drop(entry); // Release read lock
                    self.data.remove(&scoped_key);
                    return Ok(None);
                }

                Ok(Some(entry.value.clone()))
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let scoped_key = Self::scoped_key(tenant, key);
        Ok(self.data.remove(&scoped_key).is_some())
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let scoped_key = Self::scoped_key(tenant, key);
        match self.data.get(&scoped_key) {
            Some(entry) => {
                if entry.is_expired() {
                    drop(entry);
                    self.data.remove(&scoped_key);
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
            None => Ok(false),
        }
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        let mut result = HashMap::with_capacity(keys.len());
        for key in keys {
            let scoped_key = Self::scoped_key(tenant, key);
            if let Some(entry) = self.data.get(&scoped_key) {
                if entry.is_expired() {
                    drop(entry);
                    self.data.remove(&scoped_key);
                } else {
                    result.insert(key.clone(), entry.value.clone());
                }
            }
        }
        Ok(result)
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        let tenant_prefix = format!("{}:", tenant);
        let full_prefix = format!("{}{}", tenant_prefix, prefix);

        let keys: Vec<String> = self
            .data
            .iter()
            .filter(|entry| entry.key().starts_with(&full_prefix) && !entry.value().is_expired())
            .filter_map(|entry| {
                entry
                    .key()
                    .strip_prefix(&tenant_prefix)
                    .map(|k| k.to_string())
            })
            .collect();

        Ok(keys)
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

    #[tokio::test]
    async fn basic_put_get() {
        let store = DashMapKVStore::new();
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
        let store = DashMapKVStore::new();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        store.put("tenant1", "key1", &value, None).await.unwrap();

        // Tenant2 should not see tenant1's data
        let retrieved: Option<TestValue> = store.get("tenant2", "key1").await.unwrap();
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let store = DashMapKVStore::new();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        // Store with very short TTL
        store
            .put("tenant1", "key1", &value, Some(Duration::from_millis(1)))
            .await
            .unwrap();

        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(10)).await;

        let retrieved: Option<TestValue> = store.get("tenant1", "key1").await.unwrap();
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let store = DashMapKVStore::new();
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
        let store = DashMapKVStore::new();
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

        let plan_keys = store.list_keys("tenant1", "plan:").await.unwrap();
        assert_eq!(plan_keys.len(), 2);
        assert!(plan_keys.contains(&"plan:abc".to_string()));
        assert!(plan_keys.contains(&"plan:def".to_string()));
    }

    #[tokio::test]
    async fn concurrent_access() {
        let store = DashMapKVStore::new();

        // Spawn multiple tasks writing concurrently
        let mut handles = vec![];
        for i in 0..100 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                let value = TestValue {
                    name: format!("test-{}", i),
                    count: i,
                };
                store
                    .put("tenant1", &format!("key-{}", i), &value, None)
                    .await
            }));
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // Verify all were stored
        assert_eq!(store.len(), 100);
    }

    #[tokio::test]
    async fn cleanup_expired_removes_old_entries() {
        let store = DashMapKVStore::new();
        let value = TestValue {
            name: "test".to_string(),
            count: 42,
        };

        // Store with short TTL
        store
            .put("tenant1", "short", &value, Some(Duration::from_millis(1)))
            .await
            .unwrap();

        // Store with long TTL
        store
            .put("tenant1", "long", &value, Some(Duration::from_secs(3600)))
            .await
            .unwrap();

        // Wait for short TTL to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Before cleanup
        assert_eq!(store.len(), 2);

        // Cleanup
        store.cleanup_expired();

        // After cleanup - only long TTL entry remains
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn permanent_entry_survives_cleanup() {
        let store = DashMapKVStore::new();
        let value = TestValue {
            name: "permanent".to_string(),
            count: 1,
        };

        // Store with no TTL (permanent)
        store.put("tenant1", "forever", &value, None).await.unwrap();

        // Store with short TTL
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

        store.cleanup_expired();

        // Permanent entry survives, ephemeral is gone
        assert_eq!(store.len(), 1);
        let retrieved: Option<TestValue> = store.get("tenant1", "forever").await.unwrap();
        assert_eq!(retrieved, Some(value));
        let gone: Option<TestValue> = store.get("tenant1", "ephemeral").await.unwrap();
        assert_eq!(gone, None);
    }

    // =========================================================================
    // get_many tests
    // =========================================================================

    #[tokio::test]
    async fn get_many_returns_existing_keys() {
        let store = DashMapKVStore::new();
        let v1 = TestValue {
            name: "a".to_string(),
            count: 1,
        };
        let v2 = TestValue {
            name: "b".to_string(),
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
    async fn get_many_empty_keys() {
        let store = DashMapKVStore::new();
        let result = store.get_many_json("t", &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn get_many_skips_expired() {
        let store = DashMapKVStore::new();
        let v = TestValue {
            name: "x".to_string(),
            count: 1,
        };

        store
            .put("t", "short", &v, Some(Duration::from_millis(1)))
            .await
            .unwrap();
        store
            .put("t", "long", &v, Some(Duration::from_secs(3600)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let keys = vec!["short".to_string(), "long".to_string()];
        let result = store.get_many_json("t", &keys).await.unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("long"));
    }

    #[tokio::test]
    async fn get_many_tenant_isolation() {
        let store = DashMapKVStore::new();
        let v = TestValue {
            name: "x".to_string(),
            count: 1,
        };

        store.put("t1", "k1", &v, None).await.unwrap();
        store.put("t2", "k2", &v, None).await.unwrap();

        let keys = vec!["k1".to_string(), "k2".to_string()];
        let result = store.get_many_json("t1", &keys).await.unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("k1"));
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

    /// L1: Get-After-Put (forall TTL t)
    /// put(k, v, t); get(k) == Some(v)
    #[hegel::test]
    fn law_get_after_set(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        let ttl = draw_ttl(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put(&tenant, &key, &value, ttl).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(value));
        });
    }

    /// L2: Put-Overwrites (forall TTL t)
    /// put(k, v1, t); put(k, v2, t); get(k) == Some(v2)
    #[hegel::test]
    fn law_set_overwrites(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let v1 = draw_value(&tc);
        let v2 = draw_value(&tc);
        let ttl = draw_ttl(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put(&tenant, &key, &v1, ttl).await.unwrap();
            store.put(&tenant, &key, &v2, ttl).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(v2));
        });
    }

    /// L3: Delete-Removes
    /// put(k, v); delete(k); get(k) == None
    #[hegel::test]
    fn law_delete_removes(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put(&tenant, &key, &value, None).await.unwrap();
            store.delete(&tenant, &key).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L4: Get-Missing
    /// get(k) on empty store == None
    #[hegel::test]
    fn law_get_missing(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L5: Delete-Idempotent
    /// delete(k) on absent key succeeds, returns false
    #[hegel::test]
    fn law_delete_idempotent(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put(&tenant, &key, &value, None).await.unwrap();

            let first = store.delete(&tenant, &key).await.unwrap();
            let second = store.delete(&tenant, &key).await.unwrap();

            assert!(first); // present -> true
            assert!(!second); // absent -> false
        });
    }

    /// L6: Exists-Get-Consistency
    /// exists(k) iff get(k).is_some()
    #[hegel::test]
    fn law_exists_reflects_presence(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        let should_set: bool = tc.draw(generators::booleans());
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            if should_set {
                store.put(&tenant, &key, &value, None).await.unwrap();
            }
            let exists = store.exists(&tenant, &key).await.unwrap();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(exists, retrieved.is_some());
        });
    }

    /// L8: Permanence
    /// put(k, v, None); cleanup_expired(); get(k) == Some(v)
    #[hegel::test]
    fn law_permanence_survives_cleanup(tc: hegel::TestCase) {
        let tenant = draw_tenant(&tc);
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put(&tenant, &key, &value, None).await.unwrap();
            store.cleanup_expired();
            let retrieved: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
            assert_eq!(retrieved, Some(value));
        });
    }

    /// L9: Tenant-Isolation
    /// tenant1.put(k, v); tenant2.get(k) == None
    #[hegel::test]
    fn law_tenant_isolation(tc: hegel::TestCase) {
        let key = draw_key(&tc);
        let value = draw_value(&tc);
        tokio_test::block_on(async {
            let store = DashMapKVStore::new();
            store.put("tenant1", &key, &value, None).await.unwrap();
            let retrieved: Option<TestValue> = store.get("tenant2", &key).await.unwrap();
            assert_eq!(retrieved, None);
        });
    }

    /// L10: GetMany-Consistency
    /// get_many([k1, k2]) == individual get(k1) + get(k2)
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
            let store = DashMapKVStore::new();
            if set_k1 {
                store.put(&tenant, &k1, &v1, None).await.unwrap();
            }
            if set_k2 {
                store.put(&tenant, &k2, &v2, None).await.unwrap();
            }

            // Individual gets
            let individual_1: Option<serde_json::Value> =
                store.get_json(&tenant, &k1).await.unwrap();
            let individual_2: Option<serde_json::Value> =
                store.get_json(&tenant, &k2).await.unwrap();

            // Batch get
            let keys = vec![k1.clone(), k2.clone()];
            let batch = store.get_many_json(&tenant, &keys).await.unwrap();

            // Verify consistency
            assert_eq!(batch.get(&k1).cloned(), individual_1);
            // If k1 == k2 they share a slot, so only check k2 when different
            if k1 != k2 {
                assert_eq!(batch.get(&k2).cloned(), individual_2);
            }
        });
    }
}
