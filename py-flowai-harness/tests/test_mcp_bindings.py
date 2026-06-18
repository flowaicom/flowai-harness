from flowai_harness import define_tool
from flowai_harness.mcp import create_mcp_runtime


def test_list_mcp_tools_returns_python_custom_tool_schema():
    @define_tool("echo", {"message": str}, description="Echo message")
    def echo(args, ctx):
        return {"message": args["message"]}

    runtime = create_mcp_runtime(tools=[echo])

    tools = runtime.list_mcp_tools("mcp")
    assert tools[0]["name"] == "echo"
    assert tools[0]["description"] == "Echo message"
    assert tools[0]["inputSchema"]["properties"]["message"]["type"] == "string"


def test_list_mcp_tools_missing_agent_mentions_name():
    runtime = create_mcp_runtime()

    try:
        runtime.list_mcp_tools("missing")
    except ValueError as exc:
        assert "missing" in str(exc)
    else:
        raise AssertionError("missing agent should raise")
