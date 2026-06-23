//! MCP server primitives for framework tool dispatchers.

mod error;
mod schema;
mod transport;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use agent_fw_agent::ToolDispatcher;
use agent_fw_tool::ToolEnvironment;
use jsonschema::{Draft, JSONSchema};
use rmcp::{
    handler::server::tool::{ToolCallContext, ToolRoute, ToolRouter},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo, Tool},
    ErrorData, ServerHandler,
};
use serde_json::Value;

pub use error::McpError;
pub use transport::McpBoundHttpServer;

const TOOL_INPUT_SCHEMA_DRAFT: Draft = Draft::Draft202012;
const MAX_MCP_TOOL_ARGUMENT_BYTES: usize = 64 * 1024;

/// Metadata and execution limits for an MCP tool server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Server name returned during MCP initialization.
    pub name: String,
    /// Server version returned during MCP initialization.
    pub version: String,
    /// Optional instructions returned to clients during initialization.
    pub instructions: Option<String>,
    /// Maximum duration allowed for a single tool call.
    pub call_timeout: Duration,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "agent-fw-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            instructions: None,
            call_timeout: Duration::from_secs(30),
        }
    }
}

/// Streamable HTTP transport configuration.
#[derive(Debug, Clone)]
pub struct McpHttpServerConfig {
    /// Socket address to bind. Defaults to loopback.
    pub bind_addr: std::net::SocketAddr,
    /// HTTP path where the MCP service is mounted.
    pub endpoint_path: String,
    /// Browser origins allowed by the Streamable HTTP origin validator.
    pub allowed_origins: Vec<String>,
    /// Whether configured origins should be enforced for requests that carry an Origin header.
    pub require_origin: bool,
    /// Required bearer/header token for all Streamable HTTP requests.
    pub auth_token: Option<String>,
}

impl Default for McpHttpServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8765"
                .parse()
                .expect("default MCP bind address must be valid"),
            endpoint_path: "/mcp".to_string(),
            allowed_origins: Vec::new(),
            require_origin: true,
            auth_token: None,
        }
    }
}

/// MCP server facade for any framework [`ToolDispatcher`].
#[derive(Clone)]
pub struct McpToolServer {
    dispatcher: Arc<dyn ToolDispatcher>,
    env: ToolEnvironment,
    tool_router: ToolRouter<Self>,
    tool_names: Arc<BTreeSet<String>>,
    tool_schemas: Arc<BTreeMap<String, Value>>,
    config: McpServerConfig,
}

impl McpToolServer {
    /// Build an MCP server from a framework tool dispatcher.
    ///
    /// Panics if any framework tool definition contains a non-object input schema.
    /// Use [`Self::try_new`] to handle schema conversion errors explicitly.
    pub fn new(
        dispatcher: Arc<dyn ToolDispatcher>,
        env: ToolEnvironment,
        config: McpServerConfig,
    ) -> Self {
        Self::try_new(dispatcher, env, config)
            .expect("framework tool definitions must be valid MCP tool definitions")
    }

    /// Fallible constructor that validates tool definitions before serving.
    pub fn try_new(
        dispatcher: Arc<dyn ToolDispatcher>,
        env: ToolEnvironment,
        config: McpServerConfig,
    ) -> Result<Self, McpError> {
        let definitions = dispatcher.tool_definitions();
        let tool_schemas = definitions
            .iter()
            .map(|definition| (definition.name.clone(), definition.input_schema.clone()))
            .collect::<BTreeMap<_, _>>();
        let tools = definitions
            .into_iter()
            .map(schema::definition_to_mcp_tool)
            .collect::<Result<Vec<_>, _>>()?;
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<BTreeSet<_>>();
        let tool_router = build_tool_router(&tools);

        Ok(Self {
            dispatcher,
            env,
            tool_router,
            tool_names: Arc::new(tool_names),
            tool_schemas: Arc::new(tool_schemas),
            config,
        })
    }

    /// Return the MCP tool list exposed by this server.
    pub fn list_mcp_tools(&self) -> Vec<Tool> {
        self.tool_router.list_all()
    }

    /// Call a tool using MCP semantics without going through a transport.
    pub async fn call_mcp_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.tool_names.contains(tool_name) {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool: {tool_name}"),
                None,
            ));
        }
        if !arguments.is_object() {
            return Err(ErrorData::invalid_params(
                format!("MCP tool arguments for `{tool_name}` must be a JSON object"),
                None,
            ));
        }
        self.validate_tool_arguments(tool_name, &arguments)?;

        let tool_use_id = uuid::Uuid::new_v4().to_string();
        let result = tokio::time::timeout(
            self.config.call_timeout,
            self.dispatcher.dispatch(tool_name, &tool_use_id, arguments),
        )
        .await
        .map_err(|_| error::timeout_error(tool_name, self.config.call_timeout))?;

        Ok(schema::tool_result_to_mcp_result(result))
    }

    /// Access the framework tool environment captured by this server.
    pub fn environment(&self) -> &ToolEnvironment {
        &self.env
    }

    fn validate_tool_arguments(&self, tool_name: &str, arguments: &Value) -> Result<(), ErrorData> {
        let encoded = serde_json::to_vec(arguments).map_err(|error| {
            ErrorData::invalid_params(
                format!("MCP tool arguments for `{tool_name}` could not be encoded: {error}"),
                None,
            )
        })?;
        if encoded.len() > MAX_MCP_TOOL_ARGUMENT_BYTES {
            return Err(ErrorData::invalid_params(
                format!(
                    "MCP tool arguments for `{tool_name}` exceed maximum size of {MAX_MCP_TOOL_ARGUMENT_BYTES} bytes"
                ),
                None,
            ));
        }
        let schema = self.tool_schemas.get(tool_name).ok_or_else(|| {
            ErrorData::invalid_params(format!("Unknown tool: {tool_name}"), None)
        })?;
        let compiled = JSONSchema::options()
            .with_draft(TOOL_INPUT_SCHEMA_DRAFT)
            .compile(schema)
            .map_err(|error| {
                ErrorData::invalid_params(
                    format!("MCP tool `{tool_name}` input schema is invalid: {error}"),
                    None,
                )
            })?;
        if let Err(errors) = compiled.validate(arguments) {
            let messages = errors
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ErrorData::invalid_params(
                format!("MCP tool arguments for `{tool_name}` do not match schema: {messages}"),
                None,
            ));
        }
        Ok(())
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for McpToolServer {
    fn get_info(&self) -> ServerInfo {
        let info =
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
                Implementation::new(self.config.name.clone(), self.config.version.clone()),
            );
        match &self.config.instructions {
            Some(instructions) => info.with_instructions(instructions.clone()),
            None => info,
        }
    }
}

fn build_tool_router(tools: &[Tool]) -> ToolRouter<McpToolServer> {
    let mut router = ToolRouter::new();
    for tool in tools {
        router.add_route(ToolRoute::new_dyn(
            tool.clone(),
            |context: ToolCallContext<'_, McpToolServer>| {
                let tool_name = context.name().to_string();
                let arguments = context
                    .arguments
                    .clone()
                    .map(Value::Object)
                    .unwrap_or_else(|| Value::Object(Default::default()));
                Box::pin(async move { context.service.call_mcp_tool(&tool_name, arguments).await })
            },
        ));
    }
    router
}
