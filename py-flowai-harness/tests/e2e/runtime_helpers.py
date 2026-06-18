from __future__ import annotations

import asyncio
import json
from typing import Any

from flowai_harness import create_runtime, define_runtime, define_specialist, define_tenant


async def collect(stream):
    return [event async for event in stream]


def run_tool(runtime: Any, specialist: str, tool: str, args: dict[str, Any]) -> dict[str, Any]:
    prompt = json.dumps({"tool": tool, "args": args})
    events = asyncio.run(collect(runtime.run_specialist(specialist, prompt, thread_id="e2e-thread")))
    return tool_result(events, tool)


def tool_result(events: list[dict[str, Any]], tool_name: str) -> dict[str, Any]:
    for event in events:
        if (
            event.get("type") == "tool-invocation"
            and event.get("toolName") == tool_name
            and event.get("state") == "result"
        ):
            return event["result"]
    raise AssertionError(f"missing result event for {tool_name}: {events}")


def catalog_runtime(data_environment: dict[str, Any], *, interpreter: str = "scripted"):
    reader = define_specialist(
        name="retail_reader",
        model="claude-sonnet-4-6",
        prompt=(
            "Use catalog tools to inspect data, retrieve policy documents, "
            "and answer retail revenue questions."
        ),
        toolkits=["catalog"],
    )
    return create_runtime(
        define_runtime(
            tenant=define_tenant("flowai-e2e", "v1"),
            agents=[reader],
            providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
        ),
        interpreter=interpreter,
        data_environment=data_environment,
    )
