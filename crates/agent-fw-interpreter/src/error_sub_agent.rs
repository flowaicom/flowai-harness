//! Error-returning SubAgentInvoker — used when sub-agents are not configured.

use agent_fw_algebra::sub_agent::{
    SubAgentError, SubAgentInvoker, SubAgentRequest, SubAgentResult,
};
use async_trait::async_trait;

/// A SubAgentInvoker implementation that always returns errors.
pub struct ErrorSubAgentInvoker {
    message: String,
}

impl ErrorSubAgentInvoker {
    /// Create a new error invoker with a custom message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Default for ErrorSubAgentInvoker {
    fn default() -> Self {
        Self::new("sub-agent invoker not configured")
    }
}

#[async_trait]
impl SubAgentInvoker for ErrorSubAgentInvoker {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        Err(SubAgentError::Internal(format!(
            "{} (attempted: {})",
            self.message, request.agent_name
        )))
    }

    fn has_agent(&self, _name: &str) -> bool {
        false
    }

    fn available_agents(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invoke_returns_error() {
        let invoker = ErrorSubAgentInvoker::default();
        let request = SubAgentRequest::new("test", "hello").with_invocation_id("inv-1");
        let result = invoker.invoke(request).await;
        assert!(
            matches!(result, Err(SubAgentError::Internal(msg)) if msg.contains("sub-agent invoker not configured") && msg.contains("test"))
        );
    }

    #[tokio::test]
    async fn custom_message_is_preserved() {
        let invoker = ErrorSubAgentInvoker::new("No LLM configured");
        let request = SubAgentRequest::new("planner", "Do something");
        let result = invoker.invoke(request).await;
        assert!(
            matches!(result, Err(SubAgentError::Internal(msg)) if msg.contains("No LLM configured") && msg.contains("planner"))
        );
    }

    #[test]
    fn has_agent_returns_false() {
        let invoker = ErrorSubAgentInvoker::default();
        assert!(!invoker.has_agent("anything"));
    }

    #[test]
    fn available_agents_returns_empty() {
        let invoker = ErrorSubAgentInvoker::default();
        assert!(invoker.available_agents().is_empty());
    }
}
