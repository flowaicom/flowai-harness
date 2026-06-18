//! `agents` toolkit — Flow AI-routed sub-agent delegation.
//!
//! runtime ownership keeps the framework's
//! [`CallAgentHandler`](agent_fw_agent::CallAgentHandler) generic and
//! unrestricted, while the Flow AI harness wraps it with route-policy
//! enforcement from [`AgentSpec::routes`](crate::AgentSpec::routes).

use std::sync::Arc;

use agent_fw_agent::{CallAgentHandler, ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;

use super::{filter_by_config, ToolkitConfig, ToolkitError};

pub(super) fn handlers(
    toolkit_id: &str,
    cfg: &ToolkitConfig,
    allowed_routes: Vec<String>,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    let handlers: Vec<Arc<dyn ToolHandler>> =
        vec![Arc::new(RoutedCallAgentHandler::new(allowed_routes))];
    filter_by_config(toolkit_id, handlers, cfg)
}

#[derive(Debug, Clone)]
struct RoutedCallAgentHandler {
    allowed_routes: Vec<String>,
    inner: CallAgentHandler,
}

impl RoutedCallAgentHandler {
    fn new(allowed_routes: Vec<String>) -> Self {
        Self {
            allowed_routes,
            inner: CallAgentHandler,
        }
    }
}

#[async_trait]
impl ToolHandler for RoutedCallAgentHandler {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let agent = match input.get("agent").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolCallResult::error(tool_use_id, "Missing 'agent' name string"),
        };

        if !self.allowed_routes.iter().any(|route| route == agent) {
            return ToolCallResult::error(
                tool_use_id,
                format!(
                    "Agent '{agent}' is not in this caller's allowed routes: {:?}",
                    self.allowed_routes
                ),
            );
        }

        self.inner.handle(tool_use_id, input, env).await
    }
}
