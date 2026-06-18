//! TableRole classifier algebraic law test harness.
//!
//! # Laws
//!
//! - L1 (Totality): Never panics for any input
//! - L2 (Determinism): Same input → same role
//! - L3 (Naming precedence): Name prefixes override all heuristics
//! - L4 (CardinalityBucket monotonicity): Higher count → same or higher bucket
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn table_role_satisfies_laws() {
//!     agent_fw_test::table_role_laws::test_all();
//! }
//! ```

use agent_fw_catalog::table_role::{classify_table_role, TableClassificationInput, TableRole};
use agent_fw_interpreter::column_signature_cache::CardinalityBucket;
use hegel::generators;

/// Run all table role + cardinality laws.
pub fn test_all() {
    law_totality();
    law_determinism();
    law_naming_precedence();
    law_cardinality_monotonicity();
}

/// L1 (Totality): classify_table_role never panics.
pub fn law_totality() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let name: String = tc.draw(generators::text().min_size(1).max_size(20));
        let rows: usize = tc.draw(
            generators::integers::<usize>()
                .min_value(0)
                .max_value(999_999),
        );
        let cols: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(199));
        let fk_out: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(19));
        let fk_in: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(19));
        let measures: bool = tc.draw(generators::booleans());
        let input = TableClassificationInput {
            table_name: name,
            row_count: rows,
            column_count: cols,
            fk_outbound_count: fk_out,
            fk_inbound_count: fk_in,
            has_measures: measures,
        };
        // Must not panic
        let _ = classify_table_role(&input);
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// L2 (Determinism): Same input → same role.
pub fn law_determinism() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let name: String = tc.draw(generators::text().min_size(1).max_size(20));
        let rows: usize = tc.draw(
            generators::integers::<usize>()
                .min_value(0)
                .max_value(999_999),
        );
        let cols: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(199));
        let fk_out: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(19));
        let fk_in: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(19));
        let measures: bool = tc.draw(generators::booleans());
        let input = TableClassificationInput {
            table_name: name,
            row_count: rows,
            column_count: cols,
            fk_outbound_count: fk_out,
            fk_inbound_count: fk_in,
            has_measures: measures,
        };
        let a = classify_table_role(&input);
        let b = classify_table_role(&input);
        assert_eq!(a, b, "L2: same input must produce same role");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// L3 (Naming precedence): Name prefixes override structural heuristics.
pub fn law_naming_precedence() {
    // Exhaustive: every prefix always maps to its role regardless of structure
    let cases = vec![
        ("fact_sales", TableRole::Fact),
        ("fct_orders", TableRole::Fact),
        ("FACT_REVENUE", TableRole::Fact),
        ("dim_products", TableRole::Dimension),
        ("DIM_CUSTOMERS", TableRole::Dimension),
        ("bridge_order_product", TableRole::Bridge),
        ("xref_user_role", TableRole::Bridge),
        ("BRIDGE_AB", TableRole::Bridge),
        ("XREF_CD", TableRole::Bridge),
    ];

    for (name, expected) in cases {
        // Give it structural hints that would normally override — naming wins
        let input = TableClassificationInput {
            table_name: name.to_string(),
            row_count: 1_000_000,
            column_count: 100,
            fk_outbound_count: 10,
            fk_inbound_count: 10,
            has_measures: true,
        };
        assert_eq!(
            classify_table_role(&input),
            expected,
            "L3: '{}' should always be {:?} regardless of structure",
            name,
            expected,
        );
    }
}

/// L4 (CardinalityBucket monotonicity): higher count → same or higher bucket.
pub fn law_cardinality_monotonicity() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let a: usize = tc.draw(
            generators::integers::<usize>()
                .min_value(0)
                .max_value(99_999),
        );
        let b: usize = tc.draw(
            generators::integers::<usize>()
                .min_value(0)
                .max_value(99_999),
        );
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let bucket_lo = CardinalityBucket::from_count(lo);
        let bucket_hi = CardinalityBucket::from_count(hi);
        assert!(
            bucket_ord(bucket_hi) >= bucket_ord(bucket_lo),
            "L4: monotonicity: count {} ({:?}) > count {} ({:?})",
            hi,
            bucket_hi,
            lo,
            bucket_lo,
        );
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

fn bucket_ord(b: CardinalityBucket) -> u8 {
    match b {
        CardinalityBucket::Constant => 0,
        CardinalityBucket::Low => 1,
        CardinalityBucket::Medium => 2,
        CardinalityBucket::High => 3,
        CardinalityBucket::VeryHigh => 4,
    }
}
