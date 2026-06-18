//! In-memory EventLog implementation for development and testing.
//!
//! Events are stored in a `Vec` protected by a `RwLock`. TTL-based pruning
//! happens lazily on `replay` calls.
//!
//! # Production Use
//!
//! For production, implement `EventLog` on top of Redis or a persistent store.
//! This in-memory implementation is suitable for single-process deployments
//! and testing.

use agent_fw_algebra::event_log::{EventEntry, EventLog, EventLogError};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

/// In-memory event log for development and testing.
///
/// Satisfies EventLog laws L1-L5.
///
/// Uses `tokio::sync::RwLock` (not `std::sync::RwLock`) to avoid blocking
/// the async runtime under contention. This is critical because `append`
/// and `replay` are async methods — holding a blocking lock across an
/// await point would starve other tasks on the same runtime thread.
pub struct MemoryEventLog {
    channels: RwLock<HashMap<String, Vec<EventEntry<serde_json::Value>>>>,
    open: AtomicBool,
    ttl: Duration,
}

impl MemoryEventLog {
    /// Create a new in-memory event log with the given TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            open: AtomicBool::new(true),
            ttl,
        }
    }

    /// Create with default TTL of 1 hour.
    pub fn default_ttl() -> Self {
        Self::new(Duration::from_secs(3600))
    }
}

#[async_trait]
impl EventLog for MemoryEventLog {
    async fn append(&self, channel: &str, event: serde_json::Value) -> Result<u64, EventLogError> {
        if !self.is_open() {
            return Err(EventLogError::Closed);
        }

        let mut channels = self.channels.write().await;

        let entries = channels.entry(channel.to_string()).or_default();
        let offset = entries.len() as u64;

        entries.push(EventEntry {
            offset,
            event,
            timestamp: SystemTime::now(),
        });

        Ok(offset)
    }

    async fn replay(
        &self,
        channel: &str,
        from_offset: u64,
    ) -> Result<Vec<EventEntry<serde_json::Value>>, EventLogError> {
        let channels = self.channels.read().await;

        let Some(entries) = channels.get(channel) else {
            return Ok(Vec::new());
        };

        let cutoff = SystemTime::now()
            .checked_sub(self.ttl)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let result: Vec<_> = entries
            .iter()
            .filter(|e| e.offset >= from_offset && e.timestamp >= cutoff)
            .cloned()
            .collect();

        Ok(result)
    }

    fn close(&self) {
        self.open.store(false, Ordering::Release);
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    fn ttl(&self) -> Duration {
        self.ttl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify MemoryEventLog satisfies all EventLog algebraic laws
    /// using the reusable harness from agent-fw-test.
    #[tokio::test]
    async fn satisfies_event_log_laws() {
        agent_fw_test::event_log_laws::test_all(|| async { MemoryEventLog::default_ttl() }).await;
    }

    /// L5 (TTL-Bounded) requires sleeping, tested separately.
    #[tokio::test]
    async fn ttl_bounded() {
        let log = MemoryEventLog::new(Duration::from_millis(50));
        let _ = log
            .append("ttl_ch", serde_json::json!("ephemeral"))
            .await
            .unwrap();

        // Present before TTL
        let entries = log.replay("ttl_ch", 0).await.unwrap();
        assert_eq!(entries.len(), 1);

        // Wait past TTL
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Pruned after TTL
        let entries = log.replay("ttl_ch", 0).await.unwrap();
        assert!(entries.is_empty(), "L5: events past TTL should be pruned");
    }
}
