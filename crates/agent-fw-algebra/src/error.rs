//! Error types and the Either coproduct.
//!
//! - `Either<L, R>` — Type-safe coproduct for error handling
//! - `AgentError` — Framework-level error type
//! - `ResultExt` — Extension methods for Result
//!
//! # Laws
//!
//! ## Either
//! - **L1 (Fold totality):** `fold(f, g)` handles both `Left` and `Right`.
//! - **L2 (Swap involution):** `swap().swap() == identity`.
//!
//! ## ErrorAccumulator (Monoid)
//! - **L3 (Left identity):** `combine(empty, a) == a`.
//! - **L4 (Right identity):** `combine(a, empty) == a`.
//! - **L5 (Associativity):** `combine(combine(a, b), c) == combine(a, combine(b, c))`.
//! - **L6 (Into-result):** `empty.into_result(v) == Ok(v)`; `non_empty.into_result(v) == Err(errors)`.
//!
//! ## ResultExt
//! - **L7 (Recover totality):** `recover(f)` always produces `Ok`.
//! - **L8 (Tap transparency):** `tap_error(f)` returns same result.

use std::fmt;

/// Type-safe coproduct (sum type) for two possible values.
///
/// Used for composing error types without losing type information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    /// Construct the left variant.
    pub fn left(value: L) -> Self {
        Self::Left(value)
    }

    /// Construct the right variant.
    pub fn right(value: R) -> Self {
        Self::Right(value)
    }

    /// Fold both sides into a single value.
    pub fn fold<T>(self, left: impl FnOnce(L) -> T, right: impl FnOnce(R) -> T) -> T {
        match self {
            Either::Left(l) => left(l),
            Either::Right(r) => right(r),
        }
    }

    /// Map over the left side.
    pub fn map_left<T>(self, f: impl FnOnce(L) -> T) -> Either<T, R> {
        match self {
            Either::Left(l) => Either::Left(f(l)),
            Either::Right(r) => Either::Right(r),
        }
    }

    /// Map over the right side.
    pub fn map_right<T>(self, f: impl FnOnce(R) -> T) -> Either<L, T> {
        match self {
            Either::Left(l) => Either::Left(l),
            Either::Right(r) => Either::Right(f(r)),
        }
    }

    /// Swap left and right.
    pub fn swap(self) -> Either<R, L> {
        match self {
            Either::Left(l) => Either::Right(l),
            Either::Right(r) => Either::Left(r),
        }
    }

    /// Check if this is the Left variant.
    pub fn is_left(&self) -> bool {
        matches!(self, Either::Left(_))
    }

    /// Check if this is the Right variant.
    pub fn is_right(&self) -> bool {
        matches!(self, Either::Right(_))
    }

    /// Borrow the left value, if present.
    pub fn left_value(&self) -> Option<&L> {
        match self {
            Either::Left(l) => Some(l),
            Either::Right(_) => None,
        }
    }

    /// Borrow the right value, if present.
    pub fn right_value(&self) -> Option<&R> {
        match self {
            Either::Left(_) => None,
            Either::Right(r) => Some(r),
        }
    }

    /// Map both branches at once.
    pub fn bimap<L2, R2>(
        self,
        left: impl FnOnce(L) -> L2,
        right: impl FnOnce(R) -> R2,
    ) -> Either<L2, R2> {
        match self {
            Either::Left(l) => Either::Left(left(l)),
            Either::Right(r) => Either::Right(right(r)),
        }
    }

    /// Extract the left value, panicking if this is `Right`.
    pub fn unwrap_left(self) -> L {
        match self {
            Either::Left(l) => l,
            Either::Right(_) => panic!("called unwrap_left on Right value"),
        }
    }

    /// Extract the right value, panicking if this is `Left`.
    pub fn unwrap_right(self) -> R {
        match self {
            Either::Left(_) => panic!("called unwrap_right on Left value"),
            Either::Right(r) => r,
        }
    }
}

impl<L: fmt::Display, R: fmt::Display> fmt::Display for Either<L, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Either::Left(l) => write!(f, "Left({})", l),
            Either::Right(r) => write!(f, "Right({})", r),
        }
    }
}

/// Framework-level error type.
///
/// Covers all error categories that can occur during agent execution.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("sub-agent error: {0}")]
    SubAgent(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("cancelled")]
    Cancelled,
    #[error("timeout after {0}ms")]
    Timeout(u64),
    #[error("IO error: {0}")]
    Io(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AgentError {
    /// Structured constructor for sub-agent failures.
    pub fn sub_agent(agent_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::SubAgent(format!("{}: {}", agent_name.into(), message.into()))
    }

    /// Structured constructor for tool failures.
    pub fn tool(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Tool(format!("{}: {}", tool_name.into(), message.into()))
    }

    /// Structured constructor for timeouts.
    pub fn timeout(duration: std::time::Duration) -> Self {
        Self::Timeout(duration.as_millis() as u64)
    }

    /// Structured constructor for IO failures.
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }

    /// Structured constructor for database failures.
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database(message.into())
    }

    /// Structured constructor for validation failures.
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation(format!("{}: {}", field.into(), message.into()))
    }

    /// Structured constructor for internal failures.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Whether retrying this error class is usually sensible.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout(_) | Self::Io(_) | Self::Database(_))
    }

    /// Whether this is an explicit cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

/// Extension methods for Result types.
pub trait ResultExt<T, E> {
    /// Map the error type.
    fn map_error<F: FnOnce(E) -> E2, E2>(self, f: F) -> Result<T, E2>;

    /// Tap the error (observe without modifying).
    fn tap_error<F: FnOnce(&E)>(self, f: F) -> Result<T, E>;

    /// Recover from an error by producing a fallback value.
    ///
    /// Analogous to Scala's `recover` / Haskell's `catchError` when
    /// the handler is total.
    fn recover<F: FnOnce(E) -> T>(self, f: F) -> T;

    /// Catch an error and attempt recovery, potentially producing a
    /// new error type.
    ///
    /// ```ignore
    /// let result: Result<i32, AppError> = fallible_op()
    ///     .catch(|e| match e {
    ///         AppError::Transient(msg) => Ok(default_value),
    ///         other => Err(other),
    ///     });
    /// ```
    fn catch<F, E2>(self, handler: F) -> Result<T, E2>
    where
        F: FnOnce(E) -> Result<T, E2>;

    /// Recover from specific errors that match a predicate, leaving
    /// non-matching errors untouched.
    fn recover_if<P, F>(self, predicate: P, fallback: F) -> Result<T, E>
    where
        P: FnOnce(&E) -> bool,
        F: FnOnce(E) -> T;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn map_error<F: FnOnce(E) -> E2, E2>(self, f: F) -> Result<T, E2> {
        self.map_err(f)
    }

    fn tap_error<F: FnOnce(&E)>(self, f: F) -> Result<T, E> {
        if let Err(ref e) = self {
            f(e);
        }
        self
    }

    fn recover<F: FnOnce(E) -> T>(self, f: F) -> T {
        match self {
            Ok(v) => v,
            Err(e) => f(e),
        }
    }

    fn catch<F, E2>(self, handler: F) -> Result<T, E2>
    where
        F: FnOnce(E) -> Result<T, E2>,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => handler(e),
        }
    }

    fn recover_if<P, F>(self, predicate: P, fallback: F) -> Result<T, E>
    where
        P: FnOnce(&E) -> bool,
        F: FnOnce(E) -> T,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                if predicate(&e) {
                    Ok(fallback(e))
                } else {
                    Err(e)
                }
            }
        }
    }
}

/// Accumulator for collecting errors from parallel/batch operations.
///
/// Useful when multiple operations can fail independently and you want
/// to collect all errors rather than short-circuiting on the first.
pub struct ErrorAccumulator<E> {
    errors: Vec<E>,
}

impl<E> ErrorAccumulator<E> {
    /// Create an empty accumulator.
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Create an accumulator containing one error.
    pub fn single(error: E) -> Self {
        Self {
            errors: vec![error],
        }
    }

    /// Create an accumulator from an existing vector.
    pub fn from_vec(errors: Vec<E>) -> Self {
        Self { errors }
    }

    /// Create an accumulator only if the vector is non-empty.
    pub fn try_from_vec(errors: Vec<E>) -> Option<Self> {
        if errors.is_empty() {
            None
        } else {
            Some(Self { errors })
        }
    }

    /// Push an error into the accumulator.
    pub fn push(&mut self, error: E) {
        self.errors.push(error);
    }

    /// Returns true if no errors have been accumulated.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Number of accumulated errors.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Borrow the first accumulated error.
    ///
    /// Panics if the accumulator is empty.
    pub fn first(&self) -> &E {
        self.errors
            .first()
            .expect("ErrorAccumulator::first called on empty accumulator")
    }

    /// Borrow all accumulated errors.
    pub fn errors(&self) -> Vec<&E> {
        self.errors.iter().collect()
    }

    /// Combine two accumulators by concatenating their error lists.
    pub fn combine(mut self, mut other: Self) -> Self {
        self.errors.append(&mut other.errors);
        self
    }

    /// Consume into the inner error vector.
    pub fn into_errors(self) -> Vec<E> {
        self.errors
    }

    /// Alias for `into_errors`.
    pub fn into_vec(self) -> Vec<E> {
        self.errors
    }

    /// Map accumulated errors to a new type.
    pub fn map<E2>(self, f: impl Fn(E) -> E2) -> ErrorAccumulator<E2> {
        ErrorAccumulator {
            errors: self.errors.into_iter().map(f).collect(),
        }
    }

    /// If no errors, return `Ok(ok)`. Otherwise return `Err(errors)`.
    pub fn into_result<T>(self, ok: T) -> Result<T, Vec<E>> {
        if self.errors.is_empty() {
            Ok(ok)
        } else {
            Err(self.errors)
        }
    }
}

impl<E> Default for ErrorAccumulator<E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Either tests --

    #[test]
    fn either_fold() {
        let left: Either<i32, &str> = Either::Left(42);
        assert_eq!(left.fold(|l| l * 2, |_r| 0), 84);

        let right: Either<i32, &str> = Either::Right("hello");
        assert_eq!(right.fold(|_l| 0, |r| r.len() as i32), 5);
    }

    #[test]
    fn either_map_left_right() {
        let e: Either<i32, i32> = Either::Left(10);
        assert_eq!(e.map_left(|x| x + 1), Either::Left(11));

        let e: Either<i32, i32> = Either::Right(20);
        assert_eq!(e.map_right(|x| x + 1), Either::Right(21));
    }

    #[test]
    fn either_swap() {
        let e: Either<i32, &str> = Either::Left(1);
        assert_eq!(e.swap(), Either::Right(1));
    }

    #[test]
    fn either_extra_helpers() {
        let left: Either<i32, &str> = Either::left(7);
        assert_eq!(left.left_value(), Some(&7));
        assert_eq!(left.right_value(), None);
        assert_eq!(left.unwrap_left(), 7);

        let right: Either<i32, &str> = Either::right("x");
        assert_eq!(right.left_value(), None);
        assert_eq!(right.right_value(), Some(&"x"));
        assert_eq!(right.unwrap_right(), "x");

        let mapped = Either::left(4).bimap(|n| n * 2, |s: &str| s.len());
        assert_eq!(mapped, Either::Left(8));
    }

    // -- ResultExt tests --

    #[test]
    fn recover_converts_err_to_ok() {
        let result: Result<i32, &str> = Err("fail");
        assert_eq!(result.recover(|_| 42), 42);
    }

    #[test]
    fn recover_preserves_ok() {
        let result: Result<i32, &str> = Ok(10);
        assert_eq!(result.recover(|_| 42), 10);
    }

    #[test]
    fn catch_transforms_error() {
        let result: Result<i32, &str> = Err("transient");
        let caught: Result<i32, String> = result.catch(|e| {
            if e == "transient" {
                Ok(0)
            } else {
                Err(e.to_string())
            }
        });
        assert_eq!(caught, Ok(0));
    }

    #[test]
    fn catch_propagates_unhandled() {
        let result: Result<i32, &str> = Err("fatal");
        let caught: Result<i32, String> = result.catch(|e| {
            if e == "transient" {
                Ok(0)
            } else {
                Err(format!("unrecoverable: {e}"))
            }
        });
        assert_eq!(caught, Err("unrecoverable: fatal".to_string()));
    }

    #[test]
    fn catch_preserves_ok() {
        let result: Result<i32, &str> = Ok(10);
        let caught: Result<i32, String> = result.catch(|_| unreachable!());
        assert_eq!(caught, Ok(10));
    }

    #[test]
    fn recover_if_matches() {
        let result: Result<i32, &str> = Err("transient");
        let recovered = result.recover_if(|e| *e == "transient", |_| 0);
        assert_eq!(recovered, Ok(0));
    }

    #[test]
    fn recover_if_no_match_preserves_error() {
        let result: Result<i32, &str> = Err("fatal");
        let recovered = result.recover_if(|e| *e == "transient", |_| 0);
        assert_eq!(recovered, Err("fatal"));
    }

    #[test]
    fn tap_error_observes_without_modification() {
        let mut observed = false;
        let result: Result<i32, &str> = Err("oops");
        let same = result.tap_error(|_| observed = true);
        assert!(observed);
        assert_eq!(same, Err("oops"));
    }

    // -- ErrorAccumulator tests --

    #[test]
    fn accumulator_collects_and_converts() {
        let mut acc = ErrorAccumulator::new();
        assert!(acc.is_empty());

        acc.push("error1");
        acc.push("error2");
        assert_eq!(acc.len(), 2);

        let result: Result<(), Vec<&str>> = acc.into_result(());
        assert_eq!(result, Err(vec!["error1", "error2"]));
    }

    #[test]
    fn accumulator_empty_is_ok() {
        let acc: ErrorAccumulator<String> = ErrorAccumulator::new();
        let result = acc.into_result(42);
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn accumulator_extra_helpers() {
        let acc = ErrorAccumulator::single("a");
        assert_eq!(acc.first(), &"a");
        assert_eq!(acc.errors(), vec![&"a"]);

        let combined = ErrorAccumulator::single("a").combine(ErrorAccumulator::single("b"));
        assert_eq!(combined.into_vec(), vec!["a", "b"]);

        assert!(ErrorAccumulator::<&str>::try_from_vec(vec![]).is_none());
        assert_eq!(
            ErrorAccumulator::try_from_vec(vec![1, 2])
                .unwrap()
                .map(|n| n * 2)
                .into_vec(),
            vec![2, 4]
        );
    }

    #[test]
    fn agent_error_helpers() {
        assert!(AgentError::timeout(std::time::Duration::from_secs(1)).is_retryable());
        assert!(AgentError::io("reset").is_retryable());
        assert!(!AgentError::validation("field", "bad").is_retryable());
        assert!(AgentError::Cancelled.is_cancelled());
        assert!(!AgentError::sub_agent("planner", "timeout").is_cancelled());
    }
}
