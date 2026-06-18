//! Incremental filter accumulator.
//!
//! Bridges fuzzy matching results to FilterSet construction.
//! Pure, no IO, no allocations beyond the accumulated filters.
//!
//! # Laws
//!
//! - **L1 (Commutativity)**: push order does not affect the final FilterSet.
//!   `push(a); push(b); build()` == `push(b); push(a); build()`
//!   (Follows from FilterSet's BTreeMap ordering by canonical_key.)
//!
//! - **L2 (Empty identity)**: `FilterCollector::new().build()` == `empty_filters()`
//!
//! - **L3 (Matched dedup)**: Pushing the same column+values twice
//!   produces the same FilterSet as pushing once.
//!   (Values are sorted+deduped in Filter::matched.)
//!
//! - **L4 (Skip empty)**: `push_matched(col, [])` is a no-op —
//!   empty matched values are not added to the collector.
//!   This prevents accidental contradictions from failed fuzzy matches.

use agent_fw_core::FilterHash;
use rust_decimal::Decimal;

use crate::filter::{filters_from_vec, hash_filters, AggOp, Filter, FilterSet, NumericOp};

/// Accumulates filters incrementally from fuzzy match results
/// and direct specifications, then produces a FilterSet.
pub struct FilterCollector {
    filters: Vec<Filter>,
}

impl FilterCollector {
    /// Create a new empty collector.
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// Add a matched (categorical IN) filter.
    /// No-op if `values` is empty (L4).
    pub fn push_matched(
        &mut self,
        column: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> &mut Self {
        let values: Vec<String> = values.into_iter().map(Into::into).collect();
        if values.is_empty() {
            return self;
        }
        self.filters.push(Filter::matched(column, values));
        self
    }

    /// Add a numeric comparison filter.
    pub fn push_numeric(
        &mut self,
        column: impl Into<String>,
        op: NumericOp,
        value: Decimal,
    ) -> &mut Self {
        self.filters.push(Filter::numeric(column, op, value));
        self
    }

    /// Add a boolean filter.
    pub fn push_boolean(&mut self, column: impl Into<String>, value: bool) -> &mut Self {
        self.filters.push(Filter::boolean(column, value));
        self
    }

    /// Add a measure (aggregated) filter.
    pub fn push_measure(
        &mut self,
        column: impl Into<String>,
        agg: AggOp,
        op: NumericOp,
        value: Decimal,
    ) -> &mut Self {
        self.filters.push(Filter::measure(column, agg, op, value));
        self
    }

    /// Add a pre-built filter directly.
    pub fn push_filter(&mut self, filter: Filter) -> &mut Self {
        self.filters.push(filter);
        self
    }

    /// Number of filters accumulated so far.
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Whether any filters have been accumulated.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Build the FilterSet (consuming the collector).
    pub fn build(self) -> FilterSet {
        filters_from_vec(self.filters)
    }

    /// Build the FilterSet and compute its content hash.
    pub fn build_hashed(self) -> (FilterSet, FilterHash) {
        let fs = filters_from_vec(self.filters);
        let hash = hash_filters(&fs);
        (fs, hash)
    }
}

impl Default for FilterCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::empty_filters;
    use rust_decimal_macros::dec;

    // ── L2: Empty identity ──────────────────────────────────────────

    #[test]
    fn filter_collector_empty_identity() {
        let fs = FilterCollector::new().build();
        assert_eq!(fs, empty_filters());
    }

    // ── L1: Commutativity ───────────────────────────────────────────

    #[test]
    fn filter_collector_commutativity() {
        let mut c1 = FilterCollector::new();
        c1.push_matched("brand", ["Coke", "Pepsi"]);
        c1.push_numeric("price", NumericOp::Gt, dec!(10));
        let fs1 = c1.build();

        let mut c2 = FilterCollector::new();
        c2.push_numeric("price", NumericOp::Gt, dec!(10));
        c2.push_matched("brand", ["Coke", "Pepsi"]);
        let fs2 = c2.build();

        assert_eq!(fs1, fs2);
    }

    // ── L3: Matched dedup ───────────────────────────────────────────

    #[test]
    fn filter_collector_matched_dedup() {
        let mut c1 = FilterCollector::new();
        c1.push_matched("brand", ["Coke", "Pepsi"]);
        let fs1 = c1.build();

        // Pushing the same column again overwrites (BTreeMap key collision)
        let mut c2 = FilterCollector::new();
        c2.push_matched("brand", ["Coke", "Pepsi"]);
        c2.push_matched("brand", ["Coke", "Pepsi"]);
        let fs2 = c2.build();

        assert_eq!(fs1, fs2);
    }

    // ── L4: Skip empty matched ──────────────────────────────────────

    #[test]
    fn filter_collector_skip_empty_matched() {
        let mut c = FilterCollector::new();
        c.push_matched("brand", Vec::<String>::new());
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.build(), empty_filters());
    }

    // ── Numeric ─────────────────────────────────────────────────────

    #[test]
    fn filter_collector_push_numeric() {
        let mut c = FilterCollector::new();
        c.push_numeric("price", NumericOp::Ge, dec!(9.99));
        let fs = c.build();
        assert_eq!(fs.len(), 1);
    }

    // ── Boolean ─────────────────────────────────────────────────────

    #[test]
    fn filter_collector_push_boolean() {
        let mut c = FilterCollector::new();
        c.push_boolean("is_active", true);
        let fs = c.build();
        assert_eq!(fs.len(), 1);
    }

    // ── Measure ─────────────────────────────────────────────────────

    #[test]
    fn filter_collector_push_measure() {
        let mut c = FilterCollector::new();
        c.push_measure("revenue", AggOp::Sum, NumericOp::Gt, dec!(1000));
        let fs = c.build();
        assert_eq!(fs.len(), 1);
    }

    // ── build_hashed determinism ────────────────────────────────────

    #[test]
    fn filter_collector_build_hashed_determinism() {
        let mut c1 = FilterCollector::new();
        c1.push_matched("brand", ["Coke"]);
        c1.push_numeric("price", NumericOp::Gt, dec!(10));
        let (fs1, h1) = c1.build_hashed();

        let mut c2 = FilterCollector::new();
        c2.push_numeric("price", NumericOp::Gt, dec!(10));
        c2.push_matched("brand", ["Coke"]);
        let (fs2, h2) = c2.build_hashed();

        assert_eq!(fs1, fs2);
        assert_eq!(h1, h2);
    }

    // ── Mixed types ─────────────────────────────────────────────────

    #[test]
    fn filter_collector_mixed_types() {
        let mut c = FilterCollector::new();
        c.push_matched("brand", ["Coke", "Pepsi"]);
        c.push_numeric("price", NumericOp::Le, dec!(20));
        c.push_boolean("is_active", true);
        c.push_measure("sales", AggOp::Avg, NumericOp::Gt, dec!(100));
        let fs = c.build();
        assert_eq!(fs.len(), 4);
    }

    // ── push_filter ─────────────────────────────────────────────────

    #[test]
    fn filter_collector_push_filter() {
        let filter = Filter::matched("type", ["A", "B"]);
        let mut c = FilterCollector::new();
        c.push_filter(filter);
        assert_eq!(c.len(), 1);
        let fs = c.build();
        assert_eq!(fs.len(), 1);
    }

    // ── Hegel property tests ──────────────────────────────────────

    #[hegel::test]
    fn law_commutativity(tc: hegel::TestCase) {
        use hegel::generators;

        let col_a: String = tc.draw(generators::text().min_size(1).max_size(10));
        let vals_a: Vec<String> = tc.draw(
            generators::vecs(generators::text().min_size(1).max_size(5))
                .min_size(1)
                .max_size(4),
        );
        let col_b: String = tc.draw(generators::text().min_size(1).max_size(10));
        let val_b = tc.draw(generators::integers::<i32>());

        let mut c1 = FilterCollector::new();
        c1.push_matched(&col_a, &vals_a);
        c1.push_numeric(&col_b, NumericOp::Gt, Decimal::from(val_b));
        let fs1 = c1.build();

        let mut c2 = FilterCollector::new();
        c2.push_numeric(&col_b, NumericOp::Gt, Decimal::from(val_b));
        c2.push_matched(&col_a, &vals_a);
        let fs2 = c2.build();

        assert_eq!(fs1, fs2);
    }

    #[hegel::test]
    fn law_empty_identity(tc: hegel::TestCase) {
        use hegel::generators;

        let _x = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let fs = FilterCollector::new().build();
        assert_eq!(fs, empty_filters());
    }

    #[hegel::test]
    fn law_skip_empty_matched(tc: hegel::TestCase) {
        use hegel::generators;

        let col: String = tc.draw(generators::text().min_size(1).max_size(10));
        let mut c = FilterCollector::new();
        c.push_matched(&col, Vec::<String>::new());
        assert!(c.is_empty());
    }
}
