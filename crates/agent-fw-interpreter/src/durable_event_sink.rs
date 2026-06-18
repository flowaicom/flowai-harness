//! DurableEventSink — bridges EventSink (fire-and-forget) with EventLog (durable replay).
//!
//! # Design
//!
//! `EventSink` is a half-algebra: write-only, synchronous, no replay.
//! `EventLog` completes the algebra with `replay`, but is async.
//!
//! `DurableEventSink` composes them:
//! - Implements `EventSink` (synchronous `emit`) by serializing the `StreamPart`
//!   to JSON and sending it to a background task that appends to an `EventLog`.
//! - Provides `replay` via the underlying `EventLog`.
//!
//! This enables SSE reconnection: events are emitted in real-time via the sink
//! and replayed from offset on reconnect.
//!
//! # Thread Safety
//!
//! `emit` is non-blocking: it pushes to a bounded channel. If the channel is
//! full, the event is dropped (matching EventSink L3: Non-Blocking).
//! The background task drains the channel and appends to the EventLog.

use agent_fw_algebra::event_log::{EventEntry, EventLog, EventLogError, EventLogExt};
use agent_fw_algebra::EventSink;
use agent_fw_core::StreamPart;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A durable event sink that forwards to an `EventLog` for persistence.
///
/// Implements `EventSink` for real-time emission and provides `replay`
/// for SSE reconnection.
pub struct DurableEventSink {
    /// Channel for sending events to the background appender.
    tx: mpsc::Sender<StreamPart>,
    /// The underlying event log (for replay).
    log: Arc<dyn EventLog>,
    /// Channel name in the event log.
    channel: String,
    /// Whether the sink is open.
    open: AtomicBool,
}

impl DurableEventSink {
    /// Create a new durable event sink.
    ///
    /// - `log`: The underlying EventLog for durable storage.
    /// - `channel`: The channel name to use in the EventLog.
    /// - `buffer_size`: Bounded channel capacity (events dropped when full).
    ///
    /// Spawns a background task to drain the channel and append to the log.
    pub fn new(log: Arc<dyn EventLog>, channel: impl Into<String>, buffer_size: usize) -> Self {
        let channel = channel.into();
        let (tx, rx) = mpsc::channel(buffer_size);

        // Spawn background appender with panic observability via JoinHandle
        let log_clone = log.clone();
        let channel_clone = channel.clone();
        let ch = channel.clone();
        tokio::spawn(async move {
            let handle = tokio::spawn(Self::background_appender(log_clone, channel_clone, rx));
            if let Err(e) = handle.await {
                if e.is_panic() {
                    tracing::error!(channel = %ch, error = %e, "DurableEventSink background appender panicked");
                } else {
                    tracing::warn!(channel = %ch, "DurableEventSink background appender cancelled");
                }
            }
        });

        Self {
            tx,
            log,
            channel,
            open: AtomicBool::new(true),
        }
    }

    /// Create with default buffer size of 1024.
    pub fn with_defaults(log: Arc<dyn EventLog>, channel: impl Into<String>) -> Self {
        Self::new(log, channel, 1024)
    }

    /// Replay events from the underlying EventLog.
    ///
    /// Delegates to `EventLogExt::replay_typed` — the algebra provides the
    /// generic deserialization, so the interpreter doesn't duplicate it.
    pub async fn replay(
        &self,
        from_offset: u64,
    ) -> Result<Vec<EventEntry<StreamPart>>, EventLogError> {
        self.log
            .replay_typed::<StreamPart>(&self.channel, from_offset)
            .await
    }

    /// The channel name.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Background task that drains the channel and appends to the EventLog.
    async fn background_appender(
        log: Arc<dyn EventLog>,
        channel: String,
        mut rx: mpsc::Receiver<StreamPart>,
    ) {
        while let Some(part) = rx.recv().await {
            let json = match serde_json::to_value(&part) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to serialize StreamPart for EventLog");
                    continue;
                }
            };
            if let Err(e) = log.append(&channel, json).await {
                tracing::warn!(error = %e, "Failed to append to EventLog");
                // On Closed error, stop the background task
                if matches!(e, EventLogError::Closed) {
                    break;
                }
            }
        }
    }
}

impl EventSink for DurableEventSink {
    /// Emit a `StreamPart` to the durable event log via a bounded channel.
    ///
    /// # Known Limitation (#8)
    ///
    /// Returns `bool` (inherited from `EventSink` trait) which loses failure
    /// context: `false` can mean either "channel full" or "sink closed".
    /// The ideal fix is to change `EventSink::emit` to return
    /// `Result<(), EmitError>` where `EmitError` distinguishes `Full` from
    /// `Closed`. This requires a trait-level change and is deferred to avoid
    /// a breaking change across all `EventSink` implementations.
    fn emit(&self, part: StreamPart) -> bool {
        if !self.is_open() {
            return false;
        }
        // Non-blocking send via try_send (L3: Non-Blocking)
        match self.tx.try_send(part) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!("DurableEventSink channel full, event dropped");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn close(&self) {
        self.open.store(false, Ordering::Release);
        // Close the underlying log too
        self.log.close();
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryEventLog;
    use std::time::Duration;

    #[tokio::test]
    async fn emit_and_replay_roundtrip() {
        let log = Arc::new(MemoryEventLog::default_ttl());
        let sink = DurableEventSink::new(log.clone(), "test_ch", 16);

        // Emit events
        assert!(sink.emit(StreamPart::text("hello")));
        assert!(sink.emit(StreamPart::text("world")));

        // Give background task time to flush
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Replay from the sink
        let entries = sink.replay(0).await.unwrap();
        assert_eq!(entries.len(), 2);

        // Verify order (L2: causal ordering preserved)
        match &entries[0].event {
            StreamPart::Text { text } => assert_eq!(text, "hello"),
            other => panic!("Expected Text, got {other:?}"),
        }
        match &entries[1].event {
            StreamPart::Text { text } => assert_eq!(text, "world"),
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_prevents_emit() {
        let log = Arc::new(MemoryEventLog::default_ttl());
        let sink = DurableEventSink::new(log.clone(), "close_ch", 16);

        assert!(sink.emit(StreamPart::text("before")));
        sink.close();
        assert!(!sink.emit(StreamPart::text("after")));
        assert!(!sink.is_open());
    }

    #[tokio::test]
    async fn replay_with_offset() {
        let log = Arc::new(MemoryEventLog::default_ttl());
        let sink = DurableEventSink::new(log.clone(), "offset_ch", 16);

        sink.emit(StreamPart::text("a"));
        sink.emit(StreamPart::text("b"));
        sink.emit(StreamPart::text("c"));

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Replay from offset 2 should only return "c"
        let entries = sink.replay(2).await.unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0].event {
            StreamPart::Text { text } => assert_eq!(text, "c"),
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn channel_name_accessible() {
        let log = Arc::new(MemoryEventLog::default_ttl());
        let sink = DurableEventSink::with_defaults(log, "my_channel");
        assert_eq!(sink.channel(), "my_channel");
    }
}
