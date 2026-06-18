//! Validated<T, E> algebraic law test harnesses.
//!
//! Verifies that the `Validated` applicative functor satisfies its laws.
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn validated_satisfies_laws() {
//!     agent_fw_test::validated_laws::test_all();
//! }
//! ```

use agent_fw_algebra::{ensure, validate_all, Validated};

/// Run all deterministic Validated laws.
pub fn test_all() {
    law_identity();
    law_accumulation();
    law_map_preservation();
    law_and_accumulates_both();
    law_zip_with_combines();
    law_from_result_ok();
    law_from_result_err();
    law_validate_all_collects();
    law_ensure_pass();
    law_ensure_fail();
    law_and_then_short_circuits();
}

/// L1 (Identity): `Validated::from_result(Ok(x))` == `Valid(x)`.
pub fn law_identity() {
    let v: Validated<i32, String> = Validated::from_result(Ok(42));
    assert!(v.is_valid(), "L1: from_result(Ok(x)) must be Valid");
    assert_eq!(v.unwrap(), 42, "L1: Valid value must match");
}

/// L2 (Accumulation): combining two Invalid accumulates all errors.
pub fn law_accumulation() {
    let v1: Validated<i32, &str> = Validated::Invalid(vec!["err1"]);
    let v2: Validated<i32, &str> = Validated::Invalid(vec!["err2"]);
    let combined = v1.and(v2);
    assert!(
        combined.is_invalid(),
        "L2: Invalid + Invalid must be Invalid"
    );
    let errors = combined.unwrap_invalid();
    assert_eq!(errors.len(), 2, "L2: must accumulate both errors");
    assert!(errors.contains(&"err1"), "L2: must contain first error");
    assert!(errors.contains(&"err2"), "L2: must contain second error");
}

/// L3 (Map preservation): `Valid(x).map(f)` == `Valid(f(x))`.
pub fn law_map_preservation() {
    let v: Validated<i32, String> = Validated::Valid(10);
    let mapped = v.map(|x| x * 2);
    assert_eq!(mapped.unwrap(), 20, "L3: map on Valid must apply function");
}

/// L3 (Map on Invalid): `Invalid(es).map(f)` == `Invalid(es)`.
fn _law_map_on_invalid() {
    let v: Validated<i32, &str> = Validated::Invalid(vec!["err"]);
    let mapped = v.map(|x| x * 2);
    assert!(
        mapped.is_invalid(),
        "L3: map on Invalid must remain Invalid"
    );
}

/// L4 (And accumulates both): `v1.and(v2)` accumulates errors from both sides.
pub fn law_and_accumulates_both() {
    // Valid + Valid = Valid (tuple of both values)
    let v1: Validated<i32, &str> = Validated::Valid(1);
    let v2: Validated<i32, &str> = Validated::Valid(2);
    let result = v1.and(v2);
    assert!(result.is_valid(), "L4: Valid.and(Valid) must be Valid");
    assert_eq!(
        result.unwrap(),
        (1, 2),
        "L4: and returns tuple of both values"
    );

    // Valid + Invalid = Invalid
    let v1: Validated<i32, &str> = Validated::Valid(1);
    let v2: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
    let result = v1.and(v2);
    assert!(
        result.is_invalid(),
        "L4: Valid.and(Invalid) must be Invalid"
    );

    // Invalid + Valid = Invalid
    let v1: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
    let v2: Validated<i32, &str> = Validated::Valid(2);
    let result = v1.and(v2);
    assert!(
        result.is_invalid(),
        "L4: Invalid.and(Valid) must be Invalid"
    );

    // Invalid + Invalid = Invalid with both error sets
    let v1: Validated<i32, &str> = Validated::Invalid(vec!["e1", "e2"]);
    let v2: Validated<i32, &str> = Validated::Invalid(vec!["e3"]);
    let result = v1.and(v2);
    assert!(
        result.is_invalid(),
        "L4: Invalid.and(Invalid) must be Invalid"
    );
    assert_eq!(
        result.unwrap_invalid().len(),
        3,
        "L4: must accumulate all errors"
    );
}

/// zip_with combines two Valid values.
pub fn law_zip_with_combines() {
    let v1: Validated<i32, &str> = Validated::Valid(3);
    let v2: Validated<i32, &str> = Validated::Valid(4);
    let result = v1.zip_with(v2, |a, b| a + b);
    assert_eq!(result.unwrap(), 7, "zip_with must combine values");

    // zip_with with one Invalid
    let v1: Validated<i32, &str> = Validated::Valid(3);
    let v2: Validated<i32, &str> = Validated::Invalid(vec!["err"]);
    let result = v1.zip_with(v2, |a, b| a + b);
    assert!(
        result.is_invalid(),
        "zip_with(Valid, Invalid) must be Invalid"
    );
}

/// from_result converts Ok to Valid.
pub fn law_from_result_ok() {
    let r: Result<i32, String> = Ok(42);
    let v = Validated::from_result(r);
    assert_eq!(v.unwrap(), 42);
}

/// from_result converts Err to Invalid.
pub fn law_from_result_err() {
    let r: Result<i32, String> = Err("bad".into());
    let v = Validated::from_result(r);
    assert!(v.is_invalid());
    assert_eq!(v.unwrap_invalid(), vec!["bad".to_string()]);
}

/// validate_all collects all errors from a vec of validations.
pub fn law_validate_all_collects() {
    let validations: Vec<Validated<i32, &str>> = vec![
        Validated::Valid(1),
        Validated::Invalid(vec!["e1"]),
        Validated::Valid(2),
        Validated::Invalid(vec!["e2", "e3"]),
    ];
    let result = validate_all(validations);
    assert!(result.is_invalid());
    let errors = result.unwrap_invalid();
    assert_eq!(errors.len(), 3, "validate_all must collect all errors");
}

/// ensure creates Valid(value) when predicate holds.
pub fn law_ensure_pass() {
    let result: Validated<i32, &str> = ensure(42, |v| *v > 0, "should not appear");
    assert!(result.is_valid());
    assert_eq!(result.unwrap(), 42);
}

/// ensure creates Invalid when predicate fails.
pub fn law_ensure_fail() {
    let result = ensure(-1, |v| *v > 0, "must be positive");
    assert!(result.is_invalid());
    assert_eq!(result.unwrap_invalid(), vec!["must be positive"]);
}

/// and_then short-circuits on Invalid (does not call f).
pub fn law_and_then_short_circuits() {
    let v: Validated<i32, &str> = Validated::Invalid(vec!["initial"]);
    let result = v.and_then(|_| -> Validated<i32, &str> {
        panic!("should not be called on Invalid");
    });
    assert!(result.is_invalid());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_validated_laws() {
        test_all();
    }
}
