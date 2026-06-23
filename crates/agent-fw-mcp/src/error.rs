use std::{net::SocketAddr, time::Duration};

use thiserror::Error;

/// Errors raised while adapting framework tools to MCP transports.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("invalid MCP HTTP endpoint path `{0}`; expected an absolute path starting with `/`")]
    InvalidEndpointPath(String),

    #[error("tool `{tool_name}` input schema must be a JSON object")]
    InvalidInputSchema { tool_name: String },

    #[error("MCP Streamable HTTP requires a non-empty auth token")]
    MissingHttpAuthToken,

    #[error("failed to bind MCP HTTP listener on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read MCP HTTP listener address: {0}")]
    LocalAddr(#[source] std::io::Error),

    #[error("MCP HTTP server failed: {0}")]
    HttpServe(#[source] std::io::Error),

    #[error("MCP stdio server failed to initialize: {0}")]
    StdioInitialize(String),

    #[error("MCP stdio server task failed: {0}")]
    StdioJoin(#[source] tokio::task::JoinError),
}

pub(crate) fn timeout_error(tool_name: &str, timeout: Duration) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(
        format!("Tool `{tool_name}` timed out after {timeout:?}"),
        None,
    )
}
