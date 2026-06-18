//! Pure chat message data shared by framework layers.

/// Message role.
///
/// Serializes as lowercase strings: `"user"`, `"assistant"`, `"system"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// A chat message in a conversation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Tool call/result pairs from this turn.
    ///
    /// The core stores these opaquely as JSON; consumers define their own
    /// format.
    pub tool_interactions: Option<Vec<serde_json::Value>>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tool_interactions: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_interactions: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            tool_interactions: None,
        }
    }

    /// Attach tool interactions to this message.
    pub fn with_tool_interactions(mut self, interactions: Vec<serde_json::Value>) -> Self {
        self.tool_interactions = Some(interactions);
        self
    }

    /// Attach typed tool interactions by serializing them to opaque JSON.
    pub fn with_serialized_tool_interactions<T: serde::Serialize>(
        mut self,
        interactions: Vec<T>,
    ) -> Result<Self, serde_json::Error> {
        self.tool_interactions = Some(
            interactions
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?,
        );
        Ok(self)
    }

    /// Deserialize opaque tool interactions into a typed payload family.
    pub fn deserialize_tool_interactions<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<Option<Vec<T>>, serde_json::Error> {
        self.tool_interactions
            .as_ref()
            .map(|items| {
                items
                    .iter()
                    .cloned()
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ChatRole::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&ChatRole::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&ChatRole::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn chat_message_serialized_tool_interactions_round_trip() {
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct ToolTrace {
            name: String,
        }

        let msg = ChatMessage::assistant("done")
            .with_serialized_tool_interactions(vec![ToolTrace {
                name: "lookup".into(),
            }])
            .unwrap();
        let traces: Option<Vec<ToolTrace>> = msg.deserialize_tool_interactions().unwrap();
        assert_eq!(
            traces,
            Some(vec![ToolTrace {
                name: "lookup".into()
            }])
        );
    }
}
