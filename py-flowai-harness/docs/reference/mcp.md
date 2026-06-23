# MCP

Helpers for exposing Flow AI runtime tools as Model Context Protocol servers.

## Runtime MCP surface

The module helpers below are thin wrappers over methods on the native
[`Runtime`](runtime.md) handle.

| Module helper | Runtime method |
| --- | --- |
| `mcp.list_tools(...)` | `runtime.list_mcp_tools(...)` |
| `mcp.serve_stdio(...)` | `runtime.serve_mcp_stdio(...)` |
| `mcp.serve_http(...)` | `runtime.serve_mcp_http(...)` |

If you already have a runtime handle, call the runtime methods in the table.
If you only need to expose tools over MCP, use `create_mcp_runtime(...)` to
build a minimal runtime for that purpose.

From the CLI, run `flowai-harness mcp python MODULE:OBJECT --agent NAME`.
The command serves over stdio by default. Add `--transport streamable-http` to
serve over Streamable HTTP. Streamable HTTP authentication is enabled by
default; provide `--auth-token TOKEN` or `FLOWAI_MCP_HTTP_TOKEN`, and configure
clients to send `X-FlowAI-MCP-Token: TOKEN` or `Authorization: Bearer TOKEN`.

`--no-auth` and `require_auth=False` are unsafe alpha escape hatches for
trusted loopback development sessions that cannot set headers.

::: flowai_harness.mcp.list_tools

::: flowai_harness.mcp.serve_stdio

::: flowai_harness.mcp.serve_http

::: flowai_harness.mcp.create_mcp_runtime
