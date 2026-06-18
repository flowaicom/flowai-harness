//! Conversation domain types with smart constructors.
//!
//! # Laws
//!
//! - `parse_conversation` is total (always returns valid result or error)
//! - If Ok, the result has a non-empty prompt
//! - System messages are extracted but not included in history

pub use agent_fw_core::{ChatMessage, ChatRole};

/// A non-empty prompt string.
///
/// # Invariant
/// The inner string is never empty (enforced by smart constructor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt(String);

impl Prompt {
    /// Create a new Prompt, returning None if empty.
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.trim().is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    /// Create from string, using a default if empty.
    pub fn or_default(s: impl Into<String>, default: impl Into<String>) -> Self {
        Self::new(s).unwrap_or_else(|| Self(default.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Prompt {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// System prompt for agent behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPrompt(String);

impl SystemPrompt {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SystemPrompt {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// =============================================================================
// Chat Program - Programs as Values
// =============================================================================

/// A description of a chat operation to be performed.
///
/// This is a VALUE that describes WHAT should happen, not HOW.
/// The interpreter decides how to execute it.
///
/// # Algebraic Laws
///
/// - **L1 Purity**: Constructing a `ChatProgram` has no side effects. It is
///   a pure data value that can be inspected, cloned, and compared without
///   triggering any I/O or LLM calls.
///
/// - **L2 Referential Transparency**: `ChatProgram::new(conv, model, tenant)`
///   always produces an equivalent program for the same inputs. The program
///   can be freely substituted wherever it appears.
///
/// - **L3 Composition Associativity**: Sequential composition of programs
///   (e.g., feeding one program's output as context into the next) is
///   associative. `(p1 >> p2) >> p3 == p1 >> (p2 >> p3)`.
///
/// # Design Note
/// By making the program a value, we gain:
/// - Inspectability: We can examine the program before running it
/// - Composability: Programs can be combined
/// - Testability: We can test program construction without side effects
#[derive(Debug, Clone)]
pub struct ChatProgram {
    /// The validated conversation
    conversation: Conversation,
    /// System prompt (from conversation or explicit override)
    system_prompt: SystemPrompt,
    /// Model to use
    model: crate::model::ModelId,
    /// Tenant context for isolation
    tenant: agent_fw_core::tenant::TenantContext,
}

impl ChatProgram {
    /// Create a new chat program.
    ///
    /// The system prompt is extracted from the conversation if present,
    /// otherwise a default empty system prompt is used.
    pub fn new(
        conversation: Conversation,
        model: crate::model::ModelId,
        tenant: agent_fw_core::tenant::TenantContext,
    ) -> Self {
        let system_prompt = conversation
            .system()
            .cloned()
            .unwrap_or_else(|| SystemPrompt::new(""));

        Self {
            conversation,
            system_prompt,
            model,
            tenant,
        }
    }

    /// Override the system prompt.
    pub fn with_system(mut self, system: SystemPrompt) -> Self {
        self.system_prompt = system;
        self
    }

    /// Access the conversation.
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// Access the system prompt.
    pub fn system_prompt(&self) -> &SystemPrompt {
        &self.system_prompt
    }

    /// Access the model.
    pub fn model(&self) -> &crate::model::ModelId {
        &self.model
    }

    /// Access the tenant context.
    pub fn tenant(&self) -> &agent_fw_core::tenant::TenantContext {
        &self.tenant
    }
}

/// A validated conversation with at least one user message.
///
/// # Invariant
/// - Contains at least one message
/// - The last user message exists and becomes the prompt
#[derive(Debug, Clone)]
pub struct Conversation {
    messages: Vec<ChatMessage>,
    prompt: Prompt,
    system: Option<SystemPrompt>,
}

impl Conversation {
    /// The current prompt (last user message).
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// Optional system prompt override.
    pub fn system(&self) -> Option<&SystemPrompt> {
        self.system.as_ref()
    }

    /// All messages in order.
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// History messages (all except the last user message and system messages).
    pub fn history(&self) -> impl Iterator<Item = &ChatMessage> {
        let last_user_idx = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| m.role == ChatRole::User)
            .map(|(i, _)| i);

        self.messages.iter().enumerate().filter_map(move |(i, m)| {
            if m.role == ChatRole::System {
                return None;
            }
            if Some(i) == last_user_idx {
                return None;
            }
            Some(m)
        })
    }
}

/// Errors when parsing a conversation.
#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("Conversation must contain at least one user message")]
    NoUserMessage,
}

/// Parse raw messages into a validated Conversation.
///
/// This is a pure function: same input → same output.
///
/// # Laws
/// - If Ok, the result has a non-empty prompt
/// - System messages are extracted but not included in history
pub fn parse_conversation(messages: Vec<ChatMessage>) -> Result<Conversation, ConversationError> {
    let last_user_content = messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .map(|m| m.content.clone());

    let prompt = match last_user_content {
        Some(content) => Prompt::or_default(content, "Hello"),
        None => return Err(ConversationError::NoUserMessage),
    };

    let system = messages
        .iter()
        .find(|m| m.role == ChatRole::System)
        .map(|m| SystemPrompt::new(&m.content));

    Ok(Conversation {
        messages,
        prompt,
        system,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_rejects_empty() {
        assert!(Prompt::new("").is_none());
        assert!(Prompt::new("   ").is_none());
    }

    #[test]
    fn prompt_accepts_non_empty() {
        assert!(Prompt::new("hello").is_some());
        assert_eq!(Prompt::new("hello").unwrap().as_str(), "hello");
    }

    #[test]
    fn prompt_or_default_uses_default_for_empty() {
        let p = Prompt::or_default("", "fallback");
        assert_eq!(p.as_str(), "fallback");
    }

    #[test]
    fn parse_conversation_ok() {
        let msgs = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there"),
            ChatMessage::user("How are you?"),
        ];
        let conv = parse_conversation(msgs).unwrap();
        assert_eq!(conv.prompt().as_str(), "How are you?");
        assert!(conv.system().is_none());
    }

    #[test]
    fn parse_conversation_extracts_system() {
        let msgs = vec![
            ChatMessage::system("You are a helpful assistant"),
            ChatMessage::user("Hello"),
        ];
        let conv = parse_conversation(msgs).unwrap();
        assert_eq!(conv.prompt().as_str(), "Hello");
        assert_eq!(
            conv.system().unwrap().as_str(),
            "You are a helpful assistant"
        );
    }

    #[test]
    fn parse_conversation_no_user_message() {
        let msgs = vec![ChatMessage::assistant("I'm ready")];
        assert!(parse_conversation(msgs).is_err());
    }

    #[test]
    fn parse_conversation_empty() {
        let msgs = vec![];
        assert!(parse_conversation(msgs).is_err());
    }

    #[test]
    fn history_excludes_system_and_last_user() {
        let msgs = vec![
            ChatMessage::system("system"),
            ChatMessage::user("first"),
            ChatMessage::assistant("reply"),
            ChatMessage::user("second"),
        ];
        let conv = parse_conversation(msgs).unwrap();

        let history: Vec<_> = conv.history().collect();
        assert_eq!(history.len(), 2); // "first" (user) + "reply" (assistant)
        assert_eq!(history[0].role, ChatRole::User);
        assert_eq!(history[0].content, "first");
        assert_eq!(history[1].role, ChatRole::Assistant);
        assert_eq!(history[1].content, "reply");
    }

    #[test]
    fn chat_program_creation() {
        use crate::model::ModelId;
        use agent_fw_core::tenant::TenantContext;

        let msgs = vec![
            ChatMessage::system("You are helpful"),
            ChatMessage::user("Hello"),
        ];
        let conv = parse_conversation(msgs).unwrap();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new("claude-sonnet"), tenant);

        assert_eq!(program.system_prompt().as_str(), "You are helpful");
        assert_eq!(program.model().as_str(), "claude-sonnet");
        assert_eq!(program.conversation().prompt().as_str(), "Hello");
    }

    #[test]
    fn chat_program_with_system_override() {
        use crate::model::ModelId;
        use agent_fw_core::tenant::TenantContext;

        let msgs = vec![ChatMessage::user("Hello")];
        let conv = parse_conversation(msgs).unwrap();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new("model"), tenant)
            .with_system(SystemPrompt::new("Custom system"));

        assert_eq!(program.system_prompt().as_str(), "Custom system");
    }

    #[test]
    fn chat_role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ChatRole::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&ChatRole::Assistant).unwrap(),
            "\"assistant\""
        );
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    struct TestInteraction {
        tool: String,
        value: u32,
    }

    #[test]
    fn chat_message_serialized_tool_interactions_round_trip() {
        let msg = ChatMessage::assistant("done")
            .with_serialized_tool_interactions(vec![TestInteraction {
                tool: "calc".to_string(),
                value: 7,
            }])
            .unwrap();

        let decoded = msg
            .deserialize_tool_interactions::<TestInteraction>()
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded,
            vec![TestInteraction {
                tool: "calc".to_string(),
                value: 7,
            }]
        );
    }
}
