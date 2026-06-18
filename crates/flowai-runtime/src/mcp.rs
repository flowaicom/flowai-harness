//! MCP server construction for Flow AI runtime tools.
//!
//! This module only builds an MCP tool server. Transport concerns remain in
//! `agent-fw-mcp`, and Python callback handling remains in the language facade.

use std::{sync::Arc, time::Duration};

use agent_fw_agent::{ComposedDispatcher, ToolDispatcher};
use agent_fw_algebra::{
    sub_agent::{SubAgentError, SubAgentRequest, SubAgentResult},
    EventSink, SubAgentInvoker,
};
use agent_fw_core::{CostSummary, StreamPart, ThreadId};
use async_trait::async_trait;
use thiserror::Error;

use crate::{compose_dispatcher_for_agent, AgentSpec, Runtime, RuntimeError};

/// Runtime-backed MCP server options.
#[derive(Clone, Debug)]
pub struct RuntimeMcpConfig {
    /// Agent whose tools should be exposed.
    pub agent: String,
    /// Optional thread id used for tenant/tool event correlation.
    pub thread_id: Option<ThreadId>,
    /// Maximum duration allowed for each tool call.
    pub call_timeout: Duration,
    /// Whether recursive agent tools such as `call_agent` should be exposed.
    pub expose_agent_tools: bool,
}

impl RuntimeMcpConfig {
    /// Construct config for one agent using the default timeout.
    pub fn new(agent: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            thread_id: None,
            call_timeout: Duration::from_secs(30),
            expose_agent_tools: false,
        }
    }
}

/// Errors while constructing a runtime MCP server.
#[derive(Debug, Error)]
pub enum RuntimeMcpError {
    /// The requested agent is not present in the runtime spec.
    #[error("agent '{0}' not found in runtime")]
    MissingAgent(String),

    /// Direct MCP serving currently omits recursive agent tools.
    #[error(
        "direct MCP exposure of recursive agent tools is not supported for agent '{agent}'; \
         set expose_agent_tools=false or remove the agents toolkit"
    )]
    UnsupportedDirectAgentTools {
        /// Agent that requested direct agent tool exposure.
        agent: String,
    },

    /// Runtime dispatcher/environment composition failed.
    #[error("failed to construct runtime MCP tool environment: {0}")]
    Environment(#[from] RuntimeError),

    /// Framework MCP adapter construction failed.
    #[error("failed to construct MCP tool server: {0}")]
    Mcp(#[from] agent_fw_mcp::McpError),
}

impl Runtime {
    /// Build an MCP tool server for a selected runtime agent.
    pub fn mcp_tool_server(
        self: Arc<Self>,
        config: RuntimeMcpConfig,
    ) -> Result<agent_fw_mcp::McpToolServer, RuntimeMcpError> {
        let agent = self
            .spec
            .agents
            .iter()
            .find(|agent| agent.name == config.agent)
            .cloned()
            .ok_or_else(|| RuntimeMcpError::MissingAgent(config.agent.clone()))?;

        let agent = prepare_agent_for_mcp(agent, config.expose_agent_tools)?;
        let thread_id = config
            .thread_id
            .unwrap_or_else(|| ThreadId::new_unchecked(format!("mcp-{}", uuid::Uuid::new_v4())));
        let sink: Arc<dyn EventSink> = Arc::new(DirectMcpEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(DirectMcpSubAgentInvoker);
        let env = self.tool_env_for_agent(&agent, sink, thread_id, sub_agents, true);
        let dispatcher = self.dispatcher_for_agent_spec(&agent, env.clone())?;

        let server_config = agent_fw_mcp::McpServerConfig {
            name: format!("flowai-runtime:{}", agent.name),
            version: env!("CARGO_PKG_VERSION").to_string(),
            instructions: Some(format!(
                "Tools exposed by Flow AI runtime specialist {}.",
                agent.name
            )),
            call_timeout: config.call_timeout,
        };
        Ok(agent_fw_mcp::McpToolServer::try_new(
            dispatcher,
            env,
            server_config,
        )?)
    }

    fn dispatcher_for_agent_spec(
        &self,
        agent: &AgentSpec,
        env: agent_fw_tool::ToolEnvironment,
    ) -> Result<Arc<dyn ToolDispatcher>, RuntimeError> {
        let composed = compose_dispatcher_for_agent(
            agent,
            &self.spec.toolkits,
            &self.references,
            &self.plans,
            &self.host_tools,
            env.clone(),
        )?;
        let dispatcher = match composed {
            Some(composed) => composed
                .guarded()
                .approval(
                    self.approval_policy_for(&agent.name),
                    self.approval_store.clone(),
                )
                .traced(),
            None => ComposedDispatcher::new(env),
        };
        Ok(Arc::new(dispatcher))
    }
}

fn prepare_agent_for_mcp(
    mut agent: AgentSpec,
    expose_agent_tools: bool,
) -> Result<AgentSpec, RuntimeMcpError> {
    // Recursive agent tools (`call_agent`) come from two sources: an explicit
    // `agents` toolkit, and the coordinator role default that derives them from
    // `routes`. Both must be accounted for here.
    let has_agent_toolkit = agent.toolkits.iter().any(|toolkit| toolkit == "agents");
    let has_role_default_agent_tools =
        agent.role == crate::AgentRole::Coordinator && !agent.routes.is_empty();
    if expose_agent_tools && (has_agent_toolkit || has_role_default_agent_tools) {
        return Err(RuntimeMcpError::UnsupportedDirectAgentTools { agent: agent.name });
    }
    if !expose_agent_tools {
        agent.toolkits.retain(|toolkit| toolkit != "agents");
        // Clearing routes suppresses the coordinator role default so no
        // `call_agent` handler is generated. Direct MCP serving already
        // disables sub-agent invocation, so this only affects tool exposure.
        agent.routes.clear();
    }
    Ok(agent)
}

struct DirectMcpEventSink;

impl EventSink for DirectMcpEventSink {
    fn emit(&self, _part: StreamPart) -> bool {
        false
    }

    fn close(&self) {}

    fn is_open(&self) -> bool {
        false
    }
}

struct DirectMcpSubAgentInvoker;

#[async_trait]
impl SubAgentInvoker for DirectMcpSubAgentInvoker {
    async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        Err(SubAgentError::AgentFailed(format!(
            "Sub-agent invocation for '{}' is not available through direct MCP serving",
            request.agent_name
        )))
    }

    fn has_agent(&self, _name: &str) -> bool {
        false
    }

    fn available_agents(&self) -> Vec<String> {
        Vec::new()
    }

    fn cost_summary(&self) -> Option<CostSummary> {
        None
    }
}
