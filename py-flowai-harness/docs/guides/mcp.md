# Expose tools over MCP

`flowai-harness` can expose runtime tools as Model Context Protocol (MCP)
servers over stdio or Streamable HTTP. Use this when an MCP-aware client should
call Python-defined custom tools or built-in Flow AI toolkits directly.

## When to use MCP

Use MCP when an external MCP-aware client needs to discover or call Flow AI
tools.

You do not need MCP if tools are only used by agents inside your Flow AI
runtime. In that case, attach tools directly to agents and call the runtime
normally.

## Custom Python Tools

Python callbacks must run inside the Python process that hosts the MCP server.
Build a small runtime with `flowai_harness.mcp.create_mcp_runtime(...)`:

```python
from flowai_harness import define_tool
from flowai_harness import mcp


echo = define_tool(
    name="echo",
    description="Echo text.",
    input_schema={
        "type": "object",
        "properties": {"text": {"type": "string"}},
        "required": ["text"],
    },
)(lambda args, ctx: {"text": args["text"]})

runtime = mcp.create_mcp_runtime(tools=[echo])
```

Tool handlers may be sync or async: the echo handler above is a plain lambda,
and `async def` handlers (as used in the other guides) work the same here.

Serve it over stdio for subprocess-based MCP clients:

```python
import asyncio
from flowai_harness import mcp

asyncio.run(mcp.serve_stdio(runtime, agent="mcp"))
```

Or serve it over Streamable HTTP:

```python
import asyncio
import os
from flowai_harness import mcp

asyncio.run(
    mcp.serve_http(
        runtime,
        agent="mcp",
        host="127.0.0.1",
        port=8765,
        path="/mcp",
        transport="streamable-http",
        allowed_origins=["http://localhost:3000"],
        auth_token=os.environ["FLOWAI_MCP_HTTP_TOKEN"],
    )
)
```

## Built-In Toolkits

Toolkit servers use the same runtime helper. Toolkits that need catalogs, KV, or
target databases receive those dependencies through `data_environment`.

```python
from flowai_harness import mcp

runtime = mcp.create_mcp_runtime(
    toolkits=["catalog"],
    tenant="acme",
    data_environment=data_environment,
)
```

If `data_environment` includes `tenant_id`, pass the same value as `tenant`.
The runtime rejects mismatches instead of silently reading another tenant's
catalog scope.

## Verify the Tools Are Exposed

Before wiring a client, list the MCP tool metadata the server will advertise:

```python
from flowai_harness import mcp

for tool in mcp.list_tools(runtime, agent="mcp"):
    print(tool["name"], "-", tool["description"])
```

For the echo runtime above this prints:

```text
echo - Echo text.
```

`list_tools` returns the same names and schemas an MCP client sees, so an
empty or unexpected list means the runtime wiring is wrong — fix that before
debugging client configuration.

## CLI Usage

Use `flowai-harness mcp python MODULE:OBJECT ...` when your server needs
Python callbacks. The target can be a runtime object or a callable factory.

```bash
flowai-harness mcp python my_app:build_runtime --agent mcp
FLOWAI_MCP_HTTP_TOKEN=dev-token flowai-harness mcp python my_app:build_runtime --agent mcp --transport streamable-http --port 8765
```

Use `flowai-harness mcp toolkit ...` for toolkit-only servers that do not need
Python callbacks.

```bash
flowai-harness mcp toolkit --toolkit catalog --data-environment data-environment.toml --agent mcp --tenant-id acme
FLOWAI_MCP_HTTP_TOKEN=dev-token flowai-harness mcp toolkit --toolkit catalog --data-environment data-environment.toml --agent mcp --tenant-id acme --transport streamable-http --port 8765
```

For stdio MCP clients, configure the client command as the console script plus
the same arguments. For example:

```json
{
  "command": "flowai-harness",
  "args": ["mcp", "python", "my_app:build_runtime", "--agent", "mcp"]
}
```

For HTTP-capable MCP clients, start the server first and point the client at
the Streamable HTTP endpoint, such as `http://127.0.0.1:8765/mcp`. Clients must
send either `X-FlowAI-MCP-Token: <token>` or `Authorization: Bearer <token>`.

For alpha-only local workflows that cannot send headers, pass `--no-auth` on
the CLI or `require_auth=False` to `mcp.serve_http(...)`. This disables
Streamable HTTP authentication for that process and should only be used on
trusted loopback development sessions.

## Constraints

- Supported transports are stdio and Streamable HTTP.
- Streamable HTTP is the current HTTP transport; legacy HTTP+SSE endpoints are
  intentionally unsupported in this build.
- HTTP servers bind to `127.0.0.1` by default.
- Streamable HTTP authentication is enabled by default. Use `--auth-token` or
  `FLOWAI_MCP_HTTP_TOKEN`; use `--no-auth` only as an unsafe alpha escape
  hatch.
- HTTP origin validation is enabled by default. Use repeated `--allow-origin`
  flags or `allowed_origins=[...]` for browser clients.
- Toolkit servers use tenant `flowai-mcp` by default. Use `tenant=...` or
  `--tenant-id ...` when reusing a scoped `data_environment`.
- Direct MCP serving exposes direct tools only by default. Recursive agent tools
  are omitted. `expose_agent_tools` and `--expose-agent-tools` are reserved for
  future recursive agent-tool support; the runtime-generated `agents` toolkit is
  not supported in this mode.
- Approval-gated tools need a noninteractive policy; otherwise direct MCP tool
  calls return a tool error instead of waiting indefinitely.
- Python callbacks execute in the Python process hosting the MCP server.
- Tool schemas are forwarded from Flow AI tool definitions.
- Structured tracing fields for bound Streamable HTTP endpoints are not emitted
  yet. The CLI prints the bound endpoint URL to stderr; use that to point
  clients at the server.

## Common errors

| Error                                         | Fix                                                                                                                              |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| MCP client cannot start the server            | Use the console script command plus the same arguments you tested in a shell.                                                    |
| HTTP client is rejected by origin validation  | Add the browser origin with `--allow-origin` or `allowed_origins=[...]`.                                                         |
| Catalog toolkit cannot read data              | Pass `tenant` or `--tenant-id` that matches `data_environment["tenant_id"]`, and include the required catalog and search config. |
| Approval-gated tool hangs or returns an error | Use a noninteractive approval policy for direct MCP serving.                                                                     |
