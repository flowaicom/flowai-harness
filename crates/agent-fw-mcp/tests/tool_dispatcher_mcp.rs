use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolDispatcher};
use agent_fw_mcp::{McpServerConfig, McpToolServer};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use rmcp::model::{RawContent, Tool};
use serde_json::json;

#[derive(Default)]
struct FakeDispatcher {
    definitions: Vec<ToolDefinition>,
    results: Mutex<HashMap<String, ToolCallResult>>,
    calls: Mutex<Vec<(String, serde_json::Value)>>,
    delay: Option<Duration>,
}

impl FakeDispatcher {
    fn with_definitions(definitions: Vec<ToolDefinition>) -> Self {
        Self {
            definitions,
            ..Self::default()
        }
    }

    fn with_result(self, name: &str, result: ToolCallResult) -> Self {
        self.results
            .lock()
            .unwrap()
            .insert(name.to_string(), result);
        self
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

#[async_trait]
impl ToolDispatcher for FakeDispatcher {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.definitions.clone()
    }

    async fn dispatch(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: serde_json::Value,
    ) -> ToolCallResult {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.calls
            .lock()
            .unwrap()
            .push((tool_name.to_string(), input));
        self.results
            .lock()
            .unwrap()
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| {
                ToolCallResult::error(tool_use_id, format!("Unknown tool: {tool_name}"))
            })
    }
}

fn env() -> ToolEnvironment {
    ToolEnvironment::builder()
        .kv(agent_fw_algebra::testing::NullKVStore)
        .tenant("test")
        .build()
}

fn config() -> McpServerConfig {
    McpServerConfig {
        name: "test-mcp".to_string(),
        version: "0.1.0".to_string(),
        instructions: Some("test instructions".to_string()),
        call_timeout: Duration::from_secs(1),
    }
}

fn input_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "text": {"type": "string"}
        },
        "required": ["text"]
    })
}

fn tool(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: input_schema(),
    }
}

fn schema_value(tool: &Tool) -> serde_json::Value {
    serde_json::Value::Object((*tool.input_schema).clone())
}

fn first_text(result: &rmcp::model::CallToolResult) -> &str {
    match &result.content[0].raw {
        RawContent::Text(text) => &text.text,
        other => panic!("expected text content, got {other:?}"),
    }
}

#[test]
fn list_tools_preserves_names_descriptions_and_input_schema() {
    let dispatcher = Arc::new(FakeDispatcher::with_definitions(vec![
        tool("echo", "Echo text"),
        tool("count", "Count rows"),
    ]));
    let server = McpToolServer::new(dispatcher, env(), config());

    let mut tools = server.list_mcp_tools();
    tools.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "count");
    assert_eq!(tools[0].description.as_deref(), Some("Count rows"));
    assert_eq!(schema_value(&tools[0]), input_schema());
    assert_eq!(tools[1].name, "echo");
    assert_eq!(tools[1].description.as_deref(), Some("Echo text"));
    assert_eq!(schema_value(&tools[1]), input_schema());
}

#[tokio::test]
async fn call_tool_forwards_exact_name_and_json_arguments() {
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("echo", "Echo text")])
            .with_result("echo", ToolCallResult::success("ignored", json!("ok"))),
    );
    let server = McpToolServer::new(dispatcher.clone(), env(), config());

    let result = server
        .call_mcp_tool("echo", json!({"text": "hello"}))
        .await
        .unwrap();

    assert_eq!(first_text(&result), "ok");
    assert_eq!(
        dispatcher.calls.lock().unwrap().as_slice(),
        &[("echo".to_string(), json!({"text": "hello"}))]
    );
}

#[tokio::test]
async fn string_result_maps_to_mcp_text_content() {
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("echo", "Echo text")])
            .with_result("echo", ToolCallResult::success("ignored", json!("hello"))),
    );
    let server = McpToolServer::new(dispatcher, env(), config());

    let result = server
        .call_mcp_tool("echo", json!({"text": "hello"}))
        .await
        .unwrap();

    assert_eq!(first_text(&result), "hello");
    assert_eq!(result.structured_content, None);
    assert_eq!(result.is_error, None);
}

#[tokio::test]
async fn json_result_maps_to_structured_content_with_serialized_text() {
    let payload = json!({"ok": true, "rows": [1, 2, 3]});
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("json", "Return JSON")])
            .with_result("json", ToolCallResult::success("ignored", payload.clone())),
    );
    let server = McpToolServer::new(dispatcher, env(), config());

    let result = server
        .call_mcp_tool("json", json!({"text": "rows"}))
        .await
        .unwrap();

    assert_eq!(result.structured_content, Some(payload.clone()));
    assert_eq!(
        first_text(&result),
        &serde_json::to_string(&payload).unwrap()
    );
}

#[tokio::test]
async fn tool_error_result_maps_to_mcp_error_semantics() {
    let payload = json!({"error": "not allowed"});
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("guarded", "Guarded")]).with_result(
            "guarded",
            ToolCallResult {
                tool_use_id: "ignored".to_string(),
                content: payload.clone(),
                is_error: true,
                approval_dsl: None,
                display_summary: None,
            },
        ),
    );
    let server = McpToolServer::new(dispatcher, env(), config());

    let result = server
        .call_mcp_tool("guarded", json!({"text": "blocked"}))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.structured_content, Some(payload));
}

#[tokio::test]
async fn call_tool_rejects_arguments_that_do_not_match_schema() {
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("echo", "Echo text")])
            .with_result("echo", ToolCallResult::success("ignored", json!("ok"))),
    );
    let server = McpToolServer::new(dispatcher.clone(), env(), config());

    let error = server
        .call_mcp_tool("echo", json!({"text": 42}))
        .await
        .unwrap_err();

    assert!(error.message.contains("do not match schema"));
    assert!(dispatcher.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_tool_returns_mcp_call_error() {
    let dispatcher = Arc::new(FakeDispatcher::with_definitions(vec![tool(
        "known", "Known",
    )]));
    let server = McpToolServer::new(dispatcher, env(), config());

    let error = server
        .call_mcp_tool("missing", json!({}))
        .await
        .unwrap_err();

    assert!(error.message.contains("Unknown tool: missing"));
}

#[tokio::test]
async fn timeout_returns_call_error_containing_tool_name() {
    let dispatcher = Arc::new(
        FakeDispatcher::with_definitions(vec![tool("slow", "Slow")])
            .with_delay(Duration::from_millis(50))
            .with_result("slow", ToolCallResult::success("ignored", json!("late"))),
    );
    let server = McpToolServer::new(
        dispatcher,
        env(),
        McpServerConfig {
            call_timeout: Duration::from_millis(5),
            ..config()
        },
    );

    let error = server
        .call_mcp_tool("slow", json!({"text": "wait"}))
        .await
        .unwrap_err();

    assert!(error.message.contains("slow"));
    assert!(error.message.contains("timed out"));
}
