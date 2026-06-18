//! Traits for entity types that support content-addressed resolution.
//!
//! # Resolvable
//!
//! Entity types implement `Resolvable` to declare:
//! - How they serialize (via `Serialize + DeserializeOwned`)
//! - Their KV key prefix (for namespaced storage)
//! - How to compute a compact summary (glimpse)
//!
//! # Resolver
//!
//! `Resolver<T>` is the effectful boundary where fuzzy matching, DB queries,
//! and vector lookups happen. Implementations are domain-specific.
//!
//! # Laws
//!
//! - `Resolvable::kv_prefix()` produces the key: `"{prefix}:{id}"`
//! - `Resolvable::glimpse()` faithfully summarizes the entity
//! - `Resolver::resolve()` is deterministic given same database state
//! - `Resolver::resolve()` returns structured errors (not panics)

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use crate::glimpse::Glimpse;

/// Entity types that support content-addressed resolution + KV caching.
pub trait Resolvable: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The specification type used to resolve this entity.
    type Spec: Serialize + DeserializeOwned + Send + Sync;

    /// KV key prefix (e.g., "ps" for product sets, "scope" for scope sets).
    fn kv_prefix() -> &'static str;

    /// Compute a compact summary for LLM context.
    fn glimpse(&self) -> Glimpse;
}

/// Effectful resolution: spec → entity set.
///
/// This is the boundary where fuzzy matching, DB queries, and vector
/// lookups happen. Implementations are domain-specific.
#[async_trait]
pub trait Resolver<T: Resolvable>: Send + Sync {
    /// Domain-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Resolve a spec into a concrete entity set.
    async fn resolve(&self, spec: &T::Spec) -> Result<T, Self::Error>;
}
