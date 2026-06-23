use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::{
    transport::{
        io::stdio,
        streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        },
    },
    ServiceExt,
};
use tokio::net::TcpListener;

use crate::{McpError, McpHttpServerConfig, McpToolServer};

const MCP_AUTH_HEADER: &str = "x-flowai-mcp-token";

/// Bound Streamable HTTP MCP server.
pub struct McpBoundHttpServer {
    listener: TcpListener,
    router: Router,
    bound_addr: SocketAddr,
    endpoint_path: String,
}

impl McpBoundHttpServer {
    /// Local address the HTTP listener is bound to.
    pub fn bound_addr(&self) -> SocketAddr {
        self.bound_addr
    }

    /// Full HTTP endpoint URL for the MCP Streamable HTTP service.
    pub fn endpoint_url(&self) -> String {
        format!(
            "http://{}{}",
            url_host(self.bound_addr.ip()),
            self.bound_addr.port()
        ) + &self.endpoint_path
    }

    /// Serve the bound listener until the process is stopped or the task is cancelled.
    pub async fn serve(self) -> Result<(), McpError> {
        axum::serve(self.listener, self.router)
            .await
            .map_err(McpError::HttpServe)
    }
}

impl McpToolServer {
    /// Bind a Streamable HTTP MCP server and return a handle that can be served.
    pub async fn bind_streamable_http(
        self,
        config: McpHttpServerConfig,
    ) -> Result<McpBoundHttpServer, McpError> {
        if !config.endpoint_path.starts_with('/') {
            return Err(McpError::InvalidEndpointPath(config.endpoint_path));
        }
        let auth_token = config
            .auth_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or(McpError::MissingHttpAuthToken)?
            .to_string();

        let listener = TcpListener::bind(config.bind_addr)
            .await
            .map_err(|source| McpError::Bind {
                addr: config.bind_addr,
                source,
            })?;
        let bound_addr = listener.local_addr().map_err(McpError::LocalAddr)?;

        let rmcp_config = streamable_http_config(&config, bound_addr);
        let service: StreamableHttpService<McpToolServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok::<_, std::io::Error>(self.clone()),
                Arc::new(LocalSessionManager::default()),
                rmcp_config,
            );
        let router = Router::new()
            .nest_service(&config.endpoint_path, service)
            .layer(middleware::from_fn_with_state(
                auth_token,
                require_mcp_authentication,
            ));

        Ok(McpBoundHttpServer {
            listener,
            router,
            bound_addr,
            endpoint_path: config.endpoint_path,
        })
    }

    /// Serve a Streamable HTTP MCP server until the listener exits.
    pub async fn serve_streamable_http(self, config: McpHttpServerConfig) -> Result<(), McpError> {
        self.bind_streamable_http(config).await?.serve().await
    }

    /// Serve this MCP server over stdio.
    pub async fn serve_stdio(self) -> Result<(), McpError> {
        let running = self
            .serve(stdio())
            .await
            .map_err(|error| McpError::StdioInitialize(error.to_string()))?;
        running.waiting().await.map_err(McpError::StdioJoin)?;
        Ok(())
    }
}

async fn require_mcp_authentication(
    State(expected_token): State<String>,
    request: Request,
    next: Next,
) -> Response {
    if has_valid_mcp_token(request.headers(), &expected_token) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "MCP Streamable HTTP authentication is required",
    )
        .into_response()
}

fn has_valid_mcp_token(headers: &HeaderMap, expected_token: &str) -> bool {
    let supplied = headers
        .get(MCP_AUTH_HEADER)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    let (scheme, token) = value.split_once(' ')?;
                    scheme.eq_ignore_ascii_case("bearer").then_some(token)
                })
        });
    supplied.is_some_and(|token| constant_time_eq(token.as_bytes(), expected_token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let diff = left
        .iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right));
    diff == 0
}

fn streamable_http_config(
    config: &McpHttpServerConfig,
    bound_addr: SocketAddr,
) -> StreamableHttpServerConfig {
    let mut allowed_hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        bound_addr.to_string(),
    ];
    match bound_addr.ip() {
        IpAddr::V4(ip) => allowed_hosts.push(ip.to_string()),
        IpAddr::V6(ip) => {
            allowed_hosts.push(ip.to_string());
            allowed_hosts.push(format!("[{ip}]:{}", bound_addr.port()));
        }
    }

    let rmcp_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts)
        .with_stateful_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None);

    if config.require_origin {
        rmcp_config.with_allowed_origins(config.allowed_origins.clone())
    } else {
        rmcp_config.disable_allowed_origins()
    }
}

fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => format!("{ip}:"),
        IpAddr::V6(ip) => format!("[{ip}]:"),
    }
}
