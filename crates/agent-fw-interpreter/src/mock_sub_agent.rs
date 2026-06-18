//! Mock SubAgentInvoker for testing multi-agent scenarios.
//!
//! This implementation provides configurable responses for sub-agent invocations,
//! enabling isolated testing of coordinator agents without real LLM calls.

use agent_fw_algebra::sub_agent::{
    SubAgentError, SubAgentInvoker, SubAgentRequest, SubAgentResult,
};
use agent_fw_core::TokenUsage;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// Configurable mock response for a sub-agent.
#[derive(Debug, Clone)]
pub struct MockAgentResponse {
    /// Response text to return.
    pub response: String,
    /// Token usage to report.
    pub usage: TokenUsage,
    /// Model name to report.
    pub model: String,
    /// Optional error to return instead of response.
    pub error: Option<SubAgentError>,
}

impl Default for MockAgentResponse {
    fn default() -> Self {
        Self {
            response: "Mock agent response".to_string(),
            usage: TokenUsage::simple(50, 25),
            model: "mock-model".to_string(),
            error: None,
        }
    }
}

impl MockAgentResponse {
    /// Create a successful response.
    pub fn success(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            ..Default::default()
        }
    }

    /// Create a successful response with custom usage.
    pub fn with_usage(mut self, prompt: u64, completion: u64) -> Self {
        self.usage = TokenUsage::simple(prompt, completion);
        self
    }

    /// Create a successful response with custom model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Create an error response.
    pub fn error(error: SubAgentError) -> Self {
        Self {
            error: Some(error),
            ..Default::default()
        }
    }
}

/// Record of a sub-agent invocation.
#[derive(Debug, Clone)]
pub struct InvocationRecord {
    /// Agent name that was invoked.
    pub agent_name: String,
    /// Prompt that was sent.
    pub prompt: String,
    /// Invocation ID.
    pub invocation_id: String,
    /// Context if provided.
    pub context: Option<serde_json::Value>,
}

/// Mock SubAgentInvoker for testing.
pub struct MockSubAgentInvoker {
    /// Configured responses by agent name.
    responses: HashMap<String, MockAgentResponse>,
    /// Default response for unknown agents (if set).
    default_response: Option<MockAgentResponse>,
    /// Recorded invocations.
    invocations: Mutex<Vec<InvocationRecord>>,
}

impl MockSubAgentInvoker {
    /// Create a new mock invoker with no configured agents.
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            default_response: None,
            invocations: Mutex::new(Vec::new()),
        }
    }

    /// Configure a response for a specific agent.
    pub fn with_agent(mut self, name: impl Into<String>, response: MockAgentResponse) -> Self {
        self.responses.insert(name.into(), response);
        self
    }

    /// Set a default response for any agent not explicitly configured.
    pub fn with_default(mut self, response: MockAgentResponse) -> Self {
        self.default_response = Some(response);
        self
    }

    /// Get recorded invocations.
    pub fn invocations(&self) -> Vec<InvocationRecord> {
        self.invocations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Clear recorded invocations.
    pub fn clear_invocations(&self) {
        self.invocations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Get invocations for a specific agent.
    pub fn invocations_for(&self, agent_name: &str) -> Vec<InvocationRecord> {
        self.invocations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|r| r.agent_name == agent_name)
            .cloned()
            .collect()
    }

    /// Get the count of invocations for a specific agent.
    pub fn invocation_count(&self, agent_name: &str) -> usize {
        self.invocations_for(agent_name).len()
    }
}

impl Default for MockSubAgentInvoker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SubAgentInvoker for MockSubAgentInvoker {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        let invocation_id = request.resolved_invocation_id();

        {
            let mut invocations = self.invocations.lock().unwrap_or_else(|e| e.into_inner());
            invocations.push(InvocationRecord {
                agent_name: request.agent_name.clone(),
                prompt: request.prompt.clone(),
                invocation_id: invocation_id.clone(),
                context: request.context.clone(),
            });
        }

        let response = self
            .responses
            .get(&request.agent_name)
            .or(self.default_response.as_ref())
            .ok_or_else(|| SubAgentError::NotFound(request.agent_name.clone()))?;

        if let Some(error) = &response.error {
            return Err(error.clone());
        }

        Ok(SubAgentResult::new(
            &request.agent_name,
            invocation_id,
            &response.response,
            response.usage.clone(),
            &response.model,
        ))
    }

    fn has_agent(&self, name: &str) -> bool {
        self.responses.contains_key(name) || self.default_response.is_some()
    }

    fn available_agents(&self) -> Vec<String> {
        self.responses.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_configured_response() {
        let mock = MockSubAgentInvoker::new()
            .with_agent("planner", MockAgentResponse::success("The plan is ready"));

        let result = mock
            .invoke(SubAgentRequest::new("planner", "Plan task"))
            .await
            .expect("mock planner should return configured response");

        assert_eq!(result.agent_name, "planner");
        assert_eq!(result.response, "The plan is ready");
    }

    #[tokio::test]
    async fn mock_records_invocations() {
        let mock =
            MockSubAgentInvoker::new().with_agent("planner", MockAgentResponse::success("Done"));

        let _ = mock.invoke(SubAgentRequest::new("planner", "Task 1")).await;
        let _ = mock.invoke(SubAgentRequest::new("planner", "Task 2")).await;

        let invocations = mock.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].prompt, "Task 1");
        assert_eq!(invocations[1].prompt, "Task 2");
    }

    #[tokio::test]
    async fn mock_returns_not_found_for_unknown_agent() {
        let mock = MockSubAgentInvoker::new();
        let result = mock.invoke(SubAgentRequest::new("unknown", "Task")).await;
        assert!(matches!(result, Err(SubAgentError::NotFound(_))));
    }

    #[tokio::test]
    async fn mock_returns_configured_error() {
        let mock = MockSubAgentInvoker::new().with_agent(
            "planner",
            MockAgentResponse::error(SubAgentError::AgentFailed("boom".to_string())),
        );

        let result = mock.invoke(SubAgentRequest::new("planner", "Task")).await;
        assert!(matches!(result, Err(SubAgentError::AgentFailed(msg)) if msg == "boom"));
    }
}
