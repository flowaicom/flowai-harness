//! Content-addressed entity resolution with KV caching.
//!
//! This crate provides the building blocks for a content-addressed
//! entity resolution pipeline:
//!
//! 1. Hash the spec + tenant → `ContentId<T>`
//! 2. Check KV cache → hit returns `ResolvedRef<T>`
//! 3. On miss: resolve via `Resolver<T>`, store, return ref
//! 4. Compose resolvers in parallel via `resolve_pair`
//!
//! # Core Types
//!
//! - [`ContentId<T>`] — Phantom-typed content-addressed identifier
//! - [`Glimpse`] — Compact entity summary for LLM context
//! - [`Resolvable`] — Trait for entity types with KV prefix + glimpse
//! - [`Resolver<T>`] — Effectful resolution trait
//! - [`CachedResolver`] — KV-caching wrapper around any Resolver
//! - [`ResolvedRef<T>`] — Lightweight reference (ID + glimpse)
//!
//! # Parallel Resolution
//!
//! - [`resolve_pair`] — Resolve two independent entities in parallel
//! - [`resolve_pair_optional`] — Resolve required A + optional B
//!
//! # Generalization placeholders
//!
//! `column_resolve`, `corpus_cache`, and `measure_filter` reserve neutral
//! extension points for future catalog/reference work. They intentionally avoid
//! the retired product-specific implementation while keeping the generic
//! concepts discoverable.

pub mod cached;
pub mod column_resolve;
pub mod content_id;
pub mod corpus_cache;
pub mod glimpse;
pub mod loader;
pub mod measure_filter;
pub mod parallel;
pub mod resolvable;

// Re-exports
pub use cached::{CachedResolver, ResolveError, ResolvedRef, StoredEntity};
pub use column_resolve::{
    ColumnMatch, ColumnResolution, ColumnResolutionSpec, ColumnResolutionStrategy,
};
pub use content_id::ContentId;
pub use corpus_cache::CorpusCache;
pub use glimpse::{Glimpse, GlimpseFacet, MAX_SAMPLE};
pub use loader::{load_entity, load_glimpse, require_entity, LoadError};
pub use measure_filter::{AggregateOp, ComparisonOp, MeasureFilterError, MeasureFilterSpec};
pub use parallel::{resolve_all, resolve_all_full, resolve_pair, resolve_pair_optional};
pub use resolvable::{Resolvable, Resolver};
