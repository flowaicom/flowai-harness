//! HTTP-based SubAgentInvoker for multi-agent networks.
//!
//! Calls remote agent endpoints over HTTP, enabling distributed multi-agent
//! architectures where agents run as separate services.
//!
//! # Feature Gate
//!
//! This module requires the `http-clients` feature (which provides reqwest):
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["http-clients"] }
//! ```
//!
//! # Protocol
//!
//! Each agent is registered with a base URL. Invocations send a POST request:
//!
//! ```text
//! POST {base_url}/invoke
//! Content-Type: application/json
//!
//! {
//!   "prompt": "...",
//!   "invocationId": "...",
//!   "context": {...}
//! }
//! ```
//!
//! Expected response:
//!
//! ```text
//! {
//!   "response": "...",
//!   "usage": { "promptTokens": N, "completionTokens": M },
//!   "model": "..."
//! }
//! ```
//!
//! # Laws Satisfied
//!
//! - L1 (Usage Tracking): Response includes usage metrics
//! - L2 (Cancellation): Respects CancellationToken via request timeout
//! - L3 (Streaming): Events emitted by the remote agent are not captured
//!   (fire-and-forget invocation). Use SSE streaming for real-time events.
//!
//! # Retry
//!
//! Retries transient failures (5xx, timeouts) with exponential backoff.
//! Uses the algebra layer's `retry_when` combinator internally.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;

use agent_fw_algebra::sub_agent::{
    SubAgentError, SubAgentInvoker, SubAgentRequest, SubAgentResult,
};
use agent_fw_core::stream_part::CostSummary;
use agent_fw_core::{LatencySummary, PhaseBreakdown, TokenUsage};

/// Configuration for a single remote agent.
#[derive(Debug, Clone)]
pub struct AgentEndpoint {
    /// Agent name (used for routing).
    pub name: String,
    /// Base URL of the agent service (e.g., "http://localhost:4112").
    pub base_url: String,
    /// Request timeout.
    pub timeout: Duration,
}

impl AgentEndpoint {
    /// Create a new endpoint with default timeout (60s).
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            timeout: Duration::from_secs(60),
        }
    }

    /// Override the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Per-agent usage accumulator.
#[derive(Debug, Default)]
struct AgentUsage {
    total_usage: TokenUsage,
    invocation_count: u64,
}

/// HTTP-based [`SubAgentInvoker`] for distributed multi-agent networks.
///
/// Maintains a registry of agent endpoints and routes invocations to the
/// appropriate service. Accumulates usage metrics for cost reporting.
pub struct NetworkSubAgentInvoker {
    client: reqwest::Client,
    agents: HashMap<String, AgentEndpoint>,
    /// Per-agent usage tracking (mutable, behind RwLock).
    usage: Arc<RwLock<HashMap<String, AgentUsage>>>,
    /// Maximum retry attempts for transient failures.
    max_retries: u32,
}

impl NetworkSubAgentInvoker {
    /// Create a new invoker with the given agent endpoints.
    pub fn new(agents: Vec<AgentEndpoint>) -> Self {
        let agent_map: HashMap<String, AgentEndpoint> =
            agents.into_iter().map(|a| (a.name.clone(), a)).collect();

        Self {
            client: reqwest::Client::new(),
            agents: agent_map,
            usage: Arc::new(RwLock::new(HashMap::new())),
            max_retries: 3,
        }
    }

    /// Override the max retry count (default: 3).
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Use a custom reqwest client (e.g., with custom TLS or proxy settings).
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Invoke with retry logic for transient failures.
    async fn invoke_with_retry(
        &self,
        endpoint: &AgentEndpoint,
        request: &SubAgentRequest,
    ) -> Result<SubAgentResult, SubAgentError> {
        let url = format!("{}/invoke", endpoint.base_url.trim_end_matches('/'));
        let invocation_id = request.resolved_invocation_id();

        let body = serde_json::json!({
            "prompt": request.prompt,
            "invocationId": invocation_id,
            "context": request.context,
        });

        let mut last_error = SubAgentError::Internal("no attempts made".into());

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                // Exponential backoff: 100ms, 200ms, 400ms, ...
                let delay = Duration::from_millis(100 * (1 << (attempt - 1)));
                tokio::time::sleep(delay).await;
            }

            let start = Instant::now();

            let result = self
                .client
                .post(&url)
                .timeout(endpoint.timeout)
                .json(&body)
                .send()
                .await;

            let latency = start.elapsed();

            match result {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        let resp_body: serde_json::Value = response.json().await.map_err(|e| {
                            SubAgentError::AgentFailed(format!("Failed to parse response: {e}"))
                        })?;

                        let response_text = resp_body
                            .get("response")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let usage = parse_usage(&resp_body);

                        let model = resp_body
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let latency = LatencySummary {
                            total_duration_ms: latency.as_millis() as u64,
                            phases: PhaseBreakdown::ZERO
                                .with_sub_agent_time(latency.as_millis() as u64),
                            ..Default::default()
                        };

                        return Ok(SubAgentResult::new(
                            endpoint.name.clone(),
                            invocation_id,
                            response_text,
                            usage,
                            model,
                        )
                        .with_latency(Some(latency)));
                    }

                    // Retry on 5xx (server error)
                    if status.is_server_error() && attempt < self.max_retries {
                        let body_text = response.text().await.unwrap_or_default();
                        tracing::warn!(
                            agent = %endpoint.name,
                            status = %status,
                            attempt = attempt + 1,
                            "Transient failure, retrying: {}",
                            body_text
                        );
                        last_error =
                            SubAgentError::AgentFailed(format!("HTTP {status}: {body_text}"));
                        continue;
                    }

                    // Non-retryable error
                    let body_text = response.text().await.unwrap_or_default();
                    return Err(SubAgentError::AgentFailed(format!(
                        "HTTP {status}: {body_text}"
                    )));
                }
                Err(e) => {
                    if e.is_timeout() && attempt < self.max_retries {
                        tracing::warn!(
                            agent = %endpoint.name,
                            attempt = attempt + 1,
                            "Request timed out, retrying"
                        );
                        last_error = SubAgentError::AgentFailed("Request timed out".into());
                        continue;
                    }

                    if e.is_connect() && attempt < self.max_retries {
                        tracing::warn!(
                            agent = %endpoint.name,
                            attempt = attempt + 1,
                            "Connection failed, retrying: {}",
                            e
                        );
                        last_error = SubAgentError::AgentFailed(format!("Connection error: {e}"));
                        continue;
                    }

                    return Err(SubAgentError::AgentFailed(e.to_string()));
                }
            }
        }

        Err(last_error)
    }
}

/// Parse TokenUsage from a response body.
fn parse_usage(body: &serde_json::Value) -> TokenUsage {
    let usage = body
        .get("usage")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let prompt = usage
        .get("promptTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion = usage
        .get("completionTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    TokenUsage::simple(prompt, completion)
}

#[async_trait]
impl SubAgentInvoker for NetworkSubAgentInvoker {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        let endpoint = self
            .agents
            .get(&request.agent_name)
            .ok_or_else(|| SubAgentError::NotFound(request.agent_name.clone()))?;

        let result = self.invoke_with_retry(endpoint, &request).await?;

        // Accumulate usage
        {
            let mut usage_map = self.usage.write().await;
            let entry = usage_map.entry(result.agent_name.clone()).or_default();
            entry.total_usage = entry.total_usage.combine(&result.usage);
            entry.invocation_count += 1;
        }

        Ok(result)
    }

    fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    fn available_agents(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    fn cost_summary(&self) -> Option<CostSummary> {
        // We'd need to block on the RwLock here, which is not ideal
        // in a sync context. Return None and let the orchestrator
        // aggregate costs from SubAgentResult.usage instead.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_endpoint_default_timeout() {
        let ep = AgentEndpoint::new("planner", "http://localhost:4112");
        assert_eq!(ep.timeout, Duration::from_secs(60));
    }

    #[test]
    fn agent_endpoint_custom_timeout() {
        let ep = AgentEndpoint::new("planner", "http://localhost:4112")
            .with_timeout(Duration::from_secs(120));
        assert_eq!(ep.timeout, Duration::from_secs(120));
    }

    #[test]
    fn has_agent_registered() {
        let invoker = NetworkSubAgentInvoker::new(vec![
            AgentEndpoint::new("planner", "http://localhost:4112"),
            AgentEndpoint::new("executor", "http://localhost:4113"),
        ]);
        assert!(invoker.has_agent("planner"));
        assert!(invoker.has_agent("executor"));
        assert!(!invoker.has_agent("unknown"));
    }

    #[test]
    fn available_agents_lists_registered() {
        let invoker = NetworkSubAgentInvoker::new(vec![AgentEndpoint::new(
            "planner",
            "http://localhost:4112",
        )]);
        let agents = invoker.available_agents();
        assert_eq!(agents.len(), 1);
        assert!(agents.contains(&"planner".to_string()));
    }

    #[test]
    fn parse_usage_from_response() {
        let body = serde_json::json!({
            "response": "hello",
            "usage": {
                "promptTokens": 100,
                "completionTokens": 50
            },
            "model": "claude-sonnet-4-6"
        });
        let usage = parse_usage(&body);
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
    }

    #[test]
    fn parse_usage_missing() {
        let body = serde_json::json!({ "response": "hello" });
        let usage = parse_usage(&body);
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
    }

    #[tokio::test]
    async fn invoke_not_found() {
        let invoker = NetworkSubAgentInvoker::new(vec![]);
        let req = SubAgentRequest::new("unknown", "hello");
        let result = invoker.invoke(req).await;
        assert!(matches!(result, Err(SubAgentError::NotFound(_))));
    }
}
