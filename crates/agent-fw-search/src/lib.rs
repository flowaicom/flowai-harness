//! Pure fuzzy matching, FilterSet lattice, and set operations.
//!
//! This crate contains no async code and no IO. All types are pure data
//! with algebraic properties (lattice laws, scoring monotonicity).
//!
//! # Modules
//!
//! - [`filter`] — FilterSet distributive lattice (meet, join, diff, hash)
//! - [`fuzzy`] — Point-based fuzzy matching with token-boundary awareness

/// Column filter wrapper (`HashMap<String, Vec<String>>`).
pub mod column_filters;

/// FilterSet distributive lattice — laws: meet/join associativity, commutativity,
/// distributivity, absorption, De Morgan. Tested in `agent-fw-test::filter_laws`.
pub mod filter;

/// Fluent builder for FilterSet construction (push_matched, push_numeric, build).
pub mod filter_collector;

/// Point-based fuzzy matching with token-boundary awareness and scoring.
/// Laws: score ∈ [0,1], normalization idempotence, vector fallback ≥ phrase score.
pub mod fuzzy;

/// Sorted-array set algebra — union, intersect, diff (pure, no allocation on empty).
pub mod set_ops;

// Re-export key types at crate root
pub use agent_fw_core::FilterHash;
pub use column_filters::ColumnFilters;
pub use filter::{
    diff_filters, empty_filters, filters_from_vec, hash_filters, is_contradiction, is_tautology,
    join_filters, meet_filters, singleton_filter, AggOp, ComparisonOp, Filter, FilterSet,
    NumericOp,
};
pub use filter_collector::FilterCollector;
pub use fuzzy::{
    collect_matched_values, edit_distance_at_most_one, find_all_matches, find_best_match,
    fuzzy_score, fuzzy_score_prepared, jaccard_containment, needs_vector_fallback, normalize,
    rank_matches, resolve_multi_field, resolve_multi_field_prepared, resolve_value,
    resolve_value_amortized, resolve_value_prepared, MatchScore, MatchSignal, MultiFieldMatch,
    MultiFieldScore, NormalizedCorpus, PreparedQuery, RankedMatch, ResolveConfig, ResolveStrategy,
    ScoredField, ScoredMatch, VectorMatch,
};
