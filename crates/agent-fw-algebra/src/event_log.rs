//! EventLog algebra — durable event stream with replay.
//!
//! Extends the fire-and-forget [`EventSink`](super::event_sink::EventSink) with
//! persistence and replay capabilities needed for SSE reconnection.
//!
//! # Laws
//!
//! - **L1 (Append-Replay)**: `append(e); replay(0)` yields a stream containing `e`.
//! - **L2 (Causal Ordering)**: `append(a); append(b); replay(0)` yields `a` before `b`.
//! - **L3 (Offset-Skip)**: `replay(n)` skips the first `n` events.
//! - **L4 (Idempotent-Close)**: After `close()`, `append` returns `false`.
//!   `replay` still works (the log is durable).
//! - **L5 (TTL-Bounded)**: Events older than the log's TTL are not returned by `replay`.
//!
//! # Design
//!
//! `EventSink` is a half-algebra: it has the write side but not the read side.
//! `EventLog` completes the algebra with `replay`, making it suitable for
//! durable streaming with reconnection support.

use async_trait::async_trait;
use std::time::Duration;

/// An event entry in the log, carrying the payload and a monotonic offset.
#[derive(Debug, Clone)]
pub struct EventEntry<E> {
    /// Monotonically increasing offset (0-based).
    pub offset: u64,
    /// The event payload.
    pub event: E,
    /// Wall-clock time when the event was appended.
    pub timestamp: std::time::SystemTime,
}

/// Errors from event log operations.
///
/// Serialization and deserialization errors are distinct from storage errors:
/// storage errors are transient (disk full, network timeout) and potentially
/// retryable; serde errors are deterministic and permanent.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventLogError {
    #[error("event log storage error: {0}")]
    Storage(String),
    #[error("event serialization error: {0}")]
    Serialization(String),
    #[error("event deserialization error: {0}")]
    Deserialization(String),
    #[error("event log closed")]
    Closed,
}

/// Durable event stream with replay (async, object-safe).
///
/// Generic over the event type `E` which must be serializable for storage.
///
/// # Object Safety
///
/// This trait uses `serde_json::Value` internally for storage.
/// The typed `EventLogExt` trait provides generic convenience methods.
#[async_trait]
pub trait EventLog: Send + Sync {
    /// Append an event to the log.
    ///
    /// Returns the assigned offset, or `Err(Closed)` if the log is closed.
    async fn append(&self, channel: &str, event: serde_json::Value) -> Result<u64, EventLogError>;

    /// Replay events from the given offset (inclusive).
    ///
    /// Returns events in causal order. Events past TTL are excluded.
    async fn replay(
        &self,
        channel: &str,
        from_offset: u64,
    ) -> Result<Vec<EventEntry<serde_json::Value>>, EventLogError>;

    /// Close the log, preventing further appends.
    ///
    /// Existing events remain available for replay.
    fn close(&self);

    /// Check if the log is still accepting appends.
    fn is_open(&self) -> bool;

    /// The TTL for events in this log.
    ///
    /// Events older than this duration may be pruned.
    fn ttl(&self) -> Duration;
}

/// Extension trait with typed convenience methods for EventLog.
#[async_trait]
pub trait EventLogExt: EventLog {
    /// Append a typed event (serialized to JSON).
    async fn append_typed<E: serde::Serialize + Send + Sync>(
        &self,
        channel: &str,
        event: &E,
    ) -> Result<u64, EventLogError> {
        let json =
            serde_json::to_value(event).map_err(|e| EventLogError::Serialization(e.to_string()))?;
        self.append(channel, json).await
    }

    /// Replay and deserialize events into a typed stream.
    async fn replay_typed<E: serde::de::DeserializeOwned + Send>(
        &self,
        channel: &str,
        from_offset: u64,
    ) -> Result<Vec<EventEntry<E>>, EventLogError> {
        let entries = self.replay(channel, from_offset).await?;
        entries
            .into_iter()
            .map(|entry| {
                let event: E = serde_json::from_value(entry.event)
                    .map_err(|e| EventLogError::Deserialization(e.to_string()))?;
                Ok(EventEntry {
                    offset: entry.offset,
                    event,
                    timestamp: entry.timestamp,
                })
            })
            .collect()
    }
}

#[async_trait]
impl<T: EventLog + ?Sized> EventLogExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_entry_fields() {
        let entry = EventEntry {
            offset: 42,
            event: serde_json::json!({"type": "test"}),
            timestamp: std::time::SystemTime::now(),
        };
        assert_eq!(entry.offset, 42);
    }

    #[test]
    fn event_log_error_display() {
        let e = EventLogError::Storage("disk full".into());
        assert_eq!(e.to_string(), "event log storage error: disk full");

        let e = EventLogError::Serialization("bad value".into());
        assert_eq!(e.to_string(), "event serialization error: bad value");

        let e = EventLogError::Deserialization("invalid json".into());
        assert_eq!(e.to_string(), "event deserialization error: invalid json");

        let e = EventLogError::Closed;
        assert_eq!(e.to_string(), "event log closed");
    }
}
