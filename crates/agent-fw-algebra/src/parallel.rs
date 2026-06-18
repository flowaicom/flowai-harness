//! Parallel combinators for async operations.
//!
//! - `race` — Run two futures, return whichever finishes first
//! - `race_success` — Race two fallible futures, return first `Ok`
//! - `race_with_cancel` — Race a future against cancellation
//! - `zip_par` — Run two futures in parallel, return both results
//! - `zip_par_result` — Like zip_par but short-circuits on first error
//! - `zip_par_cancellable` — Join two futures with cancellation
//! - `zip_all` — Join N futures
//! - `zip_all_result` — try_join N futures
//!
//! # Laws
//!
//! - **L1 (Race completion):** `race(a, b)` returns the first future to complete.
//! - **L2 (Race-success first-ok):** `race_success` returns first `Ok`, ignoring concurrent `Err`.
//! - **L3 (Zip-par both):** `zip_par(a, b)` awaits both, returns both results.
//! - **L4 (Zip-par-result short-circuit):** If either future errors, result is `Err` immediately.
//! - **L5 (Cancel precedence):** Cancel token takes priority in `race_with_cancel`.
//! - **L6 (Zip-all order):** `zip_all(futures)` returns results in input order.
//! - **L7 (Zip-all-result short-circuit):** First error stops remaining futures.

use crate::cancellation::CancellationToken;

/// Race two futures, returning whichever completes first.
///
/// The losing future is dropped (cancelled).
pub async fn race<A, B, T>(a: A, b: B) -> T
where
    A: std::future::Future<Output = T>,
    B: std::future::Future<Output = T>,
{
    tokio::select! {
        result = a => result,
        result = b => result,
    }
}

/// Race two fallible futures, returning the first `Ok`.
///
/// If both fail, returns the error from whichever finishes second.
/// The first error is logged at warn level but not propagated.
pub async fn race_success<A, B, T, E>(a: A, b: B) -> Result<T, E>
where
    A: std::future::Future<Output = Result<T, E>> + Send,
    B: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: Send,
{
    tokio::pin!(a);
    tokio::pin!(b);

    // Wait for the first to complete
    let a_failed_first = tokio::select! {
        biased;
        result = &mut a => {
            match result {
                Ok(v) => return Ok(v),
                Err(_) => {
                    tracing::debug!("race_success: first branch (a) failed, waiting for (b)");
                    true
                }
            }
        }
        result = &mut b => {
            match result {
                Ok(v) => return Ok(v),
                Err(_) => {
                    tracing::debug!("race_success: first branch (b) failed, waiting for (a)");
                    false
                }
            }
        }
    };

    // First failed — wait for the other
    if a_failed_first {
        b.await
    } else {
        a.await
    }
}

/// Race a future against a cancellation token.
///
/// Returns `Some(result)` if the future completes first,
/// or `None` if the token was cancelled.
pub async fn race_with_cancel<F, T>(f: F, cancel: &CancellationToken) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::select! {
        result = f => Some(result),
        _ = cancel.cancelled() => None,
    }
}

/// Join two futures in parallel with cancellation support.
///
/// Returns `Some((ra, rb))` if both complete, or `None` if cancelled.
pub async fn zip_par_cancellable<A, B, RA, RB>(
    a: A,
    b: B,
    cancel: &CancellationToken,
) -> Option<(RA, RB)>
where
    A: std::future::Future<Output = RA>,
    B: std::future::Future<Output = RB>,
{
    tokio::select! {
        result = async { tokio::join!(a, b) } => Some(result),
        _ = cancel.cancelled() => None,
    }
}

/// Race two fallible futures with cancellation support.
///
/// Returns `Ok(Some(result))` from first `Ok`, `Err` on error, `Ok(None)` on cancel.
pub async fn race_result_cancel<A, B, T, E>(
    a: A,
    b: B,
    cancel: &CancellationToken,
) -> Result<Option<T>, E>
where
    A: std::future::Future<Output = Result<T, E>> + Send,
    B: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: Send,
{
    tokio::select! {
        result = race_success(a, b) => result.map(Some),
        _ = cancel.cancelled() => Ok(None),
    }
}

/// try_join two futures with cancellation support.
///
/// Returns `Ok(Some((ra, rb)))` on success, `Err` on first error, `Ok(None)` on cancel.
pub async fn zip_par_result_cancellable<A, B, RA, RB, E>(
    a: A,
    b: B,
    cancel: &CancellationToken,
) -> Result<Option<(RA, RB)>, E>
where
    A: std::future::Future<Output = Result<RA, E>>,
    B: std::future::Future<Output = Result<RB, E>>,
{
    tokio::select! {
        result = async { tokio::try_join!(a, b) } => result.map(Some),
        _ = cancel.cancelled() => Ok(None),
    }
}

/// Run two futures in parallel, returning both results.
pub async fn zip_par<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: std::future::Future<Output = RA>,
    B: std::future::Future<Output = RB>,
{
    tokio::join!(a, b)
}

/// Run two fallible futures in parallel, short-circuiting on first error.
pub async fn zip_par_result<A, B, RA, RB, E>(a: A, b: B) -> Result<(RA, RB), E>
where
    A: std::future::Future<Output = Result<RA, E>>,
    B: std::future::Future<Output = Result<RB, E>>,
{
    tokio::try_join!(a, b)
}

/// Race multiple futures, returning the first to complete.
pub async fn race_all<T, F>(futures: Vec<F>) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    use futures::future::FutureExt;
    let (result, _, _) = futures::future::select_all(futures.into_iter().map(|f| f.boxed())).await;
    result
}

/// Join N futures, returning all results.
pub async fn zip_all<T, F>(futures: Vec<F>) -> Vec<T>
where
    F: std::future::Future<Output = T>,
{
    futures::future::join_all(futures).await
}

/// try_join N futures, short-circuiting on first error.
pub async fn zip_all_result<T, E, F>(futures: Vec<F>) -> Result<Vec<T>, E>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    futures::future::try_join_all(futures).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn zip_par_returns_both() {
        let (a, b) = zip_par(async { 1 }, async { 2 }).await;
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    }

    #[tokio::test]
    async fn zip_par_result_ok() {
        let result: Result<(i32, i32), &str> =
            zip_par_result(async { Ok(1) }, async { Ok(2) }).await;
        assert_eq!(result, Ok((1, 2)));
    }

    #[tokio::test]
    async fn zip_par_result_short_circuits() {
        let result: Result<(i32, i32), &str> =
            zip_par_result(async { Err("fail") }, async { Ok(2) }).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn race_returns_fastest() {
        let result = race(
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                1
            },
            async { 2 },
        )
        .await;
        assert_eq!(result, 2);
    }

    #[tokio::test]
    async fn race_with_cancel_completes() {
        let cancel = CancellationToken::new();
        let result = race_with_cancel(async { 42 }, &cancel).await;
        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    async fn race_with_cancel_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = race_with_cancel(
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                42
            },
            &cancel,
        )
        .await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn zip_all_returns_all() {
        use std::future::Future;
        use std::pin::Pin;
        let futures: Vec<Pin<Box<dyn Future<Output = i32>>>> = vec![
            Box::pin(async { 1 }),
            Box::pin(async { 2 }),
            Box::pin(async { 3 }),
        ];
        let results = zip_all(futures).await;
        assert_eq!(results, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn zip_all_result_ok() {
        use std::future::Future;
        use std::pin::Pin;
        let futures: Vec<Pin<Box<dyn Future<Output = Result<i32, &str>>>>> = vec![
            Box::pin(async { Ok(1) }),
            Box::pin(async { Ok(2) }),
            Box::pin(async { Ok(3) }),
        ];
        let results = zip_all_result(futures).await;
        assert_eq!(results, Ok(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn zip_all_result_short_circuits() {
        use std::future::Future;
        use std::pin::Pin;
        let futures: Vec<Pin<Box<dyn Future<Output = Result<i32, &str>>>>> = vec![
            Box::pin(async { Ok(1) }),
            Box::pin(async { Err("fail") }),
            Box::pin(async { Ok(3) }),
        ];
        let result = zip_all_result(futures).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn zip_par_cancellable_completes() {
        let cancel = CancellationToken::new();
        let result = zip_par_cancellable(async { 1i32 }, async { 2i32 }, &cancel).await;
        assert_eq!(result, Some((1, 2)));
    }

    #[tokio::test]
    async fn zip_par_cancellable_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result: Option<(i32, i32)> = zip_par_cancellable(
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                1i32
            },
            async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                2i32
            },
            &cancel,
        )
        .await;
        assert_eq!(result, None);
    }
}
