//! EventLog algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Append-Replay: `append(e); replay(0)` yields a stream containing `e`.
//! - L2. Causal Ordering: `append(a); append(b); replay(0)` yields `a` before `b`.
//! - L3. Offset-Skip: `replay(n)` skips the first `n` events.
//! - L4. Idempotent-Close: After `close()`, `append` returns `Err(Closed)`.
//!   `replay` still works (the log is durable).
//! - L5. TTL-Bounded: Events older than the log's TTL are not returned by `replay`.
//!
//! # Design
//!
//! Each law receives a **fresh instance** via a factory closure. This eliminates
//! temporal coupling — laws are independent and can run in any order.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_log_satisfies_laws() {
//!     agent_fw_test::event_log_laws::test_all(|| async {
//!         MyEventLog::new(Duration::from_secs(3600))
//!     }).await;
//! }
//! ```

use agent_fw_algebra::event_log::{EventLog, EventLogError};
use std::future::Future;

/// Run all deterministic EventLog laws, giving each law a fresh instance.
///
/// Each law gets its own `EventLog` from the factory, so there is no
/// temporal coupling between laws. They can run in any order.
///
/// L5 (TTL-Bounded) is tested separately because it requires sleeping.
pub async fn test_all<F, Fut, L>(factory: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = L>,
    L: EventLog,
{
    law_append_replay(&factory().await).await;
    law_causal_ordering(&factory().await).await;
    law_offset_skip(&factory().await).await;
    law_channel_isolation(&factory().await).await;
    law_replay_empty_channel(&factory().await).await;
    law_replay_high_offset(&factory().await).await;
    law_monotonic_offsets(&factory().await).await;
    law_idempotent_close(&factory().await).await;
}

/// L1: Append-Replay — append(e); replay(0) contains e.
pub async fn law_append_replay(log: &dyn EventLog) {
    let event = serde_json::json!({"type": "test_l1", "data": 42});
    let offset = log.append("l1_ch", event.clone()).await.unwrap();
    assert_eq!(offset, 0, "L1: first append should return offset 0");

    let entries = log.replay("l1_ch", 0).await.unwrap();
    assert_eq!(entries.len(), 1, "L1: replay(0) should return 1 entry");
    assert_eq!(entries[0].event, event, "L1: replayed event should match");
    assert_eq!(entries[0].offset, 0, "L1: replayed offset should be 0");
}

/// L2: Causal Ordering — append(a); append(b) → replay yields a before b.
pub async fn law_causal_ordering(log: &dyn EventLog) {
    let _ = log.append("l2_ch", serde_json::json!("a")).await.unwrap();
    let _ = log.append("l2_ch", serde_json::json!("b")).await.unwrap();
    let _ = log.append("l2_ch", serde_json::json!("c")).await.unwrap();

    let entries = log.replay("l2_ch", 0).await.unwrap();
    assert_eq!(entries.len(), 3, "L2: should have 3 entries");
    assert_eq!(entries[0].event, serde_json::json!("a"), "L2: first is 'a'");
    assert_eq!(
        entries[1].event,
        serde_json::json!("b"),
        "L2: second is 'b'"
    );
    assert_eq!(entries[2].event, serde_json::json!("c"), "L2: third is 'c'");
}

/// L3: Offset-Skip — replay(2) skips events with offset < 2.
pub async fn law_offset_skip(log: &dyn EventLog) {
    let _ = log.append("l3_ch", serde_json::json!("a")).await.unwrap();
    let _ = log.append("l3_ch", serde_json::json!("b")).await.unwrap();
    let _ = log.append("l3_ch", serde_json::json!("c")).await.unwrap();

    let entries = log.replay("l3_ch", 2).await.unwrap();
    assert_eq!(entries.len(), 1, "L3: replay(2) should return 1 entry");
    assert_eq!(
        entries[0].event,
        serde_json::json!("c"),
        "L3: should be 'c'"
    );
}

/// L4: Idempotent-Close — after close(), append fails; replay still works.
pub async fn law_idempotent_close(log: &dyn EventLog) {
    // Append something first so replay has data
    let _ = log
        .append("l4_ch", serde_json::json!("before_close"))
        .await
        .unwrap();

    // Close is idempotent
    log.close();
    log.close();

    assert!(!log.is_open(), "L4: log should be closed");

    // Append fails
    let result = log.append("l4_ch", serde_json::json!("after_close")).await;
    assert!(
        matches!(result, Err(EventLogError::Closed)),
        "L4: append after close should return Closed, got: {:?}",
        result
    );

    // Replay still works on previously-appended data
    let entries = log.replay("l4_ch", 0).await.unwrap();
    assert!(
        !entries.is_empty(),
        "L4: replay should still work after close"
    );
}

/// Channel isolation — events in channel A don't appear in channel B.
pub async fn law_channel_isolation(log: &dyn EventLog) {
    let _ = log
        .append("iso_a", serde_json::json!("alpha"))
        .await
        .unwrap();
    let _ = log
        .append("iso_b", serde_json::json!("beta"))
        .await
        .unwrap();

    let a = log.replay("iso_a", 0).await.unwrap();
    let b = log.replay("iso_b", 0).await.unwrap();

    assert_eq!(a.len(), 1, "Channel A should have 1 entry");
    assert_eq!(b.len(), 1, "Channel B should have 1 entry");
    assert_eq!(a[0].event, serde_json::json!("alpha"));
    assert_eq!(b[0].event, serde_json::json!("beta"));
}

/// Replay of non-existent channel returns empty vec.
pub async fn law_replay_empty_channel(log: &dyn EventLog) {
    let entries = log.replay("nonexistent_law_test", 0).await.unwrap();
    assert!(
        entries.is_empty(),
        "Replay of empty channel should be empty"
    );
}

/// Replay with offset beyond last event returns empty vec.
pub async fn law_replay_high_offset(log: &dyn EventLog) {
    let _ = log
        .append("high_off_ch", serde_json::json!("x"))
        .await
        .unwrap();
    let entries = log.replay("high_off_ch", 999).await.unwrap();
    assert!(
        entries.is_empty(),
        "Replay with high offset should be empty"
    );
}

/// L5: TTL-Bounded — events older than the log's TTL are not returned by replay.
///
/// NOT included in `test_all()` because it requires sleeping past the log's TTL.
/// Implementers should call this separately with a log configured with a short TTL.
pub async fn law_ttl_bounded(log: &dyn EventLog, ttl: std::time::Duration) {
    let event = serde_json::json!({"type": "ttl_test", "data": "should_expire"});
    let offset = log.append("l5_ttl_ch", event.clone()).await.unwrap();
    assert_eq!(
        offset, 0,
        "L5 TTL-Bounded: first append should return offset 0"
    );

    // Event should be present immediately
    let before = log.replay("l5_ttl_ch", 0).await.unwrap();
    assert_eq!(
        before.len(),
        1,
        "L5 TTL-Bounded: event must be present before TTL expires"
    );

    // Wait past the TTL
    tokio::time::sleep(ttl + std::time::Duration::from_millis(50)).await;

    // Event should be gone
    let after = log.replay("l5_ttl_ch", 0).await.unwrap();
    assert!(
        after.is_empty(),
        "L5 TTL-Bounded: events older than TTL must not be returned by replay, got {} entries",
        after.len()
    );
}

/// Offsets are monotonically increasing within a channel.
pub async fn law_monotonic_offsets(log: &dyn EventLog) {
    let o1 = log.append("mono_ch", serde_json::json!("a")).await.unwrap();
    let o2 = log.append("mono_ch", serde_json::json!("b")).await.unwrap();
    let o3 = log.append("mono_ch", serde_json::json!("c")).await.unwrap();

    assert_eq!(o1, 0, "First offset should be 0");
    assert!(o2 > o1, "Offsets should be monotonically increasing");
    assert!(o3 > o2, "Offsets should be monotonically increasing");
}
