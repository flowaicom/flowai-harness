use std::{pin::Pin, sync::Arc, time::Duration};

use agent_fw_agent::{ChatInterpreter, ChatProgram, ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_algebra::{testing::NullEventSink, CancellationToken, EventSink};
use agent_fw_catalog::{
    CatalogError, CatalogSearchBackend, CatalogSearchHealth, CatalogSearchRequest,
    CatalogSearchResults,
};
use agent_fw_core::{tenant::TenantContext, StreamPart, TenantId};
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use flowai_runtime::{
    AgentRole, AgentSpec, ApprovalRule, HostToolBinding, ModelSpec, ProviderConfig, Runtime,
    RuntimeDeps, RuntimeMcpConfig, RuntimeMcpError, RuntimeSpec, ToolkitSpec,
};
use futures::{stream, Stream};
use serde_json::json;

struct NoopInterpreter;

impl ChatInterpreter for NoopInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        Box::pin(stream::empty())
    }
}

struct ReadyCatalogSearchBackend;

#[async_trait]
impl CatalogSearchBackend for ReadyCatalogSearchBackend {
    async fn search(
        &self,
        _scope: &agent_fw_catalog::CatalogScope,
        _request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogError> {
        Ok(CatalogSearchResults {
            hits: vec![],
            facets: Default::default(),
            has_more: false,
            next_cursor: None,
            candidate_count: 0,
            warnings: vec![],
        })
    }

    async fn health(
        &self,
        _scope: &agent_fw_catalog::CatalogScope,
    ) -> Result<CatalogSearchHealth, CatalogError> {
        Ok(CatalogSearchHealth::Ready {
            indexed_entries: 0,
            projection_version: 1,
        })
    }
}

#[derive(Clone)]
struct EchoTool;

#[async_trait]
impl ToolHandler for EchoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "echo".to_string(),
            description: "Echo a JSON payload".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"message": {"type": "string"}},
                "required": ["message"]
            }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        _env: &ToolEnvironment,
    ) -> ToolCallResult {
        ToolCallResult::success(tool_use_id, input)
    }
}

fn spec_with_agent(agent: AgentSpec) -> RuntimeSpec {
    let mut spec = RuntimeSpec::minimal("tenant-1", "v1");
    spec.providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    spec.agents.push(agent);
    spec
}

fn deps() -> RuntimeDeps {
    RuntimeDeps::new(
        Arc::new(NoopInterpreter),
        Arc::new(NullEventSink) as Arc<dyn EventSink>,
        TenantContext::new(TenantId::new_unchecked("tenant-1")),
        Arc::new(DashMapKVStore::new()),
    )
}

fn deps_with_catalog_search() -> RuntimeDeps {
    deps().with_catalog_search_backend(Arc::new(ReadyCatalogSearchBackend))
}

fn runtime_with_echo_tool(approval: Option<ApprovalRule>) -> Arc<Runtime> {
    let agent = AgentSpec::new(
        "mcp",
        AgentRole::Specialist,
        ModelSpec::new("claude-sonnet-4-6"),
        "You expose MCP tools.",
    );
    let mut binding = HostToolBinding::new(Arc::new(EchoTool));
    if let Some(rule) = approval {
        binding = binding.with_approval(rule);
    }
    Arc::new(
        Runtime::new(
            spec_with_agent(agent),
            deps().with_host_tool("mcp", binding),
        )
        .expect("runtime should build"),
    )
}

fn first_text(result: &agent_fw_mcp::McpToolServer, name: &str) -> Option<String> {
    result
        .list_mcp_tools()
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.description.unwrap_or_default().into_owned())
}

fn result_text(result: &agent_fw_mcp::McpToolServer) -> usize {
    result.list_mcp_tools().len()
}

fn call_result_text(result: &rmcp::model::CallToolResult) -> String {
    serde_json::to_value(&result.content[0]).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn runtime_mcp_lists_host_tool() {
    let server = runtime_with_echo_tool(None)
        .mcp_tool_server(RuntimeMcpConfig::new("mcp"))
        .expect("mcp server");

    assert_eq!(result_text(&server), 1);
    assert_eq!(
        first_text(&server, "echo").as_deref(),
        Some("Echo a JSON payload")
    );
}

#[tokio::test]
async fn runtime_mcp_calls_host_tool_with_json_arguments() {
    let server = runtime_with_echo_tool(None)
        .mcp_tool_server(RuntimeMcpConfig::new("mcp"))
        .expect("mcp server");

    let result = server
        .call_mcp_tool("echo", json!({"message": "hello"}))
        .await
        .unwrap();

    assert_eq!(result.structured_content, Some(json!({"message": "hello"})));
}

#[test]
fn runtime_mcp_lists_catalog_toolkit_tools() {
    let mut agent = AgentSpec::new(
        "catalog",
        AgentRole::Specialist,
        ModelSpec::new("claude-sonnet-4-6"),
        "Search catalog metadata.",
    );
    agent.toolkits = vec!["catalog".to_string()];
    let mut spec = spec_with_agent(agent);
    spec.toolkits = vec![ToolkitSpec {
        id: "catalog".to_string(),
        config: serde_json::Value::Null,
    }];
    let runtime =
        Arc::new(Runtime::new(spec, deps_with_catalog_search()).expect("runtime should build"));

    let server = runtime
        .mcp_tool_server(RuntimeMcpConfig::new("catalog"))
        .expect("mcp server");
    let names = server
        .list_mcp_tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(names.len(), 7);
    for name in [
        "search_catalog",
        "get_catalog_entities",
        "list_schema_fields",
        "get_catalog_relations",
        "get_relation_paths_between",
        "sample_table_data",
        "execute_query",
    ] {
        assert!(names.contains(name), "missing catalog MCP tool {name}");
    }
}

#[test]
fn runtime_mcp_missing_agent_returns_error() {
    let runtime = runtime_with_echo_tool(None);
    match runtime.mcp_tool_server(RuntimeMcpConfig::new("missing")) {
        Err(RuntimeMcpError::MissingAgent(agent)) => assert_eq!(agent, "missing"),
        Err(error) => panic!("expected missing-agent error, got {error}"),
        Ok(_) => panic!("expected missing-agent error"),
    }
}

#[tokio::test]
async fn runtime_mcp_approval_gated_tool_errors_without_approval_channel() {
    let server = runtime_with_echo_tool(Some(ApprovalRule::Always))
        .mcp_tool_server(RuntimeMcpConfig {
            call_timeout: Duration::from_millis(250),
            ..RuntimeMcpConfig::new("mcp")
        })
        .expect("mcp server");

    let result = server
        .call_mcp_tool("echo", json!({"message": "needs approval"}))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(true));
    assert!(
        call_result_text(&result).contains("Approval event sink closed"),
        "unexpected result: {result:?}"
    );
}

#[test]
fn runtime_mcp_omits_recursive_agent_tools_by_default() {
    let mut agent = AgentSpec::new(
        "coordinator",
        AgentRole::Coordinator,
        ModelSpec::new("claude-sonnet-4-6"),
        "Coordinate work.",
    );
    agent.routes = vec!["worker".to_string()];
    agent.toolkits = vec!["agents".to_string()];
    let worker = AgentSpec::new(
        "worker",
        AgentRole::Specialist,
        ModelSpec::new("claude-sonnet-4-6"),
        "Do work.",
    );
    let mut spec = spec_with_agent(agent);
    spec.agents.push(worker);
    spec.toolkits = vec![ToolkitSpec {
        id: "agents".to_string(),
        config: serde_json::Value::Null,
    }];
    let runtime = Arc::new(Runtime::new(spec, deps()).expect("runtime should build"));

    let server = runtime
        .mcp_tool_server(RuntimeMcpConfig::new("coordinator"))
        .expect("mcp server");

    assert!(server.list_mcp_tools().is_empty());
}

#[tokio::test]
async fn runtime_mcp_http_streamable_http_lists_runtime_tools() {
    let server = runtime_with_echo_tool(None)
        .mcp_tool_server(RuntimeMcpConfig::new("mcp"))
        .expect("mcp server");
    let bound = server
        .bind_streamable_http(agent_fw_mcp::McpHttpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: vec!["http://localhost:3000".to_string()],
            require_origin: true,
        })
        .await
        .unwrap();
    let endpoint = bound.endpoint_url();
    let handle = tokio::spawn(bound.serve());

    let client = reqwest::Client::new();
    let initialize = client
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
    assert!(initialize.status().is_success());

    let tools = client
        .post(&endpoint)
        .header("Origin", "http://localhost:3000")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert!(tools.status().is_success());
    let body: serde_json::Value = serde_json::from_str(&tools.text().await.unwrap()).unwrap();
    assert_eq!(body["result"]["tools"][0]["name"], "echo");

    handle.abort();
}
