//! Meet-semilattice algebraic law harness.
//!
//! Verifies that an operation forms a meet-semilattice:
//!
//! - **L1 (Associativity)**: `merge(merge(a, b), c) == merge(a, merge(b, c))`
//! - **L2 (Commutativity)**: `merge(a, b) == merge(b, a)`
//! - **L3 (Identity)**: `merge(a, identity) == a`
//! - **L4 (Idempotence)**: `merge(a, a) == a`
//!
//! # Usage
//!
//! ```ignore
//! use proptest::prelude::*;
//!
//! #[test]
//! fn price_bounds_semilattice() {
//!     agent_fw_test::semilattice_laws::test_all(
//!         arb_bounds().boxed(),  // proptest BoxedStrategy<T>
//!         Bounds::NONE,     // identity element
//!         |a, b| a.merge(b),    // merge function
//!         200,                   // number of proptest cases
//!     );
//! }
//! ```

use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use proptest::test_runner::{Config, TestRunner};
use std::fmt::Debug;

/// Run all four meet-semilattice laws (L1–L4) against an arbitrary type.
///
/// - `strategy`: boxed proptest strategy generating values of type `T`
/// - `identity`: the identity element (e.g. `Bounds::NONE`)
/// - `merge`: the binary operation `(&T, &T) -> T`
/// - `cases`: number of proptest iterations per law
pub fn test_all<T, F>(strategy: BoxedStrategy<T>, identity: T, merge: F, cases: u32)
where
    T: Clone + Debug + PartialEq + 'static,
    F: Fn(&T, &T) -> T,
{
    law_associativity(strategy.clone(), &merge, cases);
    law_commutativity(strategy.clone(), &merge, cases);
    law_identity(strategy.clone(), &identity, &merge, cases);
    law_idempotence(strategy, &merge, cases);
}

/// L1 (Associativity): `merge(merge(a, b), c) == merge(a, merge(b, c))`.
pub fn law_associativity<T, F>(strategy: BoxedStrategy<T>, merge: &F, cases: u32)
where
    T: Clone + Debug + PartialEq + 'static,
    F: Fn(&T, &T) -> T,
{
    let config = Config::with_cases(cases);
    let mut runner = TestRunner::new(config);

    runner
        .run(
            &(strategy.clone(), strategy.clone(), strategy),
            |(a, b, c)| {
                let left = merge(&merge(&a, &b), &c);
                let right = merge(&a, &merge(&b, &c));
                prop_assert_eq!(left, right, "L1: merge must be associative");
                Ok(())
            },
        )
        .expect("L1 (Associativity) proptest");
}

/// L2 (Commutativity): `merge(a, b) == merge(b, a)`.
pub fn law_commutativity<T, F>(strategy: BoxedStrategy<T>, merge: &F, cases: u32)
where
    T: Clone + Debug + PartialEq + 'static,
    F: Fn(&T, &T) -> T,
{
    let config = Config::with_cases(cases);
    let mut runner = TestRunner::new(config);

    runner
        .run(&(strategy.clone(), strategy), |(a, b)| {
            prop_assert_eq!(
                merge(&a, &b),
                merge(&b, &a),
                "L2: merge must be commutative"
            );
            Ok(())
        })
        .expect("L2 (Commutativity) proptest");
}

/// L3 (Identity): `merge(a, identity) == a` and `merge(identity, a) == a`.
pub fn law_identity<T, F>(strategy: BoxedStrategy<T>, identity: &T, merge: &F, cases: u32)
where
    T: Clone + Debug + PartialEq + 'static,
    F: Fn(&T, &T) -> T,
{
    let config = Config::with_cases(cases);
    let mut runner = TestRunner::new(config);

    runner
        .run(&strategy, |a| {
            prop_assert_eq!(merge(&a, identity), a.clone(), "L3: identity right");
            prop_assert_eq!(merge(identity, &a), a, "L3: identity left");
            Ok(())
        })
        .expect("L3 (Identity) proptest");
}

/// L4 (Idempotence): `merge(a, a) == a`.
pub fn law_idempotence<T, F>(strategy: BoxedStrategy<T>, merge: &F, cases: u32)
where
    T: Clone + Debug + PartialEq + 'static,
    F: Fn(&T, &T) -> T,
{
    let config = Config::with_cases(cases);
    let mut runner = TestRunner::new(config);

    runner
        .run(&strategy, |a| {
            prop_assert_eq!(merge(&a, &a), a, "L4: merge must be idempotent");
            Ok(())
        })
        .expect("L4 (Idempotence) proptest");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trivial semilattice: min over u32 with identity = u32::MAX
    fn arb_u32() -> BoxedStrategy<u32> {
        (0u32..1000).boxed()
    }

    #[test]
    fn u32_min_semilattice() {
        test_all(arb_u32(), u32::MAX, |a, b| *a.min(b), 200);
    }
}
