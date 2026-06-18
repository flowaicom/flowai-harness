//! PipelineCtx algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1 (Cancellation): If token is cancelled before stage, stage is skipped (None)
//! - L2 (Progress): Every stage transition emits exactly one progress event
//! - L3 (Completion): Cancel mid-pipeline skips remaining stages
//! - L4 (Disconnect): Client disconnect (receiver dropped) returns None
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn pipeline_ctx_satisfies_all_laws() {
//!     let make_kv = || Arc::new(DashMapKVStore::new()) as Arc<dyn KVStore>;
//!     agent_fw_test::pipeline_ctx_laws::test_all(make_kv).await;
//! }
//! ```

use std::sync::Arc;

use agent_fw_algebra::{CancellationToken, KVStore, PipelineCtx};
use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
enum TestStatus {
    Step1,
    Step2,
    Done,
}

/// Run all PipelineCtx laws, parameterized over a KVStore supplier.
pub async fn test_all(make_kv: impl Fn() -> Arc<dyn KVStore>) {
    law_cancelled_skips_stage(&make_kv).await;
    law_progress_per_stage(&make_kv).await;
    law_cancel_mid_pipeline(&make_kv).await;
    law_disconnect_returns_none(&make_kv).await;
    law_stage_propagates_error(&make_kv).await;
    law_is_alive_reflects_state(&make_kv).await;
}

/// L1: If token is cancelled before stage, stage is skipped (None).
pub async fn law_cancelled_skips_stage(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    let cancel = CancellationToken::new();
    cancel.cancel();

    let (tx, _rx) = mpsc::channel(16);
    let ctx = PipelineCtx::new("tenant", "status-key", make_kv(), cancel, tx);

    let result = ctx
        .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
        .await;

    assert!(
        result.is_none(),
        "L1: cancelled token must skip stage (return None)"
    );
}

/// L2: Every stage transition emits exactly one progress event.
pub async fn law_progress_per_stage(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(16);
    let ctx = PipelineCtx::new("tenant", "status-key", make_kv(), cancel, tx);

    let result = ctx
        .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
        .await;

    assert_eq!(
        result,
        Some(Ok(42)),
        "L2: stage should complete successfully"
    );

    let event = rx.try_recv().expect("L2: must receive a progress event");
    assert_eq!(
        event,
        TestStatus::Step1,
        "L2: event must match the stage status"
    );

    // No extra events
    assert!(
        rx.try_recv().is_err(),
        "L2: must emit exactly one event per stage"
    );
}

/// L3: Cancel mid-pipeline skips remaining stages.
pub async fn law_cancel_mid_pipeline(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(16);
    let ctx = PipelineCtx::new("tenant", "key", make_kv(), cancel.clone(), tx);

    // Stage 1 succeeds
    let r1 = ctx
        .run_stage(TestStatus::Step1, || async { Ok::<_, String>(1) })
        .await;
    assert_eq!(r1, Some(Ok(1)), "L3: first stage should succeed");

    // Cancel before stage 2
    cancel.cancel();

    let r2 = ctx
        .run_stage(TestStatus::Done, || async { Ok::<_, String>(2) })
        .await;
    assert!(r2.is_none(), "L3: cancelled stage must return None");

    // Only Step1 event should have been emitted
    let event = rx.try_recv().expect("L3: must receive Step1 event");
    assert_eq!(event, TestStatus::Step1);
    assert!(
        rx.try_recv().is_err(),
        "L3: cancelled stage must not emit progress event"
    );
}

/// L4: Client disconnect (receiver dropped) returns None.
pub async fn law_disconnect_returns_none(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel(16);
    let ctx = PipelineCtx::new("tenant", "status-key", make_kv(), cancel, tx);

    // Drop the receiver to simulate client disconnect
    drop(rx);

    let result = ctx
        .run_stage(TestStatus::Step1, || async { Ok::<_, String>(42) })
        .await;

    assert!(result.is_none(), "L4: disconnect must return None");
}

/// Stage propagates error when op fails.
pub async fn law_stage_propagates_error(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    let cancel = CancellationToken::new();
    let (tx, _rx) = mpsc::channel(16);
    let ctx = PipelineCtx::new("tenant", "status-key", make_kv(), cancel, tx);

    let result = ctx
        .run_stage(TestStatus::Step1, || async {
            Err::<i32, _>("stage failed".to_string())
        })
        .await;

    assert_eq!(
        result,
        Some(Err("stage failed".to_string())),
        "stage error must be propagated"
    );
}

/// is_alive reflects cancellation and disconnect state.
pub async fn law_is_alive_reflects_state(make_kv: &impl Fn() -> Arc<dyn KVStore>) {
    // Initially alive
    let cancel = CancellationToken::new();
    let (tx, _rx) = mpsc::channel::<TestStatus>(16);
    let ctx = PipelineCtx::new("tenant", "status-key", make_kv(), cancel.clone(), tx);
    assert!(ctx.is_alive(), "is_alive: must be true initially");

    // After cancel
    cancel.cancel();
    assert!(!ctx.is_alive(), "is_alive: must be false after cancel");

    // After disconnect
    let cancel2 = CancellationToken::new();
    let (tx2, rx2) = mpsc::channel::<TestStatus>(16);
    let ctx2 = PipelineCtx::new("tenant", "status-key", make_kv(), cancel2, tx2);
    drop(rx2);
    assert!(!ctx2.is_alive(), "is_alive: must be false after disconnect");
}
