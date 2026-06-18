//! Resource management combinators.
//!
//! - `bracket` — Acquire/use/release pattern (like try-with-resources). **Always** releases.
//! - `bracket_simple` — Infallible acquire/use, always release
//! - `bracket_result` — Fallible use with fallible release
//! - `compensating` — Saga pattern: run action, **only** compensate on failure
//! - `with_resource` — Simplified bracket (no explicit release, relies on Drop)
//! - `with_finally` — Run f, then always run finally
//! - `try_finally` — Like with_finally but f returns Result
//!
//! ## `bracket` vs `compensating`
//!
//! `bracket` always runs the release function — correct for connection pools,
//! file handles, locks (resources that must be released regardless of outcome).
//!
//! `compensating` only runs the cleanup on failure — correct for saga/compensating
//! transactions where a previous step must be *undone* if a later step fails,
//! but should be *kept* on success (e.g., provisioned databases, created records).
//!
//! # Laws
//!
//! - **L1 (Bracket guarantee):** Release always runs, even if use fails.
//! - **L2 (Error preservation):** If use fails, its error is returned (release errors swallowed).
//! - **L3 (Compensating success):** `compensating(Ok(v), c)` returns `Ok(v)`, `c` never called.
//! - **L4 (Compensating failure):** `compensating(Err(e), c)` calls `c`, returns `Err(e)`.
//! - **L5 (Finally guarantee):** `with_finally`/`try_finally` always execute the finalizer.

/// Bracket pattern: acquire a resource, use it, then release it.
///
/// The release function runs even if the use function fails.
/// This is the async equivalent of try-with-resources.
pub async fn bracket<R, T, E, Acquire, Use, Release>(
    acquire: Acquire,
    use_fn: Use,
    release: Release,
) -> Result<T, E>
where
    Acquire: std::future::Future<Output = Result<R, E>>,
    Use: FnOnce(
        &R,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + '_>>,
    Release: FnOnce(R) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
{
    let resource = acquire.await?;
    let result = use_fn(&resource).await;
    release(resource).await;
    result
}

/// Simplified bracket: acquire and use are infallible, release always runs.
pub async fn bracket_simple<R, T, Acquire, Use, Release>(
    acquire: Acquire,
    use_fn: Use,
    release: Release,
) -> T
where
    Acquire: std::future::Future<Output = R>,
    Use: FnOnce(&R) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + '_>>,
    Release: FnOnce(R) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
{
    let resource = acquire.await;
    let result = use_fn(&resource).await;
    release(resource).await;
    result
}

/// Bracket where both use and release can fail.
///
/// If use fails, release still runs. If both fail, the use error is returned
/// (release error is swallowed to preserve the original).
pub async fn bracket_result<R, T, E, Acquire, Use, Release>(
    acquire: Acquire,
    use_fn: Use,
    release: Release,
) -> Result<T, E>
where
    Acquire: std::future::Future<Output = Result<R, E>>,
    Use: FnOnce(
        &R,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + '_>>,
    Release:
        FnOnce(R) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), E>> + Send>>,
{
    let resource = acquire.await?;
    let result = use_fn(&resource).await;
    let release_result = release(resource).await;
    // Use error takes priority; only surface release error if use succeeded
    match result {
        Ok(v) => release_result.map(|_| v),
        Err(e) => Err(e),
    }
}

/// Simplified bracket: no explicit release (relies on Drop).
pub async fn with_resource<R, T, Acquire, Use>(acquire: Acquire, use_fn: Use) -> T
where
    Acquire: std::future::Future<Output = R>,
    Use: FnOnce(&R) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + '_>>,
{
    let resource = acquire.await;
    use_fn(&resource).await
    // resource dropped here
}

/// Run `f`, then always run `finally` regardless of outcome.
///
/// **Note:** `finally` will NOT run if `f` panics, because Rust does not
/// have async destructors. For panic-safe cleanup, use synchronous `Drop`.
pub async fn with_finally<T, F, Fin>(f: F, finally: Fin) -> T
where
    F: std::future::Future<Output = T>,
    Fin: std::future::Future<Output = ()>,
{
    let result = f.await;
    finally.await;
    result
}

/// Like `with_finally` but `f` returns Result. Finally always runs.
///
/// **Note:** Same panic caveat as [`with_finally`] — no async destructors.
pub async fn try_finally<T, E, F, Fin>(f: F, finally: Fin) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
    Fin: std::future::Future<Output = ()>,
{
    let result = f.await;
    finally.await;
    result
}

/// Compensating transaction: run `action`, and **only on failure** run the
/// `compensate` closure for cleanup. On success, `compensate` is never called.
///
/// This is the saga/compensating-transaction pattern. Unlike [`bracket`] which
/// always releases (correct for connections/handles), `compensating` only runs
/// cleanup when the action fails (correct for provisioned resources that should
/// persist on success).
///
/// # Laws
///
/// - **L1 (Success-passthrough):** If `action` returns `Ok(v)`, `compensate` is
///   never invoked and `Ok(v)` is returned.
/// - **L2 (Failure-compensation):** If `action` returns `Err(e)`, `compensate()`
///   is awaited before returning `Err(e)`.
/// - **L3 (Error-preservation):** The original error `e` is always returned,
///   regardless of what `compensate` does.
///
/// # Example
///
/// ```ignore
/// let env = provisioner.provision(req).await?;
/// compensating(
///     store.save(&entity),
///     || async { provisioner.deprovision(&env.id).await.ok(); },
/// ).await?;
/// ```
pub async fn compensating<T, E, F, C, CF>(action: F, compensate: C) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
    C: FnOnce() -> CF,
    CF: std::future::Future<Output = ()>,
{
    match action.await {
        Ok(v) => Ok(v),
        Err(e) => {
            compensate().await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    async fn acquire_ok() -> Result<i32, &'static str> {
        Ok(42)
    }

    #[tokio::test]
    async fn bracket_releases_on_success() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();

        let result = bracket(
            acquire_ok(),
            |r| Box::pin(async move { Ok(*r) }),
            move |_| {
                released_clone.store(true, Ordering::SeqCst);
                Box::pin(async {})
            },
        )
        .await;

        assert_eq!(result, Ok(42));
        assert!(released.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn bracket_releases_on_error() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();

        let result: Result<i32, &str> = bracket(
            acquire_ok(),
            |_| Box::pin(async { Err("use failed") }),
            move |_| {
                released_clone.store(true, Ordering::SeqCst);
                Box::pin(async {})
            },
        )
        .await;

        assert!(result.is_err());
        assert!(released.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn bracket_simple_works() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();

        let result = bracket_simple(
            async { 42 },
            |r| Box::pin(async move { *r * 2 }),
            move |_| {
                released_clone.store(true, Ordering::SeqCst);
                Box::pin(async {})
            },
        )
        .await;

        assert_eq!(result, 84);
        assert!(released.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn with_finally_always_runs() {
        let finalized = Arc::new(AtomicBool::new(false));
        let finalized_clone = finalized.clone();

        let result = with_finally(async { 42 }, async move {
            finalized_clone.store(true, Ordering::SeqCst)
        })
        .await;

        assert_eq!(result, 42);
        assert!(finalized.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn try_finally_always_runs() {
        let finalized = Arc::new(AtomicBool::new(false));
        let finalized_clone = finalized.clone();

        let result: Result<i32, &str> = try_finally(async { Err("oops") }, async move {
            finalized_clone.store(true, Ordering::SeqCst)
        })
        .await;

        assert!(result.is_err());
        assert!(finalized.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn with_resource_drops() {
        let result = with_resource(async { 42 }, |r| Box::pin(async move { *r * 2 })).await;
        assert_eq!(result, 84);
    }

    // =========================================================================
    // compensating tests
    // =========================================================================

    #[tokio::test]
    async fn compensating_skips_on_success() {
        let compensated = Arc::new(AtomicBool::new(false));
        let comp = compensated.clone();

        let result = compensating(async { Ok::<_, &str>(42) }, move || async move {
            comp.store(true, Ordering::SeqCst);
        })
        .await;

        assert_eq!(result, Ok(42));
        assert!(
            !compensated.load(Ordering::SeqCst),
            "L1 violated: compensate must not run on success"
        );
    }

    #[tokio::test]
    async fn compensating_runs_on_failure() {
        let compensated = Arc::new(AtomicBool::new(false));
        let comp = compensated.clone();

        let result = compensating(async { Err::<i32, _>("failed") }, move || async move {
            comp.store(true, Ordering::SeqCst);
        })
        .await;

        assert_eq!(result, Err("failed"));
        assert!(
            compensated.load(Ordering::SeqCst),
            "L2 violated: compensate must run on failure"
        );
    }

    #[tokio::test]
    async fn compensating_preserves_error() {
        let result = compensating(async { Err::<i32, _>("original error") }, || async {
            // Compensation runs but the original error is preserved
        })
        .await;

        assert_eq!(
            result,
            Err("original error"),
            "L3 violated: original error must be preserved"
        );
    }
}
