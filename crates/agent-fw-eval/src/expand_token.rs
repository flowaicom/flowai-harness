//! Expand token for injecting test cases into running evals.
//!
//! Arc-backed, clone-sharing MPMC channel.
//!
//! # Laws
//!
//! - **L1 FIFO**: Injected cases drain in insertion order
//! - **L2 Non-blocking send**: `inject()` never blocks
//! - **L3 Non-blocking drain**: `drain()` never blocks
//! - **L4 Clone-sharing**: All clones share the same underlying state
//! - **L5 Closed semantics**: After `close()`, `inject()` returns false

use std::sync::{Arc, Mutex};

/// Token for dynamically injecting test cases into a running eval.
///
/// Cheap to clone (Arc-backed). All clones share the same queue.
#[derive(Clone)]
pub struct ExpandToken<T = crate::types::EvalTestCase> {
    inner: Arc<ExpandInner<T>>,
}

struct ExpandInner<T> {
    queue: Mutex<Vec<T>>,
    closed: std::sync::atomic::AtomicBool,
}

impl<T> ExpandToken<T> {
    /// Create a new expand token.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ExpandInner {
                queue: Mutex::new(Vec::new()),
                closed: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Inject test cases into the queue.
    ///
    /// Returns `true` if accepted, `false` if the token is closed.
    /// The closed check is performed while holding the mutex to prevent
    /// a TOCTOU race with `close()`.
    pub fn inject(&self, cases: Vec<T>) -> bool {
        let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        if self.inner.closed.load(std::sync::atomic::Ordering::Acquire) {
            return false;
        }
        queue.extend(cases);
        true
    }

    /// Drain all pending test cases (non-blocking).
    ///
    /// Returns an empty vec if no cases are pending.
    pub fn drain(&self) -> Vec<T> {
        let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    }

    /// Close the token — no more injections will be accepted.
    ///
    /// Returns any test cases that were still in the queue at close time.
    /// This prevents silent data loss from a race between the last `drain()`
    /// and `close()`.
    ///
    /// # Law — L6 Drain-on-close
    /// `close()` returns all pending cases; after `close()`, `drain()` returns `[]`.
    pub fn close(&self) -> Vec<T> {
        let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        self.inner
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        std::mem::take(&mut *queue)
    }

    /// Check if there are pending test cases.
    pub fn has_pending(&self) -> bool {
        let queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        !queue.is_empty()
    }

    /// Check if the token is closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl<T> Default for ExpandToken<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Debug for ExpandToken<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpandToken")
            .field("has_pending", &self.has_pending())
            .field("is_closed", &self.is_closed())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TrajectoryMode;
    use crate::EvalTestCase;
    use agent_fw_core::TestCaseId;

    fn make_test_case(id: &str) -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked(id),
            tags: vec![],
            input: format!("test {}", id),
            expected_trajectory: vec![],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        }
    }

    /// L1: FIFO
    #[test]
    fn fifo_ordering() {
        let token = ExpandToken::new();
        token.inject(vec![make_test_case("1")]);
        token.inject(vec![make_test_case("2")]);
        let cases = token.drain();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].id.as_str(), "1");
        assert_eq!(cases[1].id.as_str(), "2");
    }

    /// L2: Non-blocking send
    #[test]
    fn inject_is_non_blocking() {
        let token = ExpandToken::new();
        // Should return immediately
        assert!(token.inject(vec![make_test_case("1")]));
    }

    /// L3: Non-blocking drain
    #[test]
    fn drain_empty() {
        let token: ExpandToken<EvalTestCase> = ExpandToken::new();
        let cases = token.drain();
        assert!(cases.is_empty());
    }

    /// L4: Clone-sharing
    #[test]
    fn clone_sharing() {
        let t1 = ExpandToken::new();
        let t2 = t1.clone();
        t1.inject(vec![make_test_case("1")]);
        let cases = t2.drain();
        assert_eq!(cases.len(), 1);
    }

    /// L5: Closed semantics
    #[test]
    fn closed_rejects_inject() {
        let token = ExpandToken::new();
        let orphaned = token.close();
        assert!(orphaned.is_empty());
        assert!(!token.inject(vec![make_test_case("1")]));
        assert!(token.is_closed());
    }

    /// L6: Drain-on-close returns pending cases
    #[test]
    fn close_returns_pending() {
        let token = ExpandToken::new();
        token.inject(vec![make_test_case("1"), make_test_case("2")]);
        let orphaned = token.close();
        assert_eq!(orphaned.len(), 2);
        assert_eq!(orphaned[0].id.as_str(), "1");
        // Queue is now empty
        assert!(!token.has_pending());
        // And drain after close returns nothing
        assert!(token.drain().is_empty());
    }

    #[test]
    fn has_pending() {
        let token = ExpandToken::new();
        assert!(!token.has_pending());
        token.inject(vec![make_test_case("1")]);
        assert!(token.has_pending());
        token.drain();
        assert!(!token.has_pending());
    }

    #[test]
    fn drain_clears_queue() {
        let token = ExpandToken::new();
        token.inject(vec![make_test_case("1"), make_test_case("2")]);
        assert_eq!(token.drain().len(), 2);
        assert_eq!(token.drain().len(), 0);
    }
}
