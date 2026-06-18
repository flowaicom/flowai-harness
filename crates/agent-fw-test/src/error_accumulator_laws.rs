//! ErrorAccumulator monoid law test harnesses.
//!
//! # Laws
//!
//! - L1 (Left identity): `combine(empty, a) == a`
//! - L2 (Right identity): `combine(a, empty) == a`
//! - L3 (Associativity): `combine(combine(a, b), c) == combine(a, combine(b, c))`
//! - L4 (Into-result empty): `empty.into_result(v) == Ok(v)`
//! - L5 (Into-result non-empty): `non_empty.into_result(v) == Err(errors)`
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn error_accumulator_satisfies_all_laws() {
//!     agent_fw_test::error_accumulator_laws::test_all();
//! }
//! ```

use agent_fw_algebra::ErrorAccumulator;

/// Run all ErrorAccumulator monoid laws.
pub fn test_all() {
    law_left_identity();
    law_right_identity();
    law_associativity();
    law_into_result_empty();
    law_into_result_non_empty();
}

/// L1: combine(empty, a) == a — left identity.
pub fn law_left_identity() {
    let empty = ErrorAccumulator::<String>::new();
    let mut a = ErrorAccumulator::<String>::new();
    a.push("error1".to_string());
    a.push("error2".to_string());

    let result = empty.combine(a);
    assert_eq!(
        result.into_vec(),
        vec!["error1".to_string(), "error2".to_string()],
        "L1: combine(empty, a) must equal a"
    );
}

/// L2: combine(a, empty) == a — right identity.
pub fn law_right_identity() {
    let empty = ErrorAccumulator::<String>::new();
    let mut a = ErrorAccumulator::<String>::new();
    a.push("error1".to_string());
    a.push("error2".to_string());

    let result = a.combine(empty);
    assert_eq!(
        result.into_vec(),
        vec!["error1".to_string(), "error2".to_string()],
        "L2: combine(a, empty) must equal a"
    );
}

/// L3: combine(combine(a, b), c) == combine(a, combine(b, c)) — associativity.
pub fn law_associativity() {
    // Left-associated: combine(combine(a, b), c)
    let left = {
        let a = ErrorAccumulator::single("a".to_string());
        let b = ErrorAccumulator::single("b".to_string());
        let c = ErrorAccumulator::single("c".to_string());
        a.combine(b).combine(c)
    };

    // Right-associated: combine(a, combine(b, c))
    let right = {
        let a = ErrorAccumulator::single("a".to_string());
        let b = ErrorAccumulator::single("b".to_string());
        let c = ErrorAccumulator::single("c".to_string());
        a.combine(b.combine(c))
    };

    assert_eq!(
        left.into_vec(),
        right.into_vec(),
        "L3: combine must be associative"
    );
}

/// L4: empty.into_result(v) == Ok(v).
pub fn law_into_result_empty() {
    let acc = ErrorAccumulator::<String>::new();
    let result = acc.into_result(42);
    assert_eq!(result, Ok(42), "L4: empty accumulator must yield Ok(v)");
}

/// L5: non_empty.into_result(v) == Err(errors).
pub fn law_into_result_non_empty() {
    let mut acc = ErrorAccumulator::new();
    acc.push("err1".to_string());
    acc.push("err2".to_string());

    let result = acc.into_result(42);
    assert_eq!(
        result,
        Err(vec!["err1".to_string(), "err2".to_string()]),
        "L5: non-empty accumulator must yield Err(errors)"
    );
}
