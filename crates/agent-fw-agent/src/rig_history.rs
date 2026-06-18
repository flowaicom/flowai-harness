//! Canonical Rig history reconstruction from framework chat messages.
//!
//! Applications that persist `PersistedToolInteraction` alongside chat messages
//! should not need to rebuild the same Rig replay logic locally. This module
//! connects the framework-owned conversation model with the framework-owned
//! persisted tool-interaction contract and produces Rig `Message` history that
//! preserves tool call/result continuity across turns.

use crate::conversation::{ChatMessage, ChatRole, Conversation};
use agent_fw_workspace::PersistedToolInteraction;
use rig::message::{AssistantContent, Message, ToolCall, ToolFunction};
use rig::OneOrMany;

/// Convert a framework chat message to one or more Rig messages.
///
/// When the message carries tool interactions, this reconstructs:
/// 1. an assistant message with `ToolCall` content
/// 2. user tool-result messages
/// 3. an optional text message when the original content is non-empty
pub fn chat_message_to_rig_messages(msg: &ChatMessage) -> Vec<Message> {
    let mut out = Vec::new();

    let tools = match msg.deserialize_tool_interactions::<PersistedToolInteraction>() {
        Ok(Some(tools)) => tools,
        Ok(None) => Vec::new(),
        Err(err) => {
            tracing::warn!(error = %err, "Failed to decode persisted tool interactions");
            Vec::new()
        }
    };

    if !tools.is_empty() {
        let calls: Vec<AssistantContent> = tools
            .iter()
            .map(|t| {
                AssistantContent::ToolCall(ToolCall::new(
                    t.call_id.clone(),
                    ToolFunction::new(t.tool_name.clone(), t.arguments.clone()),
                ))
            })
            .collect();
        if let Ok(content) = OneOrMany::many(calls) {
            out.push(Message::Assistant { id: None, content });
        }

        for t in &tools {
            let result_str = serde_json::to_string(&t.result).unwrap_or_default();
            out.push(Message::tool_result(&t.call_id, result_str));
        }
    }

    if !msg.content.trim().is_empty() {
        match msg.role {
            ChatRole::User => out.push(Message::user(&msg.content)),
            ChatRole::Assistant => out.push(Message::assistant(&msg.content)),
            ChatRole::System => {}
        }
    }

    out
}

/// Convert framework conversation history to Rig format.
pub fn conversation_to_rig_history(conversation: &Conversation) -> Vec<Message> {
    conversation
        .history()
        .flat_map(chat_message_to_rig_messages)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_messages_are_filtered() {
        assert!(chat_message_to_rig_messages(&ChatMessage::assistant("")).is_empty());
        assert!(chat_message_to_rig_messages(&ChatMessage::assistant("  \n  ")).is_empty());
        assert!(chat_message_to_rig_messages(&ChatMessage::user("")).is_empty());
        assert!(!chat_message_to_rig_messages(&ChatMessage::assistant("hello")).is_empty());
    }

    #[test]
    fn tool_interactions_expand_to_call_result_and_text() {
        let msg = ChatMessage::assistant("Built the plan")
            .with_serialized_tool_interactions(vec![PersistedToolInteraction {
                call_id: "call-123".to_string(),
                tool_name: "draft_plan".to_string(),
                arguments: serde_json::json!({"objective":"profit"}),
                result: serde_json::json!({"planId":"plan-123"}),
            }])
            .expect("tool interactions should serialize");

        let rig_msgs = chat_message_to_rig_messages(&msg);

        assert_eq!(rig_msgs.len(), 3);
        assert!(matches!(&rig_msgs[0], Message::Assistant { content, .. }
            if content.iter().any(|c| matches!(c, AssistantContent::ToolCall(_)))));
        assert!(matches!(&rig_msgs[1], Message::User { content }
            if content.iter().any(|c| matches!(c, rig::message::UserContent::ToolResult(_)))));
        assert!(matches!(&rig_msgs[2], Message::Assistant { .. }));
    }

    #[test]
    fn conversation_history_excludes_system_and_last_user() {
        let messages = vec![
            ChatMessage::system("system"),
            ChatMessage::user("first"),
            ChatMessage::assistant("response"),
            ChatMessage::user("last"),
        ];

        let conv = crate::conversation::parse_conversation(messages).unwrap();
        let history = conversation_to_rig_history(&conv);

        assert_eq!(history.len(), 2);
    }
}
