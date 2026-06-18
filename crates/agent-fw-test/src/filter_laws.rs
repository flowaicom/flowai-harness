//! FilterSet distributive lattice law test harnesses.
//!
//! Verifies that `FilterSet` satisfies the documented algebraic laws for a
//! distributive lattice with top element (empty = tautology).
//!
//! ## Meet Laws (AND -- `meet_filters`)
//!
//! - **L1 (Identity)**:      `meet(empty, a) = a = meet(a, empty)`
//! - **L2 (Commutativity)**: `meet(a, b) = meet(b, a)`
//! - **L3 (Associativity)**: `meet(a, meet(b, c)) = meet(meet(a, b), c)`
//! - **L4 (Idempotence)**:   `meet(a, a) = a`
//!
//! ## Join Laws (OR -- `join_filters`)
//!
//! - **J1 (Idempotence)**:   `join(a, a) = a`
//! - **J2 (Commutativity)**: `join(a, b) = join(b, a)`
//! - **J3 (Associativity)**: `join(join(a, b), c) = join(a, join(b, c))`
//! - **J4 (Absorption)**:    `join(empty, a) = empty` (empty is absorbing element)
//!
//! ## Lattice Interaction
//!
//! - **A1 (Absorption)**:     `meet(a, join(a, b)) = a`
//! - **A2 (Absorption)**:     `join(a, meet(a, b)) = a`
//! - **D1 (Distributivity)**: `meet(a, join(b, c)) = join(meet(a, b), meet(a, c))`
//!
//! ## Diff Laws (Relative Complement -- `diff_filters`)
//!
//! - **Diff-D1 (Self-annihilation)**: `diff(a, a)` yields empty Matched values (contradiction)
//! - **Diff-D2 (Identity)**:          `diff(a, empty) = a`
//!
//! ## Contradiction Monotonicity
//!
//! - If `is_contradiction(a)` then `is_contradiction(meet(a, b))` for all `b`.
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn filter_set_satisfies_laws() {
//!     agent_fw_test::filter_laws::test_all();
//! }
//! ```

use agent_fw_search::{
    diff_filters, empty_filters, is_contradiction, is_tautology, join_filters, meet_filters,
    singleton_filter, Filter, FilterSet, NumericOp,
};
use rust_decimal::Decimal;

// ─── Helpers ─────────────────────────────────────────────────────────────

fn matched(column: &str, values: &[&str]) -> FilterSet {
    singleton_filter(Filter::matched(column, values.iter().copied()))
}

fn numeric(column: &str, val: i64) -> FilterSet {
    singleton_filter(Filter::numeric(column, NumericOp::Gt, Decimal::from(val)))
}

fn boolean(column: &str, val: bool) -> FilterSet {
    singleton_filter(Filter::boolean(column, val))
}

/// Run all deterministic FilterSet lattice laws.
pub fn test_all() {
    // Meet laws
    law_meet_identity_left();
    law_meet_identity_right();
    law_meet_commutativity();
    law_meet_associativity();
    law_meet_idempotence();

    // Join laws
    law_join_idempotence();
    law_join_commutativity();
    law_join_associativity();
    law_join_absorbing_left();
    law_join_absorbing_right();

    // Lattice interaction
    law_absorption_meet_join();
    law_absorption_join_meet();
    law_distributivity_meet_over_join();

    // Diff laws
    law_diff_self_annihilation();
    law_diff_identity();

    // Contradiction monotonicity
    law_contradiction_monotone_meet();

    // Tautology / contradiction classification
    law_empty_is_tautology();
    law_singleton_not_tautology();
    law_disjoint_meet_is_contradiction();
}

// ═════════════════════════════════════════════════════════════════════════
// Meet Laws
// ═════════════════════════════════════════════════════════════════════════

/// L1 (left): meet(empty, a) = a
pub fn law_meet_identity_left() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        meet_filters(&empty_filters(), &a),
        a,
        "L1 violated: meet(empty, a) != a"
    );
}

/// L1 (right): meet(a, empty) = a
pub fn law_meet_identity_right() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        meet_filters(&a, &empty_filters()),
        a,
        "L1 violated: meet(a, empty) != a"
    );
}

/// L2: meet(a, b) = meet(b, a)
pub fn law_meet_commutativity() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    let b = matched("brand", &["Pepsi", "Fanta"]);
    assert_eq!(
        meet_filters(&a, &b),
        meet_filters(&b, &a),
        "L2 violated: meet is not commutative"
    );

    // Also with different column types
    let c = numeric("price", 10);
    assert_eq!(
        meet_filters(&a, &c),
        meet_filters(&c, &a),
        "L2 violated: meet is not commutative (mixed types)"
    );
}

/// L3: meet(a, meet(b, c)) = meet(meet(a, b), c)
pub fn law_meet_associativity() {
    let a = matched("brand", &["Coke", "Pepsi", "Fanta"]);
    let b = matched("brand", &["Pepsi", "Fanta", "Sprite"]);
    let c = matched("brand", &["Fanta", "Sprite", "DrPepper"]);

    let left = meet_filters(&meet_filters(&a, &b), &c);
    let right = meet_filters(&a, &meet_filters(&b, &c));
    assert_eq!(left, right, "L3 violated: meet is not associative");
}

/// L4: meet(a, a) = a
pub fn law_meet_idempotence() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        meet_filters(&a, &a),
        a,
        "L4 violated: meet is not idempotent"
    );

    let b = numeric("price", 42);
    assert_eq!(
        meet_filters(&b, &b),
        b,
        "L4 violated: meet is not idempotent (numeric)"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Join Laws
// ═════════════════════════════════════════════════════════════════════════

/// J1: join(a, a) = a
pub fn law_join_idempotence() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        join_filters(&a, &a),
        a,
        "J1 violated: join is not idempotent"
    );
}

/// J2: join(a, b) = join(b, a)
pub fn law_join_commutativity() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    let b = matched("brand", &["Pepsi", "Fanta"]);
    assert_eq!(
        join_filters(&a, &b),
        join_filters(&b, &a),
        "J2 violated: join is not commutative"
    );
}

/// J3: join(join(a, b), c) = join(a, join(b, c))
pub fn law_join_associativity() {
    let a = matched("brand", &["Coke"]);
    let b = matched("brand", &["Pepsi"]);
    let c = matched("brand", &["Fanta"]);

    let left = join_filters(&join_filters(&a, &b), &c);
    let right = join_filters(&a, &join_filters(&b, &c));
    assert_eq!(left, right, "J3 violated: join is not associative");
}

/// J4 (left): join(empty, a) = empty
pub fn law_join_absorbing_left() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        join_filters(&empty_filters(), &a),
        empty_filters(),
        "J4 violated: empty is not left-absorbing for join"
    );
}

/// J4 (right): join(a, empty) = empty
pub fn law_join_absorbing_right() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        join_filters(&a, &empty_filters()),
        empty_filters(),
        "J4 violated: empty is not right-absorbing for join"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Lattice Interaction
// ═════════════════════════════════════════════════════════════════════════

/// A1: meet(a, join(a, b)) = a
pub fn law_absorption_meet_join() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    let b = matched("brand", &["Pepsi", "Fanta"]);
    let result = meet_filters(&a, &join_filters(&a, &b));
    assert_eq!(result, a, "A1 violated: meet(a, join(a, b)) != a");
}

/// A2: join(a, meet(a, b)) = a
pub fn law_absorption_join_meet() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    let b = matched("brand", &["Pepsi", "Fanta"]);
    let result = join_filters(&a, &meet_filters(&a, &b));
    assert_eq!(result, a, "A2 violated: join(a, meet(a, b)) != a");
}

/// D1: meet(a, join(b, c)) = join(meet(a, b), meet(a, c))
pub fn law_distributivity_meet_over_join() {
    let a = matched("brand", &["Coke", "Pepsi", "Fanta"]);
    let b = matched("brand", &["Coke", "Sprite"]);
    let c = matched("brand", &["Pepsi", "DrPepper"]);

    let left = meet_filters(&a, &join_filters(&b, &c));
    let right = join_filters(&meet_filters(&a, &b), &meet_filters(&a, &c));
    assert_eq!(left, right, "D1 violated: distributivity");
}

// ═════════════════════════════════════════════════════════════════════════
// Diff Laws
// ═════════════════════════════════════════════════════════════════════════

/// Diff-D1: diff(a, a) yields contradiction (empty Matched values)
pub fn law_diff_self_annihilation() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    let result = diff_filters(&a, &a);
    assert!(
        is_contradiction(&result),
        "Diff-D1 violated: diff(a, a) should be a contradiction"
    );
}

/// Diff-D2: diff(a, empty) = a
pub fn law_diff_identity() {
    let a = matched("brand", &["Coke", "Pepsi"]);
    assert_eq!(
        diff_filters(&a, &empty_filters()),
        a,
        "Diff-D2 violated: diff(a, empty) != a"
    );

    // Also with numeric
    let b = numeric("price", 99);
    assert_eq!(
        diff_filters(&b, &empty_filters()),
        b,
        "Diff-D2 violated: diff(numeric, empty) != numeric"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Contradiction Monotonicity
// ═════════════════════════════════════════════════════════════════════════

/// If is_contradiction(a) then is_contradiction(meet(a, b))
pub fn law_contradiction_monotone_meet() {
    // Create a contradiction via disjoint meet
    let a = meet_filters(&matched("brand", &["Coke"]), &matched("brand", &["Pepsi"]));
    assert!(
        is_contradiction(&a),
        "precondition: a must be a contradiction"
    );

    let b = matched("category", &["soda"]);
    assert!(
        is_contradiction(&meet_filters(&a, &b)),
        "Contradiction monotonicity violated: meet with non-contradictory should preserve contradiction"
    );

    let c = numeric("price", 10);
    assert!(
        is_contradiction(&meet_filters(&a, &c)),
        "Contradiction monotonicity violated: meet with numeric should preserve contradiction"
    );

    let d = boolean("active", true);
    assert!(
        is_contradiction(&meet_filters(&a, &d)),
        "Contradiction monotonicity violated: meet with boolean should preserve contradiction"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Tautology / Contradiction Classification
// ═════════════════════════════════════════════════════════════════════════

/// Empty filter set is tautology.
pub fn law_empty_is_tautology() {
    assert!(
        is_tautology(&empty_filters()),
        "empty FilterSet should be tautology"
    );
    assert!(
        !is_contradiction(&empty_filters()),
        "empty FilterSet should not be contradiction"
    );
}

/// Non-empty filter set is not tautology.
pub fn law_singleton_not_tautology() {
    let a = matched("brand", &["Coke"]);
    assert!(
        !is_tautology(&a),
        "singleton FilterSet should not be tautology"
    );
}

/// Disjoint Matched meet produces contradiction.
pub fn law_disjoint_meet_is_contradiction() {
    let a = matched("brand", &["Coke"]);
    let b = matched("brand", &["Pepsi"]);
    let result = meet_filters(&a, &b);
    assert!(
        is_contradiction(&result),
        "disjoint Matched meet should be contradiction"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Proptest-based exhaustive verification
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod hegel_laws {
    use super::*;
    use agent_fw_search::filters_from_vec;
    use hegel::generators;

    fn draw_filter(tc: &hegel::TestCase) -> Filter {
        let kind: u8 = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        match kind {
            0 => {
                let col: String = tc.draw(generators::text().min_size(3).max_size(8));
                let num_vals: usize =
                    tc.draw(generators::integers::<usize>().min_value(1).max_value(4));
                let vals: Vec<String> = (0..num_vals)
                    .map(|_| tc.draw(generators::text().min_size(1).max_size(5)))
                    .collect();
                Filter::matched(col, vals)
            }
            1 => {
                let col: String = tc.draw(generators::text().min_size(3).max_size(8));
                let val: i32 = tc.draw(generators::integers::<i32>());
                Filter::numeric(col, NumericOp::Gt, Decimal::from(val))
            }
            _ => {
                let col: String = tc.draw(generators::text().min_size(3).max_size(8));
                let val: bool = tc.draw(generators::booleans());
                Filter::boolean(col, val)
            }
        }
    }

    fn draw_filter_set(tc: &hegel::TestCase) -> FilterSet {
        let len: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(4));
        let filters: Vec<Filter> = (0..len).map(|_| draw_filter(tc)).collect();
        filters_from_vec(filters)
    }

    // ── Meet Laws ────────────────────────────────────────────────────────

    /// L1 (left): meet(empty, a) = a
    #[hegel::test]
    fn hegel_meet_identity_left(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&empty_filters(), &a), a);
    }

    /// L1 (right): meet(a, empty) = a
    #[hegel::test]
    fn hegel_meet_identity_right(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &empty_filters()), a);
    }

    /// L2: meet(a, b) = meet(b, a)
    #[hegel::test]
    fn hegel_meet_commutativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &b), meet_filters(&b, &a));
    }

    /// L3: meet(meet(a, b), c) = meet(a, meet(b, c))
    #[hegel::test]
    fn hegel_meet_associativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = meet_filters(&meet_filters(&a, &b), &c);
        let right = meet_filters(&a, &meet_filters(&b, &c));
        assert_eq!(left, right);
    }

    /// L4: meet(a, a) = a
    #[hegel::test]
    fn hegel_meet_idempotence(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(meet_filters(&a, &a), a);
    }

    // ── Join Laws ────────────────────────────────────────────────────────

    /// J1: join(a, a) = a
    #[hegel::test]
    fn hegel_join_idempotence(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &a), a);
    }

    /// J2: join(a, b) = join(b, a)
    #[hegel::test]
    fn hegel_join_commutativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &b), join_filters(&b, &a));
    }

    /// J3: join(join(a, b), c) = join(a, join(b, c))
    #[hegel::test]
    fn hegel_join_associativity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = join_filters(&join_filters(&a, &b), &c);
        let right = join_filters(&a, &join_filters(&b, &c));
        assert_eq!(left, right);
    }

    /// J4 (left): join(empty, a) = empty
    #[hegel::test]
    fn hegel_join_absorbing_left(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&empty_filters(), &a), empty_filters());
    }

    /// J4 (right): join(a, empty) = empty
    #[hegel::test]
    fn hegel_join_absorbing_right(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(join_filters(&a, &empty_filters()), empty_filters());
    }

    // ── Lattice Interaction (Absorption + Distributivity) ────────────────

    /// A1: meet(a, join(a, b)) = a
    #[hegel::test]
    fn hegel_absorption_meet_join(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let result = meet_filters(&a, &join_filters(&a, &b));
        assert_eq!(result, a);
    }

    /// A2: join(a, meet(a, b)) = a
    #[hegel::test]
    fn hegel_absorption_join_meet(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let result = join_filters(&a, &meet_filters(&a, &b));
        assert_eq!(result, a);
    }

    /// D1: meet(a, join(b, c)) = join(meet(a, b), meet(a, c))
    #[hegel::test]
    fn hegel_distributivity_meet_over_join(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        let c = draw_filter_set(&tc);
        let left = meet_filters(&a, &join_filters(&b, &c));
        let right = join_filters(&meet_filters(&a, &b), &meet_filters(&a, &c));
        assert_eq!(left, right);
    }

    // ── Diff Laws ────────────────────────────────────────────────────────

    /// Diff-D2: diff(a, empty) = a
    #[hegel::test]
    fn hegel_diff_identity(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(diff_filters(&a, &empty_filters()), a);
    }

    /// diff(empty, a) = empty (left zero)
    #[hegel::test]
    fn hegel_diff_empty_left(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        assert_eq!(diff_filters(&empty_filters(), &a), empty_filters());
    }

    // ── Contradiction Monotonicity ───────────────────────────────────────

    /// If is_contradiction(a) then is_contradiction(meet(a, b))
    #[hegel::test]
    fn hegel_contradiction_monotone(tc: hegel::TestCase) {
        let a = draw_filter_set(&tc);
        let b = draw_filter_set(&tc);
        if is_contradiction(&a) {
            assert!(
                is_contradiction(&meet_filters(&a, &b)),
                "Contradiction not preserved under meet"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_laws_pass() {
        test_all();
    }
}
