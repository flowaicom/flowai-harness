//! Tool dispatch bridge for interpreter-driven tool execution.
//!
//! The `ToolDispatcher` trait bridges the gap between LLM tool_use blocks
//! and the framework's tool system. An interpreter calls `dispatch()` when
//! the model wants to invoke a tool, and the dispatcher routes to the
//! concrete tool handler.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Definition of a tool for registration with an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name (e.g., "search_catalog").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON schema for input parameters.
    pub input_schema: serde_json::Value,
}

impl std::fmt::Display for ToolDefinition {
    /// Reads as a sentence: `"draft_plan: Create a pricing plan"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.description)
    }
}

/// Result of dispatching a tool call.
///
/// Beyond the core `content` (sent to the LLM), a result may carry typed
/// UI channels that the framework routes to the frontend automatically:
///
/// - `approval_dsl` → `DataFlowUI` SSE event (approval card rendering)
/// - `display_summary` → `Text` SSE event (replaces LLM narration)
///
/// These channels never reach the LLM context — keeping tool results clean.
/// The `TracedHandler` combinator emits them as dedicated `StreamPart` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// The tool_use_id from the LLM's request.
    pub tool_use_id: String,
    /// Result content (JSON) — sent to the LLM as tool_result.
    pub content: serde_json::Value,
    /// Whether the tool call resulted in an error.
    pub is_error: bool,
    /// Approval DSL for frontend card rendering (DataFlowUI channel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_dsl: Option<String>,
    /// Display summary text for chat UI (replaces LLM narration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_summary: Option<String>,
}

impl ToolCallResult {
    /// Create a successful tool result.
    pub fn success(tool_use_id: impl Into<String>, content: serde_json::Value) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content,
            is_error: false,
            approval_dsl: None,
            display_summary: None,
        }
    }

    /// Create an error tool result.
    pub fn error(tool_use_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: serde_json::json!({ "error": message.into() }),
            is_error: true,
            approval_dsl: None,
            display_summary: None,
        }
    }

    /// Attach an approval DSL for frontend card rendering.
    ///
    /// The DSL is emitted as a `DataFlowUI` SSE event by the `TracedHandler`.
    /// It never reaches the LLM context.
    ///
    /// ```rust,ignore
    /// ToolCallResult::success(id, payload)
    ///     .with_approval_dsl(card_json)
    /// ```
    pub fn with_approval_dsl(mut self, dsl: impl Into<String>) -> Self {
        self.approval_dsl = Some(dsl.into());
        self
    }

    /// Attach a display summary for the chat UI.
    ///
    /// Emitted as a `Text` SSE event by the `TracedHandler`, replacing
    /// LLM narration. Never reaches the LLM context.
    ///
    /// ```rust,ignore
    /// ToolCallResult::success(id, payload)
    ///     .with_display_summary("Found 142 matching products.")
    /// ```
    pub fn with_display_summary(mut self, summary: impl Into<String>) -> Self {
        self.display_summary = Some(summary.into());
        self
    }
}

/// Trait for dispatching tool calls from an LLM interpreter.
///
/// Implementations bridge the gap between model-requested tool_use blocks
/// and the framework's tool handlers.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    /// Get all tool definitions for inclusion in API requests.
    fn tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Get latent tool definitions that are available for request-scoped activation.
    ///
    /// These tools are implemented by the runtime but intentionally hidden
    /// from the default registry until explicitly activated.
    fn latent_tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    /// Dispatch a single tool call.
    ///
    /// The interpreter calls this when the model emits a tool_use block.
    /// Returns the result to be sent back to the model.
    async fn dispatch(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: serde_json::Value,
    ) -> ToolCallResult;

    /// Return the current tool call ID when the dispatcher is running inside a
    /// hook-aware tool environment.
    ///
    /// Interpreters that do not expose a stable provider tool-call ID can use
    /// this to keep framework progress/card channels correlated with the LLM's
    /// own tool invocation lifecycle.
    fn current_tool_call_id(&self) -> Option<String> {
        None
    }

    /// Shared tool-call-ID cell for hook-driven runtimes.
    ///
    /// Runtimes such as Rig can expose a provider tool-call ID before invoking
    /// the framework dispatcher. Sharing the cell keeps tool progress, cards,
    /// and tool results correlated without bespoke app-side plumbing.
    fn tool_call_id_cell(&self) -> Option<Arc<Mutex<Option<String>>>> {
        None
    }

    /// Shared pending-card cell for hook-driven command-card emission.
    ///
    /// Dispatchers backed by `ToolEnvironment` can expose the same cell used by
    /// their hook bridge so buffered cards flush at the right point in the
    /// streaming lifecycle.
    fn pending_card_cell(&self) -> Option<Arc<Mutex<Option<agent_fw_tool::CommandCardPayload>>>> {
        None
    }

    /// Dispatch using the current hook-provided tool-call ID when available.
    ///
    /// This is the low-ceremony path for runtimes like Rig that do not pass the
    /// provider tool-call ID into the tool implementation directly.
    async fn dispatch_current(&self, tool_name: &str, input: serde_json::Value) -> ToolCallResult {
        let tool_use_id = self
            .current_tool_call_id()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.dispatch(tool_name, &tool_use_id, input).await
    }

    /// Create a request-scoped dispatcher with a different event sink.
    ///
    /// Returns `None` if the dispatcher does not support per-request sink
    /// binding (the default). Implementations that store a `ToolEnvironment`
    /// (e.g., `ComposedDispatcher`) override this to return a clone with
    /// the new sink wired in.
    fn with_event_sink(
        self: Arc<Self>,
        _sink: Arc<dyn agent_fw_algebra::EventSink>,
    ) -> Option<Arc<dyn ToolDispatcher>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_result_success() {
        let result = ToolCallResult::success("id-1", serde_json::json!({"rows": 42}));
        assert!(!result.is_error);
        assert_eq!(result.tool_use_id, "id-1");
    }

    #[test]
    fn tool_call_result_error() {
        let result = ToolCallResult::error("id-2", "Table not found");
        assert!(result.is_error);
        assert_eq!(result.content["error"], "Table not found");
    }

    #[test]
    fn tool_definition_serde_roundtrip() {
        let def = ToolDefinition {
            name: "search".into(),
            description: "Search for things".into(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&def).unwrap();
        let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "search");
    }

    #[test]
    fn tool_definition_display_reads_as_sentence() {
        let def = ToolDefinition {
            name: "draft_plan".into(),
            description: "Create a pricing plan".into(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        assert_eq!(def.to_string(), "draft_plan: Create a pricing plan");
    }
}
