use std::{borrow::Cow, sync::Arc};

use agent_fw_agent::{ToolCallResult, ToolDefinition};
use rmcp::model::{CallToolResult, Content, JsonObject, Tool};
use serde_json::Value;

use crate::McpError;

pub(crate) fn definition_to_mcp_tool(definition: ToolDefinition) -> Result<Tool, McpError> {
    let input_schema = match definition.input_schema {
        Value::Object(schema) => schema,
        _ => {
            return Err(McpError::InvalidInputSchema {
                tool_name: definition.name,
            });
        }
    };

    Ok(Tool::new_with_raw(
        Cow::Owned(definition.name),
        Some(Cow::Owned(definition.description)),
        Arc::<JsonObject>::new(input_schema),
    ))
}

pub(crate) fn tool_result_to_mcp_result(result: ToolCallResult) -> CallToolResult {
    let (content, structured_content) = match result.content {
        Value::String(text) => (vec![Content::text(text)], None),
        value => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
            (vec![Content::text(text)], Some(value))
        }
    };

    let mut result = if result.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = structured_content;
    if result.is_error == Some(false) {
        result.is_error = None;
    }
    result
}
