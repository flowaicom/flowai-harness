//! Schedule algebraic law test harnesses.
//!
//! # Laws
//!
//! - **L1 (Recurrence bound):** `recurs(n).step(n+1, _) == Done`
//! - **L2 (Forever unbounded):** `forever().step(k, _) != Done` for all k
//! - **L3 (Union commutativity):** `a.union(b) == b.union(a)`
//! - **L4 (Intersect commutativity):** `a.intersect(b) == b.intersect(a)`
//! - **L5 (Cap bounded):** `s.capped(d).delay() <= d`
//! - **L6 (UpTo termination):** `s.up_to(d).step(_, elapsed >= d) == Done`
//! - **L7 (Jitter bounded):** jittered delay in `[d/2, d]`
//! - **L8 (Jitter positivity):** positive delay stays positive after jitter
//! - **L9 (Union associativity):** `(a.union(b)).union(c) == a.union(b.union(c))`
//! - **L10 (Intersect associativity):** `(a.intersect(b)).intersect(c) == a.intersect(b.intersect(c))`
//! - **L11 (RetryPolicy bridge):** Schedule::from_retry_policy matches policy.delay_for_attempt
//! - **L12 (Union identity):** `s.union(never()).step(k, t) == s.step(k, t)`
//! - **L13 (Intersect annihilator):** `s.intersect(never()).step(k, t) == Done`
//! - **L14 (Jitter determinism):** `s.step(k, t) == s.step(k, t)` for jittered schedules
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn schedule_satisfies_laws() {
//!     agent_fw_test::schedule_laws::test_all();
//! }
//! ```

use std::time::Duration;

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_algebra::schedule::{Decision, Schedule};

/// Run all schedule laws (synchronous — no async needed for pure step function).
pub fn test_all() {
    law_l1_recurrence_bound();
    law_l2_forever_unbounded();
    law_l3_union_commutativity();
    law_l4_intersect_commutativity();
    law_l5_cap_bounded();
    law_l6_up_to_termination();
    law_l7_jitter_bounded();
    law_l8_jitter_positivity();
    law_l9_union_associativity();
    law_l10_intersect_associativity();
    law_l11_retry_policy_bridge();
    law_l12_union_identity();
    law_l13_intersect_annihilator();
    law_l14_jitter_determinism();
}

/// L1: `recurs(n).step(n+1, _) == Done`
pub fn law_l1_recurrence_bound() {
    for n in 0..10 {
        let s = Schedule::recurs(n);
        // Attempts 0..=n should Continue
        for k in 0..=n {
            assert!(
                s.step(k, Duration::ZERO).is_continue(),
                "L1: recurs({n}).step({k}, _) should Continue"
            );
        }
        // Attempt n+1 should be Done
        assert_eq!(
            s.step(n + 1, Duration::ZERO),
            Decision::Done,
            "L1: recurs({n}).step({}, _) should be Done",
            n + 1,
        );
    }
}

/// L2: `forever().step(k, _) != Done` for all k
pub fn law_l2_forever_unbounded() {
    let s = Schedule::forever();
    for k in 0..10_000 {
        assert!(
            s.step(k, Duration::ZERO).is_continue(),
            "L2: forever().step({k}, _) should never be Done"
        );
    }
}

/// L3: `a.union(b).step(k,t) == b.union(a).step(k,t)`
pub fn law_l3_union_commutativity() {
    let durations = [
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(200),
    ];
    let elapsed_values = [
        Duration::ZERO,
        Duration::from_secs(1),
        Duration::from_secs(5),
    ];

    for &d1 in &durations {
        for &d2 in &durations {
            let a = Schedule::spaced(d1);
            let b = Schedule::spaced(d2);
            let a2 = Schedule::spaced(d1);
            let b2 = Schedule::spaced(d2);

            let ab = a.union(b);
            let ba = b2.union(a2);

            for attempt in 0..5 {
                for &elapsed in &elapsed_values {
                    assert_eq!(
                        ab.step(attempt, elapsed),
                        ba.step(attempt, elapsed),
                        "L3: union commutativity violated for d1={d1:?}, d2={d2:?}, attempt={attempt}"
                    );
                }
            }
        }
    }

    // Also test with finite schedules
    for n1 in 0..3 {
        for n2 in 0..3 {
            let a = Schedule::recurs(n1);
            let b = Schedule::recurs(n2);
            let a2 = Schedule::recurs(n1);
            let b2 = Schedule::recurs(n2);

            let ab = a.union(b);
            let ba = b2.union(a2);

            for attempt in 0..5 {
                assert_eq!(
                    ab.step(attempt, Duration::ZERO),
                    ba.step(attempt, Duration::ZERO),
                    "L3: union commutativity violated for recurs({n1}), recurs({n2}), attempt={attempt}"
                );
            }
        }
    }
}

/// L4: `a.intersect(b).step(k,t) == b.intersect(a).step(k,t)`
pub fn law_l4_intersect_commutativity() {
    let durations = [
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(200),
    ];

    for &d1 in &durations {
        for &d2 in &durations {
            let a = Schedule::spaced(d1);
            let b = Schedule::spaced(d2);
            let a2 = Schedule::spaced(d1);
            let b2 = Schedule::spaced(d2);

            let ab = a.intersect(b);
            let ba = b2.intersect(a2);

            for attempt in 0..5 {
                assert_eq!(
                    ab.step(attempt, Duration::ZERO),
                    ba.step(attempt, Duration::ZERO),
                    "L4: intersect commutativity violated for d1={d1:?}, d2={d2:?}, attempt={attempt}"
                );
            }
        }
    }
}

/// L5: `s.capped(d).delay() <= d`
pub fn law_l5_cap_bounded() {
    let cap = Duration::from_millis(150);
    let s = Schedule::exponential(Duration::from_millis(100), 2.0).capped(cap);

    for attempt in 0..20 {
        match s.step(attempt, Duration::ZERO) {
            Decision::Continue(d) => {
                assert!(
                    d <= cap,
                    "L5: capped delay {d:?} exceeds cap {cap:?} at attempt {attempt}"
                );
            }
            Decision::Done => {}
        }
    }
}

/// L6: `s.up_to(d).step(_, elapsed >= d) == Done`
pub fn law_l6_up_to_termination() {
    let limit = Duration::from_secs(10);
    let s = Schedule::forever().up_to(limit);

    // Under limit: should continue
    assert!(
        s.step(0, Duration::from_secs(5)).is_continue(),
        "L6: should continue when elapsed < limit"
    );

    // At limit: should be done
    assert_eq!(
        s.step(0, limit),
        Decision::Done,
        "L6: should be Done when elapsed == limit"
    );

    // Over limit: should be done
    assert_eq!(
        s.step(0, Duration::from_secs(15)),
        Decision::Done,
        "L6: should be Done when elapsed > limit"
    );
}

/// L7: Jittered delay in `[d/2, d]`
pub fn law_l7_jitter_bounded() {
    let base_delay = Duration::from_millis(200);
    let s = Schedule::spaced(base_delay).jittered();
    let half = base_delay / 2;

    for _ in 0..200 {
        let d = s.step(0, Duration::ZERO).delay().unwrap();
        assert!(d >= half, "L7: jittered delay {d:?} below half {half:?}");
        assert!(
            d <= base_delay,
            "L7: jittered delay {d:?} above base {base_delay:?}"
        );
    }
}

/// L8: Positive delay stays positive after jitter
pub fn law_l8_jitter_positivity() {
    // Test with small delays (1ms) — the tricky edge case
    let s = Schedule::spaced(Duration::from_millis(1)).jittered();
    for _ in 0..200 {
        let d = s.step(0, Duration::ZERO).delay().unwrap();
        assert!(
            d >= Duration::from_millis(1),
            "L8: positive delay must stay positive, got {d:?}"
        );
    }

    // Zero delay stays zero (jitter of zero = zero)
    let s_zero = Schedule::forever().jittered();
    assert_eq!(
        s_zero.step(0, Duration::ZERO).delay(),
        Some(Duration::ZERO),
        "L8: zero delay should remain zero after jitter"
    );
}

/// L9: `(a.union(b)).union(c) == a.union(b.union(c))`
pub fn law_l9_union_associativity() {
    let delays = [
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(200),
    ];

    for &d1 in &delays {
        for &d2 in &delays {
            for &d3 in &delays {
                // (a.union(b)).union(c)
                let left = Schedule::spaced(d1)
                    .union(Schedule::spaced(d2))
                    .union(Schedule::spaced(d3));

                // a.union(b.union(c))
                let right =
                    Schedule::spaced(d1).union(Schedule::spaced(d2).union(Schedule::spaced(d3)));

                for attempt in 0..5 {
                    assert_eq!(
                        left.step(attempt, Duration::ZERO),
                        right.step(attempt, Duration::ZERO),
                        "L9: union associativity violated for d1={d1:?}, d2={d2:?}, d3={d3:?}, attempt={attempt}"
                    );
                }
            }
        }
    }
}

/// L10: `(a.intersect(b)).intersect(c) == a.intersect(b.intersect(c))`
pub fn law_l10_intersect_associativity() {
    let delays = [
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(200),
    ];

    for &d1 in &delays {
        for &d2 in &delays {
            for &d3 in &delays {
                let left = Schedule::spaced(d1)
                    .intersect(Schedule::spaced(d2))
                    .intersect(Schedule::spaced(d3));

                let right = Schedule::spaced(d1)
                    .intersect(Schedule::spaced(d2).intersect(Schedule::spaced(d3)));

                for attempt in 0..5 {
                    assert_eq!(
                        left.step(attempt, Duration::ZERO),
                        right.step(attempt, Duration::ZERO),
                        "L10: intersect associativity violated for d1={d1:?}, d2={d2:?}, d3={d3:?}, attempt={attempt}"
                    );
                }
            }
        }
    }
}

/// L11: `Schedule::from_retry_policy(p).step(k, _).delay() == p.delay_for_attempt(k)`
pub fn law_l11_retry_policy_bridge() {
    // Test without jitter (deterministic)
    let policies = [
        RetryPolicy::fixed(5, Duration::from_millis(100)),
        RetryPolicy::exponential_backoff(5, Duration::from_millis(100)),
        RetryPolicy::new(3, Duration::from_millis(50))
            .with_multiplier(1.5)
            .with_max_delay(Duration::from_secs(10)),
    ];

    for policy in &policies {
        let schedule = Schedule::from_retry_policy(policy);
        for k in 0..=policy.max_retries() {
            let schedule_delay = schedule.step(k, Duration::ZERO).delay();
            let policy_delay = policy.delay_for_attempt(k);
            assert_eq!(
                schedule_delay,
                Some(policy_delay),
                "L11: schedule delay mismatch at attempt {k} for policy {policy:?}"
            );
        }
        // After exhaustion, schedule should be Done
        assert_eq!(
            schedule.step(policy.max_retries() + 1, Duration::ZERO),
            Decision::Done,
            "L11: schedule should be Done after max_retries for policy {policy:?}"
        );
    }
}

/// L12: `s.union(never()).step(k, t) == s.step(k, t)` (Union identity)
pub fn law_l12_union_identity() {
    let schedules = [
        Schedule::once(),
        Schedule::recurs(3),
        Schedule::forever(),
        Schedule::spaced(Duration::from_millis(100)),
    ];

    for s in &schedules {
        for attempt in 0..5 {
            let elapsed = Duration::from_millis(attempt as u64 * 100);
            let with_never = s.clone().union(Schedule::never());
            assert_eq!(
                with_never.step(attempt, elapsed),
                s.step(attempt, elapsed),
                "L12: s.union(never()) should equal s at attempt {attempt}"
            );
            // Commutativity too
            let never_with = Schedule::never().union(s.clone());
            assert_eq!(
                never_with.step(attempt, elapsed),
                s.step(attempt, elapsed),
                "L12: never().union(s) should equal s at attempt {attempt}"
            );
        }
    }
}

/// L13: `s.intersect(never()).step(k, t) == Done` (Intersect annihilator)
pub fn law_l13_intersect_annihilator() {
    let schedules = [
        Schedule::once(),
        Schedule::recurs(3),
        Schedule::forever(),
        Schedule::spaced(Duration::from_millis(100)),
    ];

    for s in &schedules {
        for attempt in 0..5 {
            let elapsed = Duration::from_millis(attempt as u64 * 100);
            let with_never = s.clone().intersect(Schedule::never());
            assert_eq!(
                with_never.step(attempt, elapsed),
                Decision::Done,
                "L13: s.intersect(never()) should be Done at attempt {attempt}"
            );
        }
    }
}

/// L14: Jittered schedule is deterministic per instance (referential transparency).
pub fn law_l14_jitter_determinism() {
    let s = Schedule::spaced(Duration::from_millis(200)).jittered();
    for attempt in 0..20 {
        let d1 = s.step(attempt, Duration::ZERO);
        let d2 = s.step(attempt, Duration::ZERO);
        assert_eq!(
            d1, d2,
            "L14: jittered schedule must be deterministic per instance at attempt {attempt}"
        );
    }

    // Also test with different delay patterns
    let s2 = Schedule::exponential(Duration::from_millis(100), 2.0)
        .jittered()
        .take(5);
    for attempt in 0..7 {
        let d1 = s2.step(attempt, Duration::ZERO);
        let d2 = s2.step(attempt, Duration::ZERO);
        assert_eq!(
            d1, d2,
            "L14: jittered exponential schedule must be deterministic at attempt {attempt}"
        );
    }
}

// ─── Proptest ────────────────────────────────────────────────────────

#[cfg(test)]
mod hegel_laws {
    use super::*;
    use hegel::generators;

    fn draw_duration(tc: &hegel::TestCase) -> Duration {
        let ms: u64 = tc.draw(generators::integers::<u64>().min_value(1).max_value(999));
        Duration::from_millis(ms)
    }

    fn draw_attempt(tc: &hegel::TestCase) -> u32 {
        tc.draw(generators::integers::<u32>().min_value(0).max_value(99))
    }

    /// L3 hegel: union commutativity
    #[hegel::test]
    fn union_commutative(tc: hegel::TestCase) {
        let d1 = draw_duration(&tc);
        let d2 = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let ab = Schedule::spaced(d1).union(Schedule::spaced(d2));
        let ba = Schedule::spaced(d2).union(Schedule::spaced(d1));
        assert_eq!(
            ab.step(attempt, Duration::ZERO),
            ba.step(attempt, Duration::ZERO),
        );
    }

    /// L4 hegel: intersect commutativity
    #[hegel::test]
    fn intersect_commutative(tc: hegel::TestCase) {
        let d1 = draw_duration(&tc);
        let d2 = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let ab = Schedule::spaced(d1).intersect(Schedule::spaced(d2));
        let ba = Schedule::spaced(d2).intersect(Schedule::spaced(d1));
        assert_eq!(
            ab.step(attempt, Duration::ZERO),
            ba.step(attempt, Duration::ZERO),
        );
    }

    /// L5 hegel: cap bounded
    #[hegel::test]
    fn cap_bounded(tc: hegel::TestCase) {
        let base_ms: u64 = tc.draw(generators::integers::<u64>().min_value(1).max_value(499));
        let cap_ms: u64 = tc.draw(generators::integers::<u64>().min_value(1).max_value(499));
        let attempt = draw_attempt(&tc);
        let cap = Duration::from_millis(cap_ms);
        let s = Schedule::exponential(Duration::from_millis(base_ms), 2.0).capped(cap);
        if let Decision::Continue(d) = s.step(attempt, Duration::ZERO) {
            assert!(d <= cap, "capped delay {:?} exceeds cap {:?}", d, cap);
        }
    }

    /// L9 hegel: union associativity
    #[hegel::test]
    fn union_associative(tc: hegel::TestCase) {
        let d1 = draw_duration(&tc);
        let d2 = draw_duration(&tc);
        let d3 = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let left = Schedule::spaced(d1)
            .union(Schedule::spaced(d2))
            .union(Schedule::spaced(d3));
        let right = Schedule::spaced(d1).union(Schedule::spaced(d2).union(Schedule::spaced(d3)));
        assert_eq!(
            left.step(attempt, Duration::ZERO),
            right.step(attempt, Duration::ZERO),
        );
    }

    /// L10 hegel: intersect associativity
    #[hegel::test]
    fn intersect_associative(tc: hegel::TestCase) {
        let d1 = draw_duration(&tc);
        let d2 = draw_duration(&tc);
        let d3 = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let left = Schedule::spaced(d1)
            .intersect(Schedule::spaced(d2))
            .intersect(Schedule::spaced(d3));
        let right =
            Schedule::spaced(d1).intersect(Schedule::spaced(d2).intersect(Schedule::spaced(d3)));
        assert_eq!(
            left.step(attempt, Duration::ZERO),
            right.step(attempt, Duration::ZERO),
        );
    }

    /// L1 hegel: recurrence bound
    #[hegel::test]
    fn recurrence_bound(tc: hegel::TestCase) {
        let n: u32 = tc.draw(generators::integers::<u32>().min_value(0).max_value(49));
        let s = Schedule::recurs(n);
        for k in 0..=n {
            assert!(s.step(k, Duration::ZERO).is_continue());
        }
        assert_eq!(s.step(n + 1, Duration::ZERO), Decision::Done);
    }

    /// L6 hegel: up_to termination
    #[hegel::test]
    fn up_to_terminates(tc: hegel::TestCase) {
        let limit_ms: u64 = tc.draw(generators::integers::<u64>().min_value(1).max_value(9999));
        let elapsed_extra_ms: u64 =
            tc.draw(generators::integers::<u64>().min_value(0).max_value(4999));
        let limit = Duration::from_millis(limit_ms);
        let elapsed = limit + Duration::from_millis(elapsed_extra_ms);
        let s = Schedule::forever().up_to(limit);
        assert_eq!(s.step(0, elapsed), Decision::Done);
    }

    /// L12 hegel: union identity (never)
    #[hegel::test]
    fn union_identity(tc: hegel::TestCase) {
        let d = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let s = Schedule::spaced(d);
        let with_never = Schedule::spaced(d).union(Schedule::never());
        assert_eq!(
            s.step(attempt, Duration::ZERO),
            with_never.step(attempt, Duration::ZERO),
        );
    }

    /// L13 hegel: intersect annihilator (never)
    #[hegel::test]
    fn intersect_annihilator(tc: hegel::TestCase) {
        let d = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let with_never = Schedule::spaced(d).intersect(Schedule::never());
        assert_eq!(with_never.step(attempt, Duration::ZERO), Decision::Done,);
    }

    /// L14 hegel: jitter determinism per instance
    #[hegel::test]
    fn jitter_deterministic(tc: hegel::TestCase) {
        let d = draw_duration(&tc);
        let attempt = draw_attempt(&tc);
        let s = Schedule::spaced(d).jittered();
        assert_eq!(
            s.step(attempt, Duration::ZERO),
            s.step(attempt, Duration::ZERO),
        );
    }
}
