//! Filter types for entity selection
//!
//! Filters represent conditions for selecting entities (products, SKUs, etc.).
//! FilterSet is a sorted map implementing conjunction (AND) semantics.
//!
//! # Design: Free Functions Over Typeclasses
//!
//! Following the "Radical Simplicity" principle, we use free functions
//! instead of typeclass instances. We have exactly one implementation
//! of filter composition.
//!
//! # Algebraic Structure: Distributive Lattice with Top Element
//!
//! FilterSet forms a distributive lattice with ⊤ = empty FilterSet (tautology).
//! Note: ⊥ (contradiction) is not unique — different contradictions carry different
//! column names — so the lattice is bounded above only.
//!
//! ## Meet (AND) — `meet_filters`
//! - L1. Identity:      meet(empty, a) = a = meet(a, empty)
//! - L2. Commutativity: meet(a, b) = meet(b, a)
//! - L3. Associativity: meet(a, meet(b, c)) = meet(meet(a, b), c)
//! - L4. Idempotence:   meet(a, a) = a
//!
//! ## Join (OR) — `join_filters`
//! - J1. Idempotence:   join(a, a) = a
//! - J2. Commutativity: join(a, b) = join(b, a)
//! - J3. Associativity: join(join(a, b), c) = join(a, join(b, c))
//! - J4. Absorption:    empty is absorbing: join(empty, a) = empty
//!
//! ## Lattice Interaction
//! - A1. Absorption:    meet(a, join(a, b)) = a
//! - A2. Absorption:    join(a, meet(a, b)) = a
//! - D1. Distributivity: meet(a, join(b, c)) = join(meet(a, b), meet(a, c))
//!
//! ## Diff (Relative Complement) — `diff_filters`
//! - D1. Self-annihilation: diff(a, a) yields empty Matched values (contradiction)
//! - D2. Identity:          diff(a, empty) = a

use crate::set_ops;
use agent_fw_core::FilterHash;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Numeric comparison operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Aggregation operators for measure filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggOp {
    Avg,
    Min,
    Max,
    Sum,
    Any,
}

impl AggOp {
    /// Lowercase string representation (matches serde/Display).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::Sum => "sum",
            Self::Any => "any",
        }
    }

    /// SQL aggregate function name (uppercased).
    pub fn sql_fn(&self) -> &'static str {
        match self {
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::Sum => "SUM",
            Self::Any => "ANY",
        }
    }
}

impl std::fmt::Display for AggOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Comparison operator for measure filter specifications.
///
/// Extends [`NumericOp`] with `Between` for range queries. Uses the
/// symbolic notation that the LLM produces (e.g. `"<="` not `"le"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComparisonOp {
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "between")]
    Between,
}

impl ComparisonOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Eq => "=",
            Self::Between => "between",
        }
    }

    /// Convert to the core [`NumericOp`] for filter construction.
    /// Returns `None` for `Between` (which expands to two filters).
    pub fn to_numeric_op(self) -> Option<NumericOp> {
        match self {
            Self::Lt => Some(NumericOp::Lt),
            Self::Le => Some(NumericOp::Le),
            Self::Gt => Some(NumericOp::Gt),
            Self::Ge => Some(NumericOp::Ge),
            Self::Eq => Some(NumericOp::Eq),
            Self::Between => None,
        }
    }
}

impl std::fmt::Display for ComparisonOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single filter condition.
///
/// Each variant represents a different type of entity selection criterion.
///
/// # Invariant
/// For `Matched` filters, `values` is always sorted and deduplicated.
/// This is enforced at both construction (`Filter::matched()`) and
/// deserialization boundaries (`From<FilterRaw>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Filter {
    /// Match column value against a set of allowed values.
    /// Semantics: column IN (values)
    /// Invariant: values is sorted for canonical representation.
    Matched { column: String, values: Vec<String> },

    /// Numeric comparison.
    /// Semantics: column op value
    Numeric {
        column: String,
        op: NumericOp,
        #[serde(with = "rust_decimal::serde::str")]
        value: Decimal,
    },

    /// Boolean flag check.
    /// Semantics: column = value
    Boolean { column: String, value: bool },

    /// Aggregated measure comparison.
    /// Semantics: agg(column) op value
    Measure {
        column: String,
        agg: AggOp,
        op: NumericOp,
        #[serde(with = "rust_decimal::serde::str")]
        value: Decimal,
    },
}

/// Private mirror of `Filter` used only for deserialization.
/// Normalizes `Matched` values (sort + dedup) on conversion to `Filter`.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FilterRaw {
    Matched {
        column: String,
        values: Vec<String>,
    },
    Numeric {
        column: String,
        op: NumericOp,
        #[serde(with = "rust_decimal::serde::str")]
        value: Decimal,
    },
    Boolean {
        column: String,
        value: bool,
    },
    Measure {
        column: String,
        agg: AggOp,
        op: NumericOp,
        #[serde(with = "rust_decimal::serde::str")]
        value: Decimal,
    },
}

impl From<FilterRaw> for Filter {
    fn from(raw: FilterRaw) -> Self {
        match raw {
            FilterRaw::Matched { column, mut values } => {
                values.sort();
                values.dedup();
                Filter::Matched { column, values }
            }
            FilterRaw::Numeric { column, op, value } => Filter::Numeric { column, op, value },
            FilterRaw::Boolean { column, value } => Filter::Boolean { column, value },
            FilterRaw::Measure {
                column,
                agg,
                op,
                value,
            } => Filter::Measure {
                column,
                agg,
                op,
                value,
            },
        }
    }
}

impl<'de> Deserialize<'de> for Filter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        FilterRaw::deserialize(deserializer).map(Filter::from)
    }
}

impl Filter {
    /// Create a Matched filter with sorted, deduplicated values (canonical form).
    pub fn matched(
        column: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut values: Vec<String> = values.into_iter().map(Into::into).collect();
        values.sort();
        values.dedup();
        Self::Matched {
            column: column.into(),
            values,
        }
    }

    /// Create a Numeric filter.
    pub fn numeric(column: impl Into<String>, op: NumericOp, value: Decimal) -> Self {
        Self::Numeric {
            column: column.into(),
            op,
            value,
        }
    }

    /// Create a Boolean filter.
    pub fn boolean(column: impl Into<String>, value: bool) -> Self {
        Self::Boolean {
            column: column.into(),
            value,
        }
    }

    /// Create a Measure filter.
    pub fn measure(column: impl Into<String>, agg: AggOp, op: NumericOp, value: Decimal) -> Self {
        Self::Measure {
            column: column.into(),
            agg,
            op,
            value,
        }
    }

    /// Compute a canonical key for this filter.
    ///
    /// The canonical key identifies WHAT the filter constrains (type + column),
    /// not the specific values. This allows filters on the same column to be
    /// merged via the meet operation.
    pub fn canonical_key(&self) -> String {
        match self {
            Self::Matched { column, .. } => {
                format!("matched:{}", column)
            }
            Self::Numeric { column, op, value } => {
                format!("numeric:{}:{:?}:{}", column, op, value)
            }
            Self::Boolean { column, value } => {
                format!("boolean:{}:{}", column, value)
            }
            Self::Measure {
                column,
                agg,
                op,
                value,
            } => {
                format!("measure:{}:{:?}:{:?}:{}", column, agg, op, value)
            }
        }
    }

    /// Get a content key for hashing that includes values.
    ///
    /// This is used for computing deterministic hashes of the actual filter content.
    pub fn content_key(&self) -> String {
        match self {
            Self::Matched { column, values } => {
                debug_assert!(
                    values.windows(2).all(|w| w[0] <= w[1]),
                    "Matched values must be sorted"
                );
                format!("matched:{}:{}", column, values.join(","))
            }
            Self::Numeric { column, op, value } => {
                format!("numeric:{}:{:?}:{}", column, op, value)
            }
            Self::Boolean { column, value } => {
                format!("boolean:{}:{}", column, value)
            }
            Self::Measure {
                column,
                agg,
                op,
                value,
            } => {
                format!("measure:{}:{:?}:{:?}:{}", column, agg, op, value)
            }
        }
    }
}

/// A set of filters representing conjunction (AND) of conditions.
///
/// Internally stored as a sorted map from canonical keys to filters.
/// The sorted map ensures deterministic serialization and hash computation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSet {
    filters: BTreeMap<String, Filter>,
}

impl FilterSet {
    /// Check if the filter set is empty.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Get the number of filters.
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Get all filters as a list.
    pub fn to_vec(&self) -> Vec<&Filter> {
        self.filters.values().collect()
    }

    /// Iterate over filters.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Filter)> {
        self.filters.iter()
    }
}

//=============================================================================
// OPERATIONS: Free functions (not typeclass methods)
//=============================================================================

/// Create an empty filter set (identity element for meet).
pub fn empty_filters() -> FilterSet {
    FilterSet::default()
}

/// Create a filter set with a single filter.
pub fn singleton_filter(f: Filter) -> FilterSet {
    let mut filters = BTreeMap::new();
    filters.insert(f.canonical_key(), f);
    FilterSet { filters }
}

/// Create a filter set from multiple filters.
pub fn filters_from_vec(filters: Vec<Filter>) -> FilterSet {
    let map: BTreeMap<String, Filter> = filters
        .into_iter()
        .map(|f| (f.canonical_key(), f))
        .collect();
    FilterSet { filters: map }
}

/// Compute the conjunction (AND) of two filter sets.
///
/// This is the core operation implementing distributive lattice meet.
/// When two filters have the same canonical key:
/// - Matched filters: intersection of values (may become empty = contradiction)
/// - Other filters: left operand takes precedence (last-write-wins)
///
/// # Laws
/// - Identity:      meet(empty, a) = a = meet(a, empty)
/// - Commutativity: meet(a, b) = meet(b, a)
/// - Associativity: meet(a, meet(b, c)) = meet(meet(a, b), c)
/// - Idempotence:   meet(a, a) = a
pub fn meet_filters(a: &FilterSet, b: &FilterSet) -> FilterSet {
    let mut result = a.filters.clone();

    for (key, filter) in &b.filters {
        result
            .entry(key.clone())
            .and_modify(|existing| *existing = meet_single_filter(existing, filter))
            .or_insert_with(|| filter.clone());
    }

    FilterSet { filters: result }
}

/// Compute a deterministic hash of a filter set.
///
/// The hash is computed from the sorted content keys (which include values),
/// ensuring:
/// - Same filters always produce the same hash
/// - Order of filter addition doesn't affect the hash
pub fn hash_filters(fs: &FilterSet) -> FilterHash {
    let mut content_keys: Vec<String> = fs.filters.values().map(|f| f.content_key()).collect();
    content_keys.sort();
    let payload = content_keys.join("|");
    let digest = Sha256::digest(payload.as_bytes());
    FilterHash::from_bytes(&digest[..12])
}

/// Compute the disjunction (OR) of two filter sets.
///
/// The dual of `meet_filters`. Keeps only constraints shared by both sets
/// (intersection of canonical keys), relaxing Matched filters via union.
///
/// # Laws
/// - Idempotence:   join(a, a) = a
/// - Commutativity: join(a, b) = join(b, a)
/// - Associativity: join(join(a, b), c) = join(a, join(b, c))
/// - Absorption:    join(empty, a) = empty   [empty = no constraints = tautology]
pub fn join_filters(a: &FilterSet, b: &FilterSet) -> FilterSet {
    let mut result = BTreeMap::new();

    // Intersection of keys: only keep filters present in BOTH sets
    for (key, filter_a) in &a.filters {
        if let Some(filter_b) = b.filters.get(key) {
            result.insert(key.clone(), join_single_filter(filter_a, filter_b));
        }
    }

    FilterSet { filters: result }
}

/// Compute the relative complement (difference) of two filter sets.
///
/// For Matched filters with the same key: set difference of values
/// (values in `a` but not in `b`). For other filter types with the same key:
/// the filter is removed (fully subtracted). Filters only in `a` are kept.
///
/// # Laws
/// - Self-annihilation: diff(a, a) yields empty Matched values
/// - Identity:          diff(a, empty) = a
pub fn diff_filters(a: &FilterSet, b: &FilterSet) -> FilterSet {
    let mut result = BTreeMap::new();

    for (key, filter_a) in &a.filters {
        if let Some(filter_b) = b.filters.get(key) {
            if let Some(diffed) = diff_single_filter(filter_a, filter_b) {
                result.insert(key.clone(), diffed);
            }
        } else {
            result.insert(key.clone(), filter_a.clone());
        }
    }

    FilterSet { filters: result }
}

/// Detect when a FilterSet provably matches zero rows.
///
/// Currently detects: Matched filters with empty values (from meet intersection).
pub fn is_contradiction(fs: &FilterSet) -> bool {
    fs.filters
        .values()
        .any(|f| matches!(f, Filter::Matched { values, .. } if values.is_empty()))
}

/// Detect when a FilterSet imposes no constraints (matches all rows).
///
/// An empty FilterSet is a tautology — no filters means no restrictions.
pub fn is_tautology(fs: &FilterSet) -> bool {
    fs.filters.is_empty()
}

/// Meet two filters with the same canonical key (AND / meet).
fn meet_single_filter(a: &Filter, b: &Filter) -> Filter {
    match (a, b) {
        (
            Filter::Matched {
                column: c1,
                values: v1,
            },
            Filter::Matched {
                column: c2,
                values: v2,
            },
        ) if c1 == c2 => {
            let intersection = set_ops::intersect_sorted(v1, v2);
            Filter::Matched {
                column: c1.clone(),
                values: intersection,
            }
        }
        _ => b.clone(),
    }
}

/// Join two filters with the same canonical key (OR / join).
fn join_single_filter(a: &Filter, b: &Filter) -> Filter {
    match (a, b) {
        (
            Filter::Matched {
                column: c1,
                values: v1,
            },
            Filter::Matched {
                column: c2,
                values: v2,
            },
        ) if c1 == c2 => {
            let combined = set_ops::union_sorted(v1, v2);
            Filter::Matched {
                column: c1.clone(),
                values: combined,
            }
        }
        _ => b.clone(),
    }
}

/// Diff two filters with the same canonical key (relative complement).
/// Returns None if the filter is fully subtracted.
fn diff_single_filter(a: &Filter, b: &Filter) -> Option<Filter> {
    match (a, b) {
        (
            Filter::Matched {
                column: c1,
                values: v1,
            },
            Filter::Matched {
                column: c2,
                values: v2,
            },
        ) if c1 == c2 => {
            let difference = set_ops::diff_sorted(v1, v2);
            Some(Filter::Matched {
                column: c1.clone(),
                values: difference,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::generators;
    use rust_decimal_macros::dec;

    fn matched(column: &str, values: &[&str]) -> Filter {
        Filter::matched(column, values.iter().copied())
    }

    fn numeric(column: &str, op: NumericOp, value: Decimal) -> Filter {
        Filter::numeric(column, op, value)
    }

    #[test]
    fn empty_filter_set() {
        let fs = empty_filters();
        assert!(fs.is_empty());
        assert_eq!(fs.len(), 0);
    }

    #[test]
    fn singleton_filter_set() {
        let f = matched("brand", &["Coke", "Pepsi"]);
        let fs = singleton_filter(f);
        assert_eq!(fs.len(), 1);
        assert!(!fs.is_empty());
    }

    #[test]
    fn meet_identity_left() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let result = meet_filters(&empty_filters(), &a);
        assert_eq!(result, a);
    }

    #[test]
    fn meet_identity_right() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let result = meet_filters(&a, &empty_filters());
        assert_eq!(result, a);
    }

    #[test]
    fn meet_idempotence() {
        let a = singleton_filter(matched("brand", &["Coke", "Pepsi"]));
        let result = meet_filters(&a, &a);
        assert_eq!(result, a);
    }

    #[test]
    fn meet_matched_intersection() {
        let a = singleton_filter(matched("brand", &["Coke", "Pepsi", "Fanta"]));
        let b = singleton_filter(matched("brand", &["Pepsi", "Fanta", "Sprite"]));
        let result = meet_filters(&a, &b);
        let expected = singleton_filter(matched("brand", &["Fanta", "Pepsi"]));
        assert_eq!(result, expected);
    }

    #[test]
    fn meet_different_columns() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let b = singleton_filter(numeric("price", NumericOp::Gt, dec!(10)));
        let result = meet_filters(&a, &b);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn hash_deterministic() {
        let f1 = matched("brand", &["Coke"]);
        let f2 = numeric("price", NumericOp::Gt, dec!(10));
        let fs1 = meet_filters(&singleton_filter(f1.clone()), &singleton_filter(f2.clone()));
        let fs2 = meet_filters(&singleton_filter(f2), &singleton_filter(f1));
        assert_eq!(hash_filters(&fs1), hash_filters(&fs2));
    }

    // =========================================================================
    // Property-Based Tests
    // =========================================================================

    fn draw_filter(tc: &hegel::TestCase) -> Filter {
        let variant = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        let col: String = tc.draw(generators::from_regex("[a-z]{3,8}").fullmatch(true));
        match variant {
            0 => {
                let vals: Vec<String> = tc.draw(
                    generators::vecs(generators::from_regex("[A-Za-z0-9]{1,5}").fullmatch(true))
                        .min_size(1)
                        .max_size(4),
                );
                Filter::matched(col, vals)
            }
            1 => {
                let val = tc.draw(generators::integers::<i32>());
                Filter::numeric(col, NumericOp::Gt, Decimal::from(val))
            }
            _ => {
                let val = tc.draw(generators::booleans());
                Filter::boolean(col, val)
            }
        }
    }

    fn draw_filter_set(tc: &hegel::TestCase) -> FilterSet {
        let filters: Vec<Filter> = {
            let n = tc.draw(generators::integers::<usize>().min_value(0).max_value(4));
            (0..n).map(|_| draw_filter(tc)).collect()
        };
        filters_from_vec(filters)
    }

    // ── Meet Laws ────────────────────────────────────────────────────────

    #[hegel::test]
    fn law_meet_identity_left(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&empty_filters(), &a), a);
    }

    #[hegel::test]
    fn law_meet_identity_right(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &empty_filters()), a);
    }

    #[hegel::test]
    fn law_meet_idempotence(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &a), a);
    }

    #[hegel::test]
    fn law_meet_associativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = meet_filters(&meet_filters(&a, &b), &c);
        let right = meet_filters(&a, &meet_filters(&b, &c));
        assert_eq!(left, right);
    }

    #[hegel::test]
    fn law_contradiction_monotone(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        if is_contradiction(&a) {
            assert!(is_contradiction(&meet_filters(&a, &b)));
        }
    }

    // ── Join Laws ────────────────────────────────────────────────────────

    #[hegel::test]
    fn law_join_idempotence(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &a), a);
    }

    #[hegel::test]
    fn law_join_absorbing_empty(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&empty_filters(), &a), empty_filters());
    }

    #[hegel::test]
    fn law_join_absorbing_empty_right(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &empty_filters()), empty_filters());
    }

    #[hegel::test]
    fn law_join_associativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = join_filters(&join_filters(&a, &b), &c);
        let right = join_filters(&a, &join_filters(&b, &c));
        assert_eq!(left, right);
    }

    #[hegel::test]
    fn law_join_commutativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &b), join_filters(&b, &a));
    }

    #[hegel::test]
    fn law_meet_commutativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &b), meet_filters(&b, &a));
    }

    // ── Absorption Laws ──────────────────────────────────────────────────

    #[hegel::test]
    fn law_absorption_meet_join(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let result = meet_filters(&a, &join_filters(&a, &b));
        assert_eq!(result, a);
    }

    #[hegel::test]
    fn law_absorption_join_meet(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let result = join_filters(&a, &meet_filters(&a, &b));
        assert_eq!(result, a);
    }

    // ── Distributivity ───────────────────────────────────────────────────

    #[hegel::test]
    fn law_distributivity_meet_over_join(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = meet_filters(&a, &join_filters(&b, &c));
        let right = join_filters(&meet_filters(&a, &b), &meet_filters(&a, &c));
        assert_eq!(left, right);
    }

    // ── Diff Laws ────────────────────────────────────────────────────────

    #[hegel::test]
    fn law_diff_identity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(diff_filters(&a, &empty_filters()), a);
    }

    #[hegel::test]
    fn law_diff_empty_left(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(diff_filters(&empty_filters(), &a), empty_filters());
    }

    // ── Unit Tests ───────────────────────────────────────────────────────

    #[test]
    fn contradiction_empty_matched() {
        let f = Filter::Matched {
            column: "brand".to_string(),
            values: vec![],
        };
        let fs = singleton_filter(f);
        assert!(is_contradiction(&fs));
    }

    #[test]
    fn no_contradiction_nonempty_matched() {
        let fs = singleton_filter(matched("brand", &["Coke"]));
        assert!(!is_contradiction(&fs));
    }

    #[test]
    fn no_contradiction_empty_set() {
        assert!(!is_contradiction(&empty_filters()));
    }

    #[test]
    fn contradiction_from_disjoint_meet() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let b = singleton_filter(matched("brand", &["Pepsi"]));
        let result = meet_filters(&a, &b);
        assert!(is_contradiction(&result));
    }

    #[test]
    fn tautology_empty_set() {
        assert!(is_tautology(&empty_filters()));
    }

    #[test]
    fn not_tautology_nonempty() {
        let fs = singleton_filter(matched("brand", &["Coke"]));
        assert!(!is_tautology(&fs));
    }

    #[test]
    fn join_matched_union() {
        let a = singleton_filter(matched("brand", &["Coke", "Pepsi"]));
        let b = singleton_filter(matched("brand", &["Pepsi", "Fanta"]));
        let result = join_filters(&a, &b);
        let expected = singleton_filter(matched("brand", &["Coke", "Fanta", "Pepsi"]));
        assert_eq!(result, expected);
    }

    #[test]
    fn join_drops_unshared_keys() {
        let a = meet_filters(
            &singleton_filter(matched("brand", &["Coke"])),
            &singleton_filter(numeric("price", NumericOp::Gt, dec!(10))),
        );
        let b = singleton_filter(matched("brand", &["Pepsi"]));
        let result = join_filters(&a, &b);
        assert_eq!(result.len(), 1);
        let expected = singleton_filter(matched("brand", &["Coke", "Pepsi"]));
        assert_eq!(result, expected);
    }

    #[test]
    fn join_disjoint_keys_produces_empty() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let b = singleton_filter(matched("category", &["soda"]));
        let result = join_filters(&a, &b);
        assert!(is_tautology(&result));
    }

    #[test]
    fn diff_matched_set_difference() {
        let a = singleton_filter(matched("brand", &["Coke", "Fanta", "Pepsi"]));
        let b = singleton_filter(matched("brand", &["Pepsi", "Sprite"]));
        let result = diff_filters(&a, &b);
        let expected = singleton_filter(matched("brand", &["Coke", "Fanta"]));
        assert_eq!(result, expected);
    }

    #[test]
    fn diff_self_produces_contradiction() {
        let a = singleton_filter(matched("brand", &["Coke", "Pepsi"]));
        let result = diff_filters(&a, &a);
        assert!(is_contradiction(&result));
    }

    #[test]
    fn diff_with_empty_is_identity() {
        let a = singleton_filter(matched("brand", &["Coke"]));
        let result = diff_filters(&a, &empty_filters());
        assert_eq!(result, a);
    }

    // ── Serde Roundtrip ──────────────────────────────────────────────────

    #[test]
    fn deserialize_matched_normalizes_unsorted_values() {
        let json = r#"{"type":"matched","column":"x","values":["C","A","B"]}"#;
        let f: Filter = serde_json::from_str(json).unwrap();
        match f {
            Filter::Matched { values, .. } => assert_eq!(values, vec!["A", "B", "C"]),
            _ => panic!("expected Matched"),
        }
    }

    #[test]
    fn deserialize_matched_deduplicates_values() {
        let json = r#"{"type":"matched","column":"x","values":["B","A","B","A"]}"#;
        let f: Filter = serde_json::from_str(json).unwrap();
        match f {
            Filter::Matched { values, .. } => assert_eq!(values, vec!["A", "B"]),
            _ => panic!("expected Matched"),
        }
    }

    #[test]
    fn deserialize_numeric_roundtrip() {
        let json = r#"{"type":"numeric","column":"weight","op":"gt","value":"10.5"}"#;
        let f: Filter = serde_json::from_str(json).unwrap();
        assert!(matches!(f, Filter::Numeric { .. }));
        let reserialized = serde_json::to_string(&f).unwrap();
        let f2: Filter = serde_json::from_str(&reserialized).unwrap();
        assert_eq!(f, f2);
    }
}
