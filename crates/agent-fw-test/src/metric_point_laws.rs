//! MetricPoint commutative monoid law test harnesses.
//!
//! Verifies that `MetricPoint` satisfies:
//!
//! - **M1 (Left identity)**: `ZERO.combine(a) == a`
//! - **M2 (Right identity)**: `a.combine(ZERO) == a`
//! - **M3 (Associativity)**: `(a.combine(b)).combine(c) == a.combine(b.combine(c))`
//! - **M4 (Commutativity)**: `a.combine(b) == b.combine(a)`
//! - **M5 (Delta consistency)**: `delta == modified - baseline`
//! - **M6 (Percent consistency)**: `percent_change == delta / baseline * 100` (when baseline != 0)
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn metric_point_laws() {
//!     agent_fw_test::metric_point_laws::test_all();
//! }
//! ```

use agent_fw_plan::MetricPoint;
use rust_decimal::Decimal;

fn mp(baseline: i64, modified: i64) -> MetricPoint {
    MetricPoint::from_baseline_modified(Decimal::from(baseline), Decimal::from(modified))
}

/// Run all deterministic MetricPoint monoid laws.
pub fn test_all() {
    law_left_identity();
    law_right_identity();
    law_associativity();
    law_commutativity();
    law_delta_consistency();
    law_percent_consistency();
    law_percent_zero_baseline();
    law_combine_additivity();
}

// ─── M1, M2: Identity ────────────────────────────────────────────────

fn law_left_identity() {
    let a = mp(100, 110);
    assert_eq!(MetricPoint::ZERO.combine(&a), a, "M1: left identity");
}

fn law_right_identity() {
    let a = mp(100, 110);
    assert_eq!(a.combine(&MetricPoint::ZERO), a, "M2: right identity");
}

// ─── M3: Associativity ───────────────────────────────────────────────

fn law_associativity() {
    let a = mp(100, 110);
    let b = mp(200, 180);
    let c = mp(50, 60);

    let left = (a.combine(&b)).combine(&c);
    let right = a.combine(&(b.combine(&c)));
    assert_eq!(left, right, "M3: associativity");
}

// ─── M4: Commutativity ───────────────────────────────────────────────

fn law_commutativity() {
    let a = mp(100, 130);
    let b = mp(200, 190);
    assert_eq!(a.combine(&b), b.combine(&a), "M4: commutativity");
}

// ─── M5: Delta consistency ────────────────────────────────────────────

fn law_delta_consistency() {
    for (b, m) in [(100, 110), (200, 180), (0, 50), (50, 50), (-10, 10)] {
        let point = mp(b, m);
        assert_eq!(
            point.delta,
            point.modified - point.baseline,
            "M5: delta = modified - baseline for ({b}, {m})"
        );
    }
}

// ─── M6: Percent consistency ──────────────────────────────────────────

fn law_percent_consistency() {
    let point = mp(200, 220);
    let expected = (point.delta / point.baseline) * Decimal::ONE_HUNDRED;
    assert_eq!(
        point.percent_change, expected,
        "M6: percent_change = delta / baseline * 100"
    );
}

fn law_percent_zero_baseline() {
    let point = mp(0, 50);
    assert_eq!(
        point.percent_change,
        Decimal::ZERO,
        "M6: percent_change = 0 when baseline = 0"
    );
}

// ─── Derived: Combine additivity ──────────────────────────────────────

fn law_combine_additivity() {
    let a = mp(100, 115);
    let b = mp(300, 280);
    let combined = a.combine(&b);
    assert_eq!(
        combined.baseline,
        Decimal::from(400),
        "combine adds baselines"
    );
    assert_eq!(
        combined.modified,
        Decimal::from(395),
        "combine adds modified values"
    );
    assert_eq!(
        combined.delta,
        Decimal::from(-5),
        "delta recomputed from sums"
    );
}
