use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agent_fw_agent::{ChatInterpreter, ChatProgram};
use agent_fw_algebra::testing::NullEventSink;
use agent_fw_algebra::CancellationToken;
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{StreamPart, TenantId, ThreadId};
use agent_fw_interpreter::DashMapKVStore;
use clap::{Args, Subcommand, ValueEnum};
use flowai_runtime::storage::DataEnvironmentConfig;
use flowai_runtime::{
    AgentRole, AgentSpec, ModelSpec, ProviderConfig, Runtime, RuntimeDeps, RuntimeMcpConfig,
    RuntimeSpec, ToolkitSpec,
};

use crate::{load_required_data_environment, CliError, CommonArgs};

const MCP_TENANT: &str = "flowai-mcp";
const MCP_VERSION: &str = "v1";

struct NoopInterpreter;

impl ChatInterpreter for NoopInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
        Box::pin(futures::stream::empty())
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Serve built-in toolkit tools as an MCP server.
    Toolkit(McpToolkitArgs),
}

#[derive(Debug, Clone, Args)]
pub(crate) struct McpToolkitArgs {
    /// Toolkit id to expose. Repeat to compose multiple toolkits.
    #[arg(long = "toolkit", required = true)]
    toolkits: Vec<String>,
    /// Synthetic specialist agent name.
    #[arg(long, default_value = "mcp")]
    agent: String,
    /// Tenant id for runtime state and data-environment scope checks.
    #[arg(long, default_value = MCP_TENANT)]
    tenant_id: String,
    /// MCP transport to serve.
    #[arg(long, value_enum, default_value = "stdio")]
    transport: McpTransport,
    /// HTTP bind host for Streamable HTTP.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// HTTP bind port for Streamable HTTP. Use 0 for an ephemeral port.
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// HTTP endpoint path for Streamable HTTP.
    #[arg(long, default_value = "/mcp")]
    path: String,
    /// Allowed browser origin for Streamable HTTP. Repeatable.
    #[arg(long = "allow-origin")]
    allow_origins: Vec<String>,
    /// Disable Streamable HTTP origin checks.
    #[arg(long)]
    no_origin_check: bool,
    /// Optional thread id for tool event correlation.
    #[arg(long)]
    thread_id: Option<String>,
    /// Per-tool call timeout in seconds.
    #[arg(long, default_value_t = 30.0)]
    call_timeout_secs: f64,
    /// Reserved for future recursive agent tools; the agents toolkit is not supported in this mode.
    #[arg(long)]
    expose_agent_tools: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum McpTransport {
    Stdio,
    StreamableHttp,
}

pub(crate) async fn run_mcp_command(
    common: &CommonArgs,
    command: McpCommand,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    match command {
        McpCommand::Toolkit(args) => run_mcp_toolkit(common, args, stderr).await,
    }
}

async fn run_mcp_toolkit(
    common: &CommonArgs,
    args: McpToolkitArgs,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    let data_environment = load_required_data_environment(common)?;
    let runtime = toolkit_runtime(&args, data_environment).await?;
    let server = runtime
        .mcp_tool_server(RuntimeMcpConfig {
            agent: args.agent.clone(),
            thread_id: args.thread_id.clone().map(ThreadId::new_unchecked),
            call_timeout: duration_from_secs(args.call_timeout_secs)?,
            expose_agent_tools: args.expose_agent_tools,
        })
        .map_err(|err| CliError::Execution(err.to_string()))?;

    match args.transport {
        McpTransport::Stdio => server
            .serve_stdio()
            .await
            .map_err(|err| CliError::Execution(err.to_string())),
        McpTransport::StreamableHttp => {
            let bind_addr = parse_bind_addr(&args.host, args.port)?;
            let bound = server
                .bind_streamable_http(agent_fw_mcp::McpHttpServerConfig {
                    bind_addr,
                    endpoint_path: args.path.clone(),
                    allowed_origins: args.allow_origins,
                    require_origin: !args.no_origin_check,
                })
                .await
                .map_err(|err| CliError::Execution(err.to_string()))?;
            writeln!(stderr, "{}", bound.endpoint_url())?;
            bound
                .serve()
                .await
                .map_err(|err| CliError::Execution(err.to_string()))
        }
    }
}

async fn toolkit_runtime(
    args: &McpToolkitArgs,
    data_environment: DataEnvironmentConfig,
) -> Result<Arc<Runtime>, CliError> {
    let tenant_id = TenantId::new(args.tenant_id.clone())
        .ok_or_else(|| CliError::Parse("--tenant-id must not be blank".to_string()))?;
    let mut spec = RuntimeSpec::minimal(tenant_id.as_str(), MCP_VERSION);
    spec.providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(serde_json::json!({"apiKey": "unused"})),
    );
    spec.toolkits = args
        .toolkits
        .iter()
        .map(|id| ToolkitSpec {
            id: id.clone(),
            config: serde_json::Value::Null,
        })
        .collect();
    let mut agent = AgentSpec::new(
        args.agent.clone(),
        AgentRole::Specialist,
        ModelSpec::new("claude-sonnet-4-6"),
        "Expose Flow AI toolkit tools over MCP.",
    );
    agent.toolkits = args.toolkits.clone();
    spec.agents.push(agent);

    let deps = RuntimeDeps::new(
        Arc::new(NoopInterpreter),
        Arc::new(NullEventSink),
        TenantContext::new(tenant_id),
        Arc::new(DashMapKVStore::new()),
    );
    let deps = flowai_runtime::storage::apply_to_runtime_deps(deps, data_environment)
        .await
        .map_err(|err| CliError::Execution(err.to_string()))?;
    Runtime::new(spec, deps)
        .map(Arc::new)
        .map_err(|err| CliError::Execution(err.to_string()))
}

fn duration_from_secs(seconds: f64) -> Result<Duration, CliError> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(CliError::Parse(
            "--call-timeout-secs must be a positive finite number".to_string(),
        ));
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn parse_bind_addr(host: &str, port: u16) -> Result<std::net::SocketAddr, CliError> {
    use std::net::ToSocketAddrs;

    let raw = format!("{host}:{port}");
    raw.to_socket_addrs()
        .map_err(|err| CliError::Parse(format!("invalid MCP bind address {raw}: {err}")))?
        .next()
        .ok_or_else(|| CliError::Parse(format!("invalid MCP bind address {raw}")))
}
