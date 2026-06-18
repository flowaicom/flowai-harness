//! AgentMemoryStore algebraic law test harnesses.
//!
//! # Laws Tested
//!
//! - L1. Empty load: a fresh tenant/thread/agent has no messages.
//! - L2. Append/load roundtrip: appended user + assistant messages are loaded
//!   in order.
//! - L3. Agent isolation: one agent's memory is not visible to another agent.
//! - L4. Thread isolation: one thread's memory is not visible to another
//!   thread.
//! - L5. Thread required: stateful memory operations fail visibly when no
//!   thread id is present.

use agent_fw_algebra::{AgentMemoryError, AgentMemoryStore};
use agent_fw_core::{ChatMessage, TenantContext, TenantId, ThreadId};

/// Run all deterministic AgentMemoryStore laws against a fresh or
/// uniquely-keyed store.
pub async fn test_all(store: &dyn AgentMemoryStore) {
    law_empty_load(store).await;
    law_append_then_load_roundtrip(store).await;
    law_agent_isolation(store).await;
    law_thread_isolation(store).await;
    law_thread_required(store).await;
}

fn tenant(label: &str) -> TenantContext {
    TenantContext::new(TenantId::new_unchecked(format!("tenant-{label}")))
        .with_thread(ThreadId::new_unchecked(format!("thread-{label}")))
}

/// L1: Empty load returns no messages for a fresh tenant/thread/agent.
pub async fn law_empty_load(store: &dyn AgentMemoryStore) {
    let tenant = tenant(&format!("empty-{}", uuid::Uuid::new_v4()));
    let messages = store.load(&tenant, "planner").await.expect("load succeeds");
    assert!(
        messages.is_empty(),
        "L1: fresh memory should be empty, got {messages:?}"
    );
}

/// L2: Append then load returns both messages in order.
pub async fn law_append_then_load_roundtrip(store: &dyn AgentMemoryStore) {
    let tenant = tenant(&format!("roundtrip-{}", uuid::Uuid::new_v4()));
    let user = ChatMessage::user("hello");
    let assistant = ChatMessage::assistant("world");
    store
        .append_turn(&tenant, "planner", user.clone(), assistant.clone())
        .await
        .expect("append succeeds");

    let messages = store.load(&tenant, "planner").await.expect("load succeeds");
    assert_eq!(messages, vec![user, assistant]);
}

/// L3: Agent memory is isolated by agent name.
pub async fn law_agent_isolation(store: &dyn AgentMemoryStore) {
    let tenant = tenant(&format!("agent-{}", uuid::Uuid::new_v4()));
    store
        .append_turn(
            &tenant,
            "planner",
            ChatMessage::user("planner prompt"),
            ChatMessage::assistant("planner reply"),
        )
        .await
        .expect("append succeeds");

    let executor_messages = store
        .load(&tenant, "executor")
        .await
        .expect("load succeeds");
    assert!(
        executor_messages.is_empty(),
        "L3: executor should not see planner memory"
    );
}

/// L4: Memory is isolated by thread id.
pub async fn law_thread_isolation(store: &dyn AgentMemoryStore) {
    let id = uuid::Uuid::new_v4();
    let tenant_a = tenant(&format!("thread-a-{id}"));
    let tenant_b = tenant(&format!("thread-b-{id}"));
    store
        .append_turn(
            &tenant_a,
            "planner",
            ChatMessage::user("thread a"),
            ChatMessage::assistant("reply a"),
        )
        .await
        .expect("append succeeds");

    let messages = store
        .load(&tenant_b, "planner")
        .await
        .expect("load succeeds");
    assert!(
        messages.is_empty(),
        "L4: different thread should not see memory from another thread"
    );
}

/// L5: Missing thread ids fail visibly instead of silently dropping state.
pub async fn law_thread_required(store: &dyn AgentMemoryStore) {
    let tenant = TenantContext::new(TenantId::new_unchecked(format!(
        "tenant-no-thread-{}",
        uuid::Uuid::new_v4()
    )));
    let load = store.load(&tenant, "planner").await;
    assert!(
        matches!(load, Err(AgentMemoryError::MissingThreadId { .. })),
        "L5: load without thread should return MissingThreadId, got {load:?}"
    );

    let append = store
        .append_turn(
            &tenant,
            "planner",
            ChatMessage::user("hello"),
            ChatMessage::assistant("world"),
        )
        .await;
    assert!(
        matches!(append, Err(AgentMemoryError::MissingThreadId { .. })),
        "L5: append without thread should return MissingThreadId, got {append:?}"
    );
}
