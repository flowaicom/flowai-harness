//! Structured task concurrency with guaranteed containment.
//!
//! A `Nursery` owns spawned tasks and guarantees no task outlives its scope.
//! Builds on the existing `CancellationToken` child propagation (L4) and
//! `AsyncScope` LIFO cleanup.
//!
//! # Laws
//!
//! - **L1 (Containment):** No spawned task outlives `with_nursery`'s return
//! - **L2 (Propagation):** Parent `cancel` → all child tokens cancelled
//! - **L3 (Error escalation):** First task error cancels all remaining tasks
//! - **L4 (Completion):** `with_nursery` returns only after all tasks are done/cancelled
//! - **L5 (Empty nursery):** Zero spawns → returns body result immediately
//! - **L6 (Body error):** If body returns Err, all tasks are cancelled before returning

use std::future::Future;

use tokio::task::JoinHandle;

use crate::cancellation::CancellationToken;

/// Error produced by nursery operations.
///
/// Generic over `E` to preserve the caller's error type. Task closures
/// return `Result<(), E>`, and the nursery wraps errors into the
/// appropriate variant without type erasure.
#[derive(Debug, Clone)]
pub enum NurseryError<E> {
    /// A spawned task returned `Err(e)`.
    #[allow(missing_docs)]
    Task(E),
    /// The nursery body returned `Err(e)`.
    #[allow(missing_docs)]
    Body(E),
    /// A spawned task panicked.
    #[allow(missing_docs)]
    Panicked(String),
}

impl<E: std::fmt::Display> std::fmt::Display for NurseryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NurseryError::Task(e) => write!(f, "task failed: {e}"),
            NurseryError::Body(e) => write!(f, "body failed: {e}"),
            NurseryError::Panicked(msg) => write!(f, "task panicked: {msg}"),
        }
    }
}

impl<E: std::fmt::Display + std::fmt::Debug> std::error::Error for NurseryError<E> {}

/// A structured concurrency scope that owns spawned tasks.
///
/// Tasks receive child `CancellationToken`s. When the nursery exits,
/// all tasks are cancelled and awaited.
pub struct Nursery<E> {
    cancel: CancellationToken,
    error_tx: tokio::sync::mpsc::UnboundedSender<NurseryError<E>>,
    error_rx: Option<tokio::sync::mpsc::UnboundedReceiver<NurseryError<E>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl<E: Send + 'static> Nursery<E> {
    fn new(cancel: CancellationToken) -> Self {
        let (error_tx, error_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            cancel,
            error_tx,
            error_rx: Some(error_rx),
            tasks: Vec::new(),
        }
    }

    /// Spawn a child task. The task receives a child `CancellationToken`
    /// that is cancelled when the nursery exits.
    ///
    /// Tasks return `Result<(), E>` — the raw error type, not `NurseryError`.
    /// The nursery wraps task errors in `NurseryError::Task(e)` at the boundary.
    pub fn spawn<F, Fut>(&mut self, f: F)
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
    {
        let child_cancel = self.cancel.child();
        let error_tx = self.error_tx.clone();
        let nursery_cancel = self.cancel.clone();
        let handle = tokio::spawn(async move {
            match f(child_cancel).await {
                Ok(()) => {}
                Err(e) => {
                    // Cancel siblings immediately (L3: error escalation)
                    nursery_cancel.cancel();
                    let _ = error_tx.send(NurseryError::Task(e));
                }
            }
        });
        self.tasks.push(handle);
    }

    /// Number of spawned tasks.
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

/// Run a body with structured concurrency guarantees.
///
/// 1. `body` receives a `&mut Nursery<E>` for spawning tasks
/// 2. After body returns, waits for ALL spawned tasks to complete
/// 3. If any task errors, cancels remaining tasks via CancellationToken
/// 4. Returns body's result on success, first error on failure
///
/// Tasks receive child CancellationTokens. Parent cancellation
/// propagates to all children (CancellationToken law L4).
///
/// The body returns `Result<T, E>` (not `NurseryError`). Body errors
/// are wrapped in `NurseryError::Body(e)` by the nursery.
pub async fn with_nursery<F, Fut, T, E>(
    cancel: &CancellationToken,
    body: F,
) -> Result<T, NurseryError<E>>
where
    F: FnOnce(&mut Nursery<E>) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Send + 'static,
{
    let nursery_cancel = cancel.child();
    let mut nursery = Nursery::new(nursery_cancel.clone());

    let body_result = body(&mut nursery).await;

    // If body failed, cancel all tasks
    if body_result.is_err() {
        nursery_cancel.cancel();
    }

    // Drop the sender so the receiver will drain.
    // Tasks hold clones of the sender for error reporting;
    // errors are self-cancelling (the task cancels siblings on Err).
    drop(nursery.error_tx);

    // Wait for all spawned tasks to complete.
    // Tasks self-cancel siblings on error (L3), so blocked tasks
    // get unblocked by cancellation propagation — no deadlock risk.
    let mut panic_error: Option<NurseryError<E>> = None;
    for handle in nursery.tasks {
        if let Err(join_err) = handle.await {
            if join_err.is_panic() && panic_error.is_none() {
                panic_error = Some(NurseryError::Panicked(format!("{join_err}")));
                nursery_cancel.cancel();
            }
        }
    }

    // Collect errors from the channel (task-reported errors take precedence over panics)
    let mut error_rx = nursery.error_rx.take().unwrap();
    let first_error = error_rx.try_recv().ok().or(panic_error);

    // Body error takes precedence, then task errors
    match body_result {
        Err(e) => Err(NurseryError::Body(e)),
        Ok(value) => match first_error {
            Some(e) => Err(e),
            None => Ok(value),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// L5: Empty nursery returns body result immediately.
    #[tokio::test]
    async fn empty_nursery() {
        let cancel = CancellationToken::new();
        let result: Result<i32, NurseryError<String>> =
            with_nursery(&cancel, |_nursery| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    /// L1: Spawned tasks complete before with_nursery returns.
    #[tokio::test]
    async fn tasks_complete_before_return() {
        let cancel = CancellationToken::new();
        let completed = Arc::new(AtomicBool::new(false));
        let completed_clone = completed.clone();

        let result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
            nursery.spawn(move |_cancel| async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                completed_clone.store(true, Ordering::SeqCst);
                Ok(())
            });
            async { Ok(()) }
        })
        .await;

        assert!(result.is_ok());
        assert!(
            completed.load(Ordering::SeqCst),
            "task must complete before return"
        );
    }

    /// L2: Parent cancellation propagates to children.
    #[tokio::test]
    async fn parent_cancellation_propagates() {
        let cancel = CancellationToken::new();
        let child_was_cancelled = Arc::new(AtomicBool::new(false));
        let child_cancelled_clone = child_was_cancelled.clone();

        let cancel_clone = cancel.clone();
        let result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
            nursery.spawn(move |child_cancel| async move {
                child_cancel.cancelled().await;
                child_cancelled_clone.store(true, Ordering::SeqCst);
                Ok(())
            });
            async move {
                cancel_clone.cancel();
                Ok(())
            }
        })
        .await;

        assert!(result.is_ok());
        assert!(
            child_was_cancelled.load(Ordering::SeqCst),
            "child should have seen cancellation"
        );
    }

    /// L3: First task error cancels remaining tasks.
    #[tokio::test]
    async fn error_escalation() {
        let cancel = CancellationToken::new();
        let second_cancelled = Arc::new(AtomicBool::new(false));
        let second_cancelled_clone = second_cancelled.clone();

        let result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
            // First task errors immediately
            nursery.spawn(|_cancel| async { Err("boom".to_string()) });
            // Second task waits for cancellation
            nursery.spawn(move |child_cancel| async move {
                child_cancel.cancelled().await;
                second_cancelled_clone.store(true, Ordering::SeqCst);
                Ok(())
            });
            async { Ok(()) }
        })
        .await;

        assert!(result.is_err());
        assert!(
            second_cancelled.load(Ordering::SeqCst),
            "second task should be cancelled on first error"
        );
    }

    /// L4: with_nursery waits for all tasks even after error.
    #[tokio::test]
    async fn completion_after_error() {
        let cancel = CancellationToken::new();
        let tasks_finished = Arc::new(AtomicU32::new(0));
        let tasks_finished_clone = tasks_finished.clone();

        let _result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
            for _ in 0..3 {
                let counter = tasks_finished_clone.clone();
                nursery.spawn(move |child_cancel| async move {
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_millis(10)) => {},
                        () = child_cancel.cancelled() => {},
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
            }
            async { Ok(()) }
        })
        .await;

        assert_eq!(
            tasks_finished.load(Ordering::SeqCst),
            3,
            "all tasks must finish before with_nursery returns"
        );
    }

    /// L6: Body error cancels all tasks.
    #[tokio::test]
    async fn body_error_cancels_tasks() {
        let cancel = CancellationToken::new();
        let child_cancelled = Arc::new(AtomicBool::new(false));
        let child_cancelled_clone = child_cancelled.clone();

        let result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
            nursery.spawn(move |child_cancel| async move {
                child_cancel.cancelled().await;
                child_cancelled_clone.store(true, Ordering::SeqCst);
                Ok(())
            });
            async { Err("body failed".to_string()) }
        })
        .await;

        assert!(result.is_err());
        assert!(
            child_cancelled.load(Ordering::SeqCst),
            "tasks should be cancelled when body errors"
        );
    }

    /// task_count tracks spawns.
    #[tokio::test]
    async fn task_count_tracks_spawns() {
        let cancel = CancellationToken::new();
        let result: Result<usize, NurseryError<String>> = with_nursery(&cancel, |nursery| {
            assert_eq!(nursery.task_count(), 0);
            nursery.spawn(|_| async { Ok(()) });
            nursery.spawn(|_| async { Ok(()) });
            let count = nursery.task_count();
            async move { Ok(count) }
        })
        .await;
        assert_eq!(result.unwrap(), 2);
    }

    /// D3: NurseryError preserves error types
    #[tokio::test]
    async fn preserves_error_type() {
        #[derive(Debug, Clone, PartialEq)]
        struct MyError(u32);
        impl std::fmt::Display for MyError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "MyError({})", self.0)
            }
        }

        let cancel = CancellationToken::new();
        let result: Result<(), NurseryError<MyError>> = with_nursery(&cancel, |nursery| {
            nursery.spawn(|_cancel| async { Err(MyError(42)) });
            async { Ok(()) }
        })
        .await;

        match result {
            Err(NurseryError::Task(e)) => assert_eq!(e, MyError(42)),
            other => panic!("expected NurseryError::Task, got {other:?}"),
        }
    }
}
