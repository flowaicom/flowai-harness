//! Algebraic abstractions: Semigroup, Monoid, Validated.
//!
//! These traits formalize the ad-hoc `.combine()` and `.ZERO` patterns
//! found throughout the framework. By defining them as traits, combinators
//! in `agent-fw-algebra` can be generic over any monoidal type.
//!
//! # Design Choices
//!
//! - `Semigroup` requires only associative `combine`.
//! - `Monoid` extends `Semigroup` with an identity element.
//! - Both are object-safe (no `Self: Sized` bound on methods).
//! - The `fold` free function works for any `Monoid`.
//! - `Validated<E, A>` provides applicative error accumulation.
//!
//! # Laws
//!
//! ## Semigroup
//! - **Associativity**: `a.combine(b).combine(c) == a.combine(b.combine(c))`
//!
//! ## Monoid (extends Semigroup)
//! - **Left identity**:  `Self::empty().combine(a) == a`
//! - **Right identity**: `a.combine(Self::empty()) == a`

use std::collections::HashMap;
use std::hash::Hash;

// =============================================================================
// Semigroup
// =============================================================================

/// A type with an associative binary operation.
///
/// # Law: Associativity
///
/// For all `a`, `b`, `c`:
/// ```text
/// a.combine(&b).combine(&c) == a.combine(&b.combine(&c))
/// ```
pub trait Semigroup: Sized {
    /// Associative binary operation.
    fn combine(&self, other: &Self) -> Self;
}

// =============================================================================
// Monoid
// =============================================================================

/// A `Semigroup` with an identity element.
///
/// # Laws
///
/// For all `a`:
/// ```text
/// Self::empty().combine(&a) == a   // left identity
/// a.combine(&Self::empty()) == a   // right identity
/// ```
pub trait Monoid: Semigroup {
    /// The identity element.
    fn empty() -> Self;

    /// Check if this value is the identity element.
    ///
    /// Default implementation uses `PartialEq` if available;
    /// types may override for efficiency.
    fn is_empty(&self) -> bool
    where
        Self: PartialEq,
    {
        *self == Self::empty()
    }
}

// =============================================================================
// Free functions
// =============================================================================

/// Fold a collection using the monoid operation.
///
/// Returns `Monoid::empty()` for empty iterators.
///
/// ```text
/// fold([]) == empty()
/// fold([a]) == a
/// fold([a, b, c]) == a.combine(&b).combine(&c)
/// ```
pub fn fold<M: Monoid>(items: impl IntoIterator<Item = M>) -> M {
    items
        .into_iter()
        .fold(M::empty(), |acc, item| acc.combine(&item))
}

/// Fold a collection of references using the monoid operation.
pub fn fold_ref<'a, M: Monoid + 'a>(items: impl IntoIterator<Item = &'a M>) -> M {
    items
        .into_iter()
        .fold(M::empty(), |acc, item| acc.combine(item))
}

/// Combine two values (free function form).
pub fn combine<S: Semigroup>(a: &S, b: &S) -> S {
    a.combine(b)
}

// =============================================================================
// Standard library instances
// =============================================================================

/// Additive monoid for numeric types.
macro_rules! impl_additive_monoid {
    ($($t:ty),+) => {
        $(
            impl Semigroup for $t {
                #[inline]
                fn combine(&self, other: &Self) -> Self {
                    self.saturating_add(*other)
                }
            }

            impl Monoid for $t {
                #[inline]
                fn empty() -> Self {
                    0
                }
            }
        )+
    };
}

// Only unsigned types: saturating_add is associative for unsigned integers.
// Signed types are excluded because saturating_add is NOT associative when
// saturation at MAX/MIN discards addends:
//   (-1).sat_add(1).sat_add(MAX) = MAX  ≠  (-1).sat_add(1.sat_add(MAX)) = MAX-1
impl_additive_monoid!(u8, u16, u32, u64, u128, usize);

/// String concatenation monoid.
impl Semigroup for String {
    fn combine(&self, other: &Self) -> Self {
        let mut s = self.clone();
        s.push_str(other);
        s
    }
}

impl Monoid for String {
    fn empty() -> Self {
        String::new()
    }
}

/// Vec concatenation semigroup.
impl<T: Clone> Semigroup for Vec<T> {
    fn combine(&self, other: &Self) -> Self {
        let mut v = self.clone();
        v.extend(other.iter().cloned());
        v
    }
}

impl<T: Clone> Monoid for Vec<T> {
    fn empty() -> Self {
        Vec::new()
    }
}

/// HashMap merge (last-write-wins) monoid.
impl<K: Eq + Hash + Clone, V: Clone> Semigroup for HashMap<K, V> {
    fn combine(&self, other: &Self) -> Self {
        let mut m = self.clone();
        m.extend(other.iter().map(|(k, v)| (k.clone(), v.clone())));
        m
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Monoid for HashMap<K, V> {
    fn empty() -> Self {
        HashMap::new()
    }
}

/// Boolean OR semigroup (join-semilattice).
///
/// Useful for "degraded" flags that are true if ANY component is degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Any(pub bool);

impl Semigroup for Any {
    #[inline]
    fn combine(&self, other: &Self) -> Self {
        Any(self.0 || other.0)
    }
}

impl Monoid for Any {
    #[inline]
    fn empty() -> Self {
        Any(false)
    }
}

/// Boolean AND semigroup (meet-semilattice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct All(pub bool);

impl Semigroup for All {
    #[inline]
    fn combine(&self, other: &Self) -> Self {
        All(self.0 && other.0)
    }
}

impl Monoid for All {
    #[inline]
    fn empty() -> Self {
        All(true)
    }
}

/// Max semigroup for ordered types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Max<T>(pub T);

impl<T: Ord + Clone> Semigroup for Max<T> {
    fn combine(&self, other: &Self) -> Self {
        if self.0 >= other.0 {
            self.clone()
        } else {
            other.clone()
        }
    }
}

/// Min semigroup for ordered types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Min<T>(pub T);

impl<T: Ord + Clone> Semigroup for Min<T> {
    fn combine(&self, other: &Self) -> Self {
        if self.0 <= other.0 {
            self.clone()
        } else {
            other.clone()
        }
    }
}

/// Option<T> forms a monoid when T is a semigroup (first-nonempty wins, then combine).
impl<T: Semigroup + Clone> Semigroup for Option<T> {
    fn combine(&self, other: &Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (Some(_), None) => self.clone(),
            (None, Some(_)) => other.clone(),
            (None, None) => None,
        }
    }
}

impl<T: Semigroup + Clone> Monoid for Option<T> {
    fn empty() -> Self {
        None
    }
}

// =============================================================================
// Validated<E, A> — Applicative error accumulation
// =============================================================================

/// Applicative validation: accumulates ALL errors rather than short-circuiting.
///
/// Unlike `Result<A, E>`, which stops at the first error,
/// `Validated<E, A>` collects all failures.
///
/// # Laws
///
/// - **Identity**: `valid(a).and_then(f) == f(a)`
/// - **Accumulation**: `invalid(e1).combine_with(invalid(e2)) == invalid(e1 ++ e2)`
/// - **Functor**: `valid(a).map(f) == valid(f(a))`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validated<E, A> {
    Valid(A),
    Invalid(Vec<E>),
}

impl<E, A> Validated<E, A> {
    /// Construct a valid value.
    pub fn valid(a: A) -> Self {
        Validated::Valid(a)
    }

    /// Construct an invalid value with a single error.
    pub fn invalid(e: E) -> Self {
        Validated::Invalid(vec![e])
    }

    /// Construct an invalid value with multiple errors.
    pub fn invalid_many(errors: Vec<E>) -> Self {
        Validated::Invalid(errors)
    }

    /// Check if valid.
    pub fn is_valid(&self) -> bool {
        matches!(self, Validated::Valid(_))
    }

    /// Check if invalid.
    pub fn is_invalid(&self) -> bool {
        matches!(self, Validated::Invalid(_))
    }

    /// Map the success value.
    pub fn map<B>(self, f: impl FnOnce(A) -> B) -> Validated<E, B> {
        match self {
            Validated::Valid(a) => Validated::Valid(f(a)),
            Validated::Invalid(es) => Validated::Invalid(es),
        }
    }

    /// Map the error values.
    pub fn map_errors<F>(self, f: impl FnMut(E) -> F) -> Validated<F, A> {
        match self {
            Validated::Valid(a) => Validated::Valid(a),
            Validated::Invalid(es) => Validated::Invalid(es.into_iter().map(f).collect()),
        }
    }

    /// Convert to Result, losing individual error granularity.
    pub fn to_result(self) -> Result<A, Vec<E>> {
        match self {
            Validated::Valid(a) => Ok(a),
            Validated::Invalid(es) => Err(es),
        }
    }

    /// Convert from Result.
    pub fn from_result(result: Result<A, E>) -> Self {
        match result {
            Ok(a) => Validated::Valid(a),
            Err(e) => Validated::Invalid(vec![e]),
        }
    }

    /// Flat-map (note: this short-circuits like Result; use `ap` for accumulation).
    pub fn and_then<B>(self, f: impl FnOnce(A) -> Validated<E, B>) -> Validated<E, B> {
        match self {
            Validated::Valid(a) => f(a),
            Validated::Invalid(es) => Validated::Invalid(es),
        }
    }

    /// Ensure a predicate holds, accumulating errors if not.
    ///
    /// If the value is Valid and the predicate fails, returns Invalid.
    /// If the value is already Invalid, returns it unchanged.
    pub fn ensure(self, predicate: impl FnOnce(&A) -> bool, error: E) -> Self {
        match self {
            Validated::Valid(ref a) if !predicate(a) => Validated::Invalid(vec![error]),
            other => other,
        }
    }
}

impl<E, A> Validated<E, A>
where
    A: Clone,
{
    /// Applicative combine: accumulate errors from two validations,
    /// producing a tuple on success.
    pub fn zip<B: Clone>(self, other: Validated<E, B>) -> Validated<E, (A, B)> {
        match (self, other) {
            (Validated::Valid(a), Validated::Valid(b)) => Validated::Valid((a, b)),
            (Validated::Invalid(mut e1), Validated::Invalid(e2)) => {
                e1.extend(e2);
                Validated::Invalid(e1)
            }
            (Validated::Invalid(e), _) | (_, Validated::Invalid(e)) => Validated::Invalid(e),
        }
    }

    /// Applicative combine with a mapping function.
    pub fn map2<B: Clone, C>(
        self,
        other: Validated<E, B>,
        f: impl FnOnce(A, B) -> C,
    ) -> Validated<E, C> {
        self.zip(other).map(|(a, b)| f(a, b))
    }
}

/// Accumulate validations from an iterator.
///
/// Returns `Valid(Vec<A>)` if all items are valid,
/// or `Invalid(all_errors)` if any are invalid.
pub fn sequence_validated<E, A>(
    items: impl IntoIterator<Item = Validated<E, A>>,
) -> Validated<E, Vec<A>> {
    let mut values = Vec::new();
    let mut errors = Vec::new();

    for item in items {
        match item {
            Validated::Valid(a) => values.push(a),
            Validated::Invalid(es) => errors.extend(es),
        }
    }

    if errors.is_empty() {
        Validated::Valid(values)
    } else {
        Validated::Invalid(errors)
    }
}

/// Create a `Validated` from a condition check.
pub fn validate_that<E>(condition: bool, error: E) -> Validated<E, ()> {
    if condition {
        Validated::Valid(())
    } else {
        Validated::Invalid(vec![error])
    }
}

/// Builder for accumulating validations.
///
/// ```rust,ignore
/// let result = ValidationBuilder::new()
///     .check(x > 0, "x must be positive")
///     .check(y < 100, "y must be less than 100")
///     .check(!name.is_empty(), "name required")
///     .finish(|| MyConfig { x, y, name });
/// ```
pub struct ValidationBuilder<E> {
    errors: Vec<E>,
}

impl<E> ValidationBuilder<E> {
    /// Create a new builder.
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Add a validation check.
    pub fn check(mut self, condition: bool, error: E) -> Self {
        if !condition {
            self.errors.push(error);
        }
        self
    }

    /// Add an error conditionally.
    pub fn check_with(mut self, f: impl FnOnce() -> Option<E>) -> Self {
        if let Some(e) = f() {
            self.errors.push(e);
        }
        self
    }

    /// Add multiple errors from a validation.
    pub fn accumulate(mut self, result: Validated<E, ()>) -> Self {
        if let Validated::Invalid(es) = result {
            self.errors.extend(es);
        }
        self
    }

    /// Finish validation: returns the constructed value or accumulated errors.
    pub fn finish<A>(self, f: impl FnOnce() -> A) -> Validated<E, A> {
        if self.errors.is_empty() {
            Validated::Valid(f())
        } else {
            Validated::Invalid(self.errors)
        }
    }

    /// Finish validation, returning Result.
    pub fn finish_result<A>(self, f: impl FnOnce() -> A) -> Result<A, Vec<E>> {
        self.finish(f).to_result()
    }
}

impl<E> Default for ValidationBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Semigroup / Monoid laws for u64
    // =========================================================================

    #[test]
    fn u64_semigroup_associativity() {
        let a: u64 = 10;
        let b: u64 = 20;
        let c: u64 = 30;
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[test]
    fn u64_monoid_identity() {
        let a: u64 = 42;
        assert_eq!(u64::empty().combine(&a), a);
        assert_eq!(a.combine(&u64::empty()), a);
    }

    // =========================================================================
    // String monoid
    // =========================================================================

    #[test]
    fn string_semigroup() {
        let a = "hello".to_string();
        let b = " world".to_string();
        assert_eq!(a.combine(&b), "hello world");
    }

    #[test]
    fn string_monoid_identity() {
        let a = "test".to_string();
        assert_eq!(String::empty().combine(&a), a);
        assert_eq!(a.combine(&String::empty()), a);
    }

    // =========================================================================
    // Vec monoid
    // =========================================================================

    #[test]
    fn vec_semigroup() {
        let a = vec![1, 2];
        let b = vec![3, 4];
        assert_eq!(a.combine(&b), vec![1, 2, 3, 4]);
    }

    #[test]
    fn vec_monoid_identity() {
        let a = vec![1, 2, 3];
        assert_eq!(Vec::<i32>::empty().combine(&a), a);
        assert_eq!(a.combine(&Vec::empty()), a);
    }

    // =========================================================================
    // Any / All
    // =========================================================================

    #[test]
    fn any_semilattice() {
        assert_eq!(Any(false).combine(&Any(false)), Any(false));
        assert_eq!(Any(true).combine(&Any(false)), Any(true));
        assert_eq!(Any(false).combine(&Any(true)), Any(true));
        assert_eq!(Any(true).combine(&Any(true)), Any(true));
    }

    #[test]
    fn all_semilattice() {
        assert_eq!(All(true).combine(&All(true)), All(true));
        assert_eq!(All(true).combine(&All(false)), All(false));
        assert_eq!(All(false).combine(&All(true)), All(false));
        assert_eq!(All(false).combine(&All(false)), All(false));
    }

    // =========================================================================
    // Option monoid
    // =========================================================================

    #[test]
    fn option_semigroup() {
        let a: Option<u64> = Some(10);
        let b: Option<u64> = Some(20);
        assert_eq!(a.combine(&b), Some(30));
        assert_eq!(a.combine(&None), Some(10));
        assert_eq!(None::<u64>.combine(&b), Some(20));
        assert_eq!(None::<u64>.combine(&None), None);
    }

    // =========================================================================
    // fold
    // =========================================================================

    #[test]
    fn fold_empty_returns_identity() {
        let result: u64 = fold(std::iter::empty::<u64>());
        assert_eq!(result, 0);
    }

    #[test]
    fn fold_accumulates() {
        let result: u64 = fold(vec![10u64, 20, 30]);
        assert_eq!(result, 60);
    }

    #[test]
    fn fold_ref_works() {
        let items = vec![10u64, 20, 30];
        let result = fold_ref(&items);
        assert_eq!(result, 60);
    }

    // =========================================================================
    // Max / Min
    // =========================================================================

    #[test]
    fn max_semigroup() {
        assert_eq!(Max(10).combine(&Max(20)), Max(20));
        assert_eq!(Max(30).combine(&Max(10)), Max(30));
    }

    #[test]
    fn min_semigroup() {
        assert_eq!(Min(10).combine(&Min(20)), Min(10));
        assert_eq!(Min(30).combine(&Min(10)), Min(10));
    }

    // =========================================================================
    // Validated
    // =========================================================================

    #[test]
    fn validated_valid_map() {
        let v: Validated<String, i32> = Validated::valid(42);
        assert_eq!(v.map(|x| x * 2), Validated::valid(84));
    }

    #[test]
    fn validated_invalid_propagates() {
        let v: Validated<String, i32> = Validated::invalid("oops".into());
        assert_eq!(
            v.map(|x| x * 2),
            Validated::Invalid(vec!["oops".to_string()])
        );
    }

    #[test]
    fn validated_zip_accumulates_errors() {
        let a: Validated<&str, i32> = Validated::invalid("e1");
        let b: Validated<&str, i32> = Validated::invalid("e2");
        match a.zip(b) {
            Validated::Invalid(es) => {
                assert_eq!(es, vec!["e1", "e2"]);
            }
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn validated_zip_success() {
        let a: Validated<&str, i32> = Validated::valid(1);
        let b: Validated<&str, i32> = Validated::valid(2);
        assert_eq!(a.zip(b), Validated::Valid((1, 2)));
    }

    #[test]
    fn validated_map2() {
        let a: Validated<&str, i32> = Validated::valid(1);
        let b: Validated<&str, i32> = Validated::valid(2);
        assert_eq!(a.map2(b, |x, y| x + y), Validated::valid(3));
    }

    #[test]
    fn sequence_validated_all_valid() {
        let items = vec![
            Validated::<&str, i32>::valid(1),
            Validated::valid(2),
            Validated::valid(3),
        ];
        assert_eq!(sequence_validated(items), Validated::Valid(vec![1, 2, 3]));
    }

    #[test]
    fn sequence_validated_accumulates_all_errors() {
        let items = vec![
            Validated::<&str, i32>::invalid("e1"),
            Validated::valid(2),
            Validated::invalid("e2"),
        ];
        match sequence_validated(items) {
            Validated::Invalid(es) => assert_eq!(es, vec!["e1", "e2"]),
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn validate_that_works() {
        assert!(validate_that(true, "err").is_valid());
        assert!(validate_that(false, "err").is_invalid());
    }

    #[test]
    fn validation_builder() {
        let result = ValidationBuilder::<&str>::new()
            .check(true, "a")
            .check(false, "b")
            .check(false, "c")
            .finish(|| 42);
        match result {
            Validated::Invalid(es) => assert_eq!(es, vec!["b", "c"]),
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn validated_ensure_passes() {
        let v: Validated<&str, i32> = Validated::valid(42);
        assert_eq!(
            v.ensure(|x| *x > 0, "must be positive"),
            Validated::valid(42)
        );
    }

    #[test]
    fn validated_ensure_fails() {
        let v: Validated<&str, i32> = Validated::valid(-1);
        assert_eq!(
            v.ensure(|x| *x > 0, "must be positive"),
            Validated::Invalid(vec!["must be positive"])
        );
    }

    #[test]
    fn validated_ensure_on_invalid_is_noop() {
        let v: Validated<&str, i32> = Validated::invalid("already bad");
        assert_eq!(
            v.ensure(|x| *x > 0, "must be positive"),
            Validated::Invalid(vec!["already bad"])
        );
    }

    #[test]
    fn validation_builder_success() {
        let result = ValidationBuilder::<&str>::new()
            .check(true, "a")
            .check(true, "b")
            .finish(|| 42);
        assert_eq!(result, Validated::Valid(42));
    }

    // =========================================================================
    // Property-Based Tests (Hegel)
    // =========================================================================

    mod hegel_laws {
        use super::*;
        use hegel::generators;

        // --- Numeric monoids: full-range generators (no artificial bounds) ---

        #[hegel::test]
        fn u64_associativity(tc: hegel::TestCase) {
            let a = tc.draw(generators::integers::<u64>());
            let b = tc.draw(generators::integers::<u64>());
            let c = tc.draw(generators::integers::<u64>());
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn u64_left_identity(tc: hegel::TestCase) {
            let a = tc.draw(generators::integers::<u64>());
            assert_eq!(u64::empty().combine(&a), a);
        }

        #[hegel::test]
        fn u64_right_identity(tc: hegel::TestCase) {
            let a = tc.draw(generators::integers::<u64>());
            assert_eq!(a.combine(&u64::empty()), a);
        }

        // Signed integer semigroup instances removed: saturating_add is not
        // associative for signed types (hegel found counterexample with -1, 1, i64::MAX).

        // --- Any (OR semilattice) ---

        #[hegel::test]
        fn any_associativity(tc: hegel::TestCase) {
            let a = Any(tc.draw(generators::booleans()));
            let b = Any(tc.draw(generators::booleans()));
            let c = Any(tc.draw(generators::booleans()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn any_identity(tc: hegel::TestCase) {
            let a = Any(tc.draw(generators::booleans()));
            assert_eq!(Any::empty().combine(&a), a);
            assert_eq!(a.combine(&Any::empty()), a);
        }

        #[hegel::test]
        fn any_idempotence(tc: hegel::TestCase) {
            let a = Any(tc.draw(generators::booleans()));
            assert_eq!(a.combine(&a), a);
        }

        // --- All (AND semilattice) ---

        #[hegel::test]
        fn all_associativity(tc: hegel::TestCase) {
            let a = All(tc.draw(generators::booleans()));
            let b = All(tc.draw(generators::booleans()));
            let c = All(tc.draw(generators::booleans()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn all_identity(tc: hegel::TestCase) {
            let a = All(tc.draw(generators::booleans()));
            assert_eq!(All::empty().combine(&a), a);
            assert_eq!(a.combine(&All::empty()), a);
        }

        #[hegel::test]
        fn all_idempotence(tc: hegel::TestCase) {
            let a = All(tc.draw(generators::booleans()));
            assert_eq!(a.combine(&a), a);
        }

        // --- Max semigroup ---

        #[hegel::test]
        fn max_associativity(tc: hegel::TestCase) {
            let a = Max(tc.draw(generators::integers::<i64>()));
            let b = Max(tc.draw(generators::integers::<i64>()));
            let c = Max(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn max_commutativity(tc: hegel::TestCase) {
            let a = Max(tc.draw(generators::integers::<i64>()));
            let b = Max(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&b), b.combine(&a));
        }

        #[hegel::test]
        fn max_idempotence(tc: hegel::TestCase) {
            let a = Max(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&a), a);
        }

        // --- Min semigroup ---

        #[hegel::test]
        fn min_associativity(tc: hegel::TestCase) {
            let a = Min(tc.draw(generators::integers::<i64>()));
            let b = Min(tc.draw(generators::integers::<i64>()));
            let c = Min(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn min_commutativity(tc: hegel::TestCase) {
            let a = Min(tc.draw(generators::integers::<i64>()));
            let b = Min(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&b), b.combine(&a));
        }

        #[hegel::test]
        fn min_idempotence(tc: hegel::TestCase) {
            let a = Min(tc.draw(generators::integers::<i64>()));
            assert_eq!(a.combine(&a), a);
        }

        // --- String monoid ---

        #[hegel::test]
        fn string_associativity(tc: hegel::TestCase) {
            let a: String = tc.draw(generators::text().max_size(50));
            let b: String = tc.draw(generators::text().max_size(50));
            let c: String = tc.draw(generators::text().max_size(50));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn string_identity(tc: hegel::TestCase) {
            let a: String = tc.draw(generators::text());
            assert_eq!(String::empty().combine(&a), a);
            assert_eq!(a.combine(&String::empty()), a);
        }

        // --- Vec monoid ---

        #[hegel::test]
        fn vec_associativity(tc: hegel::TestCase) {
            let a: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            let b: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            let c: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn vec_identity(tc: hegel::TestCase) {
            let a: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            assert_eq!(Vec::<i32>::empty().combine(&a), a);
            assert_eq!(a.combine(&Vec::<i32>::empty()), a);
        }

        #[hegel::test]
        fn vec_combine_length_additive(tc: hegel::TestCase) {
            let a: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            let b: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
            assert_eq!(a.combine(&b).len(), a.len() + b.len());
        }

        // --- Option monoid ---

        #[hegel::test]
        fn option_associativity(tc: hegel::TestCase) {
            let a: Option<u64> = tc.draw(generators::optional(generators::integers::<u64>()));
            let b: Option<u64> = tc.draw(generators::optional(generators::integers::<u64>()));
            let c: Option<u64> = tc.draw(generators::optional(generators::integers::<u64>()));
            assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
        }

        #[hegel::test]
        fn option_identity(tc: hegel::TestCase) {
            let a: Option<u64> = tc.draw(generators::optional(generators::integers::<u64>()));
            assert_eq!(Option::<u64>::empty().combine(&a), a);
            assert_eq!(a.combine(&Option::<u64>::empty()), a);
        }

        // --- fold is monoidal ---

        #[hegel::test]
        fn fold_agrees_with_manual(tc: hegel::TestCase) {
            let items: Vec<u64> = tc.draw(generators::vecs(generators::integers::<u64>()));
            let folded: u64 = fold(items.iter().cloned());
            let manual = items.iter().fold(u64::empty(), |acc, x| acc.combine(x));
            assert_eq!(folded, manual);
        }

        #[hegel::test]
        fn fold_ref_agrees_with_fold(tc: hegel::TestCase) {
            let items: Vec<u64> = tc.draw(generators::vecs(generators::integers::<u64>()));
            assert_eq!(fold(items.iter().cloned()), fold_ref(&items));
        }

        // --- Validated laws ---

        #[hegel::test]
        fn validated_from_result_ok_is_valid(tc: hegel::TestCase) {
            let x = tc.draw(generators::integers::<i32>());
            let v: Validated<String, i32> = Validated::from_result(Ok(x));
            assert_eq!(v, Validated::Valid(x));
        }

        #[hegel::test]
        fn validated_map_preserves_valid(tc: hegel::TestCase) {
            let x = tc.draw(generators::integers::<i32>());
            let v: Validated<String, i32> = Validated::valid(x);
            assert_eq!(
                v.map(|n| n.wrapping_mul(2)),
                Validated::valid(x.wrapping_mul(2))
            );
        }

        #[hegel::test]
        fn validated_zip_accumulates_errors(tc: hegel::TestCase) {
            let e1: String = tc.draw(generators::text().min_size(1).max_size(20));
            let e2: String = tc.draw(generators::text().min_size(1).max_size(20));
            let a: Validated<String, i32> = Validated::invalid(e1.clone());
            let b: Validated<String, i32> = Validated::invalid(e2.clone());
            match a.zip(b) {
                Validated::Invalid(es) => {
                    assert_eq!(es.len(), 2);
                    assert_eq!(es[0], e1);
                    assert_eq!(es[1], e2);
                }
                _ => panic!("expected Invalid"),
            }
        }

        #[hegel::test]
        fn validated_and_then_identity(tc: hegel::TestCase) {
            let x = tc.draw(generators::integers::<i32>());
            let v: Validated<String, i32> = Validated::valid(x);
            assert_eq!(v.and_then(Validated::valid), Validated::valid(x));
        }

        // --- Saturation boundary: numeric combine at MAX ---

        #[hegel::test]
        fn u64_saturates_at_max(tc: hegel::TestCase) {
            let other = tc.draw(generators::integers::<u64>().min_value(1));
            let result = u64::MAX.combine(&other);
            assert_eq!(result, u64::MAX);
        }

        // i64 saturation test removed — signed types no longer implement Semigroup.
    }
}
