//! Sized newtype wrapping `Arc<dyn KVStore>` for use with generic combinators.
//!
//! `ToolEnvironment::kv()` returns `&Arc<dyn KVStore>` — a trait object.
//! For direct use (`env.kv().as_ref()` or `Arc::clone(env.kv())`), no bridge
//! is needed. But generic combinators like `RetryKVStore<K>` require `K: Sized`
//! (struct field), and you can't store `dyn KVStore` in a field.
//!
//! `KvBridge` provides the missing Sized newtype:
//!
//! ```rust,ignore
//! let retrying = RetryKVStore::new(KvBridge::from_env(&env), policy);
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::{KVError, KVStore};

use crate::ToolEnvironment;

/// Sized newtype bridging `Arc<dyn KVStore>` into generic combinator fields.
///
/// Delegates all operations to the inner `Arc<dyn KVStore>` — no caching,
/// no transformation, pure delegation.
#[derive(Clone)]
pub struct KvBridge(pub Arc<dyn KVStore>);

impl KvBridge {
    /// Extract a `KvBridge` from a `ToolEnvironment`.
    pub fn from_env(env: &ToolEnvironment) -> Self {
        Self(Arc::clone(env.kv()))
    }
}

#[async_trait::async_trait]
impl KVStore for KvBridge {
    async fn put_json(
        &self,
        tenant: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        self.0.put_json(tenant, key, value, ttl).await
    }

    async fn get_json(
        &self,
        tenant: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, KVError> {
        self.0.get_json(tenant, key).await
    }

    async fn delete(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        self.0.delete(tenant, key).await
    }

    async fn exists(&self, tenant: &str, key: &str) -> Result<bool, KVError> {
        self.0.exists(tenant, key).await
    }

    async fn list_keys(&self, tenant: &str, prefix: &str) -> Result<Vec<String>, KVError> {
        self.0.list_keys(tenant, prefix).await
    }

    async fn get_many_json(
        &self,
        tenant: &str,
        keys: &[String],
    ) -> Result<HashMap<String, serde_json::Value>, KVError> {
        self.0.get_many_json(tenant, keys).await
    }
}

// ─── KVNamespace ─────────────────────────────────────────────────────

/// Typed key namespace for KV store operations.
///
/// Prevents key collisions by enforcing a consistent prefix scheme.
/// All keys produced by a namespace start with `"{prefix}:"`, ensuring
/// isolation from other namespaces.
///
/// # Laws
///
/// - **L1 (Prefix isolation)**: Two namespaces with different prefixes
///   never produce the same key for any input.
/// - **L2 (Determinism)**: `key(id)` always returns the same string
///   for the same input.
///
/// # Usage
///
/// ```rust,ignore
/// const PRODUCT_SETS: KVNamespace = KVNamespace::new("pset");
/// let key = PRODUCT_SETS.key("abc123");       // "pset:abc123"
/// let key2 = PRODUCT_SETS.key2("terms", "x"); // "pset:terms:x"
/// let idx = PRODUCT_SETS.index_key();          // "pset:index"
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KVNamespace {
    prefix: &'static str,
}

impl KVNamespace {
    /// Create a new namespace with the given prefix.
    pub const fn new(prefix: &'static str) -> Self {
        Self { prefix }
    }

    /// Produce a namespaced key: `"{prefix}:{id}"`.
    pub fn key(&self, id: &str) -> String {
        format!("{}:{}", self.prefix, id)
    }

    /// Produce a two-segment namespaced key: `"{prefix}:{segment}:{id}"`.
    pub fn key2(&self, segment: &str, id: &str) -> String {
        format!("{}:{}:{}", self.prefix, segment, id)
    }

    /// Produce the index key for this namespace: `"{prefix}:index"`.
    pub fn index_key(&self) -> String {
        format!("{}:index", self.prefix)
    }

    /// Get the raw prefix.
    pub const fn prefix(&self) -> &'static str {
        self.prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockKV;
    #[async_trait::async_trait]
    impl KVStore for MockKV {
        async fn put_json(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: Option<Duration>,
        ) -> Result<(), KVError> {
            Ok(())
        }
        async fn get_json(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, KVError> {
            Ok(Some(serde_json::json!(42)))
        }
        async fn delete(&self, _: &str, _: &str) -> Result<bool, KVError> {
            Ok(true)
        }
        async fn exists(&self, _: &str, _: &str) -> Result<bool, KVError> {
            Ok(true)
        }
        async fn list_keys(&self, _: &str, _: &str) -> Result<Vec<String>, KVError> {
            Ok(vec!["k".into()])
        }
        async fn get_many_json(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<HashMap<String, serde_json::Value>, KVError> {
            Ok(Default::default())
        }
    }

    #[tokio::test]
    async fn bridge_delegates_get() {
        let bridge = KvBridge(Arc::new(MockKV));
        let val = bridge.get_json("t", "k").await.unwrap();
        assert_eq!(val, Some(serde_json::json!(42)));
    }

    #[tokio::test]
    async fn bridge_delegates_exists() {
        let bridge = KvBridge(Arc::new(MockKV));
        assert!(bridge.exists("t", "k").await.unwrap());
    }

    #[tokio::test]
    async fn bridge_delegates_delete() {
        let bridge = KvBridge(Arc::new(MockKV));
        assert!(bridge.delete("t", "k").await.unwrap());
    }

    #[tokio::test]
    async fn bridge_delegates_list_keys() {
        let bridge = KvBridge(Arc::new(MockKV));
        let keys = bridge.list_keys("t", "").await.unwrap();
        assert_eq!(keys, vec!["k".to_string()]);
    }

    #[test]
    fn bridge_is_clone() {
        let bridge = KvBridge(Arc::new(MockKV));
        let _cloned = bridge.clone();
    }

    // ── KVNamespace tests ──────────────────────────────────────────────

    #[test]
    fn namespace_key() {
        const NS: KVNamespace = KVNamespace::new("pset");
        assert_eq!(NS.key("abc123"), "pset:abc123");
    }

    #[test]
    fn namespace_key2() {
        const NS: KVNamespace = KVNamespace::new("pset");
        assert_eq!(NS.key2("terms", "abc123"), "pset:terms:abc123");
    }

    #[test]
    fn namespace_index_key() {
        const NS: KVNamespace = KVNamespace::new("pset");
        assert_eq!(NS.index_key(), "pset:index");
    }

    #[test]
    fn namespace_prefix_isolation() {
        // L1: Different namespaces never produce the same key
        const A: KVNamespace = KVNamespace::new("alpha");
        const B: KVNamespace = KVNamespace::new("beta");
        assert_ne!(A.key("id"), B.key("id"));
        assert_ne!(A.key2("seg", "id"), B.key2("seg", "id"));
        assert_ne!(A.index_key(), B.index_key());
    }

    #[test]
    fn namespace_determinism() {
        // L2: Same input → same output
        const NS: KVNamespace = KVNamespace::new("det");
        assert_eq!(NS.key("x"), NS.key("x"));
        assert_eq!(NS.key2("a", "b"), NS.key2("a", "b"));
        assert_eq!(NS.index_key(), NS.index_key());
    }

    #[test]
    fn namespace_prefix_accessor() {
        const NS: KVNamespace = KVNamespace::new("test");
        assert_eq!(NS.prefix(), "test");
    }

    #[test]
    fn namespace_is_const() {
        // Verify const construction works
        const NS: KVNamespace = KVNamespace::new("compile_time");
        assert_eq!(NS.prefix(), "compile_time");
    }
}
