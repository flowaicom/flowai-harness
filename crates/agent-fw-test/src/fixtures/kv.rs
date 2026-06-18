//! In-memory KV store fixture for testing.
//!
//! Satisfies KVStore laws L1-L8, L10. L9 (TTL expiry) is not enforced —
//! TTL parameters are accepted but ignored.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use agent_fw_algebra::{KVError, KVStore};
use async_trait::async_trait;

/// Mutex-protected HashMap-backed KV store for tests.
///
/// Thread-safe via `Mutex`. No TTL enforcement.
pub struct InMemoryKVStore {
    data: Mutex<HashMap<String, serde_json::Value>>,
}

impl InMemoryKVStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryKVStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KVStore for InMemoryKVStore {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        _ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        self.data
            .lock()
            .unwrap()
            .insert(format!("{tenant}:{key}"), value);
        Ok(())
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .get(&format!("{tenant}:{key}"))
            .cloned())
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .remove(&format!("{tenant}:{key}"))
            .is_some())
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .contains_key(&format!("{tenant}:{key}")))
    }

    async fn list_keys(&self, _tenant: &str, _prefix: &str) -> Result<Vec<String>, KVError> {
        Ok(vec![])
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        let guard = self.data.lock().unwrap();
        let mut result = HashMap::new();
        for key in keys {
            let full_key = format!("{tenant}:{key}");
            if let Some(v) = guard.get(&full_key) {
                result.insert(key.clone(), v.clone());
            }
        }
        Ok(result)
    }
}
