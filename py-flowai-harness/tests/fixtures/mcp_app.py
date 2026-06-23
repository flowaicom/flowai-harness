from flowai_harness import define_tool, mcp


@define_tool("echo", {"message": str}, description="Echo a message", approval="never")
def echo(args, ctx):
    return {"message": args["message"], "toolUseId": ctx["tool_use_id"]}


async def async_echo(args, ctx):
    return {"message": args["message"], "toolUseId": ctx["tool_use_id"]}


async_echo_tool = define_tool(
    "async_echo",
    {"message": str},
    description="Echo a message asynchronously",
    approval="never",
)(async_echo)


runtime = mcp.create_mcp_runtime(tools=[echo, async_echo_tool])


def build_runtime():
    return runtime
