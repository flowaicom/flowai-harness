from __future__ import annotations

from typing import Any

from flowai_harness.agents import define_specialist
from flowai_harness.runtime import create_runtime, define_runtime
from flowai_harness.tenant import define_tenant


def list_tools(
    runtime: Any,
    *,
    agent: str,
    expose_agent_tools: bool = False,
) -> list[dict[str, object]]:
    """Return MCP tool metadata for one runtime agent.

    Args:
        runtime: A native `Runtime` returned by `create_runtime(...)`.
        agent: Specialist or other runtime agent whose direct tools should be exposed.
        expose_agent_tools: Reserved for future recursive agent tools. Direct MCP serving omits
            those tools by default, and the runtime-generated `agents` toolkit is not supported
            in this mode.

    Returns:
        A list of MCP tool descriptors (name, description, input schema) for
        the agent's directly bound tools and toolkit tools.
    """

    return runtime.list_mcp_tools(agent, expose_agent_tools=expose_agent_tools)


async def serve_stdio(
    runtime: Any,
    *,
    agent: str,
    thread_id: str | None = None,
    call_timeout_secs: float = 30.0,
    expose_agent_tools: bool = False,
) -> None:
    """Serve one runtime agent's tools over MCP stdio.

    This transport is intended for local MCP clients that launch the server as a subprocess.
    Python tool callbacks execute in this Python process. The coroutine runs
    until the MCP client disconnects.

    Args:
        runtime: A native `Runtime` returned by `create_runtime(...)`.
        agent: Runtime agent whose tools are exposed.
        thread_id: Optional fixed thread id used for runtime tool dispatch.
        call_timeout_secs: Per tool-call timeout in seconds.
        expose_agent_tools: Reserved for future recursive agent tools.
    """

    await runtime.serve_mcp_stdio(
        agent,
        thread_id=thread_id,
        call_timeout_secs=call_timeout_secs,
        expose_agent_tools=expose_agent_tools,
    )


async def serve_http(
    runtime: Any,
    *,
    agent: str,
    host: str = "127.0.0.1",
    port: int = 8765,
    path: str = "/mcp",
    transport: str = "streamable-http",
    thread_id: str | None = None,
    call_timeout_secs: float = 30.0,
    expose_agent_tools: bool = False,
    allowed_origins: list[str] | None = None,
    require_origin: bool = True,
    auth_token: str | None = None,
) -> None:
    """Serve one runtime agent's tools over MCP Streamable HTTP.

    The server binds to loopback by default and validates browser `Origin` headers unless
    `require_origin=False` is supplied. The coroutine runs until the server
    is stopped.

    Args:
        runtime: A native `Runtime` returned by `create_runtime(...)`.
        agent: Runtime agent whose tools are exposed.
        host: Host to bind.
        port: Port to bind.
        path: HTTP endpoint path for the MCP server.
        transport: Only `"streamable-http"` is supported.
        thread_id: Optional fixed thread id used for runtime tool dispatch.
        call_timeout_secs: Per tool-call timeout in seconds.
        expose_agent_tools: Reserved for future recursive agent tools.
        allowed_origins: Additional `Origin` header values to accept.
        require_origin: Validate browser `Origin` headers. Disable only for
            non-browser clients on trusted networks.
        auth_token: Required bearer/header token for every Streamable HTTP
            request.

    Raises:
        ValueError: If `transport` is not `"streamable-http"`.
        RuntimeError: If the server fails to bind or serving fails.
    """

    if auth_token is None or auth_token.strip() == "":
        raise ValueError("Streamable HTTP requires a non-empty auth_token")

    await runtime.serve_mcp_http(
        agent,
        host=host,
        port=port,
        path=path,
        transport=transport,
        thread_id=thread_id,
        call_timeout_secs=call_timeout_secs,
        expose_agent_tools=expose_agent_tools,
        allowed_origins=allowed_origins,
        require_origin=require_origin,
        auth_token=auth_token,
    )


def create_mcp_runtime(
    *,
    tools: list[object] | None = None,
    toolkits: list[str] | None = None,
    agent: str = "mcp",
    tenant: str = "flowai-mcp",
    data_environment: object | None = None,
    services: object | None = None,
) -> Any:
    """Build a minimal runtime for MCP-only tool serving.

    The helper creates one specialist named by `agent` under the supplied tenant and attaches
    the supplied Python tools or built-in toolkit ids. The runtime uses the
    `"noop"` interpreter, so no provider credentials are needed.

    Args:
        tools: `ToolSpec` values with Python handlers to expose.
        toolkits: Built-in toolkit ids to expose.
        agent: Name of the generated specialist agent.
        tenant: Tenant resource id for the generated runtime.
        data_environment: Rust-owned toolkit dependencies such as catalogs
            and target databases. If it includes `tenant_id`, it must match
            `tenant`.
        services: Python-owned objects needed by custom tool callbacks.

    Returns:
        A native `Runtime` handle ready for `serve_stdio(...)`,
        `serve_http(...)`, or `list_tools(...)`.
    """

    tool_specs = list(tools or [])
    toolkit_ids = list(toolkits or [])
    specialist = define_specialist(
        agent,
        model="claude-sonnet-4-6",
        prompt="Expose these tools over MCP.",
        tools=tool_specs,
        toolkits=toolkit_ids,
    )
    spec = define_runtime(
        define_tenant(tenant, "v1"),
        agents=[specialist],
        providers={"anthropic": {"apiKey": "unused"}},
    )
    return create_runtime(
        spec,
        tool_bindings=tool_specs,
        services=services if services is not None else {},
        data_environment=data_environment,
        interpreter="noop",
    )


__all__ = [
    "create_mcp_runtime",
    "list_tools",
    "serve_http",
    "serve_stdio",
]
