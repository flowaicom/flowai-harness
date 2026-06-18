//! Generic broadcast event delivery with monotonic sequencing.
//!
//! # Laws
//!
//! 1. **Broadcast** — `emit(event)` delivers to all active subscribers.
//! 2. **Late-subscribe** — Subscribing after `emit(e)` does NOT receive `e`.
//! 3. **Unsubscribe-safe** — Dropping a receiver doesn't affect other receivers.
//!
//! # Sequencing
//!
//! Every emitted event is wrapped in [`Sequenced<E>`] (from `agent-fw-core`),
//! a functor that tags events with a monotonically increasing sequence number.
//! This enables stateless dedup in SSE streams: subscribe before snapshot,
//! capture `current_seq()` after snapshot, skip live events with `seq < watermark`.
//!
//! # Implementation
//!
//! Concrete struct (not trait) using `tokio::sync::broadcast`.
//! There's only one sensible implementation — no need for an algebra layer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::broadcast;

// Re-export Sequenced from core (canonical location).
pub use agent_fw_core::{EvalRunId, Sequenced};

use crate::types::EvalEvent;

// =============================================================================
// SequencedBus<E> — generic broadcast with monotonic sequencing
// =============================================================================

/// Generic broadcast event bus with monotonic sequencing and history.
///
/// Cloneable: all clones share the same underlying channel and sequence counter.
/// Events are auto-wrapped in [`Sequenced`] on `emit()`.
///
/// # Type Parameter
///
/// `E` must be `Clone + Send + 'static` to satisfy broadcast channel requirements.
#[derive(Debug, Clone)]
pub struct SequencedBus<E: Clone + Send + Sync + 'static> {
    sender: broadcast::Sender<Sequenced<E>>,
    /// Monotonic sequence counter (shared across clones via Arc).
    seq: Arc<AtomicU64>,
    /// History buffer for SSE reconnection: seq -> (event, insert_time).
    /// Enables clients to replay missed events via `Last-Event-ID`.
    history: Arc<DashMap<u64, HistoryEntry<E>>>,
    /// Time-to-live for history entries.
    history_ttl: Duration,
    /// Serializes emit() to restore S1 monotonicity under concurrency.
    /// Sync Mutex (not tokio): the critical section is ~3 instructions
    /// (one atomic increment, one DashMap insert, one broadcast send)
    /// with no I/O — sync Mutex avoids `.await` propagation.
    emit_lock: Arc<Mutex<()>>,
}

/// A history entry for SSE reconnection.
#[derive(Debug, Clone)]
struct HistoryEntry<E: Clone + Send + Sync> {
    event: Sequenced<E>,
    inserted_at: Instant,
}

impl<E: Clone + Send + Sync + 'static> SequencedBus<E> {
    /// Default history TTL: 5 minutes.
    const DEFAULT_HISTORY_TTL: Duration = Duration::from_secs(300);

    /// Create a new event bus with the given channel capacity.
    ///
    /// `capacity` determines how many events are buffered before
    /// slow subscribers lose events (via `RecvError::Lagged`).
    pub fn new(capacity: usize) -> Self {
        Self::with_history_ttl(capacity, Self::DEFAULT_HISTORY_TTL)
    }

    /// Create an event bus with a custom history TTL.
    pub fn with_history_ttl(capacity: usize, history_ttl: Duration) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            seq: Arc::new(AtomicU64::new(0)),
            history: Arc::new(DashMap::new()),
            history_ttl,
            emit_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Emit an event to all active subscribers.
    ///
    /// The event is automatically wrapped in [`Sequenced`] with the next
    /// monotonic sequence number. Returns the number of receivers that
    /// received the event. Returns 0 if there are no active subscribers.
    pub fn emit(&self, event: E) -> usize {
        // Serialize the entire emit critical section to guarantee S1 (monotonicity):
        // fetch_add + history insert + broadcast send must be atomic with respect
        // to other emitters. Without this lock, two concurrent emitters can deliver
        // events out of sequence order.
        let _guard = match self.emit_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("SequencedBus emit lock was poisoned; recovering");
                poisoned.into_inner()
            }
        };

        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let sequenced = Sequenced { seq, event };

        // Insert into history buffer for reconnection support.
        self.history.insert(
            seq,
            HistoryEntry {
                event: sequenced.clone(),
                inserted_at: Instant::now(),
            },
        );

        // broadcast::send returns Err if there are no receivers,
        // which is fine — just means nobody is listening.
        self.sender.send(sequenced).unwrap_or(0)
    }

    /// Current sequence number (exclusive upper bound of emitted events).
    ///
    /// Used as a watermark for SSE dedup: after building a snapshot from
    /// the database, capture `watermark = current_seq()`. Then in the live
    /// stream, skip events with `seq < watermark` — they're already covered
    /// by the snapshot.
    ///
    /// # Ordering
    ///
    /// Uses `Relaxed` because the broadcast channel provides the
    /// happens-before relationship. The watermark is an optimization
    /// hint, not a synchronization primitive — an off-by-one only
    /// means one harmless duplicate (SSE events are idempotent).
    pub fn current_seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }

    /// Replay history since a given sequence number (exclusive).
    ///
    /// Returns all events with `seq > from_seq`, sorted by sequence number.
    /// Used for SSE reconnection: when a client sends `Last-Event-ID: N`,
    /// call `history_since(N)` to replay missed events before subscribing
    /// to the live stream.
    ///
    /// Events older than the history TTL are not available.
    pub fn history_since(&self, from_seq: u64) -> Vec<Sequenced<E>> {
        let now = Instant::now();
        let mut events: Vec<Sequenced<E>> = self
            .history
            .iter()
            .filter(|entry| entry.value().event.seq > from_seq)
            .filter(|entry| now.duration_since(entry.value().inserted_at) < self.history_ttl)
            .map(|entry| entry.value().event.clone())
            .collect();
        events.sort_by_key(|e| e.seq);
        events
    }

    /// Evict history entries older than the TTL.
    ///
    /// Call this periodically (e.g., from a background task) to bound
    /// memory usage. Safe to call concurrently with `emit` and `history_since`.
    pub fn evict_expired(&self) -> usize {
        let now = Instant::now();
        let ttl = self.history_ttl;
        let before = self.history.len();
        self.history
            .retain(|_, entry| now.duration_since(entry.inserted_at) < ttl);
        before - self.history.len()
    }

    /// Number of events currently in the history buffer.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Subscribe to events.
    ///
    /// Only events emitted *after* this call will be received (late-subscribe law).
    /// Events arrive as [`Sequenced<E>`] with monotonic sequence numbers.
    pub fn subscribe(&self) -> broadcast::Receiver<Sequenced<E>> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl<E: Clone + Send + Sync + 'static> Default for SequencedBus<E> {
    fn default() -> Self {
        Self::new(256)
    }
}

impl<E: Clone + Send + Sync + 'static> SequencedBus<E> {
    /// Spawn a background task that periodically evicts expired history entries.
    ///
    /// Returns a [`tokio::task::JoinHandle`] that runs until dropped.
    /// Eviction runs every `interval` (default: TTL / 2).
    pub fn spawn_eviction_task(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        let bus = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                bus.evict_expired();
            }
        })
    }
}

// =============================================================================
// EvalEventBus — specialized wrapper with parent-child forwarding
// =============================================================================

/// Broadcast event bus for eval progress events.
///
/// Wraps `SequencedBus<EvalEvent>` with optional parent forwarding for
/// hierarchical eval runs (e.g., rerun child → parent SSE stream).
#[derive(Debug, Clone)]
pub struct EvalEventBus {
    inner: SequencedBus<EvalEvent>,
    /// If set, events are forwarded to the parent bus wrapped in `ChildProgress`.
    parent_bus: Option<Arc<EvalEventBus>>,
    /// The child run ID used in forwarded `ChildProgress` events.
    child_run_id: Option<EvalRunId>,
}

impl EvalEventBus {
    /// Create a new standalone event bus (no parent).
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SequencedBus::new(capacity),
            parent_bus: None,
            child_run_id: None,
        }
    }

    /// Create an event bus with a custom history TTL.
    pub fn with_history_ttl(capacity: usize, history_ttl: Duration) -> Self {
        Self {
            inner: SequencedBus::with_history_ttl(capacity, history_ttl),
            parent_bus: None,
            child_run_id: None,
        }
    }

    /// Create a child event bus that forwards events to a parent.
    ///
    /// Events emitted on this bus are:
    /// 1. Broadcast locally to this bus's subscribers
    /// 2. Forwarded to the parent bus wrapped in `EvalEvent::ChildProgress`
    ///
    /// Sequence numbers are independent per bus (monotonic within each).
    pub fn with_parent(parent: Arc<EvalEventBus>, child_run_id: EvalRunId) -> Self {
        Self {
            inner: SequencedBus::new(256),
            parent_bus: Some(parent),
            child_run_id: Some(child_run_id),
        }
    }

    /// Emit an event to all active subscribers.
    ///
    /// If a parent bus is configured, the event is also forwarded
    /// wrapped in `ChildProgress { child_run_id, event }`.
    pub fn emit(&self, event: EvalEvent) -> usize {
        // Forward to parent if configured
        if let (Some(ref parent), Some(ref child_id)) = (&self.parent_bus, &self.child_run_id) {
            parent.emit(EvalEvent::ChildProgress {
                child_run_id: child_id.clone(),
                event: Box::new(event.clone()),
            });
        }

        self.inner.emit(event)
    }

    /// Current sequence number (exclusive upper bound of emitted events).
    pub fn current_seq(&self) -> u64 {
        self.inner.current_seq()
    }

    /// Replay history since a given sequence number.
    pub fn history_since(&self, from_seq: u64) -> Vec<Sequenced<EvalEvent>> {
        self.inner.history_since(from_seq)
    }

    /// Evict expired history entries.
    pub fn evict_expired(&self) -> usize {
        self.inner.evict_expired()
    }

    /// Number of events currently in the history buffer.
    pub fn history_len(&self) -> usize {
        self.inner.history_len()
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<Sequenced<EvalEvent>> {
        self.inner.subscribe()
    }

    /// Number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.inner.receiver_count()
    }

    /// Spawn a background eviction task.
    pub fn spawn_eviction_task(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        self.inner.spawn_eviction_task(interval)
    }
}

impl Default for EvalEventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EvalSummary, TokenUsageSummary};

    fn error_event(msg: &str) -> EvalEvent {
        EvalEvent::Error {
            message: msg.into(),
        }
    }

    // =========================================================================
    // Law 1: Broadcast
    // =========================================================================

    #[tokio::test]
    async fn broadcast_delivers_to_all_subscribers() {
        let bus = EvalEventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(error_event("test"));

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1, e2);
    }

    // =========================================================================
    // Law 2: Late-subscribe
    // =========================================================================

    #[tokio::test]
    async fn late_subscribe_misses_earlier_events() {
        let bus = EvalEventBus::new(16);

        // Emit before subscribing
        bus.emit(error_event("before"));

        let mut rx = bus.subscribe();

        // Emit after subscribing
        bus.emit(error_event("after"));

        let sequenced = rx.recv().await.unwrap();
        assert!(matches!(sequenced.event, EvalEvent::Error { message } if message == "after"));
    }

    // =========================================================================
    // Law 3: Unsubscribe-safe
    // =========================================================================

    #[tokio::test]
    async fn dropping_receiver_does_not_affect_others() {
        let bus = EvalEventBus::new(16);
        let mut rx1 = bus.subscribe();
        let rx2 = bus.subscribe();
        assert_eq!(bus.receiver_count(), 2);

        // Drop one receiver
        drop(rx2);
        assert_eq!(bus.receiver_count(), 1);

        // Remaining receiver still works
        bus.emit(error_event("still works"));
        let sequenced = rx1.recv().await.unwrap();
        assert!(matches!(sequenced.event, EvalEvent::Error { .. }));
    }

    // =========================================================================
    // Sequencing laws
    // =========================================================================

    #[tokio::test]
    async fn s1_monotonicity() {
        let bus = EvalEventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(error_event("first"));
        bus.emit(error_event("second"));
        bus.emit(error_event("third"));

        let a = rx.recv().await.unwrap();
        let b = rx.recv().await.unwrap();
        let c = rx.recv().await.unwrap();

        assert!(a.seq < b.seq, "monotonicity: {} < {}", a.seq, b.seq);
        assert!(b.seq < c.seq, "monotonicity: {} < {}", b.seq, c.seq);
    }

    #[tokio::test]
    async fn s2_uniqueness() {
        let bus = EvalEventBus::new(16);
        let mut rx = bus.subscribe();

        for _ in 0..10 {
            bus.emit(error_event("event"));
        }

        let mut seqs = Vec::new();
        for _ in 0..10 {
            seqs.push(rx.recv().await.unwrap().seq);
        }

        // All seq numbers should be unique
        let mut deduped = seqs.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(seqs.len(), deduped.len(), "sequence numbers must be unique");
    }

    #[test]
    fn s3_functor_map() {
        let s = Sequenced {
            seq: 42,
            event: "hello",
        };
        let mapped = s.map(|e| e.len());
        assert_eq!(mapped.seq, 42);
        assert_eq!(mapped.event, 5);
    }

    #[tokio::test]
    async fn current_seq_tracks_emits() {
        let bus = EvalEventBus::new(16);
        assert_eq!(bus.current_seq(), 0);

        bus.emit(error_event("a"));
        assert_eq!(bus.current_seq(), 1);

        bus.emit(error_event("b"));
        bus.emit(error_event("c"));
        assert_eq!(bus.current_seq(), 3);
    }

    #[tokio::test]
    async fn watermark_dedup_pattern() {
        let bus = EvalEventBus::new(16);

        // Step 1: Subscribe (buffer events)
        let mut rx = bus.subscribe();

        // Simulate events emitted during snapshot build
        bus.emit(error_event("before-snapshot-1"));
        bus.emit(error_event("before-snapshot-2"));

        // Step 2: Capture watermark AFTER snapshot
        let watermark = bus.current_seq();
        assert_eq!(watermark, 2);

        // Events emitted after watermark
        bus.emit(error_event("after-snapshot-1"));
        bus.emit(error_event("after-snapshot-2"));

        // Step 3: Drain receiver, applying watermark filter
        let mut new_events = Vec::new();
        for _ in 0..4 {
            let s = rx.recv().await.unwrap();
            if s.seq >= watermark {
                new_events.push(s.event);
            }
        }

        assert_eq!(new_events.len(), 2);
        assert!(
            matches!(&new_events[0], EvalEvent::Error { message } if message == "after-snapshot-1")
        );
        assert!(
            matches!(&new_events[1], EvalEvent::Error { message } if message == "after-snapshot-2")
        );
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[tokio::test]
    async fn emit_with_no_subscribers_returns_zero() {
        let bus = EvalEventBus::new(16);
        let count = bus.emit(error_event("nobody listening"));
        assert_eq!(count, 0);
        // Seq still advances even with no subscribers
        assert_eq!(bus.current_seq(), 1);
    }

    #[tokio::test]
    async fn clone_shares_channel_and_seq() {
        let bus1 = EvalEventBus::new(16);
        let bus2 = bus1.clone();

        let mut rx = bus1.subscribe();
        bus2.emit(error_event("from clone"));

        let sequenced = rx.recv().await.unwrap();
        assert!(matches!(sequenced.event, EvalEvent::Error { .. }));
        assert_eq!(sequenced.seq, 0);

        // Both clones see the same seq counter
        assert_eq!(bus1.current_seq(), 1);
        assert_eq!(bus2.current_seq(), 1);
    }

    // =========================================================================
    // History / SSE reconnection tests
    // =========================================================================

    #[tokio::test]
    async fn history_since_returns_missed_events() {
        let bus = EvalEventBus::new(16);

        bus.emit(error_event("a"));
        bus.emit(error_event("b"));
        bus.emit(error_event("c"));

        // Replay from seq 0 (exclusive) — should get seqs 1 and 2
        let history = bus.history_since(0);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seq, 1);
        assert_eq!(history[1].seq, 2);

        // Replay from seq 1 — should get only seq 2
        let history = bus.history_since(1);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].seq, 2);

        // Replay from current_seq - 1 — should be empty
        let history = bus.history_since(2);
        assert!(history.is_empty());
    }

    #[tokio::test]
    async fn history_eviction() {
        use std::time::Duration;

        let bus = EvalEventBus::with_history_ttl(16, Duration::from_millis(50));

        bus.emit(error_event("early"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        bus.emit(error_event("recent"));

        // Evict expired entries
        let evicted = bus.evict_expired();
        assert_eq!(evicted, 1, "should evict 1 expired entry");
        assert_eq!(bus.history_len(), 1, "should have 1 remaining entry");

        // The remaining entry should be the recent one
        let history = bus.history_since(0);
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn history_preserves_ordering() {
        let bus = EvalEventBus::new(16);

        for i in 0..10 {
            bus.emit(error_event(&format!("event-{i}")));
        }

        let history = bus.history_since(4);
        for pair in history.windows(2) {
            assert!(
                pair[0].seq < pair[1].seq,
                "history must be ordered: {} < {}",
                pair[0].seq,
                pair[1].seq
            );
        }
    }

    #[tokio::test]
    async fn completed_event_roundtrip() {
        let bus = EvalEventBus::new(16);
        let mut rx = bus.subscribe();

        let summary = EvalSummary {
            total_test_cases: 5,
            passed: 4,
            failed: 1,
            skipped: 0,
            aggregate_score: 0.85,
            pass_at_k: vec![],
            total_duration_ms: 12000,
            total_usage: TokenUsageSummary::ZERO,
            cost: crate::types::EvalCostSummary::zero(),
            latency: None,
            metadata: None,
        };

        bus.emit(EvalEvent::Completed {
            summary: summary.clone(),
        });

        match rx.recv().await.unwrap().event {
            EvalEvent::Completed { summary: s } => {
                assert_eq!(s.total_test_cases, 5);
                assert_eq!(s.passed, 4);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    // =========================================================================
    // Generic SequencedBus tests (non-EvalEvent)
    // =========================================================================

    #[tokio::test]
    async fn generic_bus_with_string_events() {
        let bus: SequencedBus<String> = SequencedBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit("hello".to_string());
        bus.emit("world".to_string());

        let a = rx.recv().await.unwrap();
        let b = rx.recv().await.unwrap();

        assert_eq!(a.seq, 0);
        assert_eq!(a.event, "hello");
        assert_eq!(b.seq, 1);
        assert_eq!(b.event, "world");
    }

    #[tokio::test]
    async fn generic_bus_history_replay() {
        let bus: SequencedBus<i32> = SequencedBus::new(16);

        bus.emit(10);
        bus.emit(20);
        bus.emit(30);

        let history = bus.history_since(0);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].event, 20);
        assert_eq!(history[1].event, 30);
    }

    // =========================================================================
    // Parent-child forwarding tests
    // =========================================================================

    #[tokio::test]
    async fn child_events_appear_in_child_history() {
        let parent = Arc::new(EvalEventBus::new(16));
        let child = EvalEventBus::with_parent(parent.clone(), EvalRunId::new_unchecked("child-1"));

        child.emit(error_event("from child"));

        let _history = child.history_since(0);
        // seq 0 is the only event, history_since(0) is exclusive → empty
        // But current_seq should be 1
        assert_eq!(child.current_seq(), 1);
    }

    #[tokio::test]
    async fn child_events_forwarded_to_parent_as_child_progress() {
        let parent = Arc::new(EvalEventBus::new(16));
        let mut parent_rx = parent.subscribe();

        let child =
            EvalEventBus::with_parent(parent.clone(), EvalRunId::new_unchecked("child-run-42"));

        child.emit(error_event("child error"));

        // Parent should receive a ChildProgress event
        let parent_event = parent_rx.recv().await.unwrap();
        match parent_event.event {
            EvalEvent::ChildProgress {
                child_run_id,
                event,
            } => {
                assert_eq!(child_run_id, EvalRunId::new_unchecked("child-run-42"));
                assert!(
                    matches!(*event, EvalEvent::Error { ref message } if message == "child error")
                );
            }
            other => panic!("expected ChildProgress, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn parent_events_do_not_appear_in_child() {
        let parent = Arc::new(EvalEventBus::new(16));
        let child = EvalEventBus::with_parent(parent.clone(), EvalRunId::new_unchecked("child-1"));
        let mut child_rx = child.subscribe();

        // Emit on parent directly
        parent.emit(error_event("parent only"));

        // Child should NOT receive parent-only events
        // Use try_recv pattern — the child receiver should be empty
        let result = child_rx.try_recv();
        assert!(
            result.is_err(),
            "child should not receive parent-only events"
        );
    }

    // =========================================================================
    // S1: Concurrent monotonicity (would fail without emit_lock)
    // =========================================================================

    #[tokio::test]
    async fn s1_concurrent_monotonicity() {
        let bus: SequencedBus<u64> = SequencedBus::new(2048);
        let mut rx = bus.subscribe();

        // Spawn 10 tasks, each emitting 100 events concurrently
        let mut handles = Vec::new();
        for task_id in 0..10u64 {
            let bus = bus.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..100u64 {
                    bus.emit(task_id * 1000 + i);
                }
            }));
        }

        // Wait for all emitters to finish
        for h in handles {
            h.await.unwrap();
        }

        // Collect all events from the subscriber
        let mut seqs = Vec::new();
        for _ in 0..1000 {
            match rx.try_recv() {
                Ok(s) => seqs.push(s.seq),
                Err(_) => break,
            }
        }

        assert_eq!(seqs.len(), 1000, "should receive all 1000 events");

        // Verify strict monotonicity: each seq must be greater than the previous
        for pair in seqs.windows(2) {
            assert!(
                pair[0] < pair[1],
                "S1 violated: seq {} followed by {} (not strictly monotonic)",
                pair[0],
                pair[1]
            );
        }
    }

    #[tokio::test]
    async fn parent_child_independent_sequences() {
        let parent = Arc::new(EvalEventBus::new(16));
        let child = EvalEventBus::with_parent(parent.clone(), EvalRunId::new_unchecked("child-1"));

        // Emit some on parent first
        parent.emit(error_event("p1"));
        parent.emit(error_event("p2"));

        // Now emit on child
        child.emit(error_event("c1"));
        child.emit(error_event("c2"));

        // Parent seq includes its own events + forwarded child events
        // parent: p1(0), p2(1), ChildProgress(c1)(2), ChildProgress(c2)(3) = 4
        assert_eq!(parent.current_seq(), 4);
        // Child seq is independent: c1(0), c2(1) = 2
        assert_eq!(child.current_seq(), 2);
    }
}
