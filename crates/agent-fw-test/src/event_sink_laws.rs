//! EventSink algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Totality: `emit` never panics
//! - L2. Order Preservation: `emit(a); emit(b)` → consumer sees `a` before `b`
//! - L3. Non-Blocking: `emit` returns immediately
//! - L4. Closure Semantics: After `close()`, `emit` returns `false`
//! - L5. Idempotent Close: Multiple calls to `close()` have no additional effect
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn my_sink_satisfies_laws() {
//!     let (sink, receiver) = MySink::new();
//!     agent_fw_test::event_sink_laws::test_all(&sink, receiver);
//! }
//! ```

use agent_fw_algebra::EventSink;
use agent_fw_core::StreamPart;

/// Run all deterministic EventSink laws.
///
/// The `sink` is an open sink. The `events` closure returns events
/// that were collected by the sink (for order verification).
pub fn test_all<F>(sink: &dyn EventSink, collect_events: F)
where
    F: FnOnce() -> Vec<StreamPart>,
{
    law_totality(sink);
    law_order_preservation(sink, collect_events);
    // Sink is now closed from order preservation test
    law_closure_semantics(sink);
    law_idempotent_close(sink);
}

/// L1: Totality — emit never panics, returns bool.
pub fn law_totality(sink: &dyn EventSink) {
    // Emit various event types — none should panic
    let events = vec![
        StreamPart::text("hello"),
        StreamPart::StepStart,
        StreamPart::error("test error"),
    ];
    for event in events {
        let _result = sink.emit(event); // Must not panic
    }
}

/// L4: Closure Semantics — after close(), emit returns false.
///
/// When called standalone (not via `test_all`), pass a fresh open sink
/// to also verify the open→closed transition. When called from `test_all`,
/// the sink may already be closed from L2.
pub fn law_closure_semantics(sink: &dyn EventSink) {
    sink.close();
    assert!(!sink.is_open(), "L4: sink must be closed after close()");
    let result = sink.emit(StreamPart::text("after close"));
    assert!(!result, "L4: emit after close must return false");
}

/// L5: Idempotent Close — multiple close() calls have no additional effect.
pub fn law_idempotent_close(sink: &dyn EventSink) {
    // Sink may already be closed from L4, but call close again
    sink.close();
    sink.close();
    sink.close();
    // Should not panic, and state should remain closed
    assert!(
        !sink.is_open(),
        "L5: sink must remain closed after multiple close() calls"
    );
}

/// Test order preservation with a fresh sink.
///
/// # Arguments
///
/// - `sink`: A fresh, open EventSink
/// - `drain`: A function that drains collected events from the sink
///
/// This emits events in a known order and verifies the drain
/// returns them in the same order.
pub fn law_order_preservation<F>(sink: &dyn EventSink, drain: F)
where
    F: FnOnce() -> Vec<StreamPart>,
{
    // Emit events in known order
    sink.emit(StreamPart::text("first"));
    sink.emit(StreamPart::text("second"));
    sink.emit(StreamPart::text("third"));
    sink.close();

    let events = drain();
    let texts: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            StreamPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        texts.len() >= 3,
        "L2: must receive at least 3 text events, got {}",
        texts.len()
    );
    assert_eq!(texts[0], "first", "L2: first event must be 'first'");
    assert_eq!(texts[1], "second", "L2: second event must be 'second'");
    assert_eq!(texts[2], "third", "L2: third event must be 'third'");
}

/// L2 (concurrent): Order preservation under concurrent emission.
///
/// Emits tagged events from N concurrent tasks and verifies that
/// events from each source appear in monotonically increasing order
/// in the drain output.
///
/// # Arguments
///
/// - `make_sink`: Factory that creates a fresh (sink, drain) pair.
///   The drain closure must return all events collected by the sink.
/// - `num_tasks`: Number of concurrent emitter tasks.
/// - `events_per_task`: Number of events each task emits.
pub fn law_concurrent_order_preservation<F, S, D>(
    make_sink: F,
    num_tasks: usize,
    events_per_task: usize,
) where
    F: FnOnce() -> (S, D),
    S: EventSink + 'static,
    D: FnOnce() -> Vec<StreamPart> + Send + 'static,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        let (sink, drain) = make_sink();
        let sink = std::sync::Arc::new(sink);

        let mut handles = Vec::new();

        for task_id in 0..num_tasks {
            let sink = sink.clone();
            handles.push(tokio::spawn(async move {
                for seq in 0..events_per_task {
                    // Tag: "task_id:seq" so we can verify per-source ordering
                    let tag = format!("{}:{}", task_id, seq);
                    sink.emit(StreamPart::text(tag));
                }
            }));
        }

        for handle in handles {
            handle.await.expect("task panicked");
        }

        sink.close();

        let events = drain();
        let texts: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                StreamPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        // Verify per-source monotonic ordering
        let mut last_seq_per_task: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();

        for tag in &texts {
            let parts: Vec<&str> = tag.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            let task_id: usize = parts[0].parse().expect("bad task_id");
            let seq: usize = parts[1].parse().expect("bad seq");

            if let Some(&prev_seq) = last_seq_per_task.get(&task_id) {
                assert!(
                    seq > prev_seq,
                    "L2 concurrent: task {} events out of order: saw seq {} after {}",
                    task_id,
                    seq,
                    prev_seq,
                );
            }
            last_seq_per_task.insert(task_id, seq);
        }

        // Verify we received all events
        let expected_total = num_tasks * events_per_task;
        assert_eq!(
            texts.len(),
            expected_total,
            "L2 concurrent: expected {} events, got {}",
            expected_total,
            texts.len(),
        );
    });
}
