//! Cooperative cancellation token.
//!
//! Wraps `tokio_util::sync::CancellationToken` with a cleaner API
//! and documented algebraic laws.
//!
//! # Laws
//!
//! - L1. Monotonicity: once cancelled, stays cancelled forever
//! - L2. Immediate resolution: cancelled().await resolves immediately when cancelled
//! - L3. Shared state: clones share cancellation state
//! - L4. Child propagation: parent cancel → child cancelled
//! - L5. Idempotence: cancel(); cancel() = cancel()
//! - L6. Default not cancelled: new token is not cancelled

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Cooperative cancellation token.
///
/// Thread-safe, cloneable, and cheap to create children from.
#[derive(Clone)]
pub struct CancellationToken {
    inner: tokio_util::sync::CancellationToken,
}

impl CancellationToken {
    /// Create a new cancellation token (not cancelled).
    pub fn new() -> Self {
        Self {
            inner: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// Create a child token that is cancelled when this token is cancelled.
    pub fn child(&self) -> Self {
        Self {
            inner: self.inner.child_token(),
        }
    }

    /// Cancel this token (and all children).
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Check if this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Returns a future that resolves when this token is cancelled.
    pub fn cancelled(&self) -> CancellationFuture {
        CancellationFuture::new(self.inner.clone())
    }

    /// Run a future with cancellation support.
    ///
    /// Returns `Some(result)` if the future completes, `None` if cancelled.
    pub async fn run<F, T>(&self, f: F) -> Option<T>
    where
        F: Future<Output = T>,
    {
        tokio::select! {
            result = f => Some(result),
            _ = self.inner.cancelled() => None,
        }
    }

    /// Get the inner tokio CancellationToken for interop.
    pub fn inner(&self) -> &tokio_util::sync::CancellationToken {
        &self.inner
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Future that resolves when a CancellationToken is cancelled.
///
/// The inner future is created once in `new()` and stored as a pinned box,
/// so waker registrations are preserved across polls.
pub struct CancellationFuture {
    inner: Pin<Box<dyn Future<Output = ()> + Send + Sync>>,
}

impl CancellationFuture {
    fn new(token: tokio_util::sync::CancellationToken) -> Self {
        Self {
            inner: Box::pin(async move { token.cancelled().await }),
        }
    }
}

impl Future for CancellationFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_token_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_sets_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn child_propagation() {
        let parent = CancellationToken::new();
        let child = parent.child();
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn child_does_not_cancel_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();
        child.cancel();
        assert!(!parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn run_completes_normally() {
        let token = CancellationToken::new();
        let result = token.run(async { 42 }).await;
        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    async fn run_returns_none_when_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        let result = token.run(std::future::pending::<i32>()).await;
        assert_eq!(result, None);
    }
}
