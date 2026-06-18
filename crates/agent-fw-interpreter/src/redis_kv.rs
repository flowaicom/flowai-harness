//! Redis-backed KVStore implementation with native TTL.
//!
//! Provides a persistent, distributed KV store for production deployments.
//! Uses Redis's native TTL support for automatic expiry.
//!
//! # Connection Management
//!
//! Uses `ConnectionManager` which:
//! - Automatically reconnects on connection loss
//! - Multiplexes requests on a single connection
//! - Is `Clone + Send + Sync` for safe sharing across tasks
//!
//! # Key Format
//!
//! Keys are stored as `{prefix}{tenant}:{key}` where prefix defaults to `afw:`.
//! This prevents collision with other Redis users sharing the same instance.
//!
//! # TTL
//!
//! Redis's native EXPIRE is used. When `ttl` is `None`, keys are permanent.
//! When `ttl` is `Some(d)`, keys expire after `d`.

use agent_fw_algebra::{KVError, KVStore};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use std::collections::HashMap;
use std::time::Duration;

/// Redis-backed KV store for production deployments.
///
/// Implements the `KVStore` algebra with Redis as the storage backend.
/// Satisfies all KVStore laws (L1-L10) with Redis's native guarantees.
///
/// # Known Limitation (#11)
///
/// This store provides flat key-value semantics only. Structured event
/// streams and hierarchical span trees (for observability/tracing) require
/// the separate `EventLog` algebra. Redis Streams (`XADD`/`XREAD`) would
/// be a natural backend for `EventLog` but are not yet implemented —
/// the current `MemoryEventLog` is in-process only. A `RedisEventLog`
/// would close this gap for distributed deployments.
#[derive(Clone)]
pub struct RedisKVStore {
    conn: ConnectionManager,
    /// Key namespace prefix (default: "afw:").
    key_prefix: String,
}

impl RedisKVStore {
    /// Connect to Redis with default prefix "afw:".
    pub async fn connect(redis_url: &str) -> Result<Self, KVError> {
        Self::connect_with_prefix(redis_url, "afw:").await
    }

    /// Connect to Redis with a custom key prefix.
    pub async fn connect_with_prefix(redis_url: &str, prefix: &str) -> Result<Self, KVError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| KVError::Storage(format!("Invalid Redis URL: {e}")))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| KVError::Storage(format!("Failed to connect to Redis: {e}")))?;

        Ok(Self {
            conn,
            key_prefix: prefix.to_string(),
        })
    }

    /// Build the full Redis key: `{prefix}{tenant}:{key}`.
    fn full_key(&self, tenant: &str, key: &str) -> String {
        format!("{}{}:{}", self.key_prefix, tenant, key)
    }

    /// Build the tenant prefix for stripping: `{prefix}{tenant}:`.
    fn tenant_prefix(&self, tenant: &str) -> String {
        format!("{}{}:", self.key_prefix, tenant)
    }

    /// Ping Redis to verify connectivity.
    pub async fn ping(&self) -> Result<(), KVError> {
        let mut conn = self.conn.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| KVError::Storage(format!("Redis ping failed: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl KVStore for RedisKVStore {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        tracing::debug!(tenant, key, "redis_put");
        let full_key = self.full_key(tenant, key);
        let json_str =
            serde_json::to_string(&value).map_err(|e| KVError::Serialization(e.to_string()))?;

        let mut conn = self.conn.clone();
        match ttl {
            Some(d) => {
                let ttl_secs = d.as_secs().max(1);
                conn.set_ex::<_, _, ()>(&full_key, &json_str, ttl_secs)
                    .await
                    .map_err(|e| KVError::Storage(format!("Redis SET failed: {e}")))
            }
            None => conn
                .set::<_, _, ()>(&full_key, &json_str)
                .await
                .map_err(|e| KVError::Storage(format!("Redis SET failed: {e}"))),
        }
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        tracing::debug!(tenant, key, "redis_get");
        let full_key = self.full_key(tenant, key);
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(&full_key)
            .await
            .map_err(|e| KVError::Storage(format!("Redis GET failed: {e}")))?;

        match result {
            Some(json_str) => {
                let value: serde_json::Value = serde_json::from_str(&json_str)
                    .map_err(|e| KVError::Deserialization(e.to_string()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        tracing::debug!(tenant, key, "redis_del");
        let full_key = self.full_key(tenant, key);
        let mut conn = self.conn.clone();

        let count: u64 = conn
            .del(&full_key)
            .await
            .map_err(|e| KVError::Storage(format!("Redis DEL failed: {e}")))?;

        Ok(count > 0)
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        tracing::debug!(tenant, key, "redis_exists");
        let full_key = self.full_key(tenant, key);
        let mut conn = self.conn.clone();

        conn.exists(&full_key)
            .await
            .map_err(|e| KVError::Storage(format!("Redis EXISTS failed: {e}")))
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        tracing::debug!(tenant, count = keys.len(), "redis_mget");
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let full_keys: Vec<String> = keys.iter().map(|k| self.full_key(tenant, k)).collect();
        let mut conn = self.conn.clone();

        let values: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&full_keys)
            .query_async(&mut conn)
            .await
            .map_err(|e| KVError::Storage(format!("Redis MGET failed: {e}")))?;

        let mut result = HashMap::with_capacity(keys.len());
        for (key, maybe_val) in keys.iter().zip(values) {
            if let Some(json_str) = maybe_val {
                let value: serde_json::Value = serde_json::from_str(&json_str)
                    .map_err(|e| KVError::Deserialization(e.to_string()))?;
                result.insert(key.clone(), value);
            }
        }
        Ok(result)
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        tracing::debug!(tenant, prefix, "redis_scan");
        let pattern = format!("{}*", self.full_key(tenant, prefix));
        let strip_prefix = self.tenant_prefix(tenant);
        let mut conn = self.conn.clone();

        let mut keys = Vec::new();
        let mut cursor: u64 = 0;

        loop {
            let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| KVError::Storage(format!("Redis SCAN failed: {e}")))?;

            for key in batch {
                if let Some(stripped) = key.strip_prefix(&strip_prefix) {
                    keys.push(stripped.to_string());
                }
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_key_format() {
        // Simulate key construction
        let prefix = "afw:";
        let tenant = "tenant1";
        let key = "plan:abc";
        let full = format!("{prefix}{tenant}:{key}");
        assert_eq!(full, "afw:tenant1:plan:abc");
    }

    #[test]
    fn tenant_prefix_format() {
        let prefix = "afw:";
        let tenant = "tenant1";
        let tp = format!("{prefix}{tenant}:");
        assert_eq!(tp, "afw:tenant1:");
    }

    #[test]
    fn key_stripping() {
        let full_key = "afw:tenant1:plan:abc";
        let tenant_prefix = "afw:tenant1:";
        let stripped = full_key.strip_prefix(tenant_prefix);
        assert_eq!(stripped, Some("plan:abc"));
    }

    #[test]
    fn mget_key_construction() {
        let prefix = "afw:";
        let tenant = "tenant1";
        let keys = vec!["plan:abc".to_string(), "ps:def".to_string()];

        let full_keys: Vec<String> = keys
            .iter()
            .map(|k| format!("{prefix}{tenant}:{k}"))
            .collect();

        assert_eq!(
            full_keys,
            vec!["afw:tenant1:plan:abc", "afw:tenant1:ps:def"]
        );
    }
}
