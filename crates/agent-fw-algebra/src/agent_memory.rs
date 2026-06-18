//! Conversation memory algebra for stateful agent invocations.
//!
//! Implementations persist non-system chat messages for a tenant/thread/agent
//! identity. Concrete storage belongs in interpreter/runtime crates.

use agent_fw_core::{ChatMessage, TenantContext};
use async_trait::async_trait;

/// Errors raised while loading or appending conversation memory.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum AgentMemoryError {
    #[error("agent memory requires a thread id for stateful agent '{agent}'")]
    MissingThreadId { agent: String },
    #[error("agent memory storage error: {0}")]
    Storage(String),
}

/// Object-safe conversation memory store used by stateful agent orchestrators.
#[async_trait]
pub trait AgentMemoryStore: Send + Sync {
    /// Load prior non-system messages for this tenant/thread/agent.
    async fn load(
        &self,
        tenant: &TenantContext,
        agent: &str,
    ) -> Result<Vec<ChatMessage>, AgentMemoryError>;

    /// Append one successful agent turn to memory.
    async fn append_turn(
        &self,
        tenant: &TenantContext,
        agent: &str,
        user: ChatMessage,
        assistant: ChatMessage,
    ) -> Result<(), AgentMemoryError>;
}
