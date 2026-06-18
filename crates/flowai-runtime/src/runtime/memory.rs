//! KV-backed conversation memory for stateful Flow AI harness agents.

use std::sync::Arc;

use agent_fw_algebra::{AgentMemoryError, AgentMemoryStore, KVStore, KVStoreExt};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::ChatMessage;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAgentMemory {
    messages: Vec<ChatMessage>,
}

/// Conversation memory backed by the runtime KV store.
pub struct KvAgentMemoryStore {
    kv: Arc<dyn KVStore>,
}

impl KvAgentMemoryStore {
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self { kv }
    }

    fn key(tenant: &TenantContext, agent: &str) -> Option<String> {
        let thread_id = tenant.thread_id()?;
        Some(format!("agent-memory:{}:{agent}", thread_id.as_str()))
    }
}

#[async_trait]
impl AgentMemoryStore for KvAgentMemoryStore {
    async fn load(
        &self,
        tenant: &TenantContext,
        agent: &str,
    ) -> Result<Vec<ChatMessage>, AgentMemoryError> {
        let Some(key) = Self::key(tenant, agent) else {
            return Err(AgentMemoryError::MissingThreadId {
                agent: agent.to_string(),
            });
        };
        let stored: Option<StoredAgentMemory> = self
            .kv
            .get(tenant.resource_id().as_str(), &key)
            .await
            .map_err(|err| AgentMemoryError::Storage(err.to_string()))?;
        Ok(stored.map(|memory| memory.messages).unwrap_or_default())
    }

    async fn append_turn(
        &self,
        tenant: &TenantContext,
        agent: &str,
        user: ChatMessage,
        assistant: ChatMessage,
    ) -> Result<(), AgentMemoryError> {
        let Some(key) = Self::key(tenant, agent) else {
            return Err(AgentMemoryError::MissingThreadId {
                agent: agent.to_string(),
            });
        };
        let mut stored: StoredAgentMemory = self
            .kv
            .get(tenant.resource_id().as_str(), &key)
            .await
            .map_err(|err| AgentMemoryError::Storage(err.to_string()))?
            .unwrap_or_default();
        stored.messages.push(user);
        stored.messages.push(assistant);
        self.kv
            .put(tenant.resource_id().as_str(), &key, &stored, None)
            .await
            .map_err(|err| AgentMemoryError::Storage(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::{TenantId, ThreadId};
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn kv_memory_round_trips_messages_by_tenant_thread_and_agent() {
        let kv = Arc::new(DashMapKVStore::new());
        let memory = KvAgentMemoryStore::new(kv);
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"))
            .with_thread(ThreadId::new_unchecked("thread-1"));

        memory
            .append_turn(
                &tenant,
                "planner",
                ChatMessage::user("first"),
                ChatMessage::assistant("reply"),
            )
            .await
            .expect("append");

        let messages = memory.load(&tenant, "planner").await.expect("load");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "reply");

        let executor_messages = memory.load(&tenant, "executor").await.expect("load");
        assert!(executor_messages.is_empty());
    }

    #[tokio::test]
    async fn kv_memory_requires_thread_id() {
        let kv = Arc::new(DashMapKVStore::new());
        let memory = KvAgentMemoryStore::new(kv);
        let tenant = TenantContext::new(TenantId::new_unchecked("tenant-1"));

        let err = memory.load(&tenant, "planner").await.unwrap_err();
        assert!(matches!(
            err,
            AgentMemoryError::MissingThreadId { ref agent } if agent == "planner"
        ));
    }

    #[tokio::test]
    async fn kv_memory_satisfies_agent_memory_laws() {
        let kv = Arc::new(DashMapKVStore::new());
        let memory = KvAgentMemoryStore::new(kv);
        agent_fw_test::agent_memory_laws::test_all(&memory).await;
    }
}
