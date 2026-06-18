//! Sub-agent invocation trait.
//!
//! # Laws
//!
//! - L1. Usage tracking: every invocation returns usage metrics
//! - L2. Cancellation: respects CancellationToken
//! - L3. Streaming: events are emitted to EventSink during execution
//!
//! # Protocol Note
//!
//! The orchestrator is responsible for emitting wrapper events around sub-agent
//! invocations: `StreamPart::sub_agent_call` before execution and
//! `StreamPart::sub_agent_result` after. The `SubAgentInvoker` trait itself
//! does not mandate event emission — that is the interpreter's responsibility.

use agent_fw_core::stream_part::CostSummary;
use agent_fw_core::{LatencySummary, StreamBuilderError, TokenUsage};
use async_trait::async_trait;

/// How an invocation should derive its conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadScope {
    /// Use the orchestrator's current thread unchanged.
    Current,
    /// Derive a child thread from the current thread and target agent name.
    Derived,
}

/// Error type for sub-agent operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SubAgentError {
    #[error("agent not found: {0}")]
    NotFound(String),
    #[error("agent invocation cancelled")]
    Cancelled,
    #[error("agent failed: {0}")]
    AgentFailed(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<StreamBuilderError> for SubAgentError {
    fn from(err: StreamBuilderError) -> Self {
        Self::Internal(format!("Stream protocol violation: {err}"))
    }
}

/// Request to invoke a sub-agent.
#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    /// Name of the agent to invoke.
    pub agent_name: String,
    /// Prompt/instruction for the agent.
    pub prompt: String,
    /// Unique invocation ID for correlation. Auto-generated if `None`.
    pub invocation_id: Option<String>,
    /// Optional context from parent agent.
    pub context: Option<serde_json::Value>,
    /// Thread scope for conversation memory and interpreter context.
    pub thread_scope: ThreadScope,
}

impl SubAgentRequest {
    /// Create a new request with auto-generated invocation ID.
    pub fn new(agent_name: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            prompt: prompt.into(),
            invocation_id: None,
            context: None,
            thread_scope: ThreadScope::Derived,
        }
    }

    /// Resolve the invocation ID, generating one if not set.
    pub fn resolved_invocation_id(&self) -> String {
        self.invocation_id
            .clone()
            .unwrap_or_else(|| format!("inv-{}", uuid::Uuid::new_v4()))
    }

    /// Set a specific invocation ID.
    pub fn with_invocation_id(mut self, id: impl Into<String>) -> Self {
        self.invocation_id = Some(id.into());
        self
    }

    /// Set context for the sub-agent.
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }

    /// Use the orchestrator's current thread unchanged.
    pub fn with_current_thread(mut self) -> Self {
        self.thread_scope = ThreadScope::Current;
        self
    }

    /// Explicitly set the invocation thread scope.
    pub fn with_thread_scope(mut self, scope: ThreadScope) -> Self {
        self.thread_scope = scope;
        self
    }
}

/// Result of a sub-agent invocation.
#[derive(Debug, Clone)]
pub struct SubAgentResult {
    /// Name of the invoked agent.
    pub agent_name: String,
    /// Unique invocation ID.
    pub invocation_id: String,
    /// Agent's response text.
    pub response: String,
    /// Token usage for this invocation.
    pub usage: TokenUsage,
    /// Model used by the agent.
    pub model: String,
    /// Latency summary from the sub-agent execution.
    pub latency: Option<LatencySummary>,
}

impl SubAgentResult {
    /// Create a new sub-agent result.
    pub fn new(
        agent_name: impl Into<String>,
        invocation_id: impl Into<String>,
        response: impl Into<String>,
        usage: TokenUsage,
        model: impl Into<String>,
    ) -> Self {
        Self {
            agent_name: agent_name.into(),
            invocation_id: invocation_id.into(),
            response: response.into(),
            usage,
            model: model.into(),
            latency: None,
        }
    }

    /// Attach latency information.
    pub fn with_latency(mut self, latency: Option<LatencySummary>) -> Self {
        self.latency = latency;
        self
    }
}

/// Trait for invoking sub-agents in a multi-agent network.
#[async_trait]
pub trait SubAgentInvoker: Send + Sync {
    /// Invoke a sub-agent with the given request.
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError>;

    /// Check if a named agent is available.
    fn has_agent(&self, name: &str) -> bool;

    /// List available agent names.
    fn available_agents(&self) -> Vec<String>;

    /// Retrieve aggregated cost summary across all invocations.
    ///
    /// Returns `None` if the implementation does not track costs.
    /// Implementations that track per-agent usage should return a
    /// `CostSummary` with per-agent breakdowns.
    fn cost_summary(&self) -> Option<CostSummary> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::PhaseBreakdown;

    #[test]
    fn sub_agent_request_builder() {
        let request = SubAgentRequest::new("planner", "Plan this task")
            .with_invocation_id("inv-123")
            .with_context(serde_json::json!({"key": "value"}));

        assert_eq!(request.agent_name, "planner");
        assert_eq!(request.prompt, "Plan this task");
        assert_eq!(request.invocation_id, Some("inv-123".to_string()));
        assert_eq!(request.context, Some(serde_json::json!({"key": "value"})));
        assert_eq!(request.thread_scope, ThreadScope::Derived);
    }

    #[test]
    fn sub_agent_result_defaults_latency_to_none() {
        let result = SubAgentResult::new(
            "planner",
            "inv-123",
            "Here is the plan",
            TokenUsage::simple(100, 50),
            "gpt-4",
        );

        assert_eq!(result.agent_name, "planner");
        assert_eq!(result.invocation_id, "inv-123");
        assert_eq!(result.response, "Here is the plan");
        assert_eq!(result.usage.prompt_tokens, 100);
        assert_eq!(result.model, "gpt-4");
        assert!(result.latency.is_none());
    }

    #[test]
    fn sub_agent_result_with_latency() {
        let latency = LatencySummary {
            total_duration_ms: 500,
            phases: PhaseBreakdown::ZERO.with_sub_agent_time(500),
            ..Default::default()
        };

        let result = SubAgentResult::new(
            "planner",
            "inv-123",
            "Plan",
            TokenUsage::simple(100, 50),
            "gpt-4",
        )
        .with_latency(Some(latency));

        assert_eq!(
            result.latency.as_ref().map(|l| l.total_duration_ms),
            Some(500)
        );
    }
}
