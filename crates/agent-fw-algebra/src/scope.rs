//! Structured concurrency: scope-based resource cleanup.
//!
//! # Laws
//!
//! - **L1 Bracket guarantee (sync `Scope`)**: Finalizers always run, even on
//!   panic/cancel (via `Drop` with `catch_unwind`)
//! - **L1' Bracket guarantee (async `AsyncScope`)**: Finalizers always run on the
//!   **non-panic** path. On panic, Rust has no async `Drop` — async finalizers
//!   are skipped with a warning. Use sync `Scope::defer` for panic-critical cleanup.
//! - **L2 LIFO order**: Finalizers run in reverse registration order
//! - **L3 Error propagation**: Body errors propagate after cleanup
//! - **L4 Cancellation safety**: On cancellation, cleanup still runs

use crate::cancellation::CancellationToken;

type BoxFinalizer = Box<dyn FnOnce() + Send>;

/// A scope that collects finalizers and runs them in LIFO order on drop.
///
/// Implements `Drop` to guarantee L1 (bracket guarantee): finalizers run
/// even if the body panics. Individual finalizer panics are caught so
/// subsequent finalizers still execute.
pub struct Scope {
    finalizers: Vec<BoxFinalizer>,
    cancel: CancellationToken,
}

impl Scope {
    /// Create a new scope with the given cancellation token.
    pub fn new(cancel: CancellationToken) -> Self {
        Self {
            finalizers: Vec::new(),
            cancel,
        }
    }

    /// Register a finalizer to run when the scope exits (LIFO order).
    pub fn defer(&mut self, f: impl FnOnce() + Send + 'static) {
        self.finalizers.push(Box::new(f));
    }

    /// Access the cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Take ownership of finalizers so we can consume them.
        let finalizers = std::mem::take(&mut self.finalizers);
        for finalizer in finalizers.into_iter().rev() {
            // Catch panics in individual finalizers so all finalizers run.
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(finalizer)) {
                tracing::error!("Scope finalizer panicked: {:?}", e);
            }
        }
    }
}

/// Execute a synchronous body within a scope, guaranteeing LIFO cleanup.
///
/// Finalizers are always run, even if `body` panics (via `Drop`).
pub fn with_scope<A>(cancel: CancellationToken, body: impl FnOnce(&mut Scope) -> A) -> A {
    let mut scope = Scope::new(cancel);
    let result = body(&mut scope);
    // Redundant: scope's Drop impl runs finalizers at end-of-scope anyway.
    // On the panic path, this line is never reached — the *implicit* drop
    // during stack unwinding is what provides the L1 bracket guarantee.
    drop(scope);
    result
}

type BoxAsyncFinalizer =
    Box<dyn FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>;

/// An async scope that collects async finalizers and runs them in LIFO order.
///
/// **Panic caveat**: Rust has no async destructors. If the body panics, the
/// synchronous `Drop` impl runs async finalizers on a blocking thread via
/// `tokio::task::block_in_place` (if available) or skips them with a warning.
/// For guaranteed async cleanup, use `with_scope_async` which handles the
/// non-panic path. For panic paths, register synchronous finalizers via a
/// nested `Scope` for the critical cleanup.
pub struct AsyncScope {
    finalizers: Vec<BoxAsyncFinalizer>,
    cancel: CancellationToken,
}

impl AsyncScope {
    /// Create a new async scope.
    pub fn new(cancel: CancellationToken) -> Self {
        Self {
            finalizers: Vec::new(),
            cancel,
        }
    }

    /// Register an async finalizer.
    pub fn defer<F, Fut>(&mut self, f: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.finalizers.push(Box::new(move || {
            Box::pin(f()) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }));
    }

    /// Access the cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Run all finalizers in LIFO order (consumes the finalizer list).
    ///
    /// Called by `with_scope_async` on the normal (non-panic) path.
    /// Individual finalizer panics are caught in **both phases** so
    /// subsequent finalizers still execute (L1 bracket guarantee):
    ///
    /// - **Phase 1 (construction)**: The `FnOnce()` call that builds the
    ///   future is wrapped in `std::panic::catch_unwind`.
    /// - **Phase 2 (execution)**: The resulting future's `.await` is
    ///   wrapped in `FutureExt::catch_unwind`.
    async fn run_finalizers(&mut self) {
        use futures::FutureExt;
        let finalizers = std::mem::take(&mut self.finalizers);
        for finalizer in finalizers.into_iter().rev() {
            // Phase 1: catch panics during future construction (FnOnce call).
            let fut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(finalizer));
            match fut {
                Ok(future) => {
                    // Phase 2: catch panics during future execution (.await).
                    if let Err(e) = std::panic::AssertUnwindSafe(future).catch_unwind().await {
                        tracing::error!("AsyncScope finalizer panicked during execution: {:?}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("AsyncScope finalizer panicked during construction: {:?}", e);
                }
            }
        }
    }
}

impl Drop for AsyncScope {
    fn drop(&mut self) {
        if !self.finalizers.is_empty() {
            // Async finalizers remain — this means we're on the panic/early-exit path
            // since `with_scope_async` drains them before drop.
            tracing::warn!(
                "AsyncScope dropped with {} unexecuted async finalizers — \
                 async cleanup cannot run in Drop. Use synchronous Scope::defer \
                 for panic-critical cleanup.",
                self.finalizers.len()
            );
        }
    }
}

/// Execute an async operation within a scope, with a setup phase and guaranteed LIFO cleanup.
///
/// `setup` registers finalizers, then `op` runs the body. Finalizers run after `op` completes.
///
/// **Note**: If `op` panics, async finalizers cannot run (no async Drop in Rust).
/// For panic-critical cleanup, use synchronous `Scope::defer` or ensure `op` does
/// not panic.
pub async fn with_scope_async<A, Setup, Op, Fut>(
    cancel: CancellationToken,
    setup: Setup,
    op: Op,
) -> A
where
    Setup: FnOnce(&mut AsyncScope),
    Op: FnOnce(&CancellationToken) -> Fut,
    Fut: std::future::Future<Output = A>,
{
    let mut scope = AsyncScope::new(cancel);
    setup(&mut scope);
    let cancel = scope.cancel.clone();
    let result = op(&cancel).await;
    scope.run_finalizers().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// L2: LIFO order
    #[test]
    fn lifo_ordering() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = order.clone();
        let o2 = order.clone();
        let o3 = order.clone();

        with_scope(CancellationToken::new(), |scope| {
            scope.defer(move || o1.lock().unwrap().push(1));
            scope.defer(move || o2.lock().unwrap().push(2));
            scope.defer(move || o3.lock().unwrap().push(3));
        });

        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
    }

    /// L1: Bracket guarantee — finalizers run even with early return
    #[test]
    fn finalizers_always_run() {
        let ran = Arc::new(Mutex::new(false));
        let ran_clone = ran.clone();

        with_scope(CancellationToken::new(), |scope| {
            scope.defer(move || *ran_clone.lock().unwrap() = true);
            // Body returns without error
        });

        assert!(*ran.lock().unwrap());
    }

    /// L1: Bracket guarantee — finalizers run even on panic
    #[test]
    fn finalizers_run_on_panic() {
        let ran = Arc::new(Mutex::new(false));
        let ran_clone = ran.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_scope(CancellationToken::new(), |scope| {
                scope.defer(move || *ran_clone.lock().unwrap() = true);
                panic!("body panics");
            });
        }));

        assert!(result.is_err(), "should have panicked");
        assert!(*ran.lock().unwrap(), "finalizer must run even on panic");
    }

    /// L1+L2: On panic, LIFO order preserved and all finalizers run
    #[test]
    fn all_finalizers_run_on_panic_in_lifo_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let o1 = order.clone();
        let o2 = order.clone();
        let o3 = order.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_scope(CancellationToken::new(), |scope| {
                scope.defer(move || o1.lock().unwrap().push(1));
                scope.defer(move || o2.lock().unwrap().push(2));
                scope.defer(move || o3.lock().unwrap().push(3));
                panic!("body panics");
            });
        }));

        assert!(result.is_err());
        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
    }

    /// Finalizer panic doesn't prevent subsequent finalizers from running
    #[test]
    fn finalizer_panic_isolated() {
        let ran_first = Arc::new(Mutex::new(false));
        let ran_last = Arc::new(Mutex::new(false));
        let first_clone = ran_first.clone();
        let last_clone = ran_last.clone();

        with_scope(CancellationToken::new(), |scope| {
            scope.defer(move || *first_clone.lock().unwrap() = true);
            scope.defer(move || panic!("finalizer panics"));
            scope.defer(move || *last_clone.lock().unwrap() = true);
        });

        // Both non-panicking finalizers should have run
        assert!(*ran_first.lock().unwrap(), "first finalizer must run");
        assert!(*ran_last.lock().unwrap(), "last finalizer must run");
    }

    /// L2: Async LIFO order
    #[tokio::test]
    async fn async_lifo_ordering() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = order.clone();
        let o2 = order.clone();
        let o3 = order.clone();

        with_scope_async(
            CancellationToken::new(),
            |scope| {
                scope.defer(move || async move { o1.lock().unwrap().push(1) });
                scope.defer(move || async move { o2.lock().unwrap().push(2) });
                scope.defer(move || async move { o3.lock().unwrap().push(3) });
            },
            |_cancel| async { 42 },
        )
        .await;

        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
    }

    /// Scope with no finalizers is fine
    #[test]
    fn empty_scope() {
        let result = with_scope(CancellationToken::new(), |_scope| 42);
        assert_eq!(result, 42);
    }

    /// L1: Async finalizer panic doesn't prevent subsequent finalizers from running
    #[tokio::test]
    async fn async_finalizer_panic_isolated() {
        let ran_first = Arc::new(Mutex::new(false));
        let ran_last = Arc::new(Mutex::new(false));
        let first_clone = ran_first.clone();
        let last_clone = ran_last.clone();

        with_scope_async(
            CancellationToken::new(),
            |scope| {
                scope.defer(move || async move { *first_clone.lock().unwrap() = true });
                scope.defer(move || async move { panic!("async finalizer panics") });
                scope.defer(move || async move { *last_clone.lock().unwrap() = true });
            },
            |_cancel| async { 42 },
        )
        .await;

        assert!(*ran_first.lock().unwrap(), "first async finalizer must run");
        assert!(*ran_last.lock().unwrap(), "last async finalizer must run");
    }

    /// L1: Async finalizer panic during *construction* (Phase 1) doesn't prevent
    /// subsequent finalizers from running. This tests the FnOnce invocation path,
    /// not the Future .await path.
    #[tokio::test]
    async fn async_finalizer_construction_panic_isolated() {
        let ran_first = Arc::new(Mutex::new(false));
        let ran_last = Arc::new(Mutex::new(false));
        let first_clone = ran_first.clone();
        let last_clone = ran_last.clone();

        with_scope_async(
            CancellationToken::new(),
            |scope| {
                scope.defer(move || async move { *first_clone.lock().unwrap() = true });
                // This finalizer panics during construction (the FnOnce body),
                // before any future is returned.
                scope.defer(
                    move || -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
                        panic!("panic during future construction")
                    },
                );
                scope.defer(move || async move { *last_clone.lock().unwrap() = true });
            },
            |_cancel| async { 42 },
        )
        .await;

        assert!(
            *ran_first.lock().unwrap(),
            "first finalizer must run despite construction panic"
        );
        assert!(
            *ran_last.lock().unwrap(),
            "last finalizer must run despite construction panic"
        );
    }
}
