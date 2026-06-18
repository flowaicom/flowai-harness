//! Applicative validation — accumulates ALL errors, never short-circuits.
//!
//! Unlike `Result` which short-circuits on the first error, `Validated`
//! collects every failure before returning. This makes it ideal for
//! config validation, form checking, and any context where you want to
//! report all problems at once.
//!
//! # Laws
//!
//! - **L1 (Identity)**: `Validated::from_result(Ok(x)) == Valid(x)`
//! - **L2 (Accumulation)**: `Invalid(e1).and(Invalid(e2)) == Invalid(e1 ++ e2)`
//! - **L3 (Map preservation)**: `Valid(x).map(f) == Valid(f(x))`
//! - **L4 (And-then composition)**: `valid1.and(valid2)` accumulates both error sets

/// Applicative validation — accumulates ALL errors, never short-circuits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validated<T, E> {
    /// The value passed validation.
    Valid(T),
    /// One or more validation errors accumulated.
    Invalid(Vec<E>),
}

impl<T, E> Validated<T, E> {
    /// Apply a function to the valid value (functor map).
    ///
    /// # Law L3 (Map preservation)
    /// `Valid(x).map(f) == Valid(f(x))`
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Validated<U, E> {
        match self {
            Validated::Valid(t) => Validated::Valid(f(t)),
            Validated::Invalid(es) => Validated::Invalid(es),
        }
    }

    /// Short-circuit chaining (like `Result::and_then`).
    ///
    /// Use when the second validation depends on the first value.
    /// This does NOT accumulate — it short-circuits on the first error.
    pub fn and_then<U>(self, f: impl FnOnce(T) -> Validated<U, E>) -> Validated<U, E> {
        match self {
            Validated::Valid(t) => f(t),
            Validated::Invalid(es) => Validated::Invalid(es),
        }
    }

    /// Parallel accumulation — combines two `Validated`, accumulating all errors.
    ///
    /// # Law L2 (Accumulation)
    /// `Invalid(e1).and(Invalid(e2)) == Invalid(e1 ++ e2)`
    ///
    /// # Law L4 (And-then composition)
    /// If both are `Valid`, returns `Valid((a, b))`.
    /// If either or both are `Invalid`, accumulates all errors.
    pub fn and<U>(self, other: Validated<U, E>) -> Validated<(T, U), E> {
        match (self, other) {
            (Validated::Valid(a), Validated::Valid(b)) => Validated::Valid((a, b)),
            (Validated::Invalid(mut e1), Validated::Invalid(e2)) => {
                e1.extend(e2);
                Validated::Invalid(e1)
            }
            (Validated::Invalid(es), _) | (_, Validated::Invalid(es)) => Validated::Invalid(es),
        }
    }

    /// Zip two `Validated` values with a combining function.
    ///
    /// Accumulates errors from both sides.
    pub fn zip_with<U, R>(
        self,
        other: Validated<U, E>,
        f: impl FnOnce(T, U) -> R,
    ) -> Validated<R, E> {
        self.and(other).map(|(a, b)| f(a, b))
    }

    /// Convert from `Result`, mapping `Err` to a single-element `Invalid`.
    ///
    /// # Law L1 (Identity)
    /// `Validated::from_result(Ok(x)) == Valid(x)`
    pub fn from_result(result: Result<T, E>) -> Self {
        match result {
            Ok(t) => Validated::Valid(t),
            Err(e) => Validated::Invalid(vec![e]),
        }
    }

    /// Convert to `Result`, collapsing all errors into a `Vec`.
    pub fn into_result(self) -> Result<T, Vec<E>> {
        match self {
            Validated::Valid(t) => Ok(t),
            Validated::Invalid(es) => Err(es),
        }
    }

    /// Whether this is a valid value.
    pub fn is_valid(&self) -> bool {
        matches!(self, Validated::Valid(_))
    }

    /// Whether this has validation errors.
    pub fn is_invalid(&self) -> bool {
        matches!(self, Validated::Invalid(_))
    }

    /// Extract the valid value, panicking if invalid.
    pub fn unwrap(self) -> T
    where
        E: std::fmt::Debug,
    {
        match self {
            Validated::Valid(t) => t,
            Validated::Invalid(es) => panic!("called unwrap on Invalid: {:?}", es),
        }
    }

    /// Extract the errors, panicking if valid.
    pub fn unwrap_invalid(self) -> Vec<E>
    where
        T: std::fmt::Debug,
    {
        match self {
            Validated::Valid(t) => panic!("called unwrap_invalid on Valid: {:?}", t),
            Validated::Invalid(es) => es,
        }
    }
}

/// Validate all items, accumulating every error.
///
/// Returns `Valid(results)` if all validations pass, or `Invalid(all_errors)`
/// if any fail. Errors are accumulated in order.
pub fn validate_all<T, E>(
    validations: impl IntoIterator<Item = Validated<T, E>>,
) -> Validated<Vec<T>, E> {
    let mut values = Vec::new();
    let mut errors = Vec::new();

    for v in validations {
        match v {
            Validated::Valid(t) => values.push(t),
            Validated::Invalid(es) => errors.extend(es),
        }
    }

    if errors.is_empty() {
        Validated::Valid(values)
    } else {
        Validated::Invalid(errors)
    }
}

/// Validate a predicate, returning the value if it passes.
pub fn ensure<T, E>(value: T, predicate: impl FnOnce(&T) -> bool, error: E) -> Validated<T, E> {
    if predicate(&value) {
        Validated::Valid(value)
    } else {
        Validated::Invalid(vec![error])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // L1: Identity
    // =========================================================================

    #[test]
    fn l1_from_result_ok_is_valid() {
        let v: Validated<i32, String> = Validated::from_result(Ok(42));
        assert_eq!(v, Validated::Valid(42));
    }

    #[test]
    fn l1_from_result_err_is_invalid() {
        let v: Validated<i32, String> = Validated::from_result(Err("bad".into()));
        assert_eq!(v, Validated::Invalid(vec!["bad".to_string()]));
    }

    // =========================================================================
    // L2: Accumulation
    // =========================================================================

    #[test]
    fn l2_both_invalid_accumulates() {
        let a: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
        let b: Validated<i32, &str> = Validated::Invalid(vec!["e2"]);
        let result = a.and(b);
        assert_eq!(result, Validated::Invalid(vec!["e1", "e2"]));
    }

    #[test]
    fn l2_three_invalid_accumulates_all() {
        let a: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
        let b: Validated<i32, &str> = Validated::Invalid(vec!["e2"]);
        let c: Validated<i32, &str> = Validated::Invalid(vec!["e3"]);
        // (a.and(b)) accumulates e1+e2, then .and(c) gets Invalid(e1,e2).and(Invalid(e3))
        // but the and on the tuple doesn't work directly, so use validate_all
        let result = validate_all(vec![a, b, c]);
        assert_eq!(result, Validated::Invalid(vec!["e1", "e2", "e3"]));
    }

    // =========================================================================
    // L3: Map preservation
    // =========================================================================

    #[test]
    fn l3_map_valid() {
        let v: Validated<i32, String> = Validated::Valid(10);
        assert_eq!(v.map(|x| x * 2), Validated::Valid(20));
    }

    #[test]
    fn l3_map_invalid_preserves_errors() {
        let v: Validated<i32, String> = Validated::Invalid(vec!["err".into()]);
        let result = v.map(|x| x * 2);
        assert_eq!(result, Validated::Invalid(vec!["err".to_string()]));
    }

    // =========================================================================
    // L4: And composition
    // =========================================================================

    #[test]
    fn l4_both_valid_produces_tuple() {
        let a: Validated<i32, String> = Validated::Valid(1);
        let b: Validated<i32, String> = Validated::Valid(2);
        assert_eq!(a.and(b), Validated::Valid((1, 2)));
    }

    #[test]
    fn l4_left_invalid() {
        let a: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
        let b: Validated<i32, &str> = Validated::Valid(2);
        assert_eq!(a.and(b), Validated::Invalid(vec!["e1"]));
    }

    #[test]
    fn l4_right_invalid() {
        let a: Validated<i32, &str> = Validated::Valid(1);
        let b: Validated<i32, &str> = Validated::Invalid(vec!["e2"]);
        assert_eq!(a.and(b), Validated::Invalid(vec!["e2"]));
    }

    // =========================================================================
    // and_then (short-circuit)
    // =========================================================================

    #[test]
    fn and_then_valid_chains() {
        let v: Validated<i32, String> = Validated::Valid(10);
        let result = v.and_then(|x| {
            if x > 0 {
                Validated::Valid(x * 2)
            } else {
                Validated::Invalid(vec!["must be positive".into()])
            }
        });
        assert_eq!(result, Validated::Valid(20));
    }

    #[test]
    fn and_then_invalid_short_circuits() {
        let v: Validated<i32, String> = Validated::Invalid(vec!["first".into()]);
        let result: Validated<i32, String> =
            v.and_then(|_| Validated::Invalid(vec!["second".into()]));
        assert_eq!(result, Validated::Invalid(vec!["first".to_string()]));
    }

    // =========================================================================
    // zip_with
    // =========================================================================

    #[test]
    fn zip_with_both_valid() {
        let a: Validated<i32, String> = Validated::Valid(10);
        let b: Validated<i32, String> = Validated::Valid(20);
        let result = a.zip_with(b, |x, y| x + y);
        assert_eq!(result, Validated::Valid(30));
    }

    #[test]
    fn zip_with_accumulates_errors() {
        let a: Validated<i32, &str> = Validated::Invalid(vec!["e1"]);
        let b: Validated<i32, &str> = Validated::Invalid(vec!["e2"]);
        let result = a.zip_with(b, |x, y| x + y);
        assert_eq!(result, Validated::Invalid(vec!["e1", "e2"]));
    }

    // =========================================================================
    // validate_all
    // =========================================================================

    #[test]
    fn validate_all_all_valid() {
        let vs: Vec<Validated<i32, String>> = vec![
            Validated::Valid(1),
            Validated::Valid(2),
            Validated::Valid(3),
        ];
        assert_eq!(validate_all(vs), Validated::Valid(vec![1, 2, 3]));
    }

    #[test]
    fn validate_all_mixed() {
        let vs: Vec<Validated<i32, &str>> = vec![
            Validated::Valid(1),
            Validated::Invalid(vec!["e1"]),
            Validated::Valid(3),
            Validated::Invalid(vec!["e2", "e3"]),
        ];
        assert_eq!(validate_all(vs), Validated::Invalid(vec!["e1", "e2", "e3"]));
    }

    #[test]
    fn validate_all_empty() {
        let vs: Vec<Validated<i32, String>> = vec![];
        assert_eq!(validate_all(vs), Validated::Valid(vec![]));
    }

    // =========================================================================
    // ensure
    // =========================================================================

    #[test]
    fn ensure_passes() {
        let v = ensure(42, |x| *x > 0, "must be positive");
        assert_eq!(v, Validated::Valid(42));
    }

    #[test]
    fn ensure_fails() {
        let v = ensure(-1, |x| *x > 0, "must be positive");
        assert_eq!(v, Validated::Invalid(vec!["must be positive"]));
    }

    // =========================================================================
    // into_result roundtrip
    // =========================================================================

    #[test]
    fn into_result_valid() {
        let v: Validated<i32, String> = Validated::Valid(42);
        assert_eq!(v.into_result(), Ok(42));
    }

    #[test]
    fn into_result_invalid() {
        let v: Validated<i32, String> = Validated::Invalid(vec!["err".into()]);
        assert_eq!(v.into_result(), Err(vec!["err".to_string()]));
    }

    // =========================================================================
    // Hegel — laws as properties
    // =========================================================================

    use hegel::generators;

    #[hegel::test]
    fn law_identity(tc: hegel::TestCase) {
        let x = tc.draw(generators::integers::<i32>());
        let v: Validated<i32, String> = Validated::from_result(Ok(x));
        assert_eq!(v, Validated::Valid(x));
    }

    #[hegel::test]
    fn law_map_preservation(tc: hegel::TestCase) {
        let x = tc.draw(generators::integers::<i32>());
        let v: Validated<i32, String> = Validated::Valid(x);
        let mapped = v.map(|n| n.wrapping_add(1));
        assert_eq!(mapped, Validated::Valid(x.wrapping_add(1)));
    }

    #[hegel::test]
    fn law_accumulation(tc: hegel::TestCase) {
        let e1: String = tc.draw(generators::text().min_size(1).max_size(10));
        let e2: String = tc.draw(generators::text().min_size(1).max_size(10));
        let a: Validated<i32, String> = Validated::Invalid(vec![e1.clone()]);
        let b: Validated<i32, String> = Validated::Invalid(vec![e2.clone()]);
        let result = a.and(b);
        assert_eq!(result, Validated::Invalid(vec![e1, e2]));
    }
}
