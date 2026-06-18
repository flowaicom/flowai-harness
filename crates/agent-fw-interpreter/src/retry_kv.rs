//! RetryKVStore — algebra-level retry wrapper for any KVStore.
//!
//! # Design
//!
//! Retry is a **combinator**, not an implementation detail. Rather than baking
//! retry logic into each KVStore interpreter (Redis, DashMap, SQLite), we
//! compose it at the algebra level:
//!
//! ```text
//! let store = RetryKVStore::new(RedisKVStore::connect(url).await?, policy);
//! ```
//!
//! This preserves separation of concerns:
//! - `RedisKVStore` handles Redis protocol
//! - `RetryKVStore` handles retry/backoff
//! - They compose via the `KVStore` trait
//!
//! The same `RetryKVStore` works with any `KVStore` implementation,
//! including mocks (useful for testing retry behavior).

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_algebra::{KVError, KVStore};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// A KVStore wrapper that retries failed operations according to a policy.
///
/// Wraps any `KVStore` implementation, retrying on `KVError::Storage` errors
/// (which indicate transient backend issues). Serialization/deserialization
/// errors are not retried (they're deterministic).
pub struct RetryKVStore<K: KVStore> {
    inner: K,
    policy: RetryPolicy,
}

impl<K: KVStore> RetryKVStore<K> {
    /// Wrap a KVStore with retry behavior.
    pub fn new(inner: K, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Create with a reasonable default: 3 retries, 100ms initial delay, exponential backoff.
    pub fn with_defaults(inner: K) -> Self {
        Self::new(
            inner,
            RetryPolicy::exponential_backoff(3, Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(2)),
        )
    }

    /// Access the inner store.
    pub fn inner(&self) -> &K {
        &self.inner
    }

    /// Access the retry policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

/// Should this KVError be retried?
///
/// Delegates to `TransientError::is_transient()` for consistent classification
/// across all retry combinators (KV, TargetDB, WritableDB, Catalog, VectorStore).
fn is_retryable(e: &KVError) -> bool {
    use super::retry_defaults::TransientError;
    e.is_transient()
}

#[async_trait]
impl<K: KVStore> KVStore for RetryKVStore<K> {
    /// Put a JSON value with retry.
    ///
    /// # Known Limitation (#10)
    ///
    /// The `serde_json::Value` is wrapped in `Arc` and cloned on each retry
    /// attempt because `retry_when` requires `FnMut` closures that capture by
    /// move. For large JSON values, this incurs allocation on every retry.
    /// The ideal fix is a `retry_when` variant that takes `&V` or uses
    /// `Cow<'_, Value>`, but the current `retry` algebra doesn't support
    /// borrowed captures across await points.
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        let value = Arc::new(value);
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || {
                let v = value.clone();
                async move { self.inner.put_json(tenant, key, (*v).clone(), ttl).await }
            },
            is_retryable,
        )
        .await
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async move { self.inner.get_json(tenant, key).await },
            is_retryable,
        )
        .await
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async move { self.inner.delete(tenant, key).await },
            is_retryable,
        )
        .await
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async move { self.inner.exists(tenant, key).await },
            is_retryable,
        )
        .await
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async move { self.inner.list_keys(tenant, prefix).await },
            is_retryable,
        )
        .await
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async move { self.inner.get_many_json(tenant, keys).await },
            is_retryable,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::KVStoreExt;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// A KVStore that fails N times then delegates to an inner store.
    /// Uses AtomicI32 so the counter can safely go negative without wrapping.
    struct FailNTimes<K: KVStore> {
        inner: K,
        remaining_failures: AtomicI32,
    }

    impl<K: KVStore> FailNTimes<K> {
        fn new(inner: K, failures: i32) -> Self {
            Self {
                inner,
                remaining_failures: AtomicI32::new(failures),
            }
        }
    }

    #[async_trait]
    impl<K: KVStore> KVStore for FailNTimes<K> {
        async fn put_json(
            &self,
            tenant: &str,
            key: &str,
            value: serde_json::Value,
            ttl: Option<Duration>,
        ) -> Result<(), KVError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(KVError::Storage("connection reset by peer".into()));
            }
            self.inner.put_json(tenant, key, value, ttl).await
        }

        async fn get_json(
            &self,
            tenant: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, KVError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(KVError::Storage("connection reset by peer".into()));
            }
            self.inner.get_json(tenant, key).await
        }

        async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
            self.inner.delete(tenant, key).await
        }

        async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
            self.inner.exists(tenant, key).await
        }

        async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
            self.inner.list_keys(tenant, prefix).await
        }

        async fn get_many_json(
            &self,
            tenant: &str,
            keys: &[String],
        ) -> Result<HashMap<String, serde_json::Value>, KVError> {
            self.inner.get_many_json(tenant, keys).await
        }
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_put_failure() {
        let inner = FailNTimes::new(crate::DashMapKVStore::new(), 2);
        let store = RetryKVStore::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        // Should succeed after 2 failures + 1 success
        store
            .put("tenant", "key", &serde_json::json!("value"), None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_get_failure() {
        // Pre-populate the inner store, then wrap with FailNTimes
        let dash = crate::DashMapKVStore::new();
        dash.put("tenant", "key2", &serde_json::json!("hello"), None)
            .await
            .unwrap();

        let inner = FailNTimes::new(dash, 2);
        let store = RetryKVStore::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        let val: Option<serde_json::Value> = store.get("tenant", "key2").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn retry_exhausts_on_persistent_failure() {
        let inner = FailNTimes::new(crate::DashMapKVStore::new(), 100);
        let store = RetryKVStore::new(inner, RetryPolicy::fixed(2, Duration::from_millis(1)));

        let result = store
            .put_json("tenant", "key", serde_json::json!("value"), None)
            .await;

        assert!(result.is_err(), "Should fail after exhausting retries");
    }

    #[test]
    fn retryable_classification() {
        assert!(!is_retryable(&KVError::Serialization("bad".into())));
        assert!(!is_retryable(&KVError::Deserialization("bad".into())));
        assert!(is_retryable(&KVError::Storage("connection refused".into())));
        assert!(is_retryable(&KVError::Storage("timeout".into())));
        // Storage errors without transient keywords are not retried
        assert!(!is_retryable(&KVError::Storage("permanent failure".into())));
    }

    #[tokio::test]
    async fn with_defaults_uses_sensible_policy() {
        let store = RetryKVStore::with_defaults(crate::DashMapKVStore::new());
        assert_eq!(store.policy().max_retries(), 3);
    }
}
