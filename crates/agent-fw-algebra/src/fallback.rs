//! Fallback combinator — run primary, on error run fallback with provenance tracking.
//!
//! # Laws
//!
//! - **L1 (Success passthrough)**: If primary succeeds, fallback is never called
//! - **L2 (Fallback on error)**: If primary fails, fallback is called with the error
//! - **L3 (Source tracking)**: Result carries provenance (`Primary` | `Fallback`)
//! - **L4 (Both errors preserved)**: If both fail, both errors preserved in [`FallbackError`]

use std::future::Future;

/// Provenance of a fallback result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackSource {
    /// The primary operation succeeded.
    Primary,
    /// The primary failed; the fallback produced this result.
    Fallback,
}

/// Error from a fallback combinator when both primary and fallback fail.
///
/// Preserves both errors so callers can inspect the full failure chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackError<E> {
    /// The error from the primary operation.
    pub primary: E,
    /// The error from the fallback operation.
    pub fallback: E,
}

impl<E: std::fmt::Display> std::fmt::Display for FallbackError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "both primary and fallback failed: primary={}, fallback={}",
            self.primary, self.fallback
        )
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for FallbackError<E> {}

/// Run `primary`; on error, pass the error to `fallback` and run it.
///
/// Returns the value along with which path produced it.
///
/// # Laws
///
/// - **L1**: If `primary` returns `Ok(t)`, result is `Ok((t, Primary))` and
///   `fallback` is never invoked.
/// - **L2**: If `primary` returns `Err(e)`, `fallback(e)` is called.
/// - **L3**: The [`FallbackSource`] tag always matches which path was taken.
/// - **L4**: If both fail, both errors are preserved in [`FallbackError`].
pub async fn with_fallback<T, E, F1, F2, Fut1, Fut2>(
    primary: F1,
    fallback: F2,
) -> Result<(T, FallbackSource), FallbackError<E>>
where
    F1: FnOnce() -> Fut1,
    Fut1: Future<Output = Result<T, E>>,
    F2: FnOnce(&E) -> Fut2,
    Fut2: Future<Output = Result<T, E>>,
{
    match primary().await {
        Ok(t) => Ok((t, FallbackSource::Primary)),
        Err(e) => match fallback(&e).await {
            Ok(t) => Ok((t, FallbackSource::Fallback)),
            Err(e2) => Err(FallbackError {
                primary: e,
                fallback: e2,
            }),
        },
    }
}

/// Synchronous version of [`with_fallback`] for non-async contexts.
pub fn with_fallback_sync<T, E>(
    primary: impl FnOnce() -> Result<T, E>,
    fallback: impl FnOnce(&E) -> Result<T, E>,
) -> Result<(T, FallbackSource), FallbackError<E>> {
    match primary() {
        Ok(t) => Ok((t, FallbackSource::Primary)),
        Err(e) => match fallback(&e) {
            Ok(t) => Ok((t, FallbackSource::Fallback)),
            Err(e2) => Err(FallbackError {
                primary: e,
                fallback: e2,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // L1: Success passthrough
    // =========================================================================

    #[tokio::test]
    async fn l1_primary_success_skips_fallback() {
        let mut fallback_called = false;
        let result = with_fallback(
            || async { Ok::<i32, String>(42) },
            |_e| {
                fallback_called = true;
                async { Ok(0) }
            },
        )
        .await;

        assert_eq!(result, Ok((42, FallbackSource::Primary)));
        assert!(!fallback_called);
    }

    // =========================================================================
    // L2: Fallback on error
    // =========================================================================

    #[tokio::test]
    async fn l2_primary_error_calls_fallback() {
        let result = with_fallback(
            || async { Err::<i32, String>("primary failed".into()) },
            |_e| async { Ok(99) },
        )
        .await;

        assert_eq!(result, Ok((99, FallbackSource::Fallback)));
    }

    #[tokio::test]
    async fn l2_both_fail_preserves_both_errors() {
        let result: Result<(i32, FallbackSource), FallbackError<String>> = with_fallback(
            || async { Err("primary failed".into()) },
            |_e| async { Err("fallback also failed".into()) },
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.primary, "primary failed");
        assert_eq!(err.fallback, "fallback also failed");
    }

    // =========================================================================
    // L3: Source tracking
    // =========================================================================

    #[tokio::test]
    async fn l3_source_is_primary_on_success() {
        let (_, source) = with_fallback(|| async { Ok::<_, String>(1) }, |_| async { Ok(2) })
            .await
            .unwrap();

        assert_eq!(source, FallbackSource::Primary);
    }

    #[tokio::test]
    async fn l3_source_is_fallback_on_primary_error() {
        let (_, source) = with_fallback(
            || async { Err::<i32, String>("err".into()) },
            |_| async { Ok(2) },
        )
        .await
        .unwrap();

        assert_eq!(source, FallbackSource::Fallback);
    }

    // =========================================================================
    // Sync version
    // =========================================================================

    #[test]
    fn sync_primary_success() {
        let result = with_fallback_sync(|| Ok::<_, String>(42), |_| Ok(0));
        assert_eq!(result, Ok((42, FallbackSource::Primary)));
    }

    #[test]
    fn sync_fallback_on_error() {
        let result = with_fallback_sync(|| Err::<i32, _>("err".to_string()), |_| Ok(99));
        assert_eq!(result, Ok((99, FallbackSource::Fallback)));
    }

    #[test]
    fn sync_both_fail_preserves_both_errors() {
        let result: Result<(i32, FallbackSource), FallbackError<String>> =
            with_fallback_sync(|| Err("e1".into()), |_| Err("e2".into()));
        let err = result.unwrap_err();
        assert_eq!(err.primary, "e1");
        assert_eq!(err.fallback, "e2");
    }

    // =========================================================================
    // Fallback receives the error
    // =========================================================================

    #[tokio::test]
    async fn fallback_receives_primary_error() {
        let result = with_fallback(
            || async { Err::<i32, String>("specific error".into()) },
            |e| {
                let msg = e.clone();
                async move { Ok(msg.len() as i32) }
            },
        )
        .await;

        // "specific error" has 14 chars
        assert_eq!(result, Ok((14, FallbackSource::Fallback)));
    }
}
