from __future__ import annotations

import asyncio
import json
from typing import Any

from coordinator_planner_executor.app import (
    PRICE_CHANGE_EXECUTE_PLAN_ARGS,
    PRICE_CHANGE_STORE_PLAN_ARGS,
    create_example_runtime,
)


def _tool_prompt(tool: str, args: dict[str, Any]) -> str:
    return json.dumps({"tool": tool, "args": args})


def coordinator_smoke_prompt() -> str:
    planner_prompt = _tool_prompt("storePlan", PRICE_CHANGE_STORE_PLAN_ARGS)
    executor_prompt = _tool_prompt("executePlan", PRICE_CHANGE_EXECUTE_PLAN_ARGS)
    return json.dumps(
        {
            "script": [
                {
                    "tool": "call_agent",
                    "args": {"agent": "planner", "prompt": planner_prompt},
                },
                {
                    "tool": "call_agent",
                    "args": {"agent": "executor", "prompt": executor_prompt},
                },
            ]
        }
    )


async def collect_smoke_events() -> list[dict[str, Any]]:
    runtime = create_example_runtime(interpreter="scripted")
    events: list[dict[str, Any]] = []
    async for event in runtime.query(coordinator_smoke_prompt(), thread_id="example-smoke"):
        events.append(event)
        if event["type"] == "approval-required":
            await runtime.respond_to_approval(
                event["data"]["id"],
                "approve",
                feedback="approved by example smoke check",
            )
    return events


def agent_calls(events: list[dict[str, Any]]) -> list[str]:
    return [
        event["agentName"]
        for event in events
        if event["type"] == "tool-agent" and event["state"] == "call"
    ]


def tool_result(events: list[dict[str, Any]], tool_name: str) -> dict[str, Any]:
    for event in events:
        if (
            event["type"] == "tool-invocation"
            and event["toolName"] == tool_name
            and event["state"] == "result"
        ):
            return event["result"]
    raise AssertionError(f"missing result event for {tool_name}")


def run_smoke() -> list[dict[str, Any]]:
    events = asyncio.run(collect_smoke_events())
    calls = agent_calls(events)
    if calls[:3] != ["coordinator", "planner", "executor"]:
        raise AssertionError(f"unexpected agent call order: {calls}")
    if tool_result(events, "storePlan")["id"] != PRICE_CHANGE_STORE_PLAN_ARGS["planId"]:
        raise AssertionError("planner did not store the expected plan")
    execute_result = tool_result(events, "executePlan")
    if execute_result["entitiesAffected"] != 1:
        raise AssertionError(f"unexpected executePlan result: {execute_result}")
    if not any(event["type"] == "finish" for event in events):
        raise AssertionError("runtime did not emit a finish event")
    return events


def main() -> None:
    events = run_smoke()
    print("Coordinator-planner-executor smoke passed.")
    print("Observed agents: " + " -> ".join(agent_calls(events)[:3]))


if __name__ == "__main__":
    main()
