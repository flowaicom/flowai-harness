//! Generic thread-preview and trace-segment helpers for eval/test-case authoring.

use agent_fw_core::{truncate_utf8_chars, TenantId};
use agent_fw_eval::ToolCallEntry;
use serde::{Deserialize, Serialize};

use crate::{Message, WorkspaceError, WorkspaceStore};

/// Compact preview of a thread for builder/discovery UIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub first_user_message: Option<String>,
}

/// Thread-list result with total cardinality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummaryList {
    pub threads: Vec<ThreadSummary>,
    pub total_count: usize,
}

/// Canonical authoring view of a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAuthoringSnapshot {
    pub thread: crate::Thread,
    pub first_user_message: Option<String>,
    pub tool_calls: Vec<ToolCallEntry>,
}

/// Result of forking a thread through the workspace store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedThread {
    pub thread: crate::Thread,
    pub copied_message_count: usize,
}

/// Trace-segment extraction failures.
#[derive(Debug, thiserror::Error)]
pub enum ThreadSegmentError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("Thread '{thread_id}' not found")]
    ThreadNotFound { thread_id: String },
    #[error("Index range [{from}, {to}) out of bounds (thread has {len} tool calls)")]
    IndexOutOfBounds { from: usize, to: usize, len: usize },
}

/// Thread-forking failures.
#[derive(Debug, thiserror::Error)]
pub enum ThreadForkError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("Thread '{thread_id}' not found")]
    ThreadNotFound { thread_id: String },
    #[error("fork_at_message_index {fork_at_message_index} exceeds message count {message_count}")]
    MessageIndexOutOfBounds {
        fork_at_message_index: usize,
        message_count: usize,
    },
}

/// Thread-authoring snapshot failures.
#[derive(Debug, thiserror::Error)]
pub enum ThreadAuthoringError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("Thread '{thread_id}' not found")]
    ThreadNotFound { thread_id: String },
}

/// List thread previews using the canonical workspace store.
pub async fn list_thread_summaries(
    workspace: &dyn WorkspaceStore,
    tenant: &TenantId,
    limit: usize,
) -> Result<ThreadSummaryList, WorkspaceError> {
    let all_threads = workspace.list_threads(tenant).await?;
    let total_count = all_threads.len();
    let mut summaries = Vec::new();
    for thread in all_threads.into_iter().take(limit) {
        let messages = workspace.get_all_messages(tenant, &thread.id).await?;
        let tool_calls = extract_tool_calls_from_workspace_messages(&messages);
        let first_user_message = messages.iter().find_map(|message| {
            if message.role == "user" {
                let content = message.content.trim();
                if content.is_empty() {
                    None
                } else {
                    Some(truncate_utf8_chars(content, 120))
                }
            } else {
                None
            }
        });

        summaries.push(ThreadSummary {
            id: thread.id,
            title: thread.title,
            created_at: thread.created_at,
            updated_at: thread.updated_at,
            message_count: messages.len(),
            tool_call_count: tool_calls.len(),
            first_user_message,
        });
    }

    Ok(ThreadSummaryList {
        threads: summaries,
        total_count,
    })
}

/// Extract a contiguous range of tool calls from a thread.
pub async fn extract_thread_tool_segment(
    workspace: &dyn WorkspaceStore,
    tenant: &TenantId,
    thread_id: &str,
    from_index: usize,
    to_index: usize,
) -> Result<Vec<ToolCallEntry>, ThreadSegmentError> {
    if workspace.get_thread(tenant, thread_id).await?.is_none() {
        return Err(ThreadSegmentError::ThreadNotFound {
            thread_id: thread_id.to_string(),
        });
    }

    let messages = workspace.get_all_messages(tenant, thread_id).await?;
    let tool_calls = extract_tool_calls_from_workspace_messages(&messages);

    if from_index >= to_index || from_index >= tool_calls.len() || to_index > tool_calls.len() {
        return Err(ThreadSegmentError::IndexOutOfBounds {
            from: from_index,
            to: to_index,
            len: tool_calls.len(),
        });
    }

    Ok(tool_calls[from_index..to_index].to_vec())
}

/// Load the canonical authoring snapshot for a thread.
///
/// This keeps thread-harvesting logic on top of the workspace store instead of
/// forcing consuming applications to read raw KV payloads and re-parse them.
pub async fn load_thread_authoring_snapshot(
    workspace: &dyn WorkspaceStore,
    tenant: &TenantId,
    thread_id: &str,
) -> Result<ThreadAuthoringSnapshot, ThreadAuthoringError> {
    let thread = workspace
        .get_thread(tenant, thread_id)
        .await?
        .ok_or_else(|| ThreadAuthoringError::ThreadNotFound {
            thread_id: thread_id.to_string(),
        })?;
    let messages = workspace.get_all_messages(tenant, thread_id).await?;
    let first_user_message = messages.iter().find_map(|message| {
        if message.role == "user" {
            let content = message.content.trim();
            if content.is_empty() {
                None
            } else {
                Some(content.to_string())
            }
        } else {
            None
        }
    });
    let tool_calls = extract_tool_calls_from_workspace_messages(&messages);

    Ok(ThreadAuthoringSnapshot {
        thread,
        first_user_message,
        tool_calls,
    })
}

/// Fork a thread, copying messages before the fork point and appending an edited message.
///
/// The new thread:
/// - inherits the parent `source_id`
/// - carries forward the parent title with `" (fork)"` suffix
/// - falls back to `"Fork"` when the parent has no title
/// - uses the replaced message role when forking inside the thread
/// - uses `"user"` when forking at the end of the thread
pub async fn fork_workspace_thread(
    workspace: &dyn WorkspaceStore,
    tenant: &TenantId,
    parent_thread_id: &str,
    fork_at_message_index: usize,
    edited_content: impl Into<String>,
) -> Result<ForkedThread, ThreadForkError> {
    let parent_thread = workspace
        .get_thread(tenant, parent_thread_id)
        .await?
        .ok_or_else(|| ThreadForkError::ThreadNotFound {
            thread_id: parent_thread_id.to_string(),
        })?;
    let parent_messages = workspace.get_all_messages(tenant, parent_thread_id).await?;

    if fork_at_message_index > parent_messages.len() {
        return Err(ThreadForkError::MessageIndexOutOfBounds {
            fork_at_message_index,
            message_count: parent_messages.len(),
        });
    }

    let mut forked_messages: Vec<Message> = parent_messages
        .iter()
        .take(fork_at_message_index)
        .cloned()
        .collect();
    let edited_role = parent_messages
        .get(fork_at_message_index)
        .map(|message| message.role.clone())
        .unwrap_or_else(|| "user".to_string());
    forked_messages.push(Message::new(edited_role, edited_content.into()));

    let fork_title = parent_thread
        .title
        .clone()
        .map(|title| format!("{title} (fork)"))
        .unwrap_or_else(|| "Fork".to_string());

    let mut forked_thread = crate::Thread::new(Some(fork_title));
    if parent_thread.resource_id.is_empty() {
        forked_thread = forked_thread.with_resource_id(tenant.as_str());
    } else {
        forked_thread = forked_thread.with_resource_id(parent_thread.resource_id.clone());
    }
    if let Some(source_id) = parent_thread.source_id.as_deref() {
        forked_thread = forked_thread.with_source_id(source_id);
    }

    workspace.upsert_thread(tenant, &forked_thread).await?;
    for message in &forked_messages {
        workspace
            .insert_message(tenant, message, &forked_thread.id)
            .await?;
    }

    Ok(ForkedThread {
        thread: forked_thread,
        copied_message_count: forked_messages.len(),
    })
}

/// Extract canonical tool calls directly from framework messages.
pub fn extract_tool_calls_from_workspace_messages(messages: &[Message]) -> Vec<ToolCallEntry> {
    let mut entries = Vec::new();
    let mut global_index = 0usize;

    for (message_index, message) in messages.iter().enumerate() {
        let Some(interactions) = message.tool_interactions.as_ref() else {
            continue;
        };

        for interaction in interactions {
            entries.push(ToolCallEntry {
                index: global_index,
                tool_name: interaction.tool_name.clone(),
                args: interaction.arguments.clone(),
                result: Some(interaction.result.clone()),
                invocation_id: interaction.call_id.clone(),
                message_index,
            });
            global_index += 1;
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agent_fw_algebra::KVStore;
    use agent_fw_core::id::TenantId;
    use agent_fw_interpreter::DashMapKVStore;

    use crate::{KVWorkspaceStore, MessageStore, PersistedToolInteraction, Thread, ThreadStore};

    fn tenant() -> TenantId {
        TenantId::new_unchecked("test-tenant")
    }

    async fn seed_workspace() -> KVWorkspaceStore {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv);
        let tenant = tenant();

        store
            .upsert_thread(
                &tenant,
                &Thread {
                    id: "t1".into(),
                    title: Some("Pricing scenario".into()),
                    resource_id: String::new(),
                    source_id: None,
                    created_at: "2025-01-01T00:00:00Z".into(),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                },
            )
            .await
            .unwrap();
        store
            .insert_message(
                &tenant,
                &Message::new("user", "What pricing options exist?"),
                "t1",
            )
            .await
            .unwrap();
        store
            .insert_message(
                &tenant,
                &Message::with_tool_interactions(
                    "assistant",
                    "Searching...",
                    vec![PersistedToolInteraction {
                        call_id: "inv1".into(),
                        tool_name: "searchProducts".into(),
                        arguments: serde_json::json!({}),
                        result: serde_json::json!({}),
                    }],
                ),
                "t1",
            )
            .await
            .unwrap();

        store
    }

    #[tokio::test]
    async fn list_thread_summaries_reports_counts_and_preview() {
        let store = seed_workspace().await;
        let result = list_thread_summaries(&store, &tenant(), 20).await.unwrap();

        assert_eq!(result.total_count, 1);
        assert_eq!(result.threads.len(), 1);
        let thread = &result.threads[0];
        assert_eq!(thread.id, "t1");
        assert_eq!(thread.message_count, 2);
        assert_eq!(thread.tool_call_count, 1);
        assert_eq!(
            thread.first_user_message.as_deref(),
            Some("What pricing options exist?")
        );
    }

    #[tokio::test]
    async fn extract_thread_tool_segment_reads_canonical_interactions() {
        let store = seed_workspace().await;
        let result = extract_thread_tool_segment(&store, &tenant(), "t1", 0, 1)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tool_name, "searchProducts");
        assert_eq!(result[0].index, 0);
    }

    #[tokio::test]
    async fn extract_thread_tool_segment_rejects_oob_range() {
        let store = seed_workspace().await;
        let err = extract_thread_tool_segment(&store, &tenant(), "t1", 1, 2)
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ThreadSegmentError::IndexOutOfBounds {
                from: 1,
                to: 2,
                len: 1
            }
        ));
    }

    #[tokio::test]
    async fn load_thread_authoring_snapshot_reads_first_user_message_and_tool_calls() {
        let store = seed_workspace().await;
        let snapshot = load_thread_authoring_snapshot(&store, &tenant(), "t1")
            .await
            .unwrap();

        assert_eq!(snapshot.thread.id, "t1");
        assert_eq!(
            snapshot.first_user_message.as_deref(),
            Some("What pricing options exist?")
        );
        assert_eq!(snapshot.tool_calls.len(), 1);
        assert_eq!(snapshot.tool_calls[0].tool_name, "searchProducts");
    }

    #[tokio::test]
    async fn fork_workspace_thread_copies_messages_and_inherits_source() {
        let store = seed_workspace().await;
        let tenant = tenant();
        store
            .upsert_thread(
                &tenant,
                &Thread::with_id("parent", Some("Scenario".to_string()))
                    .with_resource_id(tenant.as_str())
                    .with_source_id("source-alpha"),
            )
            .await
            .unwrap();
        store
            .insert_message(&tenant, &Message::new("user", "first"), "parent")
            .await
            .unwrap();
        store
            .insert_message(&tenant, &Message::new("assistant", "second"), "parent")
            .await
            .unwrap();

        let forked = fork_workspace_thread(&store, &tenant, "parent", 1, "edited")
            .await
            .unwrap();

        assert_eq!(forked.thread.title.as_deref(), Some("Scenario (fork)"));
        assert_eq!(forked.thread.source_id.as_deref(), Some("source-alpha"));
        assert_eq!(forked.copied_message_count, 2);

        let messages = store
            .get_all_messages(&tenant, &forked.thread.id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "edited");
    }

    #[tokio::test]
    async fn fork_workspace_thread_uses_user_role_when_appending_at_end() {
        let store = seed_workspace().await;
        let tenant = tenant();
        store
            .upsert_thread(&tenant, &Thread::with_id("parent", None))
            .await
            .unwrap();
        store
            .insert_message(&tenant, &Message::new("assistant", "only"), "parent")
            .await
            .unwrap();

        let forked = fork_workspace_thread(&store, &tenant, "parent", 1, "edited")
            .await
            .unwrap();

        let messages = store
            .get_all_messages(&tenant, &forked.thread.id)
            .await
            .unwrap();
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "edited");
        assert_eq!(forked.thread.title.as_deref(), Some("Fork"));
    }

    #[tokio::test]
    async fn fork_workspace_thread_rejects_out_of_bounds_index() {
        let store = seed_workspace().await;
        let tenant = tenant();
        store
            .upsert_thread(&tenant, &Thread::with_id("parent", None))
            .await
            .unwrap();

        let err = fork_workspace_thread(&store, &tenant, "parent", 1, "edited")
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ThreadForkError::MessageIndexOutOfBounds {
                fork_at_message_index: 1,
                message_count: 0
            }
        ));
    }
}
