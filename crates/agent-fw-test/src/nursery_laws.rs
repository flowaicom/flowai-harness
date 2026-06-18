//! Nursery algebraic law test harnesses.
//!
//! # Laws
//!
//! - **L1 (Containment):** No spawned task outlives `with_nursery`'s return
//! - **L2 (Propagation):** Parent `cancel` → all child tokens cancelled
//! - **L3 (Error escalation):** First task error cancels all remaining tasks
//! - **L4 (Completion):** `with_nursery` returns only after all tasks are done/cancelled
//! - **L5 (Empty nursery):** Zero spawns → returns body result immediately
//! - **L6 (Body error):** If body returns Err, all tasks are cancelled before returning
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn nursery_satisfies_laws() {
//!     agent_fw_test::nursery_laws::test_all().await;
//! }
//! ```

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::nursery::{with_nursery, NurseryError};
use agent_fw_algebra::CancellationToken;

/// Run all nursery laws.
pub async fn test_all() {
    law_l5_empty_nursery().await;
    law_l1_containment().await;
    law_l2_propagation().await;
    law_l3_error_escalation().await;
    law_l4_completion().await;
    law_l6_body_error().await;
}

/// L5: Zero spawns → returns body result immediately.
pub async fn law_l5_empty_nursery() {
    let cancel = CancellationToken::new();
    let result: Result<i32, NurseryError<String>> =
        with_nursery(&cancel, |_nursery| async { Ok(42) }).await;
    assert_eq!(
        result.unwrap(),
        42,
        "L5: empty nursery should return body result"
    );
}

/// L1: No spawned task outlives with_nursery's return.
pub async fn law_l1_containment() {
    let cancel = CancellationToken::new();
    let completed = Arc::new(AtomicBool::new(false));
    let completed_clone = completed.clone();

    let _result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
        nursery.spawn(move |_cancel| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            completed_clone.store(true, Ordering::SeqCst);
            Ok(())
        });
        async { Ok(()) }
    })
    .await;

    assert!(
        completed.load(Ordering::SeqCst),
        "L1: spawned task must complete before with_nursery returns"
    );
}

/// L2: Parent cancellation propagates to all child tokens.
pub async fn law_l2_propagation() {
    let cancel = CancellationToken::new();
    let child_cancelled = Arc::new(AtomicBool::new(false));
    let child_cancelled_clone = child_cancelled.clone();

    let cancel_clone = cancel.clone();
    let _result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
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

    assert!(
        child_cancelled.load(Ordering::SeqCst),
        "L2: child token must be cancelled when parent is cancelled"
    );
}

/// L3: First task error cancels all remaining tasks.
pub async fn law_l3_error_escalation() {
    let cancel = CancellationToken::new();
    let second_cancelled = Arc::new(AtomicBool::new(false));
    let second_cancelled_clone = second_cancelled.clone();

    let result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
        nursery.spawn(|_cancel| async { Err("first task failed".to_string()) });
        nursery.spawn(move |child_cancel| async move {
            child_cancel.cancelled().await;
            second_cancelled_clone.store(true, Ordering::SeqCst);
            Ok(())
        });
        async { Ok(()) }
    })
    .await;

    assert!(result.is_err(), "L3: should return error from failed task");
    assert!(
        second_cancelled.load(Ordering::SeqCst),
        "L3: second task should be cancelled after first task error"
    );
}

/// L4: with_nursery returns only after all tasks are done/cancelled.
pub async fn law_l4_completion() {
    let cancel = CancellationToken::new();
    let tasks_finished = Arc::new(AtomicU32::new(0));
    let tasks_finished_clone = tasks_finished.clone();

    let _result: Result<(), NurseryError<String>> = with_nursery(&cancel, |nursery| {
        for _ in 0..5 {
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
        5,
        "L4: all 5 tasks must finish before with_nursery returns"
    );
}

/// L6: Body error cancels all tasks.
pub async fn law_l6_body_error() {
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

    assert!(result.is_err(), "L6: should propagate body error");
    assert!(
        child_cancelled.load(Ordering::SeqCst),
        "L6: tasks should be cancelled when body errors"
    );
}
