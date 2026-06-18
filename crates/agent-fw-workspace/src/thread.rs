//! Thread and Message domain types.
//!
//! These are framework-generic chat thread and message types used by:
//! - [`crate::store::ThreadStore`] and [`crate::store::MessageStore`] traits
//! - [`crate::kv_store::KVWorkspaceStore`] interpreter
//! - HTTP handlers in consuming applications

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Thread
// =============================================================================

/// Thread metadata stored in a workspace store.
///
/// Represents a single conversation thread. The tenant is NOT embedded in the
/// thread for write operations, but consumers may project a resource/tenant ID
/// into the payload for read-side convenience.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Unique thread ID (UUID v4).
    pub id: String,
    /// Human-readable title (defaults to "New Conversation").
    pub title: Option<String>,
    /// Optional projected resource/tenant identifier for read-side consumers.
    ///
    /// This is the storage/read scope, not the selected data source.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resource_id: String,
    /// Optional explicit data source selection persisted with the thread.
    ///
    /// This is orthogonal to `resource_id`: `resource_id` says which tenant owns
    /// the thread, while `source_id` pins chat continuity to a specific
    /// source-mode database inside that tenant. `None` means tenant/workspace
    /// default runtime selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last update timestamp.
    pub updated_at: String,
}

impl Thread {
    /// Create a new thread with a random UUID.
    pub fn new(title: Option<String>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            resource_id: String::new(),
            source_id: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Create a thread with a specific ID (for auto-creation from chat).
    pub fn with_id(id: impl Into<String>, title: Option<String>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: id.into(),
            title,
            resource_id: String::new(),
            source_id: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Attach a projected resource/tenant identifier for read-side consumers.
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = resource_id.into();
        self
    }

    /// Attach an explicit source identifier for source-mode chat continuity.
    pub fn with_source_id(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }

    /// Update the title and touch the timestamp.
    pub fn update_title(&mut self, title: Option<String>) {
        self.title = title;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// Update the persisted source identifier and touch the timestamp.
    pub fn update_source_id(&mut self, source_id: Option<String>) {
        self.source_id = source_id.filter(|id| !id.is_empty());
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// Touch the updated_at timestamp.
    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

// =============================================================================
// PersistedToolInteraction
// =============================================================================

/// A persisted tool call + result pair.
///
/// Each struct holds both call and result — 1:1 correspondence.
/// This enables reconstructing proper assistant/user rig messages
/// when loading thread history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedToolInteraction {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
}

// =============================================================================
// Message
// =============================================================================

/// A chat message stored in a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Unique message ID (UUID v4).
    pub id: String,
    /// Role: "user", "assistant", or "system".
    pub role: String,
    /// Message content (plain text, used for LLM context replay).
    pub content: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// Tool call/result pairs from this turn (if any).
    /// Backward compatible: existing messages without this field deserialize to None.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_interactions: Option<Vec<PersistedToolInteraction>>,
    /// Structured message parts for rich frontend rendering.
    ///
    /// When present, the frontend uses these directly instead of `content`.
    /// Backward compatible: old messages deserialize to `None`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts: Option<Vec<serde_json::Value>>,
}

impl Message {
    /// Create a new text-only message.
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: role.into(),
            content: content.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            tool_interactions: None,
            parts: None,
        }
    }

    /// Create a message with tool interactions.
    pub fn with_tool_interactions(
        role: impl Into<String>,
        content: impl Into<String>,
        interactions: Vec<PersistedToolInteraction>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: role.into(),
            content: content.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            tool_interactions: if interactions.is_empty() {
                None
            } else {
                Some(interactions)
            },
            parts: None,
        }
    }

    /// Create a message with structured parts for rich frontend rendering.
    pub fn with_parts(
        role: impl Into<String>,
        content: impl Into<String>,
        interactions: Vec<PersistedToolInteraction>,
        parts: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: role.into(),
            content: content.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            tool_interactions: if interactions.is_empty() {
                None
            } else {
                Some(interactions)
            },
            parts: if parts.is_empty() { None } else { Some(parts) },
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_creation() {
        let thread = Thread::new(Some("Test Thread".to_string()));
        assert!(!thread.id.is_empty());
        assert_eq!(thread.title, Some("Test Thread".to_string()));
        assert!(thread.resource_id.is_empty());
        assert_eq!(thread.source_id, None);
        assert!(!thread.created_at.is_empty());
        assert_eq!(thread.created_at, thread.updated_at);
    }

    #[test]
    fn thread_with_specific_id() {
        let thread = Thread::with_id("my-id", Some("Title".to_string()));
        assert_eq!(thread.id, "my-id");
    }

    #[test]
    fn thread_with_resource_id() {
        let thread = Thread::new(None).with_resource_id("tenant-1");
        assert_eq!(thread.resource_id, "tenant-1");
    }

    #[test]
    fn thread_with_source_id() {
        let thread = Thread::new(None).with_source_id("source-1");
        assert_eq!(thread.source_id.as_deref(), Some("source-1"));
    }

    #[test]
    fn thread_update_title() {
        let mut thread = Thread::new(None);
        let original_updated = thread.updated_at.clone();

        std::thread::sleep(std::time::Duration::from_millis(10));
        thread.update_title(Some("New Title".to_string()));

        assert_eq!(thread.title, Some("New Title".to_string()));
        assert_ne!(thread.updated_at, original_updated);
    }

    #[test]
    fn message_creation() {
        let message = Message::new("user", "Hello!");
        assert!(!message.id.is_empty());
        assert_eq!(message.role, "user");
        assert_eq!(message.content, "Hello!");
        assert!(!message.created_at.is_empty());
        assert!(message.tool_interactions.is_none());
        assert!(message.parts.is_none());
    }

    #[test]
    fn thread_roundtrip_serialization() {
        let thread = Thread::new(Some("Test".to_string()))
            .with_resource_id("tenant-1")
            .with_source_id("source-1");
        let json = serde_json::to_string(&thread).unwrap();
        let parsed: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(thread.id, parsed.id);
        assert_eq!(thread.title, parsed.title);
        assert_eq!(thread.resource_id, parsed.resource_id);
        assert_eq!(thread.source_id, parsed.source_id);
    }

    #[test]
    fn message_roundtrip_serialization() {
        let message = Message::new("assistant", "Hello!");
        let json = serde_json::to_string(&message).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(message.id, parsed.id);
        assert_eq!(message.role, parsed.role);
        assert_eq!(message.content, parsed.content);
    }

    #[test]
    fn message_without_tool_interactions_deserializes() {
        let json = r#"{"id":"abc","role":"assistant","content":"hello","createdAt":"2024-01-01T00:00:00Z"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        assert!(msg.tool_interactions.is_none());
        assert!(msg.parts.is_none());
    }

    #[test]
    fn message_with_tool_interactions_roundtrips() {
        let interaction = PersistedToolInteraction {
            call_id: "call-1".to_string(),
            tool_name: "draft_plan".to_string(),
            arguments: serde_json::json!({"products": {"displayName": ["Kit"]}}),
            result: serde_json::json!({"planId": "plan-123"}),
        };
        let msg = Message::with_tool_interactions("assistant", "summary", vec![interaction]);
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert!(parsed.tool_interactions.is_some());
        let tools = parsed.tool_interactions.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].call_id, "call-1");
        assert_eq!(tools[0].tool_name, "draft_plan");
    }

    #[test]
    fn message_with_empty_tool_interactions_serializes_as_none() {
        let msg = Message::with_tool_interactions("assistant", "text", vec![]);
        assert!(msg.tool_interactions.is_none());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("toolInteractions"));
    }
}
