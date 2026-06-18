//! Retry policies and combinators.
//!
//! # Design
//!
//! RetryPolicy is a value type describing retry behavior.
//! `with_retry` is a combinator that applies the policy to a fallible operation.
//! Observer variants allow structured logging of retry attempts.
//!
//! # Invariant
//!
//! **`multiplier` is always finite and non-negative** (`>= 0.0`, not NaN, not
//! infinite). This is enforced by `with_multiplier` (the only public setter)
//! and upheld by all constructors (`new`, `fixed`, `exponential_backoff`,
//! `exponential_backoff_jitter`). Adding a new constructor that accepts a
//! caller-supplied multiplier **must** validate it. A `debug_assert!` in
//! `delay_for_attempt` provides defense-in-depth.
//!
//! # Laws
//!
//! - **L1 (Success-passthrough):** `with_retry(policy, || Ok(v))` returns `Ok(v)` without retrying.
//! - **L2 (Exhaustion):** After `max_retries` failures, the final error is returned.
//! - **L3 (Monotone delay):** `delay(n) ≤ delay(n+1)` when `multiplier ≥ 1.0` and jitter is off.
//! - **L4 (Delay cap):** `delay(n) ≤ max_delay` for all `n`, even with jitter.
//! - **L5 (Determinism):** Without jitter, `delay(n)` is deterministic for given policy.
//! - **L6 (Jitter positivity):** Jittered delay > 0 when base delay > 0.
//! - **L7 (Fixed-delay invariant):** `multiplier == 1.0` ⟹ `delay(n) == initial_delay`.
//! - **L8 (Observer transparency):** Observer does not affect retry behavior.
//! - **L9 (Predicate short-circuit):** If predicate returns false, retrying stops immediately.
//! - **L10 (Cancel priority):** Cancel signal during backoff stops immediately.

use std::time::Duration;

/// Retry policy configuration.
///
/// Describes how retries should be performed. This is a VALUE, not behavior.
/// Use the constructors (`new`, `fixed`, `exponential_backoff`, etc.) and
/// builder methods (`with_max_delay`, `with_jitter`, etc.) to configure.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    max_retries: u32,
    initial_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
    jitter: bool,
}

impl RetryPolicy {
    /// Create a new retry policy with exponential backoff defaults.
    pub fn new(max_retries: u32, initial_delay: Duration) -> Self {
        Self {
            max_retries,
            initial_delay,
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: false,
        }
    }

    /// No retries.
    pub fn never() -> Self {
        Self::new(0, Duration::ZERO)
    }

    /// Fixed delay between retries.
    pub fn fixed(max_retries: u32, delay: Duration) -> Self {
        Self {
            max_retries,
            initial_delay: delay,
            max_delay: delay,
            multiplier: 1.0,
            jitter: false,
        }
    }

    /// Exponential backoff.
    pub fn exponential_backoff(max_retries: u32, initial_delay: Duration) -> Self {
        Self::new(max_retries, initial_delay)
    }

    /// Exponential backoff with jitter.
    pub fn exponential_backoff_jitter(max_retries: u32, initial_delay: Duration) -> Self {
        Self {
            jitter: true,
            ..Self::new(max_retries, initial_delay)
        }
    }

    // -- Builder methods --

    /// Set the maximum delay cap.
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Override the maximum number of retry attempts.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Override the initial delay.
    pub fn with_initial_delay(mut self, initial_delay: Duration) -> Self {
        self.initial_delay = initial_delay;
        self
    }

    /// Set the backoff multiplier.
    ///
    /// # Panics
    ///
    /// Panics if `multiplier` is negative, NaN, or infinite. These values
    /// have no meaningful retry semantics and indicate a caller bug.
    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        assert!(
            multiplier.is_finite() && multiplier >= 0.0,
            "RetryPolicy multiplier must be finite and non-negative, got {multiplier}"
        );
        self.multiplier = multiplier;
        self
    }

    /// Enable or disable jitter.
    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }

    // -- Accessors --

    /// Maximum number of retry attempts (0 = no retries).
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Initial delay between retries.
    pub fn initial_delay(&self) -> Duration {
        self.initial_delay
    }

    /// Maximum delay between retries.
    pub fn max_delay(&self) -> Duration {
        self.max_delay
    }

    /// Backoff multiplier.
    pub fn multiplier(&self) -> f64 {
        self.multiplier
    }

    /// Whether jitter is enabled.
    pub fn jitter(&self) -> bool {
        self.jitter
    }

    /// Calculate the delay for a given attempt (0-indexed).
    ///
    /// Uses `self.multiplier` for backoff growth:
    /// `delay = initial_delay * multiplier^attempt`, capped at `max_delay`.
    ///
    /// When `multiplier == 1.0` (e.g., `RetryPolicy::fixed`), all attempts
    /// get the same delay. When `multiplier == 2.0` (default), classic
    /// exponential backoff.
    ///
    /// # Laws
    ///
    /// 1. **Positivity**: `capped > 0  ⟹  delay > 0` (jitter never erases a positive delay)
    /// 2. **Bounded**: `delay ≤ max_delay`
    /// 3. **Deterministic without jitter**: `¬jitter ⟹ delay_for_attempt(n) == delay_for_attempt(n)`
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        debug_assert!(
            self.multiplier.is_finite() && self.multiplier >= 0.0,
            "invariant violated: multiplier must be finite and non-negative, got {}",
            self.multiplier
        );
        let base_ms = self.initial_delay.as_millis() as f64;
        let uncapped_ms = base_ms * self.multiplier.powi(attempt as i32);
        let max_ms = self.max_delay.as_millis() as f64;
        let capped_ms = uncapped_ms.min(max_ms);
        // Clamp to u64 range (overflow to infinity for large attempts → 0)
        let capped = if capped_ms.is_finite() && capped_ms >= 0.0 {
            capped_ms as u64
        } else {
            0
        };

        if self.jitter {
            // Jitter: varied in [capped/2, capped] via hash-based entropy.
            // Not truly uniform (hash distribution + modulo bias), but
            // sufficient for decorrelating retries. RandomState::new()
            // provides fresh per-call entropy without pulling in `rand`.
            use std::collections::hash_map::RandomState;
            use std::hash::{BuildHasher, Hasher};
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(capped);
            hasher.write_u32(attempt);
            let hash = hasher.finish();
            let half = capped / 2;
            let range = capped - half + 1; // always ≥ 1 when capped ≥ 0
            let jittered = half + (hash % range);
            // Enforce positivity law: jitter must not erase a positive delay.
            let floored = if capped > 0 { jittered.max(1) } else { 0 };
            Duration::from_millis(floored)
        } else {
            Duration::from_millis(capped)
        }
    }
}

/// Retry context passed to observer callbacks.
///
/// Generic over `E` so observers can inspect the actual error type.
#[derive(Debug, Clone)]
pub struct RetryContext<E> {
    /// Current attempt number (0-indexed).
    pub attempt: u32,
    /// Maximum retries allowed.
    pub max_retries: u32,
    /// Delay before next attempt.
    pub delay: Duration,
    /// Elapsed time since first attempt.
    pub elapsed: Duration,
    /// The error that caused this retry.
    pub last_error: E,
}

impl<E> RetryContext<E> {
    /// Number of retries remaining after the current attempt.
    pub fn remaining(&self) -> u32 {
        self.max_retries.saturating_sub(self.attempt + 1)
    }
}

/// Execute an operation with retry policy.
///
/// Retries on error up to `policy.max_retries` times with configurable backoff.
pub async fn with_retry<F, Fut, T, E>(policy: &RetryPolicy, mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut last_error = None;

    for attempt in 0..=policy.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);
                if attempt < policy.max_retries {
                    let delay = policy.delay_for_attempt(attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// Execute with retry, calling a predicate to decide whether to retry.
pub async fn retry_when<F, Fut, T, E, P>(
    policy: &RetryPolicy,
    mut operation: F,
    mut should_retry: P,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: FnMut(&E) -> bool,
{
    let mut last_error = None;

    for attempt in 0..=policy.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt < policy.max_retries && should_retry(&e) {
                    let delay = policy.delay_for_attempt(attempt);
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                } else {
                    return Err(e);
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// Execute with retry and an observer callback on each retry.
///
/// The observer receives the full `RetryContext<E>` including the error.
pub async fn retry_with_observer<F, Fut, T, E, O>(
    policy: &RetryPolicy,
    mut operation: F,
    observer: O,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    O: Fn(&RetryContext<&E>),
{
    let start = std::time::Instant::now();

    for attempt in 0..=policy.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt < policy.max_retries {
                    let delay = policy.delay_for_attempt(attempt);
                    observer(&RetryContext {
                        attempt,
                        max_retries: policy.max_retries,
                        delay,
                        elapsed: start.elapsed(),
                        last_error: &e,
                    });
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}

/// Execute with retry, predicate, and observer.
///
/// Combines `retry_when` with an observer. The predicate receives `&E`,
/// the observer receives the full `RetryContext<&E>`.
pub async fn retry_when_observed<F, Fut, T, E, P, O>(
    policy: &RetryPolicy,
    mut operation: F,
    mut should_retry: P,
    observer: O,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: FnMut(&E) -> bool,
    O: Fn(&RetryContext<&E>),
{
    let start = std::time::Instant::now();

    for attempt in 0..=policy.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt < policy.max_retries && should_retry(&e) {
                    let delay = policy.delay_for_attempt(attempt);
                    observer(&RetryContext {
                        attempt,
                        max_retries: policy.max_retries,
                        delay,
                        elapsed: start.elapsed(),
                        last_error: &e,
                    });
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}

/// Execute with retry, context-aware predicate, and observer.
///
/// Differs from `retry_when_observed`: the predicate receives the full
/// `RetryContext<&E>` (not just `&E`), enabling retry decisions based on
/// attempt count, delay, elapsed time, and error together.
pub async fn retry_when_observed_hinted<F, Fut, T, E, P, O>(
    policy: &RetryPolicy,
    mut operation: F,
    mut should_retry: P,
    observer: O,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: FnMut(&RetryContext<&E>) -> bool,
    O: Fn(&RetryContext<&E>),
{
    let start = std::time::Instant::now();

    for attempt in 0..=policy.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let delay = policy.delay_for_attempt(attempt);
                let ctx = RetryContext {
                    attempt,
                    max_retries: policy.max_retries,
                    delay,
                    elapsed: start.elapsed(),
                    last_error: &e,
                };
                if attempt < policy.max_retries && should_retry(&ctx) {
                    observer(&ctx);
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}

/// Outcome of a `retry_until` invocation.
///
/// Named product type (not an anonymous tuple) so call-site field access
/// is self-documenting and positional swap bugs are impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryOutcome<T> {
    /// The value produced by the final attempt.
    pub value: T,
    /// Number of retries performed before `value` was accepted (or
    /// retries were exhausted). `0` means the first attempt was
    /// accepted with no retries. `n` means `n` retries occurred
    /// after the initial attempt, for a total of `n + 1` invocations.
    pub retry_count: u32,
}

/// Retry an operation that always produces a value (not a `Result`), retrying
/// while a predicate says the value is unacceptable.
///
/// Unlike `retry_when` which requires `Result<T, E>`, this combinator works
/// with `T` directly — the predicate `is_acceptable(&T) -> bool` decides
/// whether to stop.
///
/// Returns a [`RetryOutcome`] with the final value and the number of retries
/// performed. `retry_count == 0` means the first try was accepted.
///
/// This is the right combinator when the operation always produces a value
/// but that value may indicate a retryable failure (e.g., `SampleOutput`
/// with an `error` field).
pub async fn retry_until<F, Fut, T, P>(
    policy: &RetryPolicy,
    mut operation: F,
    mut is_acceptable: P,
) -> RetryOutcome<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
    P: FnMut(&T) -> bool,
{
    for attempt in 0..=policy.max_retries {
        let value = operation().await;
        if is_acceptable(&value) || attempt == policy.max_retries {
            return RetryOutcome {
                value,
                retry_count: attempt,
            };
        }
        let delay = policy.delay_for_attempt(attempt);
        tokio::time::sleep(delay).await;
    }

    unreachable!("loop covers 0..=max_retries inclusive")
}

// ─── Pause-aware retry combinators ───────────────────────────────────

use crate::cancellation::CancellationToken;
use crate::pause::PauseToken;
use crate::schedule::pausable_sleep;

/// Outcome of a pause-aware retry invocation.
#[derive(Debug)]
pub struct RetryPausableOutcome<T, E> {
    /// The final result.
    pub value: Result<T, E>,
    /// Total attempts performed.
    pub attempts: u32,
    /// Whether execution was cancelled.
    pub cancelled: bool,
    /// Time spent paused during backoff.
    pub paused_time: Duration,
}

/// Like `with_retry` but respects pause and cancellation during backoff.
///
/// During the backoff delay between retries:
/// - If paused, the timer freezes (paused time doesn't count)
/// - If cancelled, returns immediately with the last error
pub async fn retry_pausable<F, Fut, T, E>(
    policy: &RetryPolicy,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
) -> RetryPausableOutcome<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut last_error = None;
    let mut paused_total = Duration::ZERO;

    for attempt in 0..=policy.max_retries() {
        match operation().await {
            Ok(result) => {
                return RetryPausableOutcome {
                    value: Ok(result),
                    attempts: attempt + 1,
                    cancelled: false,
                    paused_time: paused_total,
                };
            }
            Err(e) => {
                last_error = Some(e);
                if attempt < policy.max_retries() {
                    let delay = policy.delay_for_attempt(attempt);
                    let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
                    paused_total += sleep_outcome.paused_time;
                    if !sleep_outcome.completed {
                        return RetryPausableOutcome {
                            value: Err(last_error.unwrap()),
                            attempts: attempt + 1,
                            cancelled: true,
                            paused_time: paused_total,
                        };
                    }
                }
            }
        }
    }

    RetryPausableOutcome {
        value: Err(last_error.unwrap()),
        attempts: policy.max_retries() + 1,
        cancelled: false,
        paused_time: paused_total,
    }
}

/// Like `retry_when` but with pause/cancel awareness.
pub async fn retry_when_pausable<F, Fut, T, E, P>(
    policy: &RetryPolicy,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
    mut should_retry: P,
) -> RetryPausableOutcome<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: FnMut(&E) -> bool,
{
    let mut last_error = None;
    let mut paused_total = Duration::ZERO;

    for attempt in 0..=policy.max_retries() {
        match operation().await {
            Ok(result) => {
                return RetryPausableOutcome {
                    value: Ok(result),
                    attempts: attempt + 1,
                    cancelled: false,
                    paused_time: paused_total,
                };
            }
            Err(e) => {
                if attempt < policy.max_retries() && should_retry(&e) {
                    let delay = policy.delay_for_attempt(attempt);
                    last_error = Some(e);
                    let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
                    paused_total += sleep_outcome.paused_time;
                    if !sleep_outcome.completed {
                        return RetryPausableOutcome {
                            value: Err(last_error.unwrap()),
                            attempts: attempt + 1,
                            cancelled: true,
                            paused_time: paused_total,
                        };
                    }
                } else {
                    return RetryPausableOutcome {
                        value: Err(e),
                        attempts: attempt + 1,
                        cancelled: false,
                        paused_time: paused_total,
                    };
                }
            }
        }
    }

    RetryPausableOutcome {
        value: Err(last_error.unwrap()),
        attempts: policy.max_retries() + 1,
        cancelled: false,
        paused_time: paused_total,
    }
}

/// Like `retry_until` but with pause/cancel awareness.
pub async fn retry_until_pausable<F, Fut, T, P>(
    policy: &RetryPolicy,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
    mut is_acceptable: P,
) -> RetryPausableOutcome<T, ()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
    P: FnMut(&T) -> bool,
{
    let mut paused_total = Duration::ZERO;

    for attempt in 0..=policy.max_retries() {
        let value = operation().await;
        if is_acceptable(&value) || attempt == policy.max_retries() {
            return RetryPausableOutcome {
                value: Ok(value),
                attempts: attempt + 1,
                cancelled: false,
                paused_time: paused_total,
            };
        }
        let delay = policy.delay_for_attempt(attempt);
        let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
        paused_total += sleep_outcome.paused_time;
        if !sleep_outcome.completed {
            return RetryPausableOutcome {
                value: Ok(value),
                attempts: attempt + 1,
                cancelled: true,
                paused_time: paused_total,
            };
        }
    }

    unreachable!("loop covers 0..=max_retries inclusive")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn with_retry_succeeds_first_try() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let result: Result<i32, &str> = with_retry(&policy, || async { Ok(42) }).await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn with_retry_succeeds_after_failures() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let result: Result<i32, &str> = with_retry(&policy, move || {
            let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err("not yet")
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(attempt.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_exhausts_retries() {
        let policy = RetryPolicy::fixed(2, Duration::from_millis(1));
        let result: Result<i32, &str> = with_retry(&policy, || async { Err("always fails") }).await;
        assert_eq!(result, Err("always fails"));
    }

    #[test]
    fn delay_for_attempt_exponential() {
        let policy = RetryPolicy::exponential_backoff(5, Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(0).as_millis(), 100);
        assert_eq!(policy.delay_for_attempt(1).as_millis(), 200);
        assert_eq!(policy.delay_for_attempt(2).as_millis(), 400);
    }

    #[test]
    fn delay_capped_at_max() {
        let policy = RetryPolicy::exponential_backoff(10, Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(10));
        // 2^10 = 1024 seconds > 10 second cap
        assert_eq!(policy.delay_for_attempt(10), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn retry_with_observer_calls_observer() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let observer_calls = Arc::new(AtomicU32::new(0));
        let observer_calls_clone = observer_calls.clone();
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let result: Result<i32, String> = retry_with_observer(
            &policy,
            move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        Err(format!("fail {}", n))
                    } else {
                        Ok(42)
                    }
                }
            },
            move |ctx| {
                observer_calls_clone.fetch_add(1, Ordering::SeqCst);
                assert!(ctx.attempt < ctx.max_retries);
            },
        )
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(observer_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn delay_for_attempt_no_jitter_is_deterministic() {
        let policy = RetryPolicy::exponential_backoff(5, Duration::from_millis(100));
        // Without jitter, same attempt always yields the same delay
        for attempt in 0..5 {
            let d1 = policy.delay_for_attempt(attempt);
            let d2 = policy.delay_for_attempt(attempt);
            assert_eq!(
                d1, d2,
                "attempt {} should be deterministic without jitter",
                attempt
            );
        }
        // Verify specific expected values (100, 200, 400, 800, 1600)
        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
        assert_eq!(policy.delay_for_attempt(4), Duration::from_millis(1600));
    }

    #[test]
    fn delay_for_attempt_jitter_in_expected_range() {
        let policy = RetryPolicy::exponential_backoff_jitter(10, Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(30));

        for attempt in 0..8 {
            let delay = policy.delay_for_attempt(attempt);
            // multiplier is 2.0 for exponential_backoff_jitter
            let uncapped = (100.0_f64 * 2.0_f64.powi(attempt as i32)) as u64;
            let capped = uncapped.min(30_000);
            let half = capped / 2;

            assert!(
                delay.as_millis() as u64 >= half,
                "attempt {}: delay {}ms should be >= half {}ms",
                attempt,
                delay.as_millis(),
                half
            );
            assert!(
                delay.as_millis() as u64 <= capped,
                "attempt {}: delay {}ms should be <= capped {}ms",
                attempt,
                delay.as_millis(),
                capped
            );
        }
    }

    #[test]
    fn delay_for_attempt_jitter_never_zero_never_exceeds_max() {
        let max = Duration::from_secs(10);
        let policy = RetryPolicy::exponential_backoff_jitter(10, Duration::from_millis(50))
            .with_max_delay(max);

        for attempt in 0..10 {
            let delay = policy.delay_for_attempt(attempt);
            assert!(
                delay > Duration::ZERO,
                "attempt {}: jittered delay should never be zero",
                attempt
            );
            assert!(
                delay <= max,
                "attempt {}: jittered delay {}ms should not exceed max_delay {}ms",
                attempt,
                delay.as_millis(),
                max.as_millis()
            );
        }
    }

    #[test]
    fn fixed_delay_is_constant() {
        let policy = RetryPolicy::fixed(5, Duration::from_millis(100));
        // multiplier=1.0 means every attempt gets the same delay
        for attempt in 0..5 {
            assert_eq!(
                policy.delay_for_attempt(attempt),
                Duration::from_millis(100),
                "fixed policy: attempt {} should be 100ms",
                attempt,
            );
        }
    }

    #[test]
    fn custom_multiplier_is_honored() {
        let policy = RetryPolicy::new(5, Duration::from_millis(100))
            .with_multiplier(1.5)
            .with_max_delay(Duration::from_secs(60));
        // 100 * 1.5^0 = 100, 100 * 1.5^1 = 150, 100 * 1.5^2 = 225, 100 * 1.5^3 = 337
        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(150));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(225));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(337));
    }

    #[test]
    fn multiplier_one_is_flat() {
        let policy = RetryPolicy::new(5, Duration::from_millis(200))
            .with_multiplier(1.0)
            .with_max_delay(Duration::from_secs(60));
        for attempt in 0..5 {
            assert_eq!(
                policy.delay_for_attempt(attempt),
                Duration::from_millis(200),
                "multiplier=1.0: attempt {} should be 200ms",
                attempt,
            );
        }
    }

    #[test]
    #[should_panic(expected = "finite and non-negative")]
    fn multiplier_rejects_nan() {
        RetryPolicy::new(3, Duration::from_millis(100)).with_multiplier(f64::NAN);
    }

    #[test]
    #[should_panic(expected = "finite and non-negative")]
    fn multiplier_rejects_negative() {
        RetryPolicy::new(3, Duration::from_millis(100)).with_multiplier(-1.0);
    }

    #[test]
    #[should_panic(expected = "finite and non-negative")]
    fn multiplier_rejects_infinity() {
        RetryPolicy::new(3, Duration::from_millis(100)).with_multiplier(f64::INFINITY);
    }

    #[test]
    fn multiplier_zero_is_valid() {
        // Multiplier 0: attempt 0 = initial * 0^0 = initial * 1 = initial,
        // subsequent = initial * 0^n = 0. Degenerate but legal.
        let policy = RetryPolicy::new(3, Duration::from_millis(100)).with_multiplier(0.0);
        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(0));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(0));
    }

    /// Law: capped > 0 ⟹ jittered delay > 0.
    /// This is the edge case that breaks when capped=1: half=0, hash%1=0 → 0ms.
    #[test]
    fn jitter_positivity_law_small_delays() {
        // 1ms initial delay — capped=1 on attempt 0, the degenerate case
        let policy = RetryPolicy::exponential_backoff_jitter(5, Duration::from_millis(1));
        for _ in 0..50 {
            let delay = policy.delay_for_attempt(0);
            assert!(
                delay >= Duration::from_millis(1),
                "positivity law violated: 1ms base produced {}ms",
                delay.as_millis()
            );
        }
    }

    /// Law: delay ≤ max_delay for all attempts, even with jitter.
    #[test]
    fn jitter_bounded_law() {
        let max = Duration::from_millis(500);
        let policy = RetryPolicy::exponential_backoff_jitter(10, Duration::from_millis(100))
            .with_max_delay(max);
        for attempt in 0..10 {
            for _ in 0..20 {
                let delay = policy.delay_for_attempt(attempt);
                assert!(
                    delay <= max,
                    "bounded law violated: attempt {} produced {}ms > {}ms",
                    attempt,
                    delay.as_millis(),
                    max.as_millis()
                );
            }
        }
    }

    /// Verify jitter produces more than one distinct value (non-degeneracy).
    #[test]
    fn jitter_non_degeneracy() {
        let policy = RetryPolicy::exponential_backoff_jitter(5, Duration::from_millis(100));
        let delays: std::collections::HashSet<u128> = (0..50)
            .map(|_| policy.delay_for_attempt(3).as_millis())
            .collect();
        assert!(
            delays.len() > 1,
            "jitter should produce varied values, got only {:?}",
            delays
        );
    }

    #[test]
    fn value_style_builder_overrides_work() {
        let policy = RetryPolicy::new(3, Duration::from_millis(100))
            .with_max_retries(5)
            .with_initial_delay(Duration::from_millis(250));

        assert_eq!(policy.max_retries(), 5);
        assert_eq!(policy.initial_delay(), Duration::from_millis(250));
    }

    #[test]
    fn retry_context_remaining_is_saturating() {
        let ctx = RetryContext {
            attempt: 1,
            max_retries: 5,
            delay: Duration::from_millis(10),
            elapsed: Duration::from_millis(20),
            last_error: "boom",
        };
        assert_eq!(ctx.remaining(), 3);
    }

    #[tokio::test]
    async fn retry_when_observed_hinted_uses_context() {
        let policy = RetryPolicy::fixed(5, Duration::from_millis(1));
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        // Only retry when attempt < 2 via the predicate
        let result: Result<i32, String> = retry_when_observed_hinted(
            &policy,
            move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move { Err(format!("fail {}", n)) }
            },
            |ctx| ctx.attempt < 2,
            |_ctx| {},
        )
        .await;

        assert!(result.is_err());
        // Should have tried 3 times: attempt 0, 1, 2 (stopped retrying at attempt 2)
        assert_eq!(attempt.load(Ordering::SeqCst), 3);
    }

    // -- retry_until tests --

    #[tokio::test]
    async fn retry_until_accepts_first_try() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let outcome = retry_until(&policy, || async { 42 }, |v| *v == 42).await;
        assert_eq!(outcome.value, 42);
        assert_eq!(outcome.retry_count, 0);
    }

    #[tokio::test]
    async fn retry_until_retries_until_acceptable() {
        let policy = RetryPolicy::fixed(5, Duration::from_millis(1));
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let outcome = retry_until(
            &policy,
            move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move { n }
            },
            |v| *v >= 3,
        )
        .await;

        assert_eq!(outcome.value, 3);
        assert_eq!(outcome.retry_count, 3);
    }

    #[tokio::test]
    async fn retry_until_returns_last_on_exhaustion() {
        let policy = RetryPolicy::fixed(2, Duration::from_millis(1));
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let outcome = retry_until(
            &policy,
            move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move { n }
            },
            |_| false, // never acceptable
        )
        .await;

        // Should have done 3 attempts (0, 1, 2) and returned the last value
        assert_eq!(outcome.value, 2);
        assert_eq!(outcome.retry_count, 2); // max_retries=2
    }

    // ── retry_pausable tests ──

    #[tokio::test]
    async fn retry_pausable_succeeds_first_try() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let outcome: RetryPausableOutcome<i32, &str> =
            retry_pausable(&policy, &cancel, &pause, || async { Ok(42) }).await;
        assert_eq!(outcome.value, Ok(42));
        assert_eq!(outcome.attempts, 1);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn retry_pausable_succeeds_after_failures() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let outcome: RetryPausableOutcome<i32, &str> =
            retry_pausable(&policy, &cancel, &pause, move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        Err("not yet")
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(outcome.value, Ok(42));
        assert_eq!(outcome.attempts, 3);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn retry_pausable_cancelled_during_backoff() {
        let policy = RetryPolicy::fixed(3, Duration::from_secs(10));
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();

        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let outcome: RetryPausableOutcome<i32, &str> =
            retry_pausable(&policy, &cancel, &pause, || async { Err("fail") }).await;

        assert!(outcome.value.is_err());
        assert!(outcome.cancelled);
    }

    #[tokio::test]
    async fn retry_when_pausable_respects_predicate() {
        let policy = RetryPolicy::fixed(5, Duration::from_millis(1));
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let outcome: RetryPausableOutcome<i32, String> = retry_when_pausable(
            &policy,
            &cancel,
            &pause,
            move || {
                let n = attempt_clone.fetch_add(1, Ordering::SeqCst);
                async move { Err(format!("fail {n}")) }
            },
            |e| e != "fail 2",
        )
        .await;

        assert!(outcome.value.is_err());
        assert_eq!(outcome.attempts, 3);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn retry_until_pausable_accepts_first_try() {
        let policy = RetryPolicy::fixed(3, Duration::from_millis(1));
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();

        let outcome =
            retry_until_pausable(&policy, &cancel, &pause, || async { 42 }, |v| *v == 42).await;

        assert_eq!(outcome.value, Ok(42));
        assert_eq!(outcome.attempts, 1);
        assert!(!outcome.cancelled);
    }
}
