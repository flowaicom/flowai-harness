import asyncio

from flowai_harness import define_tool, mcp
from flowai_harness.mcp import create_mcp_runtime, list_tools, serve_http, serve_stdio


def _catalog_search(tmp_path, name="catalog-index"):
    return {"index_path": str(tmp_path / name)}


class RecordingRuntime:
    def __init__(self):
        self.calls = []

    def list_mcp_tools(self, agent, *, expose_agent_tools=False):
        self.calls.append(("list", agent, expose_agent_tools))
        return [{"name": "echo"}]

    async def serve_mcp_stdio(self, agent, **kwargs):
        self.calls.append(("stdio", agent, kwargs))

    async def serve_mcp_http(self, agent, **kwargs):
        self.calls.append(("http", agent, kwargs))


def test_public_imports_work():
    assert mcp is not None
    assert callable(serve_stdio)
    assert callable(serve_http)
    assert callable(list_tools)
    assert callable(create_mcp_runtime)


def test_list_tools_delegates_options():
    runtime = RecordingRuntime()

    assert list_tools(runtime, agent="mcp", expose_agent_tools=True) == [{"name": "echo"}]
    assert runtime.calls == [("list", "mcp", True)]


def test_serve_stdio_delegates_options():
    runtime = RecordingRuntime()

    asyncio.run(
        serve_stdio(
            runtime,
            agent="mcp",
            thread_id="thread-1",
            call_timeout_secs=2.5,
            expose_agent_tools=True,
        )
    )

    assert runtime.calls == [
        (
            "stdio",
            "mcp",
            {
                "thread_id": "thread-1",
                "call_timeout_secs": 2.5,
                "expose_agent_tools": True,
            },
        )
    ]


def test_serve_http_delegates_options():
    runtime = RecordingRuntime()

    asyncio.run(
        serve_http(
            runtime,
            agent="mcp",
            host="127.0.0.1",
            port=0,
            path="/mcp",
            allowed_origins=["http://localhost:3000"],
            require_origin=False,
            auth_token="test-mcp-token",
        )
    )

    assert runtime.calls == [
        (
            "http",
            "mcp",
            {
                "host": "127.0.0.1",
                "port": 0,
                "path": "/mcp",
                "transport": "streamable-http",
                "thread_id": None,
                "call_timeout_secs": 30.0,
                "expose_agent_tools": False,
                "allowed_origins": ["http://localhost:3000"],
                "require_origin": False,
                "require_auth": True,
                "auth_token": "test-mcp-token",
            },
        )
    ]


def test_serve_http_can_disable_authentication():
    runtime = RecordingRuntime()

    asyncio.run(serve_http(runtime, agent="mcp", require_auth=False))

    assert runtime.calls == [
        (
            "http",
            "mcp",
            {
                "host": "127.0.0.1",
                "port": 8765,
                "path": "/mcp",
                "transport": "streamable-http",
                "thread_id": None,
                "call_timeout_secs": 30.0,
                "expose_agent_tools": False,
                "allowed_origins": None,
                "require_origin": True,
                "require_auth": False,
                "auth_token": None,
            },
        )
    ]


def test_serve_http_requires_auth_token():
    runtime = RecordingRuntime()

    try:
        asyncio.run(serve_http(runtime, agent="mcp"))
    except ValueError as exc:
        assert "auth_token" in str(exc)
    else:
        raise AssertionError("serve_http should reject missing auth_token")


def test_create_mcp_runtime_lists_custom_tool():
    @define_tool("echo", {"message": str}, description="Echo message", approval="never")
    def echo(args, ctx):
        return {"message": args["message"]}

    runtime = create_mcp_runtime(tools=[echo])

    tools = list_tools(runtime, agent="mcp")
    assert [tool["name"] for tool in tools] == ["echo"]


def test_create_mcp_runtime_accepts_matching_tenant_data_environment(tmp_path):
    runtime = create_mcp_runtime(
        toolkits=["catalog"],
        tenant="acme",
        data_environment={
            "tenant_id": "acme",
            "catalog": {"kind": "empty"},
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    names = {tool["name"] for tool in list_tools(runtime, agent="mcp")}
    assert "search_catalog" in names
    assert "get_catalog_entities" in names
    assert "execute_query" in names


def test_create_mcp_runtime_lists_catalog_toolkit(tmp_path):
    runtime = create_mcp_runtime(
        toolkits=["catalog"],
        tenant="acme",
        data_environment={
            "tenant_id": "acme",
            "kv": {"kind": "memory"},
            "catalog": {"kind": "empty"},
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    names = {tool["name"] for tool in list_tools(runtime, agent="mcp")}
    assert {
        "search_catalog",
        "get_catalog_entities",
        "list_schema_fields",
        "get_catalog_relations",
        "get_relation_paths_between",
        "sample_table_data",
        "execute_query",
    }.issubset(names)
