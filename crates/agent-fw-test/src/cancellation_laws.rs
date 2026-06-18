//! CancellationToken algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Monotonicity: once cancelled, stays cancelled forever
//! - L2. Immediate resolution: cancelled().await resolves immediately when cancelled
//! - L3. Shared state: clones share cancellation state
//! - L4. Child propagation: parent cancel → child cancelled
//! - L5. Idempotence: cancel(); cancel() = cancel()
//! - L6. Default not cancelled: new token is not cancelled
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn cancellation_satisfies_laws() {
//!     agent_fw_test::cancellation_laws::test_all().await;
//! }
//! ```

use agent_fw_algebra::CancellationToken;

/// Run all cancellation token laws.
pub async fn test_all() {
    law_default_not_cancelled();
    law_monotonicity();
    law_shared_state();
    law_child_propagation();
    law_child_does_not_cancel_parent();
    law_idempotence();
    law_immediate_resolution().await;
    law_run_completes_normally().await;
    law_run_returns_none_when_cancelled().await;
}

/// L6: Default not cancelled — new token starts uncancelled.
pub fn law_default_not_cancelled() {
    let token = CancellationToken::new();
    assert!(!token.is_cancelled(), "L6: new token must not be cancelled");
}

/// L1: Monotonicity — once cancelled, stays cancelled.
pub fn law_monotonicity() {
    let token = CancellationToken::new();
    token.cancel();
    assert!(token.is_cancelled(), "L1: must be cancelled after cancel()");
    // Stays cancelled
    assert!(
        token.is_cancelled(),
        "L1: must remain cancelled on subsequent checks"
    );
}

/// L3: Shared state — clones share cancellation state.
pub fn law_shared_state() {
    let token = CancellationToken::new();
    let clone = token.clone();
    assert!(!clone.is_cancelled(), "L3: clone starts uncancelled");
    token.cancel();
    assert!(
        clone.is_cancelled(),
        "L3: clone must be cancelled when original is"
    );
}

/// L4: Child propagation — parent cancel propagates to child.
pub fn law_child_propagation() {
    let parent = CancellationToken::new();
    let child = parent.child();
    assert!(!child.is_cancelled(), "L4: child starts uncancelled");
    parent.cancel();
    assert!(
        child.is_cancelled(),
        "L4: child must be cancelled when parent is"
    );
}

/// Child does not cancel parent (asymmetric propagation).
pub fn law_child_does_not_cancel_parent() {
    let parent = CancellationToken::new();
    let child = parent.child();
    child.cancel();
    assert!(
        !parent.is_cancelled(),
        "child cancellation must not propagate to parent"
    );
    assert!(child.is_cancelled(), "child must be cancelled");
}

/// L5: Idempotence — cancel(); cancel() = cancel().
pub fn law_idempotence() {
    let token = CancellationToken::new();
    token.cancel();
    token.cancel();
    token.cancel();
    assert!(
        token.is_cancelled(),
        "L5: multiple cancel() calls must not fail"
    );
}

/// L2: Immediate resolution — cancelled().await resolves immediately.
pub async fn law_immediate_resolution() {
    let token = CancellationToken::new();
    token.cancel();
    // Must resolve immediately, not hang
    tokio::time::timeout(std::time::Duration::from_millis(100), token.cancelled())
        .await
        .expect("L2: cancelled().await must resolve immediately when already cancelled");
}

/// Run completes normally when not cancelled.
pub async fn law_run_completes_normally() {
    let token = CancellationToken::new();
    let result = token.run(async { 42 }).await;
    assert_eq!(result, Some(42), "run must return Some when not cancelled");
}

/// Run returns None when already cancelled.
pub async fn law_run_returns_none_when_cancelled() {
    let token = CancellationToken::new();
    token.cancel();
    let result = token.run(std::future::pending::<i32>()).await;
    assert_eq!(result, None, "run must return None when cancelled");
}
