//! Tool-based ActionDispatcher — bridges Plan execution to tool calls.
//!
//! # Architecture
//!
//! This module provides the bridge between:
//! - The Plan system (state machine, lifecycle management)
//! - The Tool system (tool execution)
//!
//! `ToolActionDispatcher` implements `ActionDispatcher` by delegating each
//! action to a `ToolExecutor`. The executor is injected at construction,
//! following the Reader pattern (dependency as data, not as global state).
//!
//! A `StubToolExecutor` is provided for tests — it records calls without
//! side effects. Production executors wire into the actual tool registry.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::action::ActionSeq;
use crate::executor::ActionDispatcher;
use crate::plan::ExecutionResult;

/// Error from a single tool execution.
///
/// Returned by [`ToolExecutor::execute`]. Discriminates failure modes
/// with structured fields so callers can decide retry policy (e.g.,
/// retry on `Timeout`, fail fast on `NotFound`) without parsing strings.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolExecutionError {
    /// The tool name was not found in the registry.
    #[error("tool not found: {tool_name}")]
    NotFound { tool_name: String },
    /// The tool executed but returned an error.
    #[error("tool `{tool_name}` failed: {message}")]
    Failed { tool_name: String, message: String },
    /// The tool execution timed out.
    #[error("tool `{tool_name}` timed out: {message}")]
    Timeout { tool_name: String, message: String },
    /// Input serialization/validation error.
    #[error("invalid input for `{tool_name}`: {details}")]
    InvalidInput { tool_name: String, details: String },
}

/// Error from tool-based dispatching (orchestration-level).
///
/// Wraps [`ToolExecutionError`] from individual tool calls and adds
/// serialization errors for action conversion.
#[derive(Debug, thiserror::Error)]
pub enum ToolDispatchError {
    /// A tool call failed (wraps the structured execution error).
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(#[from] ToolExecutionError),
    /// Action serialization/conversion error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// =============================================================================
// ToolExecutor — the dependency abstraction
// =============================================================================

/// Abstraction for executing a single tool call.
///
/// Implementations wire into the actual tool registry (ToolEnvironment,
/// HTTP agent runtime, etc.). The plan crate doesn't know *how* tools
/// run — it only knows *that* they can be called by name with JSON input.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with JSON input.
    async fn execute(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolExecutionError>;
}

/// Stub executor for tests — records calls, always succeeds.
///
/// Access recorded calls via [`StubToolExecutor::calls()`] for assertions.
pub struct StubToolExecutor {
    calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
}

impl StubToolExecutor {
    pub fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Get all recorded tool calls as `(name, input)` pairs.
    pub fn calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for StubToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for StubToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        self.calls
            .lock()
            .unwrap()
            .push((tool_name.to_string(), input.clone()));
        Ok(serde_json::json!({ "status": "ok" }))
    }
}

// =============================================================================
// Context and Action types
// =============================================================================

/// Context for tool-based execution.
///
/// Typed fields only — no untyped property bags. If domain-specific
/// context is needed, extend this struct with typed fields or use
/// the `TypeMap`-based `ToolEnvironment` from the tool crate.
#[derive(Clone, Default)]
pub struct ToolExecutionContext {
    /// Optional workspace filter.
    pub workspace_id: Option<String>,
}

/// A generic action that maps to a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAction {
    /// Name of the tool to invoke.
    pub tool_name: String,
    /// Tool input as JSON.
    pub input: serde_json::Value,
    /// Optional description for logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// =============================================================================
// ToolActionDispatcher — the concrete ActionDispatcher
// =============================================================================

/// Action dispatcher that executes actions via tool calls.
///
/// Delegates each `ToolAction` to the injected `ToolExecutor`.
/// Tool name aliases allow domain actions to map to physical tool names.
pub struct ToolActionDispatcher {
    /// Tool name aliases (e.g., "logical_action" -> "physical_tool").
    aliases: HashMap<String, String>,
    /// The actual tool executor (injected dependency).
    executor: Arc<dyn ToolExecutor>,
}

impl ToolActionDispatcher {
    /// Create a new tool action dispatcher with a real executor.
    ///
    /// Starts with an empty alias map. Use [`with_aliases`](Self::with_aliases)
    /// to inject domain-specific name mappings from the application layer.
    pub fn new(executor: Arc<dyn ToolExecutor>) -> Self {
        Self {
            aliases: HashMap::new(),
            executor,
        }
    }

    /// Create with domain-specific aliases.
    ///
    /// Aliases map logical action names to physical tool names
    /// (e.g., `"logical_action" -> "physical_tool"`). This belongs in
    /// the application layer, not hardcoded in infrastructure.
    pub fn with_aliases(executor: Arc<dyn ToolExecutor>, aliases: HashMap<String, String>) -> Self {
        Self { aliases, executor }
    }

    /// Create with the stub executor and no aliases (for tests).
    pub fn stub() -> Self {
        Self::new(Arc::new(StubToolExecutor::new()))
    }

    /// Resolve a tool name through aliases.
    fn resolve_tool_name(&self, name: &str) -> String {
        self.aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }
}

#[async_trait]
impl ActionDispatcher for ToolActionDispatcher {
    type Action = ToolAction;
    type Context = ToolExecutionContext;
    type Error = ToolDispatchError;

    async fn dispatch(
        &self,
        actions: &ActionSeq<ToolAction>,
        ctx: &ToolExecutionContext,
    ) -> Result<ExecutionResult, ToolDispatchError> {
        let mut entities_affected = 0;
        let mut summaries = Vec::new();

        for (i, action) in actions.iter().enumerate() {
            let tool_name = self.resolve_tool_name(&action.tool_name);

            tracing::info!(
                tool_name = %tool_name,
                action_index = i,
                description = %action.description.as_deref().unwrap_or("no description"),
                workspace_id = ?ctx.workspace_id,
                "Executing tool action"
            );

            match self.executor.execute(&tool_name, &action.input).await {
                Ok(_output) => {
                    entities_affected += 1;
                    summaries.push(format!("Executed: {}", tool_name));
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }

        Ok(ExecutionResult {
            entities_affected,
            summary: Some(summaries.join(", ")),
            details: None,
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::single_action;

    #[test]
    fn tool_action_serialization() {
        let action = ToolAction {
            tool_name: "approve_plan".to_string(),
            input: serde_json::json!({ "plan_id": "plan-123" }),
            description: Some("Test action".to_string()),
        };

        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("approve_plan"));
        assert!(json.contains("plan-123"));
    }

    #[test]
    fn no_default_aliases() {
        // Infrastructure layer has no domain aliases — they're injected by the application.
        let dispatcher = ToolActionDispatcher::stub();
        assert_eq!(
            dispatcher.resolve_tool_name("logical_action"),
            "logical_action"
        );
        assert_eq!(dispatcher.resolve_tool_name("approve_plan"), "approve_plan");
    }

    #[tokio::test]
    async fn tool_dispatcher_executes_via_executor() {
        let executor = Arc::new(StubToolExecutor::new());
        let dispatcher = ToolActionDispatcher::new(Arc::clone(&executor) as Arc<dyn ToolExecutor>);
        let ctx = ToolExecutionContext::default();

        let action = ToolAction {
            tool_name: "approve_plan".to_string(),
            input: serde_json::json!({"planId": "p-1"}),
            description: None,
        };

        let actions = single_action(action);
        let result = dispatcher.dispatch(&actions, &ctx).await.unwrap();

        assert_eq!(result.entities_affected, 1);

        // Verify the executor was actually called
        let calls = executor.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "approve_plan");
        assert_eq!(calls[0].1["planId"], "p-1");
    }

    #[tokio::test]
    async fn tool_dispatcher_applies_injected_aliases() {
        let executor = Arc::new(StubToolExecutor::new());
        let mut aliases = HashMap::new();
        aliases.insert("logical_action".into(), "approve_plan".into());
        let dispatcher = ToolActionDispatcher::with_aliases(
            Arc::clone(&executor) as Arc<dyn ToolExecutor>,
            aliases,
        );
        let ctx = ToolExecutionContext::default();

        let action = ToolAction {
            tool_name: "logical_action".to_string(), // alias -> approve_plan
            input: serde_json::json!({}),
            description: None,
        };

        let actions = single_action(action);
        dispatcher.dispatch(&actions, &ctx).await.unwrap();

        let calls = executor.calls();
        assert_eq!(calls[0].0, "approve_plan"); // resolved through alias
    }

    #[tokio::test]
    async fn tool_dispatcher_propagates_executor_errors() {
        /// Executor that always fails.
        struct FailingExecutor;

        #[async_trait]
        impl ToolExecutor for FailingExecutor {
            async fn execute(
                &self,
                _tool_name: &str,
                _input: &serde_json::Value,
            ) -> Result<serde_json::Value, ToolExecutionError> {
                Err(ToolExecutionError::Failed {
                    tool_name: _tool_name.to_string(),
                    message: "connection refused".to_string(),
                })
            }
        }

        let dispatcher = ToolActionDispatcher::new(Arc::new(FailingExecutor));
        let ctx = ToolExecutionContext::default();

        let action = ToolAction {
            tool_name: "approve_plan".to_string(),
            input: serde_json::json!({}),
            description: None,
        };

        let actions = single_action(action);
        let err = dispatcher.dispatch(&actions, &ctx).await.unwrap_err();
        assert!(matches!(err, ToolDispatchError::ExecutionFailed(_)));
        assert!(err.to_string().contains("connection refused"));
    }

    #[tokio::test]
    async fn stub_convenience_constructor() {
        let dispatcher = ToolActionDispatcher::stub();
        let ctx = ToolExecutionContext::default();

        let action = ToolAction {
            tool_name: "anyTool".to_string(),
            input: serde_json::json!({}),
            description: None,
        };

        let actions = single_action(action);
        let result = dispatcher.dispatch(&actions, &ctx).await.unwrap();
        assert_eq!(result.entities_affected, 1);
    }
}
