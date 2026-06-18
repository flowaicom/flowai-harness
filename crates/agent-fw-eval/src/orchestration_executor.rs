//! OrchestrationSampleExecutor — default SampleExecutor that delegates to an agent.
//!
//! # Design
//!
//! Every domain application needs a `SampleExecutor` that runs test cases
//! against a real agent. This default implementation delegates to a
//! `SubAgentInvoker`, extracting tool trajectories from the response.
//!
//! # Laws
//!
//! - L1 (Timeout): Execution respects the provided timeout
//! - L2 (Trajectory extraction): Tool calls are extracted from response content
//! - L3 (Usage forwarding): Token usage from the agent is forwarded to the output

use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::sub_agent::{SubAgentRequest, SubAgentResult};
use agent_fw_algebra::SubAgentInvoker;
use async_trait::async_trait;

use crate::sample_executor::{
    ResolvedModelConfig, SampleExecutionError, SampleExecutor, SampleInput, SampleOutput,
};
use crate::types::TokenUsageSummary;

/// A `SampleExecutor` that delegates to an agent via `SubAgentInvoker`.
///
/// Sends each test case's input as a prompt to the configured agent,
/// then extracts the tool trajectory from the response.
pub struct OrchestrationSampleExecutor {
    /// The agent invoker (e.g., `AgentOrchestrator`).
    invoker: Arc<dyn SubAgentInvoker>,
    /// Default agent name to invoke (e.g., "coordinator").
    default_agent: String,
}

impl OrchestrationSampleExecutor {
    /// Create a new orchestration executor.
    pub fn new(invoker: Arc<dyn SubAgentInvoker>, default_agent: impl Into<String>) -> Self {
        Self {
            invoker,
            default_agent: default_agent.into(),
        }
    }
}

/// Extract tool names from a response string.
///
/// Looks for tool_use blocks in the format typically emitted by Claude:
/// `{"type": "tool_use", "name": "toolName", ...}`
fn extract_tool_names(response: &str) -> Vec<String> {
    let mut tools = Vec::new();

    // Look for tool_use JSON blocks
    for line in response.lines() {
        let trimmed = line.trim();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if val.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                    tools.push(name.to_string());
                }
            }
        }
    }

    // Also try regex-style extraction for embedded tool_use blocks
    if tools.is_empty() {
        let mut search = response;
        while let Some(idx) = search.find("\"name\"") {
            let rest = &search[idx..];
            // Try to extract the value after "name":
            if let Some(colon) = rest.find(':') {
                let after_colon = rest[colon + 1..].trim_start();
                if after_colon.starts_with('"') {
                    let name_start = 1;
                    if let Some(name_end) = after_colon[name_start..].find('"') {
                        let name = &after_colon[name_start..name_start + name_end];
                        // Only include if it looks like a tool name (camelCase or snake_case)
                        if !name.is_empty()
                            && !name.contains(' ')
                            && name.len() < 64
                            && name != "tool_use"
                        {
                            tools.push(name.to_string());
                        }
                    }
                }
            }
            search = &search[idx + 6..];
        }
    }

    tools
}

#[async_trait]
impl SampleExecutor for OrchestrationSampleExecutor {
    async fn execute(
        &self,
        input: SampleInput,
        _model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<SampleOutput, SampleExecutionError> {
        let start = std::time::Instant::now();

        let request =
            SubAgentRequest::new(self.default_agent.clone(), input.test_case.input.clone())
                .with_invocation_id(format!("eval-{}-s{}", input.run_id, input.sample_index));

        let result: SubAgentResult = match timeout {
            Some(t) => match tokio::time::timeout(t, self.invoker.invoke(request)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    return Err(SampleExecutionError::AgentFailed(e.to_string()));
                }
                Err(_) => {
                    return Err(SampleExecutionError::TimedOut { timeout: t });
                }
            },
            None => self
                .invoker
                .invoke(request)
                .await
                .map_err(|e| SampleExecutionError::AgentFailed(e.to_string()))?,
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let actual_trajectory = extract_tool_names(&result.response);

        Ok(SampleOutput {
            actual_trajectory,
            captured_tool_calls: vec![],
            duration_ms,
            token_usage: TokenUsageSummary::new(
                result.usage.prompt_tokens as u64,
                result.usage.completion_tokens as u64,
                0,
                0,
            ),
            error: None,
            thread_id: Some(result.invocation_id),
            extra: None,
            latency: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tool_names_from_json_lines() {
        let response = r#"
{"type": "tool_use", "name": "draft_plan", "input": {}}
Some text response
{"type": "tool_use", "name": "approve_plan", "input": {}}
"#;
        let tools = extract_tool_names(response);
        assert_eq!(tools, vec!["draft_plan", "approve_plan"]);
    }

    #[test]
    fn extract_empty_from_plain_text() {
        let response = "This is a plain text response with no tool calls.";
        let tools = extract_tool_names(response);
        assert!(tools.is_empty());
    }
}
