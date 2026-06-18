//! Timeout combinators for async operations.
//!
//! # Laws
//!
//! - **L1 (Completion):** If future completes before deadline, result is returned.
//! - **L2 (Expiry):** If future exceeds duration, `TimeoutError` is returned.
//! - **L3 (Fallback):** `timeout_or(d, default, f)` returns `default` on timeout.
//! - **L4 (Cancel priority):** Cancel signal takes precedence over timeout.
//! - **L5 (Outcome distinguished):** `TimeoutOutcome` separates completed/cancelled/timed-out.
//! - **L6 (Config unbounded):** `TimeoutConfig::none()` never times out.

use std::future::Future;
use std::time::Duration;

use crate::cancellation::CancellationToken;

/// Timeout error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation timed out after {0}ms")]
pub struct TimeoutError(pub u64);

/// Outcome of a timeout-aware operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeoutOutcome<T> {
    /// The operation completed within the deadline.
    Completed(T),
    /// The operation was cancelled cooperatively before completion.
    Cancelled,
    /// The operation did not complete in time.
    TimedOut(Duration),
}

impl<T> TimeoutOutcome<T> {
    /// Convert to `Option` (`Some` if completed, `None` otherwise).
    pub fn ok(self) -> Option<T> {
        match self {
            Self::Completed(v) => Some(v),
            Self::Cancelled | Self::TimedOut(_) => None,
        }
    }

    /// Map the inner value.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> TimeoutOutcome<U> {
        match self {
            Self::Completed(v) => TimeoutOutcome::Completed(f(v)),
            Self::Cancelled => TimeoutOutcome::Cancelled,
            Self::TimedOut(duration) => TimeoutOutcome::TimedOut(duration),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed(_))
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn is_timed_out(&self) -> bool {
        matches!(self, Self::TimedOut(_))
    }
}

/// Run a future with a timeout.
///
/// Returns `Err(TimeoutError)` if the future does not complete within the duration.
pub async fn with_timeout<F, T>(duration: Duration, f: F) -> Result<T, TimeoutError>
where
    F: Future<Output = T>,
{
    match tokio::time::timeout(duration, f).await {
        Ok(result) => Ok(result),
        Err(_) => Err(TimeoutError(duration.as_millis() as u64)),
    }
}

/// Run a future with a timeout, returning a default on timeout.
pub async fn timeout_or<F, T>(duration: Duration, default: T, f: F) -> T
where
    F: Future<Output = T>,
{
    match tokio::time::timeout(duration, f).await {
        Ok(result) => result,
        Err(_) => default,
    }
}

/// Run a future with a timeout, applying a mapping function on success.
pub async fn timeout_map<F, T, U>(
    duration: Duration,
    f: F,
    map: impl FnOnce(T) -> U,
) -> Result<U, TimeoutError>
where
    F: Future<Output = T>,
{
    match tokio::time::timeout(duration, f).await {
        Ok(result) => Ok(map(result)),
        Err(_) => Err(TimeoutError(duration.as_millis() as u64)),
    }
}

/// Run a future with a timeout, racing against a cancellation token.
pub async fn timeout_or_cancel<F, T>(
    duration: Duration,
    f: F,
    cancel: &CancellationToken,
) -> TimeoutOutcome<T>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => TimeoutOutcome::Cancelled,
        result = tokio::time::timeout(duration, f) => {
            match result {
                Ok(v) => TimeoutOutcome::Completed(v),
                Err(_) => TimeoutOutcome::TimedOut(duration),
            }
        }
    }
}

/// Run a future with a timeout and cancellation, returning `Some` only on success.
pub async fn timeout_cancellable<F, T>(
    duration: Duration,
    f: F,
    cancel: &CancellationToken,
) -> Option<T>
where
    F: Future<Output = T>,
{
    timeout_or_cancel(duration, f, cancel).await.ok()
}

/// Timeout configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutConfig {
    duration: Option<Duration>,
}

impl TimeoutConfig {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration: Some(duration),
        }
    }

    pub fn none() -> Self {
        Self { duration: None }
    }

    pub fn from_millis(ms: u64) -> Self {
        Self::new(Duration::from_millis(ms))
    }

    pub fn from_secs(secs: u64) -> Self {
        Self::new(Duration::from_secs(secs))
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn is_some(&self) -> bool {
        self.duration.is_some()
    }

    pub fn is_none(&self) -> bool {
        self.duration.is_none()
    }

    pub async fn apply<A, F>(&self, f: F) -> Result<A, TimeoutError>
    where
        F: Future<Output = A>,
    {
        match self.duration {
            Some(duration) => with_timeout(duration, f).await,
            None => Ok(f.await),
        }
    }

    pub async fn apply_cancellable<A, F>(
        &self,
        f: F,
        cancel: &CancellationToken,
    ) -> TimeoutOutcome<A>
    where
        F: Future<Output = A>,
    {
        match self.duration {
            Some(duration) => timeout_or_cancel(duration, f, cancel).await,
            None => {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => TimeoutOutcome::Cancelled,
                    result = f => TimeoutOutcome::Completed(result),
                }
            }
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self::none()
    }
}

impl From<Duration> for TimeoutConfig {
    fn from(duration: Duration) -> Self {
        Self::new(duration)
    }
}

impl From<Option<Duration>> for TimeoutConfig {
    fn from(duration: Option<Duration>) -> Self {
        Self { duration }
    }
}

/// Extension trait for futures with timeout support.
pub trait TimeoutExt: Future + Sized {
    /// Add a timeout to this future.
    fn with_timeout(
        self,
        duration: Duration,
    ) -> impl Future<Output = Result<Self::Output, TimeoutError>> {
        with_timeout(duration, self)
    }

    /// Add a timeout and cooperative cancellation to this future.
    fn with_timeout_cancel<'a>(
        self,
        duration: Duration,
        cancel: &'a CancellationToken,
    ) -> impl Future<Output = TimeoutOutcome<Self::Output>> + 'a
    where
        Self: Send + 'a,
    {
        timeout_or_cancel(duration, self, cancel)
    }
}

impl<F: Future> TimeoutExt for F {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_completes() {
        let result = with_timeout(Duration::from_secs(1), async { 42 }).await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn timeout_expires() {
        let result = with_timeout(Duration::from_millis(1), async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            42
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn timeout_or_uses_default() {
        let result = timeout_or(Duration::from_millis(1), 99, async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            42
        })
        .await;
        assert_eq!(result, 99);
    }

    #[tokio::test]
    async fn timeout_map_applies_fn() {
        let result = timeout_map(Duration::from_secs(1), async { 21 }, |x| x * 2).await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn timeout_or_cancel_completes() {
        let cancel = CancellationToken::new();
        let result = timeout_or_cancel(Duration::from_secs(1), async { 42 }, &cancel).await;
        assert_eq!(result, TimeoutOutcome::Completed(42));
    }

    #[tokio::test]
    async fn timeout_or_cancel_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = timeout_or_cancel(
            Duration::from_secs(10),
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                42
            },
            &cancel,
        )
        .await;
        assert_eq!(result, TimeoutOutcome::Cancelled);
    }

    #[tokio::test]
    async fn timeout_outcome_ok() {
        let outcome: TimeoutOutcome<i32> = TimeoutOutcome::Completed(42);
        assert_eq!(outcome.ok(), Some(42));
    }

    #[tokio::test]
    async fn timeout_outcome_timed_out() {
        let outcome: TimeoutOutcome<i32> = TimeoutOutcome::TimedOut(Duration::from_secs(1));
        assert_eq!(outcome.ok(), None);
    }

    #[tokio::test]
    async fn timeout_cancellable_completes() {
        let cancel = CancellationToken::new();
        let result = timeout_cancellable(Duration::from_secs(1), async { 42 }, &cancel).await;
        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    async fn timeout_cancellable_times_out() {
        let result = timeout_cancellable(
            Duration::from_millis(1),
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                42
            },
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn timeout_config_none_does_not_time_out() {
        let config = TimeoutConfig::none();
        let result = config
            .apply(async {
                tokio::time::sleep(Duration::from_millis(5)).await;
                42
            })
            .await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn timeout_ext_with_timeout_cancel_returns_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = async { 42 }
            .with_timeout_cancel(Duration::from_secs(1), &cancel)
            .await;
        assert_eq!(result, TimeoutOutcome::Cancelled);
    }
}
