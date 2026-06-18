//! Bridge `ToolDispatcher` into Rig dynamic tools.
//!
//! This removes a common integration seam for framework users: when the app
//! already has a `ToolDispatcher`, it should not have to re-register the same
//! tools a second time just to use a Rig-backed runtime.

use std::sync::Arc;

use rig::completion::ToolDefinition as RigToolDefinition;
use rig::tool::{ToolDyn, ToolError};

use crate::{ToolCallResult, ToolDefinition, ToolDispatcher};

/// Dynamic Rig tool backed by a framework [`ToolDispatcher`].
pub struct DispatcherRigTool {
    definition: ToolDefinition,
    dispatcher: Arc<dyn ToolDispatcher>,
}

impl DispatcherRigTool {
    pub fn new(definition: ToolDefinition, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        Self {
            definition,
            dispatcher,
        }
    }
}

impl ToolDyn for DispatcherRigTool {
    fn name(&self) -> String {
        self.definition.name.clone()
    }

    fn definition<'a>(
        &'a self,
        _prompt: String,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, RigToolDefinition> {
        let definition = self.definition.clone();
        Box::pin(async move {
            RigToolDefinition {
                name: definition.name,
                description: definition.description,
                parameters: definition.input_schema,
            }
        })
    }

    fn call<'a>(
        &'a self,
        args: String,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<String, ToolError>> {
        let dispatcher = Arc::clone(&self.dispatcher);
        let tool_name = self.definition.name.clone();
        Box::pin(async move {
            let args = serde_json::from_str(&args)?;
            let result = dispatcher.dispatch_current(&tool_name, args).await;
            serde_json::to_string(&tool_result_payload(result)).map_err(ToolError::from)
        })
    }
}

/// Convert every dispatcher definition into a Rig dynamic tool.
pub fn dispatcher_rig_tools(dispatcher: Arc<dyn ToolDispatcher>) -> Vec<Box<dyn ToolDyn>> {
    dispatcher
        .tool_definitions()
        .into_iter()
        .map(|definition| {
            Box::new(DispatcherRigTool::new(definition, Arc::clone(&dispatcher)))
                as Box<dyn ToolDyn>
        })
        .collect()
}

/// Ergonomic bridge from any concrete [`ToolDispatcher`] into Rig dynamic tools.
///
/// This removes the need to allocate the intermediate `Arc<dyn ToolDispatcher>`
/// at every call site when the caller already owns a concrete dispatcher value.
pub trait ToolDispatcherRigExt: ToolDispatcher + Sized + 'static {
    fn into_rig_tools(self) -> Vec<Box<dyn ToolDyn>> {
        dispatcher_rig_tools(Arc::new(self))
    }
}

impl<T> ToolDispatcherRigExt for T where T: ToolDispatcher + Sized + 'static {}

fn tool_result_payload(result: ToolCallResult) -> serde_json::Value {
    let ToolCallResult {
        content,
        is_error,
        approval_dsl,
        display_summary,
        ..
    } = result;

    let mut value = content;

    let needs_object =
        is_error || approval_dsl.is_some() || display_summary.is_some() || !value.is_object();

    if needs_object {
        let mut obj = match value {
            serde_json::Value::Object(obj) => obj,
            other => {
                let mut obj = serde_json::Map::new();
                obj.insert("value".to_string(), other);
                obj
            }
        };

        if is_error {
            obj.entry("isError".to_string())
                .or_insert_with(|| serde_json::json!(true));
        }
        if let Some(dsl) = approval_dsl {
            obj.insert("approvalDsl".to_string(), serde_json::json!(dsl));
        }
        if let Some(summary) = display_summary {
            obj.insert("displaySummary".to_string(), serde_json::json!(summary));
        }

        value = serde_json::Value::Object(obj);
    }

    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct TestDispatcher {
        tool_call_id: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl ToolDispatcher for TestDispatcher {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "echo".to_string(),
                description: "Echo input".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "msg": { "type": "string" } },
                    "required": ["msg"],
                }),
            }]
        }

        async fn dispatch(
            &self,
            tool_name: &str,
            tool_use_id: &str,
            input: serde_json::Value,
        ) -> ToolCallResult {
            ToolCallResult::success(
                tool_use_id,
                serde_json::json!({
                    "tool": tool_name,
                    "input": input,
                }),
            )
            .with_display_summary("Echo complete")
            .with_approval_dsl("{\"card\":true}")
        }

        fn current_tool_call_id(&self) -> Option<String> {
            self.tool_call_id
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    #[tokio::test]
    async fn dispatcher_rig_tool_uses_current_tool_call_id() {
        let dispatcher: Arc<dyn ToolDispatcher> = Arc::new(TestDispatcher {
            tool_call_id: Arc::new(Mutex::new(Some("call-123".to_string()))),
        });
        let tools = dispatcher_rig_tools(dispatcher);
        let result = tools[0]
            .call("{\"msg\":\"hello\"}".to_string())
            .await
            .expect("tool result");
        let json: serde_json::Value = serde_json::from_str(&result).expect("json");

        assert_eq!(json["tool"], "echo");
        assert_eq!(json["input"]["msg"], "hello");
        assert_eq!(json["approvalDsl"], "{\"card\":true}");
        assert_eq!(json["displaySummary"], "Echo complete");
    }

    #[test]
    fn tool_result_payload_preserves_error_shape() {
        let payload = tool_result_payload(ToolCallResult::error("call-1", "boom"));
        assert_eq!(payload["error"], "boom");
        assert_eq!(payload["isError"], true);
    }
}
