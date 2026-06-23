use std::sync::Arc;
use std::time::Duration;

use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolDispatcher};
use agent_fw_mcp::{McpHttpServerConfig, McpServerConfig, McpToolServer};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use serde_json::json;

const AUTH_TOKEN: &str = "test-mcp-token";

struct FakeDispatcher;

#[async_trait]
impl ToolDispatcher for FakeDispatcher {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "echo".to_string(),
            description: "Echo text".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }]
    }

    async fn dispatch(
        &self,
        _tool_name: &str,
        tool_use_id: &str,
        input: serde_json::Value,
    ) -> ToolCallResult {
        ToolCallResult::success(tool_use_id, input)
    }
}

fn server() -> McpToolServer {
    let env = ToolEnvironment::builder()
        .kv(agent_fw_algebra::testing::NullKVStore)
        .tenant("test")
        .build();
    McpToolServer::new(
        Arc::new(FakeDispatcher),
        env,
        McpServerConfig {
            name: "test-mcp".to_string(),
            version: "0.1.0".to_string(),
            instructions: None,
            call_timeout: Duration::from_secs(1),
        },
    )
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    origin: Option<&str>,
    session_id: Option<&str>,
    body: serde_json::Value,
) -> reqwest::Response {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(&body);
    if let Some(origin) = origin {
        request = request.header("Origin", origin);
    }
    if let Some(session_id) = session_id {
        request = request.header("Mcp-Session-Id", session_id);
    }
    request = request.header("X-FlowAI-MCP-Token", AUTH_TOKEN);
    request.send().await.unwrap()
}

fn json_from_streamable_body(text: &str) -> serde_json::Value {
    let payload = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap_or(text);
    serde_json::from_str(payload).unwrap()
}

#[tokio::test]
async fn streamable_http_initialize_and_tools_list_succeed() {
    let bound = server()
        .bind_streamable_http(McpHttpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: vec!["http://localhost:3000".to_string()],
            require_origin: true,
            auth_token: Some(AUTH_TOKEN.to_string()),
        })
        .await
        .unwrap();
    let endpoint = bound.endpoint_url();
    let handle = tokio::spawn(bound.serve());

    let client = reqwest::Client::new();
    let initialize = post_json(
        &client,
        &endpoint,
        Some("http://localhost:3000"),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1.0"}
            }
        }),
    )
    .await;
    let initialize_status = initialize.status();
    let initialize_headers = initialize.headers().clone();
    let initialize_body = initialize.text().await.unwrap();
    assert!(
        initialize_status.is_success(),
        "initialize failed with {initialize_status}: {initialize_body}"
    );
    let session_id = initialize_headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let initialize_json = json_from_streamable_body(&initialize_body);
    assert_eq!(initialize_json["id"], 1);
    assert!(initialize_json.get("result").is_some());

    let tools = post_json(
        &client,
        &endpoint,
        Some("http://localhost:3000"),
        session_id.as_deref(),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;
    let tools_status = tools.status();
    let tools_body = tools.text().await.unwrap();
    assert!(
        tools_status.is_success(),
        "tools/list failed with {tools_status}: {tools_body}"
    );
    let tools_json = json_from_streamable_body(&tools_body);
    assert_eq!(tools_json["id"], 2);
    assert_eq!(tools_json["result"]["tools"][0]["name"], "echo");

    handle.abort();
}

#[tokio::test]
async fn streamable_http_rejects_missing_authentication() {
    let bound = server()
        .bind_streamable_http(McpHttpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: vec!["http://localhost:3000".to_string()],
            require_origin: true,
            auth_token: Some(AUTH_TOKEN.to_string()),
        })
        .await
        .unwrap();
    let endpoint = bound.endpoint_url();
    let handle = tokio::spawn(bound.serve());

    let response = reqwest::Client::new()
        .post(&endpoint)
        .header("Origin", "http://localhost:3000")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1.0"}
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    handle.abort();
}

#[tokio::test]
async fn streamable_http_allows_cors_preflight_without_credentials() {
    let bound = server()
        .bind_streamable_http(McpHttpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: vec!["http://localhost:3000".to_string()],
            require_origin: true,
            auth_token: Some(AUTH_TOKEN.to_string()),
        })
        .await
        .unwrap();
    let endpoint = bound.endpoint_url();
    let handle = tokio::spawn(bound.serve());

    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &endpoint)
        .header("Origin", "http://localhost:3000")
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "x-flowai-mcp-token, content-type")
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "preflight failed with {}",
        response.status()
    );
    handle.abort();
}

#[tokio::test]
async fn streamable_http_rejects_disallowed_origin() {
    let bound = server()
        .bind_streamable_http(McpHttpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: vec!["http://localhost:3000".to_string()],
            require_origin: true,
            auth_token: Some(AUTH_TOKEN.to_string()),
        })
        .await
        .unwrap();
    let endpoint = bound.endpoint_url();
    let handle = tokio::spawn(bound.serve());

    let response = post_json(
        &reqwest::Client::new(),
        &endpoint,
        Some("http://evil.example"),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1.0"}
            }
        }),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    handle.abort();
}
