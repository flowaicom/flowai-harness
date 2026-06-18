//! Instrumented KV store decorator.
//!
//! Adds timing observation to any `KVStore` without changing the underlying
//! implementation. The emitted event vocabulary is framework-owned via
//! [`agent_fw_core::KVTimingEvent`], so latency collectors and apps can reuse
//! one canonical contract.

use agent_fw_algebra::{KVError, KVStore};
use agent_fw_core::KVTimingEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Decorator that emits timing events for KV operations.
pub struct InstrumentedKVStore<F: Fn(KVTimingEvent) + Send + Sync> {
    inner: Arc<dyn KVStore>,
    on_timing: F,
}

impl<F: Fn(KVTimingEvent) + Send + Sync> InstrumentedKVStore<F> {
    /// Wrap an existing KV store with timing observation.
    pub fn new(inner: Arc<dyn KVStore>, on_timing: F) -> Self {
        Self { inner, on_timing }
    }
}

#[async_trait]
impl<F: Fn(KVTimingEvent) + Send + Sync + 'static> KVStore for InstrumentedKVStore<F> {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<std::time::Duration>,
    ) -> Result<(), KVError> {
        let value_len = value.to_string().len();
        let start = Instant::now();
        let result = self.inner.put_json(tenant, key, value, ttl).await;

        (self.on_timing)(KVTimingEvent::Put {
            key_len: key.len(),
            value_len,
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        let start = Instant::now();
        let result = self.inner.get_json(tenant, key).await;

        (self.on_timing)(KVTimingEvent::Get {
            key_len: key.len(),
            hit: matches!(&result, Ok(Some(_))),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let start = Instant::now();
        let result = self.inner.delete(tenant, key).await;

        (self.on_timing)(KVTimingEvent::Delete {
            key_len: key.len(),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        let start = Instant::now();
        let result = self.inner.exists(tenant, key).await;

        (self.on_timing)(KVTimingEvent::Get {
            key_len: key.len(),
            hit: matches!(&result, Ok(true)),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        let start = Instant::now();
        let result = self.inner.list_keys(tenant, prefix).await;

        (self.on_timing)(KVTimingEvent::GetMany {
            key_count: 0,
            hit_count: result.as_ref().map(|keys| keys.len()).unwrap_or(0),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        let start = Instant::now();
        let result = self.inner.get_many_json(tenant, keys).await;

        (self.on_timing)(KVTimingEvent::GetMany {
            key_count: keys.len(),
            hit_count: result.as_ref().map(|m| m.len()).unwrap_or(0),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DashMapKVStore;
    use serde_json::json;
    use std::sync::Mutex;

    #[tokio::test]
    async fn records_put_timing() {
        let events: Arc<Mutex<Vec<KVTimingEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let inner: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let instrumented = InstrumentedKVStore::new(inner, move |event| {
            events_clone.lock().unwrap().push(event);
        });

        instrumented
            .put_json("tenant", "key1", json!({"hello": "world"}), None)
            .await
            .unwrap();

        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        match &recorded[0] {
            KVTimingEvent::Put {
                key_len,
                value_len,
                duration_ms,
            } => {
                assert_eq!(*key_len, 4);
                assert!(*value_len > 0);
                assert!(*duration_ms < 1000);
            }
            other => panic!("Expected Put event, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn records_get_hit_and_miss() {
        let events: Arc<Mutex<Vec<KVTimingEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let inner: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let instrumented = InstrumentedKVStore::new(Arc::clone(&inner), move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(instrumented
            .get_json("tenant", "missing")
            .await
            .unwrap()
            .is_none());

        inner
            .put_json("tenant", "key1", json!(42), None)
            .await
            .unwrap();
        assert!(instrumented
            .get_json("tenant", "key1")
            .await
            .unwrap()
            .is_some());

        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert!(matches!(recorded[0], KVTimingEvent::Get { hit: false, .. }));
        assert!(matches!(recorded[1], KVTimingEvent::Get { hit: true, .. }));
    }
}
