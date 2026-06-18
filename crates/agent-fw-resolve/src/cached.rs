//! Content-addressed cached resolver.
//!
//! Wraps any `Resolver<T>` with KV caching. On cache hit, returns stored
//! data without calling the inner resolver. On miss, resolves, stores,
//! and returns.
//!
//! # Laws
//!
//! - **Cache hit**: inner resolver NOT called, stored entity returned.
//! - **Cache miss**: inner resolver called exactly once, result stored.
//! - **TTL**: entries expire after configured duration.
//! - **Tenant isolation**: KV keys scoped by tenant.

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};
use agent_fw_core::TenantId;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::time::Duration;

use crate::content_id::ContentId;
use crate::glimpse::Glimpse;
use crate::resolvable::{Resolvable, Resolver};

/// Stored entity in KV — wraps the entity with metadata.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound = "T: Serialize + serde::de::DeserializeOwned")]
pub struct StoredEntity<T: Resolvable> {
    pub owner: TenantId,
    pub data: T,
    pub glimpse: Glimpse,
    pub created_at: chrono::DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A reference to a resolved entity (ID + glimpse, not full data).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRef<T> {
    pub id: ContentId<T>,
    pub glimpse: Glimpse,
    #[serde(skip)]
    _entity: PhantomData<T>,
}

impl<T> ResolvedRef<T> {
    /// Create a new resolved reference.
    pub fn new(id: ContentId<T>, glimpse: Glimpse) -> Self {
        Self {
            id,
            glimpse,
            _entity: PhantomData,
        }
    }
}

/// Errors from cached resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError<E: std::error::Error> {
    #[error("KV store error: {0}")]
    Kv(#[from] KVError),
    #[error("Resolution failed: {0}")]
    Resolution(E),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Content-addressed cached resolver.
///
/// Wraps any `Resolver<T>` with KV caching. On cache hit, returns stored
/// data without calling the inner resolver. On miss, resolves, stores,
/// and returns.
pub struct CachedResolver<'a, T: Resolvable, R: Resolver<T>> {
    inner: &'a R,
    kv: &'a dyn KVStore,
    tenant: &'a TenantId,
    ttl: Duration,
    _entity: PhantomData<T>,
}

impl<'a, T: Resolvable, R: Resolver<T>> CachedResolver<'a, T, R> {
    /// Create a new cached resolver.
    pub fn new(inner: &'a R, kv: &'a dyn KVStore, tenant: &'a TenantId, ttl: Duration) -> Self {
        Self {
            inner,
            kv,
            tenant,
            ttl,
            _entity: PhantomData,
        }
    }

    /// Build the KV key for a content ID.
    fn kv_key(id: &ContentId<T>) -> String {
        format!("{}:{}", T::kv_prefix(), id.as_str())
    }

    /// Resolve with caching. Returns a lightweight reference (ID + glimpse).
    pub async fn resolve(&self, spec: &T::Spec) -> Result<ResolvedRef<T>, ResolveError<R::Error>> {
        let id = ContentId::<T>::compute(spec, self.tenant);
        let key = Self::kv_key(&id);

        // Cache hit?
        if let Some(stored) = self
            .kv
            .get::<StoredEntity<T>>(self.tenant.as_str(), &key)
            .await?
        {
            return Ok(ResolvedRef::new(id, stored.glimpse));
        }

        // Cache miss: resolve and store
        let entity = self
            .inner
            .resolve(spec)
            .await
            .map_err(ResolveError::Resolution)?;
        let glimpse = entity.glimpse();
        let stored = StoredEntity {
            owner: self.tenant.clone(),
            data: entity,
            glimpse: glimpse.clone(),
            created_at: Utc::now(),
            metadata: None,
        };
        self.kv
            .put(self.tenant.as_str(), &key, &stored, Some(self.ttl))
            .await?;
        Ok(ResolvedRef::new(id, glimpse))
    }

    /// Resolve, also returning the full entity data.
    pub async fn resolve_full(
        &self,
        spec: &T::Spec,
    ) -> Result<(ResolvedRef<T>, T), ResolveError<R::Error>>
    where
        T: Clone,
    {
        let id = ContentId::<T>::compute(spec, self.tenant);
        let key = Self::kv_key(&id);

        // Cache hit?
        if let Some(stored) = self
            .kv
            .get::<StoredEntity<T>>(self.tenant.as_str(), &key)
            .await?
        {
            let r = ResolvedRef::new(id, stored.glimpse);
            return Ok((r, stored.data));
        }

        // Cache miss
        let entity = self
            .inner
            .resolve(spec)
            .await
            .map_err(ResolveError::Resolution)?;
        let glimpse = entity.glimpse();
        let stored = StoredEntity {
            owner: self.tenant.clone(),
            data: entity.clone(),
            glimpse: glimpse.clone(),
            created_at: Utc::now(),
            metadata: None,
        };
        self.kv
            .put(self.tenant.as_str(), &key, &stored, Some(self.ttl))
            .await?;
        Ok((ResolvedRef::new(id, glimpse), entity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolvable::{Resolvable, Resolver};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ─── Test Entity ──────────────────────────────────────────────────

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestProduct {
        names: Vec<String>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestProductSpec {
        query: String,
    }

    impl Resolvable for TestProduct {
        type Spec = TestProductSpec;
        fn kv_prefix() -> &'static str {
            "tp"
        }
        fn glimpse(&self) -> Glimpse {
            Glimpse::from_labels(self.names.len(), self.names.clone())
        }
    }

    // ─── Test Resolver ────────────────────────────────────────────────

    struct CountingResolver {
        call_count: AtomicUsize,
    }

    impl CountingResolver {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[derive(Debug)]
    struct TestError(String);
    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for TestError {}

    #[async_trait]
    impl Resolver<TestProduct> for CountingResolver {
        type Error = TestError;
        async fn resolve(&self, spec: &TestProductSpec) -> Result<TestProduct, TestError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(TestProduct {
                names: vec![spec.query.clone()],
            })
        }
    }

    use agent_fw_test::fixtures::kv::InMemoryKVStore;

    type InMemoryKV = InMemoryKVStore;

    // ─── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cache_miss_calls_resolver() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&resolver, &kv, &tenant, ttl);
        let spec = TestProductSpec {
            query: "widget".into(),
        };
        let result = cached.resolve(&spec).await.unwrap();

        assert_eq!(resolver.calls(), 1);
        assert_eq!(result.glimpse.total_count, 1);
        assert_eq!(result.glimpse.sample_labels, vec!["widget"]);
    }

    #[tokio::test]
    async fn cache_hit_skips_resolver() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&resolver, &kv, &tenant, ttl);
        let spec = TestProductSpec {
            query: "widget".into(),
        };

        // First call: cache miss
        let r1 = cached.resolve(&spec).await.unwrap();
        assert_eq!(resolver.calls(), 1);

        // Second call: cache hit
        let r2 = cached.resolve(&spec).await.unwrap();
        assert_eq!(resolver.calls(), 1); // NOT incremented
        assert_eq!(r1.id, r2.id);
    }

    #[tokio::test]
    async fn different_specs_different_ids() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&resolver, &kv, &tenant, ttl);
        let r1 = cached
            .resolve(&TestProductSpec { query: "a".into() })
            .await
            .unwrap();
        let r2 = cached
            .resolve(&TestProductSpec { query: "b".into() })
            .await
            .unwrap();

        assert_ne!(r1.id, r2.id);
        assert_eq!(resolver.calls(), 2);
    }

    #[tokio::test]
    async fn resolve_full_returns_entity() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&resolver, &kv, &tenant, ttl);
        let spec = TestProductSpec {
            query: "gadget".into(),
        };
        let (r, entity) = cached.resolve_full(&spec).await.unwrap();

        assert_eq!(r.glimpse.total_count, 1);
        assert_eq!(entity.names, vec!["gadget"]);
    }

    #[tokio::test]
    async fn resolve_full_cache_hit() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&resolver, &kv, &tenant, ttl);
        let spec = TestProductSpec {
            query: "gadget".into(),
        };

        let _ = cached.resolve_full(&spec).await.unwrap();
        let (_, entity) = cached.resolve_full(&spec).await.unwrap();
        assert_eq!(resolver.calls(), 1);
        assert_eq!(entity.names, vec!["gadget"]);
    }

    #[tokio::test]
    async fn tenant_isolation_in_cache() {
        let resolver = CountingResolver::new();
        let kv = InMemoryKV::new();
        let t1 = TenantId::new_unchecked("tenant-a");
        let t2 = TenantId::new_unchecked("tenant-b");
        let ttl = Duration::from_secs(3600);
        let spec = TestProductSpec {
            query: "same".into(),
        };

        let cached1 = CachedResolver::new(&resolver, &kv, &t1, ttl);
        let cached2 = CachedResolver::new(&resolver, &kv, &t2, ttl);

        let r1 = cached1.resolve(&spec).await.unwrap();
        let r2 = cached2.resolve(&spec).await.unwrap();

        // Different tenants → different IDs → different KV keys → both resolve
        assert_ne!(r1.id, r2.id);
        assert_eq!(resolver.calls(), 2);
    }
}
