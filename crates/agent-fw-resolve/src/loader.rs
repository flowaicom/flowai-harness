//! Entity loading by ContentId from KV store.
//!
//! Complements CachedResolver: where CachedResolver stores entities
//! on cache miss, these functions retrieve previously-stored entities
//! by their content-addressed ID.
//!
//! # Laws
//!
//! - **L1 (Round-trip)**: After `CachedResolver::resolve(spec)` succeeds,
//!   `load_entity(kv, tenant, id)` returns `Some(entity)` (within TTL).
//! - **L2 (Tenant isolation)**: `load_entity(kv, t1, id)` and
//!   `load_entity(kv, t2, id)` are independent.
//! - **L3 (Determinism)**: Same (kv, tenant, id) → same result
//!   (modulo TTL expiration).

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};
use agent_fw_core::TenantId;

use crate::cached::StoredEntity;
use crate::content_id::ContentId;
use crate::glimpse::Glimpse;
use crate::resolvable::Resolvable;

/// Errors from entity loading.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("Entity not found: {prefix}:{id}")]
    NotFound { prefix: String, id: String },
    #[error("KV error: {0}")]
    Kv(#[from] KVError),
}

/// Build the KV key for a content ID, matching CachedResolver's convention.
fn kv_key<T: Resolvable>(id: &ContentId<T>) -> String {
    format!("{}:{}", T::kv_prefix(), id.as_str())
}

/// Load a previously-resolved entity from KV by its ContentId.
///
/// Returns `None` if the entity is not in KV (expired or never stored).
pub async fn load_entity<T: Resolvable>(
    kv: &dyn KVStore,
    tenant: &TenantId,
    id: &ContentId<T>,
) -> Result<Option<StoredEntity<T>>, KVError> {
    let key = kv_key::<T>(id);
    kv.get::<StoredEntity<T>>(tenant.as_str(), &key).await
}

/// Load a previously-resolved entity, returning `LoadError::NotFound`
/// if absent.
///
/// Convenience wrapper for the common case where absence is an error
/// (e.g., approve_plan loading the product set referenced by a plan).
pub async fn require_entity<T: Resolvable>(
    kv: &dyn KVStore,
    tenant: &TenantId,
    id: &ContentId<T>,
) -> Result<StoredEntity<T>, LoadError> {
    load_entity(kv, tenant, id)
        .await?
        .ok_or_else(|| LoadError::NotFound {
            prefix: T::kv_prefix().to_string(),
            id: id.as_str().to_string(),
        })
}

/// Load only the Glimpse of a previously-resolved entity.
///
/// More efficient than loading the full entity when only the
/// summary is needed (e.g., for plan description assembly).
pub async fn load_glimpse<T: Resolvable>(
    kv: &dyn KVStore,
    tenant: &TenantId,
    id: &ContentId<T>,
) -> Result<Option<Glimpse>, KVError> {
    let stored = load_entity::<T>(kv, tenant, id).await?;
    Ok(stored.map(|s| s.glimpse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cached::CachedResolver;
    use crate::resolvable::{Resolvable, Resolver};
    use agent_fw_test::fixtures::kv::InMemoryKVStore;
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    // ─── Test Entity ──────────────────────────────────────────────────

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestEntity {
        names: Vec<String>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestSpec {
        query: String,
    }

    impl Resolvable for TestEntity {
        type Spec = TestSpec;
        fn kv_prefix() -> &'static str {
            "te"
        }
        fn glimpse(&self) -> Glimpse {
            Glimpse::from_labels(self.names.len(), self.names.clone())
        }
    }

    // ─── Test Resolver ────────────────────────────────────────────────

    struct TestResolver;

    #[derive(Debug)]
    struct TestError;
    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "test error")
        }
    }
    impl std::error::Error for TestError {}

    #[async_trait]
    impl Resolver<TestEntity> for TestResolver {
        type Error = TestError;
        async fn resolve(&self, spec: &TestSpec) -> Result<TestEntity, TestError> {
            Ok(TestEntity {
                names: vec![spec.query.clone()],
            })
        }
    }

    type MemKV = InMemoryKVStore;

    // ─── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn load_entity_round_trip() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&TestResolver, &kv, &tenant, ttl);
        let spec = TestSpec {
            query: "widget".into(),
        };
        let resolved = cached.resolve(&spec).await.unwrap();

        // Load by ContentId
        let loaded = load_entity::<TestEntity>(&kv, &tenant, &resolved.id)
            .await
            .unwrap();
        assert!(loaded.is_some());
        let stored = loaded.unwrap();
        assert_eq!(stored.data.names, vec!["widget"]);
        assert_eq!(stored.glimpse.total_count, 1);
    }

    #[tokio::test]
    async fn load_entity_absent() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let id = ContentId::<TestEntity>::new_unchecked("nonexistent".into());

        let loaded = load_entity::<TestEntity>(&kv, &tenant, &id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn require_entity_found() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&TestResolver, &kv, &tenant, ttl);
        let spec = TestSpec {
            query: "gadget".into(),
        };
        let resolved = cached.resolve(&spec).await.unwrap();

        let stored = require_entity::<TestEntity>(&kv, &tenant, &resolved.id)
            .await
            .unwrap();
        assert_eq!(stored.data.names, vec!["gadget"]);
    }

    #[tokio::test]
    async fn require_entity_not_found() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let id = ContentId::<TestEntity>::new_unchecked("missing".into());

        let err = require_entity::<TestEntity>(&kv, &tenant, &id)
            .await
            .unwrap_err();
        match err {
            LoadError::NotFound { prefix, id } => {
                assert_eq!(prefix, "te");
                assert_eq!(id, "missing");
            }
            _ => panic!("expected NotFound"),
        }
    }

    #[tokio::test]
    async fn load_entity_tenant_isolation() {
        let kv = MemKV::new();
        let t1 = TenantId::new_unchecked("tenant-a");
        let t2 = TenantId::new_unchecked("tenant-b");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&TestResolver, &kv, &t1, ttl);
        let spec = TestSpec {
            query: "item".into(),
        };
        let resolved = cached.resolve(&spec).await.unwrap();

        // Different tenant can't load
        let loaded = load_entity::<TestEntity>(&kv, &t2, &resolved.id)
            .await
            .unwrap();
        assert!(loaded.is_none());

        // Same tenant can load
        let loaded = load_entity::<TestEntity>(&kv, &t1, &resolved.id)
            .await
            .unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn load_glimpse_returns_summary() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let cached = CachedResolver::new(&TestResolver, &kv, &tenant, ttl);
        let spec = TestSpec {
            query: "thing".into(),
        };
        let resolved = cached.resolve(&spec).await.unwrap();

        let glimpse = load_glimpse::<TestEntity>(&kv, &tenant, &resolved.id)
            .await
            .unwrap();
        assert!(glimpse.is_some());
        let g = glimpse.unwrap();
        assert_eq!(g.total_count, 1);
        assert_eq!(g.sample_labels, vec!["thing"]);
    }
}
