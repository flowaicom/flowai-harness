//! Sample executor algebra.
//!
//! Defines the trait for executing a single sample (test case against agent runtime).
//!
//! # Laws
//!
//! - **L1 Override-wins**: `ResolvedModelConfig::resolve` uses explicit values over defaults
//! - **L2 Timeout-respected**: Execution does not exceed the provided timeout
//! - **L3 Latency-present**: `duration_ms > 0` for any non-trivial execution

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::stream_capture::CapturedToolCall;
use crate::types::{EvalMode, EvalTestCase, TokenUsageSummary};

/// Input to a sample executor.
#[derive(Debug, Clone)]
pub struct SampleInput {
    /// The test case being evaluated.
    pub test_case: EvalTestCase,
    /// Sample index within the test case (0-indexed).
    pub sample_index: u32,
    /// Which mode the eval is running in.
    pub eval_mode: EvalMode,
    /// Optional concrete target agent for agent-addressed eval modes.
    pub target_agent_id: Option<String>,
    /// The eval run ID for correlation.
    pub run_id: String,
}

/// Resolved model configuration (provider + model pair).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModelConfig {
    /// Provider name (e.g., "anthropic", "openai").
    pub provider: String,
    /// Model identifier (e.g., "claude-sonnet-4-5-20250514").
    pub model: String,
}

impl ResolvedModelConfig {
    /// Resolve model config using override-wins semantics.
    ///
    /// `eval_provider`/`eval_model` are per-eval overrides.
    /// `default_provider`/`default_model` are fallbacks from project config.
    ///
    /// Law L1: explicit values win over defaults.
    pub fn resolve(
        eval_provider: Option<&str>,
        eval_model: Option<&str>,
        default_provider: &str,
        default_model: &str,
    ) -> Self {
        Self {
            provider: eval_provider.unwrap_or(default_provider).to_string(),
            model: eval_model.unwrap_or(default_model).to_string(),
        }
    }

    /// Convert to a model spec string (e.g., "anthropic/claude-sonnet-4-5-20250514").
    pub fn to_model_spec(&self, default_provider: &str) -> String {
        if self.provider == default_provider {
            self.model.clone()
        } else {
            format!("{}/{}", self.provider, self.model)
        }
    }
}

/// Output from a sample execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleOutput {
    /// Actual tool trajectory observed.
    pub actual_trajectory: Vec<String>,
    /// Captured tool calls with args/results, when the runtime exposes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captured_tool_calls: Vec<CapturedToolCall>,
    /// Execution wall-clock time in milliseconds.
    pub duration_ms: u64,
    /// Token usage from this sample.
    pub token_usage: TokenUsageSummary,
    /// Error message if the execution failed.
    pub error: Option<String>,
    /// Thread ID for conversation replay.
    pub thread_id: Option<String>,
    /// Domain-specific metadata.
    pub extra: Option<serde_json::Value>,
    /// Structured latency breakdown from agent execution.
    /// When present, provides TTFT, phase breakdown, tool timings, retry info.
    /// This replaces the opaque `extra` field for latency data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<agent_fw_core::LatencySummary>,
}

/// Errors from sample execution.
#[derive(Debug, thiserror::Error)]
pub enum SampleExecutionError {
    #[error("agent execution failed: {0}")]
    AgentFailed(String),
    #[error("sample execution timed out after {}ms", timeout.as_millis())]
    TimedOut { timeout: Duration },
    #[error("sample execution cancelled")]
    Cancelled,
    #[error("internal error: {0}")]
    Internal(String),
}

/// Async trait for executing a single sample.
///
/// Implementations may call an agent runtime via HTTP, invoke a local function, etc.
#[async_trait]
pub trait SampleExecutor: Send + Sync {
    /// Execute a single sample, returning the output or an error.
    async fn execute(
        &self,
        input: SampleInput,
        model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<SampleOutput, SampleExecutionError>;
}

/// Stub executor for testing — copies expected trajectory and returns hardcoded usage.
pub struct StubSampleExecutor;

#[async_trait]
impl SampleExecutor for StubSampleExecutor {
    async fn execute(
        &self,
        input: SampleInput,
        _model_config: &ResolvedModelConfig,
        _timeout: Option<Duration>,
    ) -> Result<SampleOutput, SampleExecutionError> {
        Ok(SampleOutput {
            actual_trajectory: input.test_case.expected_trajectory.clone(),
            captured_tool_calls: vec![],
            duration_ms: 100,
            token_usage: TokenUsageSummary::new(100, 50, 0, 0),
            error: None,
            thread_id: Some(format!("stub-{}", input.sample_index)),
            extra: None,
            latency: None,
        })
    }
}

/// Timeout-aware executor for testing law L2.
///
/// Simulates an executor that takes `delay` to complete. When the caller
/// provides a `timeout` shorter than `delay`, execution returns
/// `SampleExecutionError::TimedOut`. Otherwise it sleeps for `delay` and
/// returns a normal `SampleOutput`.
#[cfg(any(test, feature = "test-support"))]
pub struct TimeoutSampleExecutor {
    /// How long the executor's simulated work takes.
    pub delay: Duration,
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl SampleExecutor for TimeoutSampleExecutor {
    async fn execute(
        &self,
        input: SampleInput,
        _model_config: &ResolvedModelConfig,
        timeout: Option<Duration>,
    ) -> Result<SampleOutput, SampleExecutionError> {
        match timeout {
            Some(t) => {
                match tokio::time::timeout(t, tokio::time::sleep(self.delay)).await {
                    Ok(()) => {
                        // Sleep completed before timeout — success
                        Ok(SampleOutput {
                            actual_trajectory: input.test_case.expected_trajectory.clone(),
                            captured_tool_calls: vec![],
                            duration_ms: self.delay.as_millis() as u64,
                            token_usage: TokenUsageSummary::new(100, 50, 0, 0),
                            error: None,
                            thread_id: Some(format!("timeout-{}", input.sample_index)),
                            extra: None,
                            latency: None,
                        })
                    }
                    Err(_) => {
                        // Timeout fired first
                        Err(SampleExecutionError::TimedOut { timeout: t })
                    }
                }
            }
            None => {
                // No timeout — just sleep and return success
                tokio::time::sleep(self.delay).await;
                Ok(SampleOutput {
                    actual_trajectory: input.test_case.expected_trajectory.clone(),
                    captured_tool_calls: vec![],
                    duration_ms: self.delay.as_millis() as u64,
                    token_usage: TokenUsageSummary::new(100, 50, 0, 0),
                    error: None,
                    thread_id: Some(format!("timeout-{}", input.sample_index)),
                    extra: None,
                    latency: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TrajectoryMode;
    use agent_fw_core::TestCaseId;

    fn make_test_case() -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            tags: vec![],
            input: "test query".into(),
            expected_trajectory: vec!["draft_plan".into(), "approve_plan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        }
    }

    /// L1: Override-wins
    #[test]
    fn resolve_override_wins() {
        let config = ResolvedModelConfig::resolve(
            Some("openai"),
            Some("gpt-4"),
            "anthropic",
            "claude-sonnet",
        );
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4");
    }

    #[test]
    fn resolve_uses_defaults() {
        let config = ResolvedModelConfig::resolve(None, None, "anthropic", "claude-sonnet");
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet");
    }

    #[test]
    fn to_model_spec_same_provider() {
        let config = ResolvedModelConfig {
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
        };
        assert_eq!(config.to_model_spec("anthropic"), "claude-sonnet");
    }

    #[test]
    fn to_model_spec_different_provider() {
        let config = ResolvedModelConfig {
            provider: "openai".into(),
            model: "gpt-4".into(),
        };
        assert_eq!(config.to_model_spec("anthropic"), "openai/gpt-4");
    }

    #[tokio::test]
    async fn stub_executor_copies_trajectory() {
        let executor = StubSampleExecutor;
        let input = SampleInput {
            test_case: make_test_case(),
            sample_index: 0,
            eval_mode: EvalMode::Sequential,
            target_agent_id: None,
            run_id: "run-1".into(),
        };
        let config = ResolvedModelConfig {
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
        };
        let output = executor.execute(input, &config, None).await.unwrap();
        assert_eq!(
            output.actual_trajectory,
            vec!["draft_plan".to_string(), "approve_plan".to_string()]
        );
        assert!(output.duration_ms > 0);
    }
}
