//! Cooperative pause token for flow control.
//!
//! Arc-backed, clone-sharing. Used for human-in-the-loop eval pausing.
//!
//! # Laws
//!
//! - **L1 Toggle**: `pause()` → `is_paused() == true`; `resume()` → `is_paused() == false`
//! - **L2 Idempotent**: `pause(); pause()` is same as `pause()`
//! - **L3 Clone-sharing**: Cloned tokens share the same underlying state
//! - **L4 Non-blocking check**: `is_paused()` never blocks
//! - **L5 Cooperative checkpoint**: `wait_if_paused()` returns immediately when not paused

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cooperative pause token.
///
/// Cheap to clone (Arc-backed). All clones share the same paused state.
#[derive(Clone)]
pub struct PauseToken {
    inner: Arc<PauseInner>,
}

struct PauseInner {
    paused: AtomicBool,
    notify: tokio::sync::Notify,
    pause_notify: tokio::sync::Notify,
}

impl PauseToken {
    /// Create a new unpaused token.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(PauseInner {
                paused: AtomicBool::new(false),
                notify: tokio::sync::Notify::new(),
                pause_notify: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Pause — cooperative tasks should call `wait_if_paused()` at checkpoints.
    pub fn pause(&self) {
        self.inner.paused.store(true, Ordering::Release);
        self.inner.pause_notify.notify_waiters();
    }

    /// Resume — wakes any tasks blocked in `wait_if_paused()`.
    pub fn resume(&self) {
        self.inner.paused.store(false, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    /// Check if currently paused (non-blocking).
    pub fn is_paused(&self) -> bool {
        self.inner.paused.load(Ordering::Acquire)
    }

    /// Cooperative checkpoint: if paused, wait until resumed.
    ///
    /// Returns immediately if not paused.
    /// Registers the notified future BEFORE checking the flag to avoid
    /// a TOCTOU race where a resume notification is lost between the
    /// atomic check and the await.
    pub async fn wait_if_paused(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if !self.inner.paused.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    /// Returns a future that resolves when this token transitions to paused.
    /// If already paused, resolves immediately.
    ///
    /// This is the dual of `wait_if_paused()`:
    /// - `wait_if_paused()` blocks WHILE paused (resumes on false)
    /// - `until_paused()` blocks UNTIL paused (resolves on true)
    ///
    /// # Law
    ///
    /// - **L9 (Until-paused duality):** `until_paused()` resolves iff `is_paused() == true`
    pub async fn until_paused(&self) {
        loop {
            let notified = self.inner.pause_notify.notified();
            if self.inner.paused.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

impl Default for PauseToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Composed pause token: paused when ANY constituent is paused.
///
/// This is the monoid over `PauseToken` under disjunction (OR):
/// - **Identity**: An unpaused token
/// - **Combine**: `a.combine(b)` is paused iff `a` OR `b` is paused
///
/// # Laws
///
/// - **L6 (Identity)**: `compose(new(), x).is_paused() == x.is_paused()`
/// - **L7 (Commutativity)**: `compose(a, b).is_paused() == compose(b, a).is_paused()`
/// - **L8 (Associativity)**: `compose(compose(a, b), c).is_paused() == compose(a, compose(b, c)).is_paused()`
#[derive(Clone)]
pub struct ComposedPauseToken {
    tokens: Vec<PauseToken>,
}

impl ComposedPauseToken {
    /// Create an empty composed token (never paused — identity element).
    pub fn empty() -> Self {
        Self { tokens: Vec::new() }
    }

    /// Compose two pause tokens.
    pub fn compose(a: PauseToken, b: PauseToken) -> Self {
        Self { tokens: vec![a, b] }
    }

    /// Add another token to the composition.
    pub fn and(mut self, token: PauseToken) -> Self {
        self.tokens.push(token);
        self
    }

    /// Check if ANY constituent is paused.
    pub fn is_paused(&self) -> bool {
        self.tokens.iter().any(|t| t.is_paused())
    }

    /// Resume all constituents.
    pub fn resume_all(&self) {
        for token in &self.tokens {
            token.resume();
        }
    }

    /// Cooperative checkpoint: blocks until ALL constituents are unpaused.
    ///
    /// Loops until a full pass through every token finds none paused.
    /// This prevents a TOCTOU race where token B becomes paused after
    /// we've already passed it in a single sequential sweep.
    pub async fn wait_if_paused(&self) {
        loop {
            let mut any_waited = false;
            for token in &self.tokens {
                if token.is_paused() {
                    token.wait_if_paused().await;
                    any_waited = true;
                }
            }
            if !any_waited {
                break;
            }
        }
    }
}

impl std::fmt::Debug for PauseToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PauseToken")
            .field("paused", &self.is_paused())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L1: Toggle
    #[test]
    fn toggle() {
        let token = PauseToken::new();
        assert!(!token.is_paused());
        token.pause();
        assert!(token.is_paused());
        token.resume();
        assert!(!token.is_paused());
    }

    /// L2: Idempotent
    #[test]
    fn idempotent_pause() {
        let token = PauseToken::new();
        token.pause();
        token.pause();
        assert!(token.is_paused());
        token.resume();
        token.resume();
        assert!(!token.is_paused());
    }

    /// L3: Clone-sharing
    #[test]
    fn clone_sharing() {
        let t1 = PauseToken::new();
        let t2 = t1.clone();
        t1.pause();
        assert!(t2.is_paused());
        t2.resume();
        assert!(!t1.is_paused());
    }

    /// L5: Cooperative checkpoint returns immediately when not paused
    #[tokio::test]
    async fn wait_if_paused_returns_immediately_when_not_paused() {
        let token = PauseToken::new();
        // Should return immediately
        tokio::time::timeout(std::time::Duration::from_millis(50), token.wait_if_paused())
            .await
            .expect("wait_if_paused should return immediately when not paused");
    }

    // =========================================================================
    // ComposedPauseToken Tests
    // =========================================================================

    /// L6: Empty composed token is never paused (identity).
    #[test]
    fn composed_empty_not_paused() {
        let composed = ComposedPauseToken::empty();
        assert!(!composed.is_paused());
    }

    /// Composed pauses when ANY constituent pauses.
    #[test]
    fn composed_pauses_on_any() {
        let a = PauseToken::new();
        let b = PauseToken::new();
        let composed = ComposedPauseToken::compose(a.clone(), b.clone());

        assert!(!composed.is_paused());
        a.pause();
        assert!(composed.is_paused());
        a.resume();
        assert!(!composed.is_paused());
        b.pause();
        assert!(composed.is_paused());
    }

    /// L7 (Commutativity): Order doesn't matter.
    #[test]
    fn composed_commutative() {
        let a = PauseToken::new();
        let b = PauseToken::new();
        a.pause();

        let ab = ComposedPauseToken::compose(a.clone(), b.clone());
        let ba = ComposedPauseToken::compose(b, a);
        assert_eq!(ab.is_paused(), ba.is_paused());
    }

    /// resume_all resumes all constituents.
    #[test]
    fn composed_resume_all() {
        let a = PauseToken::new();
        let b = PauseToken::new();
        a.pause();
        b.pause();

        let composed = ComposedPauseToken::compose(a.clone(), b.clone());
        assert!(composed.is_paused());

        composed.resume_all();
        assert!(!a.is_paused());
        assert!(!b.is_paused());
        assert!(!composed.is_paused());
    }

    /// L5: Cooperative checkpoint blocks when paused, unblocks on resume
    #[tokio::test]
    async fn wait_if_paused_blocks_and_resumes() {
        let token = PauseToken::new();
        token.pause();

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            token_clone.wait_if_paused().await;
            true
        });

        // Give the task time to block
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!handle.is_finished());

        token.resume();

        let result = tokio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("should complete after resume")
            .expect("task should not panic");
        assert!(result);
    }

    /// L9: until_paused resolves immediately when already paused
    #[tokio::test]
    async fn until_paused_resolves_immediately_when_paused() {
        let token = PauseToken::new();
        token.pause();
        tokio::time::timeout(std::time::Duration::from_millis(50), token.until_paused())
            .await
            .expect("until_paused should resolve immediately when already paused");
    }

    /// L9: until_paused blocks until paused, then resolves
    #[tokio::test]
    async fn until_paused_blocks_then_resolves() {
        let token = PauseToken::new();
        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            token_clone.until_paused().await;
            true
        });

        // Give the task time to block
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!handle.is_finished());

        token.pause();

        let result = tokio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("should complete after pause")
            .expect("task should not panic");
        assert!(result);
    }

    /// Composed wait_if_paused loops: if B gets paused while waiting on A,
    /// the loop re-checks B after A resumes.
    #[tokio::test]
    async fn composed_wait_if_paused_loops() {
        let a = PauseToken::new();
        let b = PauseToken::new();
        a.pause();

        let composed = ComposedPauseToken::compose(a.clone(), b.clone());

        let composed_clone = composed.clone();
        let handle = tokio::spawn(async move {
            composed_clone.wait_if_paused().await;
            true
        });

        // Let the task block on A
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!handle.is_finished());

        // Pause B while A is still paused, then resume A
        b.pause();
        a.resume();

        // The loop should now detect B is paused and wait
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!handle.is_finished(), "should still be blocked on B");

        // Resume B — now all are unpaused, task should finish
        b.resume();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("should complete after all resumed")
            .expect("task should not panic");
        assert!(result);
    }
}
