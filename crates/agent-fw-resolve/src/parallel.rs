//! Parallel entity resolution via `tokio::join!`.
//!
//! Provides applicative composition for independent resolution tasks.
//! Neither failure cancels the other — both results are always returned.

use crate::cached::{CachedResolver, ResolveError, ResolvedRef};
use crate::resolvable::{Resolvable, Resolver};

/// Resolve two independent entity sets in parallel via `tokio::join!`.
///
/// Returns both results — neither failure cancels the other.
/// This is the applicative composition pattern.
pub async fn resolve_pair<A, B, RA, RB>(
    resolver_a: &CachedResolver<'_, A, RA>,
    resolver_b: &CachedResolver<'_, B, RB>,
    spec_a: &A::Spec,
    spec_b: &B::Spec,
) -> (
    Result<ResolvedRef<A>, ResolveError<RA::Error>>,
    Result<ResolvedRef<B>, ResolveError<RB::Error>>,
)
where
    A: Resolvable,
    B: Resolvable,
    RA: Resolver<A>,
    RB: Resolver<B>,
{
    tokio::join!(resolver_a.resolve(spec_a), resolver_b.resolve(spec_b))
}

/// Resolve an optional second entity set alongside a required first.
///
/// If `spec_b` is `None`, only resolves A.
pub async fn resolve_pair_optional<A, B, RA, RB>(
    resolver_a: &CachedResolver<'_, A, RA>,
    resolver_b: &CachedResolver<'_, B, RB>,
    spec_a: &A::Spec,
    spec_b: Option<&B::Spec>,
) -> (
    Result<ResolvedRef<A>, ResolveError<RA::Error>>,
    Option<Result<ResolvedRef<B>, ResolveError<RB::Error>>>,
)
where
    A: Resolvable,
    B: Resolvable,
    RA: Resolver<A>,
    RB: Resolver<B>,
{
    match spec_b {
        Some(sb) => {
            let (a, b) = tokio::join!(resolver_a.resolve(spec_a), resolver_b.resolve(sb));
            (a, Some(b))
        }
        None => {
            let a = resolver_a.resolve(spec_a).await;
            (a, None)
        }
    }
}

/// Resolve N independent specs of the same entity type in parallel.
///
/// Uses `futures::future::join_all` — all resolutions run concurrently,
/// and all results are collected (no short-circuit on failure).
///
/// # Laws
/// - L1 (Independence): Each resolution is independent — failure of
///   one does not affect others.
/// - L2 (Order preservation): Results are in the same order as input specs.
/// - L3 (Empty): resolve_all(resolver, []) == []
pub async fn resolve_all<T, R>(
    resolver: &CachedResolver<'_, T, R>,
    specs: &[T::Spec],
) -> Vec<Result<ResolvedRef<T>, ResolveError<R::Error>>>
where
    T: Resolvable,
    R: Resolver<T>,
{
    let futs: Vec<_> = specs.iter().map(|s| resolver.resolve(s)).collect();
    futures::future::join_all(futs).await
}

/// Resolve N specs, returning full entities alongside refs.
///
/// Same semantics as resolve_all but also returns entity data.
pub async fn resolve_all_full<T, R>(
    resolver: &CachedResolver<'_, T, R>,
    specs: &[T::Spec],
) -> Vec<Result<(ResolvedRef<T>, T), ResolveError<R::Error>>>
where
    T: Resolvable + Clone,
    R: Resolver<T>,
{
    let futs: Vec<_> = specs.iter().map(|s| resolver.resolve_full(s)).collect();
    futures::future::join_all(futs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glimpse::Glimpse;
    use agent_fw_core::TenantId;
    use agent_fw_test::fixtures::kv::InMemoryKVStore;
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    // ─── Type A ───────────────────────────────────────────────────────

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct EntityA {
        label: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct SpecA {
        query: String,
    }

    impl Resolvable for EntityA {
        type Spec = SpecA;
        fn kv_prefix() -> &'static str {
            "a"
        }
        fn glimpse(&self) -> Glimpse {
            Glimpse::from_labels(1, vec![self.label.clone()])
        }
    }

    // ─── Type B ───────────────────────────────────────────────────────

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct EntityB {
        value: i32,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct SpecB {
        id: i32,
    }

    impl Resolvable for EntityB {
        type Spec = SpecB;
        fn kv_prefix() -> &'static str {
            "b"
        }
        fn glimpse(&self) -> Glimpse {
            Glimpse::from_labels(1, vec![format!("val-{}", self.value)])
        }
    }

    // ─── Resolvers ────────────────────────────────────────────────────

    #[derive(Debug)]
    struct SimpleError(String);
    impl std::fmt::Display for SimpleError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for SimpleError {}

    struct ResolverA;
    #[async_trait]
    impl Resolver<EntityA> for ResolverA {
        type Error = SimpleError;
        async fn resolve(&self, spec: &SpecA) -> Result<EntityA, SimpleError> {
            Ok(EntityA {
                label: spec.query.clone(),
            })
        }
    }

    struct ResolverB;
    #[async_trait]
    impl Resolver<EntityB> for ResolverB {
        type Error = SimpleError;
        async fn resolve(&self, spec: &SpecB) -> Result<EntityB, SimpleError> {
            Ok(EntityB { value: spec.id })
        }
    }

    struct FailingResolverB;
    #[async_trait]
    impl Resolver<EntityB> for FailingResolverB {
        type Error = SimpleError;
        async fn resolve(&self, _spec: &SpecB) -> Result<EntityB, SimpleError> {
            Err(SimpleError("B failed".into()))
        }
    }

    type MemKV = InMemoryKVStore;

    // ─── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_pair_both_succeed() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);
        let rb = CachedResolver::new(&ResolverB, &kv, &tenant, ttl);

        let (a, b) = resolve_pair(
            &ra,
            &rb,
            &SpecA {
                query: "hello".into(),
            },
            &SpecB { id: 42 },
        )
        .await;

        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.glimpse.sample_labels, vec!["hello"]);
        assert_eq!(b.glimpse.sample_labels, vec!["val-42"]);
    }

    #[tokio::test]
    async fn resolve_pair_one_fails_other_succeeds() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);
        let rb = CachedResolver::new(&FailingResolverB, &kv, &tenant, ttl);

        let (a, b) = resolve_pair(
            &ra,
            &rb,
            &SpecA {
                query: "hello".into(),
            },
            &SpecB { id: 1 },
        )
        .await;

        assert!(a.is_ok());
        assert!(b.is_err());
    }

    #[tokio::test]
    async fn resolve_pair_optional_with_some() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);
        let rb = CachedResolver::new(&ResolverB, &kv, &tenant, ttl);
        let spec_b = SpecB { id: 7 };

        let (a, b) =
            resolve_pair_optional(&ra, &rb, &SpecA { query: "x".into() }, Some(&spec_b)).await;

        assert!(a.is_ok());
        assert!(b.is_some());
        assert!(b.unwrap().is_ok());
    }

    #[tokio::test]
    async fn resolve_pair_optional_with_none() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);
        let rb = CachedResolver::new(&ResolverB, &kv, &tenant, ttl);

        let (a, b) = resolve_pair_optional::<EntityA, EntityB, _, _>(
            &ra,
            &rb,
            &SpecA { query: "x".into() },
            None,
        )
        .await;

        assert!(a.is_ok());
        assert!(b.is_none());
    }

    // ── resolve_all Tests ────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_all_empty() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);
        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);

        let results = resolve_all(&ra, &[]).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn resolve_all_single() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);
        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);

        let specs = vec![SpecA {
            query: "one".into(),
        }];
        let results = resolve_all(&ra, &specs).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
        assert_eq!(
            results[0].as_ref().unwrap().glimpse.sample_labels,
            vec!["one"]
        );
    }

    #[tokio::test]
    async fn resolve_all_multiple_succeed() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);
        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);

        let specs = vec![
            SpecA {
                query: "alpha".into(),
            },
            SpecA {
                query: "beta".into(),
            },
            SpecA {
                query: "gamma".into(),
            },
        ];
        let results = resolve_all(&ra, &specs).await;
        assert_eq!(results.len(), 3);
        // L2: Order preservation
        assert_eq!(
            results[0].as_ref().unwrap().glimpse.sample_labels,
            vec!["alpha"]
        );
        assert_eq!(
            results[1].as_ref().unwrap().glimpse.sample_labels,
            vec!["beta"]
        );
        assert_eq!(
            results[2].as_ref().unwrap().glimpse.sample_labels,
            vec!["gamma"]
        );
    }

    #[tokio::test]
    async fn resolve_all_partial_failure() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);

        // EntityB with a failing resolver
        let rb = CachedResolver::new(&FailingResolverB, &kv, &tenant, ttl);

        let specs = vec![SpecB { id: 1 }, SpecB { id: 2 }];
        let results = resolve_all(&rb, &specs).await;
        assert_eq!(results.len(), 2);
        // L1: All fail independently (all fail because resolver always fails)
        assert!(results[0].is_err());
        assert!(results[1].is_err());
    }

    #[tokio::test]
    async fn resolve_all_full_returns_data() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);
        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);

        let specs = vec![SpecA { query: "x".into() }, SpecA { query: "y".into() }];
        let results = resolve_all_full(&ra, &specs).await;
        assert_eq!(results.len(), 2);
        let (ref_x, entity_x) = results[0].as_ref().unwrap();
        assert_eq!(ref_x.glimpse.sample_labels, vec!["x"]);
        assert_eq!(entity_x.label, "x");
        let (ref_y, entity_y) = results[1].as_ref().unwrap();
        assert_eq!(ref_y.glimpse.sample_labels, vec!["y"]);
        assert_eq!(entity_y.label, "y");
    }

    #[tokio::test]
    async fn resolve_all_uses_cache() {
        let kv = MemKV::new();
        let tenant = TenantId::new_unchecked("t1");
        let ttl = Duration::from_secs(3600);
        let ra = CachedResolver::new(&ResolverA, &kv, &tenant, ttl);

        let specs = vec![
            SpecA {
                query: "cached".into(),
            },
            SpecA {
                query: "cached".into(),
            },
        ];

        // First call populates cache, second hits cache
        let results = resolve_all(&ra, &specs).await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        // Both return same ID (same spec)
        assert_eq!(
            results[0].as_ref().unwrap().id,
            results[1].as_ref().unwrap().id
        );
    }
}
