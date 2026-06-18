//! Canonical null interpreters for test and bootstrap use.
//!
//! These are always-available (not `cfg(test)`) because runtime code like
//! `test_env()` constructors and Python binding bootstraps need them.
//!
//! Most null interpreters satisfy their algebra's laws trivially:
//! - `NullEventSink`: accepts and discards all events (always open)
//! - `NullSubAgentInvoker`: no agents available (always returns NotFound)
//! - `NullKVStore`: empty store (get→None, exists→false, list→empty)
//! - `NullAgentMemoryStore`: explicit no-op memory store for call sites that
//!   intentionally disable persistence

use async_trait::async_trait;

use crate::agent_memory::{AgentMemoryError, AgentMemoryStore};
use crate::event_sink::EventSink;
use crate::kv_store::{KVError, KVStore};
use crate::sub_agent::{SubAgentError, SubAgentInvoker, SubAgentRequest, SubAgentResult};

// ─── NullAgentMemoryStore ────────────────────────────────────────────

/// An explicit no-op AgentMemoryStore.
///
/// Use this only when stateful memory is intentionally disabled in a test or
/// bootstrap path. `AgentOrchestratorBuilder` never installs it implicitly for
/// stateful registrations.
pub struct NullAgentMemoryStore;

#[async_trait]
impl AgentMemoryStore for NullAgentMemoryStore {
    async fn load(
        &self,
        _tenant: &agent_fw_core::TenantContext,
        _agent: &str,
    ) -> Result<Vec<agent_fw_core::ChatMessage>, AgentMemoryError> {
        Ok(Vec::new())
    }

    async fn append_turn(
        &self,
        _tenant: &agent_fw_core::TenantContext,
        _agent: &str,
        _user: agent_fw_core::ChatMessage,
        _assistant: agent_fw_core::ChatMessage,
    ) -> Result<(), AgentMemoryError> {
        Ok(())
    }
}

// ─── NullEventSink ──────────────────────────────────────────────────

/// An EventSink that accepts and discards all events.
///
/// Always open, never blocks. Useful for test environments and
/// bootstrap contexts where event streaming isn't wired yet.
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _: agent_fw_core::StreamPart) -> bool {
        true
    }
    fn close(&self) {}
    fn is_open(&self) -> bool {
        true
    }
}

// ─── RecordingEventSink ────────────────────────────────────────────

/// An EventSink that captures all emitted events for test assertions.
///
/// Complements `NullEventSink` (discard) with a "capture" double.
/// Satisfies all EventSink laws: totality, order preservation,
/// closure semantics, idempotent close.
pub struct RecordingEventSink {
    events: std::sync::Mutex<Vec<agent_fw_core::StreamPart>>,
    open: std::sync::atomic::AtomicBool,
}

impl RecordingEventSink {
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
            open: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Returns a clone of all recorded events.
    pub fn events(&self) -> Vec<agent_fw_core::StreamPart> {
        self.events.lock().expect("not poisoned").clone()
    }

    /// Returns the number of recorded events.
    pub fn event_count(&self) -> usize {
        self.events.lock().expect("not poisoned").len()
    }
}

impl EventSink for RecordingEventSink {
    fn emit(&self, part: agent_fw_core::StreamPart) -> bool {
        if !self.is_open() {
            return false;
        }
        self.events.lock().expect("not poisoned").push(part);
        true
    }
    fn close(&self) {
        self.open.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    fn is_open(&self) -> bool {
        self.open.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// ─── NullSubAgentInvoker ────────────────────────────────────────────

/// A SubAgentInvoker with no agents.
///
/// Returns `NotFound` for all invocations. Useful for environments
/// where sub-agent delegation isn't needed (single-agent tests, etc.).
pub struct NullSubAgentInvoker;

#[async_trait]
impl SubAgentInvoker for NullSubAgentInvoker {
    async fn invoke(&self, _: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        Err(SubAgentError::NotFound("null".into()))
    }
    fn has_agent(&self, _: &str) -> bool {
        false
    }
    fn available_agents(&self) -> Vec<String> {
        vec![]
    }
}

// ─── NullKVStore ────────────────────────────────────────────────────

/// A KVStore that stores nothing.
///
/// All reads return None/empty, all writes succeed silently.
/// Useful when handlers don't need persistence in tests.
pub struct NullKVStore;

#[async_trait]
impl KVStore for NullKVStore {
    async fn put_json(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
        _: Option<std::time::Duration>,
    ) -> Result<(), KVError> {
        Ok(())
    }

    async fn get_json(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, KVError> {
        Ok(None)
    }

    async fn delete(&self, _: &str, _: &str) -> Result<bool, KVError> {
        Ok(false)
    }

    async fn exists(&self, _: &str, _: &str) -> Result<bool, KVError> {
        Ok(false)
    }

    async fn list_keys(&self, _: &str, _: &str) -> Result<Vec<String>, KVError> {
        Ok(vec![])
    }

    async fn get_many_json(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, KVError> {
        Ok(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_event_sink_is_always_open() {
        let sink = NullEventSink;
        assert!(sink.is_open());
        assert!(sink.emit(agent_fw_core::StreamPart::Finish {
            reason: agent_fw_core::stream_part::FinishReason::Stop,
            usage: agent_fw_core::TokenUsage::ZERO,
        }));
        sink.close();
    }

    #[test]
    fn null_sub_agent_has_no_agents() {
        let invoker = NullSubAgentInvoker;
        assert!(!invoker.has_agent("any"));
        assert!(invoker.available_agents().is_empty());
    }

    #[tokio::test]
    async fn null_kv_store_reads_empty() {
        let kv = NullKVStore;
        assert_eq!(kv.get_json("ns", "key").await.unwrap(), None);
        assert!(!kv.exists("ns", "key").await.unwrap());
        assert!(kv.list_keys("ns", "").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn null_kv_store_writes_succeed() {
        let kv = NullKVStore;
        kv.put_json("ns", "key", serde_json::json!(1), None)
            .await
            .unwrap();
        assert!(!kv.delete("ns", "key").await.unwrap());
    }
}
