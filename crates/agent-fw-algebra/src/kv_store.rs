//! KVStore algebra for tenant-scoped key-value storage.
//!
//! This trait provides a simple key-value store with TTL (time-to-live) support.
//! Keys are scoped by tenant to ensure isolation in multi-tenant deployments.
//!
//! # Laws
//!
//! Implementations must satisfy these laws:
//!
//! - **L1. Get-After-Put**: `put(k, v, t); get(k)` returns `Some(v)` (for any TTL `t`, before expiry).
//!
//! - **L2. Put-Overwrites**: `put(k, v1, t); put(k, v2, t); get(k)` returns `Some(v2)`.
//!
//! - **L3. Delete-Removes**: `put(k, v); delete(k); get(k)` returns `None`.
//!
//! - **L4. Get-Missing**: `get(k)` for non-existent key returns `None`.
//!
//! - **L5. Delete-Idempotent**: `delete(k)` on absent key succeeds, returns `false`.
//!
//! - **L6. Exists-Get-Consistency**: `exists(k)` ⟺ `get(k).is_some()`.
//!
//! - **L7. TTL-Expiry**: `put(k, v, Some(d)); /* wait > d */; get(k)` returns `None`.
//!
//! - **L8. Permanence**: `put(k, v, None); /* any elapsed time */; get(k)` returns `Some(v)`.
//!   Entries with no TTL never expire. Dual of L7.
//!
//! - **L9. Tenant-Isolation**: Keys are scoped by tenant; tenant A cannot see tenant B's keys.
//!
//! - **L10. GetMany-Consistency**: `get_many_json(t, [k1, k2])` returns the same values as
//!   individual `get_json(t, k1)` and `get_json(t, k2)` calls.
//!
//! # Object Safety
//!
//! The `KVStore` trait is object-safe (can be used as `dyn KVStore`) by using
//! `serde_json::Value` for storage. The `KVStoreExt` trait provides generic
//! convenience methods for typed access.

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

/// Standard TTL for ephemeral entries (24 hours).
///
/// Use `Some(EPHEMERAL_TTL)` for transient data (job status, import progress).
pub const EPHEMERAL_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// TTL for session-scoped artifacts (7 days).
///
/// Plans, product sets, scope sets, actions, scenarios, and sweeps are created
/// during conversations and rarely accessed after the session ends. Without a TTL
/// these grow unboundedly in KV storage.
pub const SESSION_DATA_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// TTL for sub-agent response storage (1 hour).
///
/// Sub-agent full responses are stored for coordinator retrieval. They are only
/// useful during the active conversation.
pub const SUB_AGENT_RESPONSE_TTL: Duration = Duration::from_secs(3600);

/// Build a canonical `prefix:id` key.
///
/// Empty prefixes are allowed and return the raw `id` string unchanged.
pub fn prefixed_key(prefix: &str, id: impl fmt::Display) -> String {
    if prefix.is_empty() {
        id.to_string()
    } else {
        format!("{prefix}:{id}")
    }
}

/// Errors from KV store operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum KVError {
    /// Serialization failed.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Deserialization failed.
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// Storage backend error.
    #[error("Storage error: {0}")]
    Storage(String),
}

/// Errors from bound prefixed-record store operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PrefixedRecordStoreError {
    #[error("Record not found: {0}")]
    NotFound(String),
    #[error("KV error: {0}")]
    Kv(#[from] KVError),
}

impl From<serde_json::Error> for KVError {
    fn from(err: serde_json::Error) -> Self {
        KVError::Serialization(err.to_string())
    }
}

/// Tenant-scoped key-value storage with TTL.
///
/// This trait is **object-safe** and can be used as `Arc<dyn KVStore>`.
/// It operates on `serde_json::Value` directly. Use `KVStoreExt` for
/// typed access with automatic serialization/deserialization.
#[async_trait]
pub trait KVStore: Send + Sync {
    /// Store a JSON value with optional TTL.
    ///
    /// - `None` = permanent (no expiry).
    /// - `Some(d)` = expires after duration `d`.
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError>;

    /// Retrieve a JSON value by key.
    ///
    /// Returns `None` if the key doesn't exist or has expired.
    async fn get_json(&self, tenant: &str, key: &str)
        -> Result<Option<serde_json::Value>, KVError>;

    /// Delete a value by key.
    ///
    /// Returns `true` if a value was deleted, `false` if key didn't exist.
    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError>;

    /// Check if a key exists (without deserializing the value).
    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError>;

    /// List all keys for a tenant matching a prefix.
    ///
    /// Returns keys without the tenant prefix.
    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError>;

    /// Batch-retrieve multiple JSON values in a single operation.
    ///
    /// Missing or expired keys are omitted from the result (not errors).
    ///
    /// # Law L10: GetMany-Consistency
    /// `get_many_json(t, [k1, k2])` returns the same values as
    /// individual `get_json(t, k1)` and `get_json(t, k2)` calls.
    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError>;
}

/// Extension trait with typed convenience methods.
///
/// This trait is NOT object-safe due to generic methods.
/// Use it when you have a concrete type or `impl KVStore`.
#[async_trait]
pub trait KVStoreExt: KVStore {
    /// Store a typed value with optional TTL.
    ///
    /// Automatically serializes the value to JSON.
    async fn put<V: Serialize + Send + Sync>(
        &self,
        tenant: &str,
        key: &str,
        value: &V,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        let json =
            serde_json::to_value(value).map_err(|e| KVError::Serialization(e.to_string()))?;
        self.put_json(tenant, key, json, ttl).await
    }

    /// Retrieve a typed value by key.
    ///
    /// Automatically deserializes from JSON.
    async fn get<V: DeserializeOwned + Send>(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<V>, KVError> {
        match self.get_json(tenant, key).await? {
            Some(json) => {
                let value: V = serde_json::from_value(json)
                    .map_err(|e| KVError::Deserialization(e.to_string()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Get a value or return a default.
    async fn get_or_default<V: DeserializeOwned + Default + Send>(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<V, KVError> {
        match self.get::<V>(tenant, key).await? {
            Some(v) => Ok(v),
            None => Ok(V::default()),
        }
    }

    /// Batch-retrieve multiple typed values.
    ///
    /// Missing or expired keys are omitted. Deserialization errors are reported per key.
    async fn get_many<V: DeserializeOwned + Send>(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, V>, KVError> {
        let json_map = self.get_many_json(tenant, keys).await?;
        let mut result = HashMap::with_capacity(json_map.len());
        for (key, json) in json_map {
            let value: V = serde_json::from_value(json)
                .map_err(|e| KVError::Deserialization(format!("key '{}': {}", key, e)))?;
            result.insert(key, value);
        }
        Ok(result)
    }

    /// Put with no expiry (permanent storage).
    async fn put_default<V: Serialize + Send + Sync>(
        &self,
        tenant: &str,
        key: &str,
        value: &V,
    ) -> Result<(), KVError> {
        self.put(tenant, key, value, None).await
    }

    /// Compute the full scoped key (for debugging/logging).
    fn scoped_key(tenant: &str, key: &str) -> String {
        format!("{}:{}", tenant, key)
    }
}

/// Bound typed-record helper for repeated `(kv, tenant, prefix, ttl)` access.
///
/// This is useful for app/domain records that do not warrant a full framework
/// store trait but still benefit from one canonical persistence policy.
#[derive(Clone, Copy)]
pub struct PrefixedRecordStore<'a, K: KVStore + ?Sized> {
    kv: &'a K,
    tenant: &'a str,
    prefix: &'a str,
    ttl: Option<Duration>,
}

impl<'a, K: KVStore + ?Sized> PrefixedRecordStore<'a, K> {
    pub fn new(kv: &'a K, tenant: &'a str, prefix: &'a str) -> Self {
        Self {
            kv,
            tenant,
            prefix,
            ttl: None,
        }
    }

    pub fn with_ttl(mut self, ttl: Option<Duration>) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn kv(&self) -> &'a K {
        self.kv
    }

    pub fn tenant(&self) -> &'a str {
        self.tenant
    }

    pub fn prefix(&self) -> &'a str {
        self.prefix
    }

    pub fn ttl(&self) -> Option<Duration> {
        self.ttl
    }

    pub fn key<I: fmt::Display>(&self, id: &I) -> String {
        prefixed_key(self.prefix, id)
    }

    pub async fn load<I: fmt::Display, V: DeserializeOwned + Send>(
        &self,
        id: &I,
    ) -> Result<Option<V>, KVError> {
        self.kv.get(self.tenant, &self.key(id)).await
    }

    pub async fn load_required<I: fmt::Display, V: DeserializeOwned + Send>(
        &self,
        id: &I,
    ) -> Result<V, PrefixedRecordStoreError> {
        self.load(id)
            .await?
            .ok_or_else(|| PrefixedRecordStoreError::NotFound(self.key(id)))
    }

    pub async fn store<I: fmt::Display, V: Serialize + Send + Sync>(
        &self,
        id: &I,
        value: &V,
    ) -> Result<(), KVError> {
        self.kv
            .put(self.tenant, &self.key(id), value, self.ttl)
            .await
    }

    pub async fn delete<I: fmt::Display>(&self, id: &I) -> Result<bool, KVError> {
        self.kv.delete(self.tenant, &self.key(id)).await
    }
}

// Blanket implementation for all KVStore implementations
#[async_trait]
impl<T: KVStore + ?Sized> KVStoreExt for T {}

// ============================================================================
// KV Operation Metrics
// ============================================================================

/// Metrics from a single KV operation.
#[derive(Debug, Clone, Copy, Default)]
pub struct KVOperationMetrics {
    /// Bytes written (for put operations).
    pub bytes_written: u64,
    /// Bytes read (for get operations).
    pub bytes_read: u64,
    /// Operation duration in milliseconds.
    pub duration_ms: u64,
}

impl KVOperationMetrics {
    /// Create metrics for a write operation.
    pub fn write(bytes: u64, duration_ms: u64) -> Self {
        Self {
            bytes_written: bytes,
            bytes_read: 0,
            duration_ms,
        }
    }

    /// Create metrics for a read operation.
    pub fn read(bytes: u64, duration_ms: u64) -> Self {
        Self {
            bytes_written: 0,
            bytes_read: bytes,
            duration_ms,
        }
    }

    /// Combine with another metrics record (monoid operation).
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            bytes_written: self.bytes_written.saturating_add(other.bytes_written),
            bytes_read: self.bytes_read.saturating_add(other.bytes_read),
            duration_ms: self.duration_ms.saturating_add(other.duration_ms),
        }
    }
}

/// Aggregated KV metrics for the latency panel.
#[derive(Debug, Clone, Default)]
pub struct KVMetricsAccumulator {
    /// Total bytes written across all operations.
    pub total_bytes_written: u64,
    /// Total bytes read across all operations.
    pub total_bytes_read: u64,
    /// Total duration of all KV operations (ms).
    pub total_duration_ms: u64,
    /// Number of put operations.
    pub put_count: u64,
    /// Number of get operations.
    pub get_count: u64,
    /// Number of delete operations.
    pub delete_count: u64,
    /// Number of get_many operations.
    pub get_many_count: u64,
}

impl KVMetricsAccumulator {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a put operation.
    pub fn record_put(&mut self, bytes: u64, duration_ms: u64) {
        self.total_bytes_written = self.total_bytes_written.saturating_add(bytes);
        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.put_count = self.put_count.saturating_add(1);
    }

    /// Record a get operation.
    pub fn record_get(&mut self, bytes: u64, duration_ms: u64) {
        self.total_bytes_read = self.total_bytes_read.saturating_add(bytes);
        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.get_count = self.get_count.saturating_add(1);
    }

    /// Record a delete operation.
    pub fn record_delete(&mut self, duration_ms: u64) {
        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.delete_count = self.delete_count.saturating_add(1);
    }

    /// Record a get_many operation.
    pub fn record_get_many(&mut self, bytes: u64, duration_ms: u64) {
        self.total_bytes_read = self.total_bytes_read.saturating_add(bytes);
        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.get_many_count = self.get_many_count.saturating_add(1);
    }

    /// Combine with another accumulator (monoid operation).
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            total_bytes_written: self
                .total_bytes_written
                .saturating_add(other.total_bytes_written),
            total_bytes_read: self.total_bytes_read.saturating_add(other.total_bytes_read),
            total_duration_ms: self
                .total_duration_ms
                .saturating_add(other.total_duration_ms),
            put_count: self.put_count.saturating_add(other.put_count),
            get_count: self.get_count.saturating_add(other.get_count),
            delete_count: self.delete_count.saturating_add(other.delete_count),
            get_many_count: self.get_many_count.saturating_add(other.get_many_count),
        }
    }

    /// Check if no operations have been recorded.
    pub fn is_empty(&self) -> bool {
        self.put_count == 0
            && self.get_count == 0
            && self.delete_count == 0
            && self.get_many_count == 0
    }

    /// Total number of operations.
    pub fn total_operations(&self) -> u64 {
        self.put_count + self.get_count + self.delete_count + self.get_many_count
    }

    /// Average operation latency in milliseconds.
    pub fn avg_latency_ms(&self) -> Option<f64> {
        let total = self.total_operations();
        if total == 0 {
            None
        } else {
            Some(self.total_duration_ms as f64 / total as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryKV {
        values: Mutex<HashMap<(String, String), serde_json::Value>>,
    }

    #[async_trait]
    impl KVStore for MemoryKV {
        async fn put_json(
            &self,
            tenant: &str,
            key: &str,
            value: serde_json::Value,
            _ttl: Option<Duration>,
        ) -> Result<(), KVError> {
            self.values
                .lock()
                .unwrap()
                .insert((tenant.to_string(), key.to_string()), value);
            Ok(())
        }

        async fn get_json(
            &self,
            tenant: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, KVError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(&(tenant.to_string(), key.to_string()))
                .cloned())
        }

        async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .remove(&(tenant.to_string(), key.to_string()))
                .is_some())
        }

        async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .contains_key(&(tenant.to_string(), key.to_string())))
        }

        async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .keys()
                .filter(|(t, key)| t == tenant && key.starts_with(prefix))
                .map(|(_, key)| key.clone())
                .collect())
        }

        async fn get_many_json(
            &self,
            tenant: &str,
            keys: &[String],
        ) -> Result<HashMap<String, serde_json::Value>, KVError> {
            let values = self.values.lock().unwrap();
            Ok(keys
                .iter()
                .filter_map(|key| {
                    values
                        .get(&(tenant.to_string(), key.clone()))
                        .cloned()
                        .map(|value| (key.clone(), value))
                })
                .collect())
        }
    }

    #[test]
    fn kv_operation_metrics_combine() {
        let m1 = KVOperationMetrics::write(100, 10);
        let m2 = KVOperationMetrics::read(50, 5);
        let combined = m1.combine(&m2);

        assert_eq!(combined.bytes_written, 100);
        assert_eq!(combined.bytes_read, 50);
        assert_eq!(combined.duration_ms, 15);
    }

    #[test]
    fn kv_metrics_accumulator_records_puts() {
        let mut acc = KVMetricsAccumulator::new();
        acc.record_put(100, 10);
        acc.record_put(200, 20);

        assert_eq!(acc.put_count, 2);
        assert_eq!(acc.total_bytes_written, 300);
        assert_eq!(acc.total_duration_ms, 30);
    }

    #[test]
    fn kv_metrics_accumulator_records_gets() {
        let mut acc = KVMetricsAccumulator::new();
        acc.record_get(50, 5);
        acc.record_get(100, 10);

        assert_eq!(acc.get_count, 2);
        assert_eq!(acc.total_bytes_read, 150);
        assert_eq!(acc.total_duration_ms, 15);
    }

    #[test]
    fn kv_metrics_accumulator_combines() {
        let mut acc1 = KVMetricsAccumulator::new();
        acc1.record_put(100, 10);

        let mut acc2 = KVMetricsAccumulator::new();
        acc2.record_get(50, 5);

        let combined = acc1.combine(&acc2);
        assert_eq!(combined.put_count, 1);
        assert_eq!(combined.get_count, 1);
        assert_eq!(combined.total_bytes_written, 100);
        assert_eq!(combined.total_bytes_read, 50);
    }

    #[test]
    fn kv_metrics_accumulator_avg_latency() {
        let mut acc = KVMetricsAccumulator::new();
        acc.record_put(100, 10);
        acc.record_get(50, 30);
        acc.record_delete(20);

        assert_eq!(acc.total_operations(), 3);
        assert_eq!(acc.avg_latency_ms(), Some(20.0));
    }

    #[test]
    fn prefixed_key_handles_empty_prefix() {
        assert_eq!(prefixed_key("", "abc"), "abc");
        assert_eq!(prefixed_key("plan", "abc"), "plan:abc");
    }

    #[tokio::test]
    async fn prefixed_record_store_round_trips_and_deletes() {
        let kv = MemoryKV::default();
        let store =
            PrefixedRecordStore::new(&kv, "tenant-a", "plan").with_ttl(Some(SESSION_DATA_TTL));

        store
            .store(&"plan-1", &serde_json::json!({"ok": true}))
            .await
            .unwrap();

        let loaded: Option<serde_json::Value> = store.load(&"plan-1").await.unwrap();
        assert_eq!(loaded, Some(serde_json::json!({"ok": true})));
        assert_eq!(store.key(&"plan-1"), "plan:plan-1");
        assert_eq!(store.ttl(), Some(SESSION_DATA_TTL));

        assert!(store.delete(&"plan-1").await.unwrap());
        let missing: Option<serde_json::Value> = store.load(&"plan-1").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn prefixed_record_store_load_required_reports_not_found_key() {
        let kv = MemoryKV::default();
        let store = PrefixedRecordStore::new(&kv, "tenant-a", "scenario");
        let error = store
            .load_required::<_, serde_json::Value>(&"missing-1")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            PrefixedRecordStoreError::NotFound(ref key) if key == "scenario:missing-1"
        ));
    }
}
