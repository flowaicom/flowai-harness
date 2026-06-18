//! Channel-backed EventSink implementation.
//!
//! Converts push-based event emission into a pull-based async stream.
//! This bridges the callback-style hooks with Rust's async stream model.
//!
//! # Design
//!
//! Uses `try_send` for non-blocking emission. This is critical because
//! LLM streaming callbacks must not block. If the buffer is full,
//! events are dropped with a warning (backpressure via bounded channel).

use agent_fw_algebra::event_sink::EventSink;
use agent_fw_core::StreamPart;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// EventSink backed by an async channel (non-blocking).
///
/// Events pushed via `emit()` can be consumed as a stream.
/// The channel has a bounded capacity for backpressure.
///
/// # Example
///
/// ```ignore
/// let (sink, stream) = ChannelEventSink::new(100);
///
/// // Producer (non-blocking)
/// sink.emit(StreamPart::text("Hello"));
/// sink.emit(StreamPart::text("World"));
/// sink.close();
///
/// // Consumer
/// while let Some(part) = stream.next().await {
///     println!("{:?}", part);
/// }
/// ```
pub struct ChannelEventSink {
    sender: mpsc::Sender<StreamPart>,
    open: AtomicBool,
    /// Count of events dropped due to buffer full (observability).
    dropped: AtomicU64,
}

impl ChannelEventSink {
    /// Create a new channel-backed sink with the given capacity.
    ///
    /// Returns the sink and a stream receiver.
    pub fn new(capacity: usize) -> (Self, ReceiverStream<StreamPart>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let sink = Self {
            sender,
            open: AtomicBool::new(true),
            dropped: AtomicU64::new(0),
        };
        (sink, ReceiverStream::new(receiver))
    }

    /// Create with default capacity (1024 events).
    pub fn with_default_capacity() -> (Self, ReceiverStream<StreamPart>) {
        Self::new(1024)
    }
}

impl ChannelEventSink {
    /// Number of events dropped due to buffer-full backpressure.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, part: StreamPart) -> bool {
        if !self.is_open() {
            return false;
        }

        // Non-blocking send - critical for LLM streaming callbacks
        match self.sender.try_send(part) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Buffer full — track count for observability
                let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                // Log sparingly: first drop, then powers of 2
                if n == 1 || n.is_power_of_two() {
                    tracing::warn!(dropped = n, "EventSink buffer full, dropping event");
                }
                false // Event was dropped due to buffer full — not accepted
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Channel closed - mark as closed
                self.open.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            tracing::warn!(dropped, "EventSink closed with dropped events");
        }
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Arc;

    #[tokio::test]
    async fn basic_emit_and_receive() {
        let (sink, mut stream) = ChannelEventSink::new(10);

        assert!(sink.emit(StreamPart::text("hello")));
        assert!(sink.emit(StreamPart::text("world")));

        // Drop sink to close the channel
        drop(sink);

        let mut events = vec![];
        while let Some(part) = stream.next().await {
            events.push(part);
        }

        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn close_prevents_emit() {
        let (sink, _stream) = ChannelEventSink::new(10);

        assert!(sink.emit(StreamPart::text("before")));
        sink.close();

        assert!(!sink.emit(StreamPart::text("after")));
    }

    #[tokio::test]
    async fn concurrent_emit() {
        let (sink, stream) = ChannelEventSink::new(100);
        let sink = Arc::new(sink);

        // Spawn multiple producers
        let mut handles = vec![];
        for i in 0..10 {
            let sink = sink.clone();
            handles.push(tokio::spawn(async move {
                sink.emit(StreamPart::text(format!("msg-{}", i)))
            }));
        }

        // Wait for all to complete
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result);
        }

        // Close and drop
        sink.close();
        drop(sink);

        // Count received
        let count = stream.count().await;
        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn buffer_full_tracks_dropped_count() {
        // Capacity of 2 — third emit will be dropped
        let (sink, _stream) = ChannelEventSink::new(2);

        assert!(sink.emit(StreamPart::text("a")));
        assert!(sink.emit(StreamPart::text("b")));
        assert_eq!(sink.dropped_count(), 0);

        // Buffer full — returns false (event not accepted, dropped)
        assert!(!sink.emit(StreamPart::text("c")));
        assert_eq!(sink.dropped_count(), 1);

        assert!(!sink.emit(StreamPart::text("d")));
        assert_eq!(sink.dropped_count(), 2);
    }

    #[tokio::test]
    async fn can_use_as_dyn_trait() {
        let (sink, _stream) = ChannelEventSink::new(10);
        let sink: Arc<dyn EventSink> = Arc::new(sink);

        assert!(sink.emit(StreamPart::text("dynamic")));
        sink.close();
        assert!(!sink.is_open());
    }
}
