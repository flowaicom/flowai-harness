import asyncio
import json

import pytest
from pydantic import BaseModel

from flowai_harness import (
    create_runtime,
    define_coordinator,
    define_executor,
    define_plan,
    define_planner,
    define_reference,
    define_runtime,
    define_specialist,
    define_tenant,
    define_tool,
)


PLAN_ID = "demo-plan-1"
PLAN_SPEC = "DemoPlan"


class ProductSetPayload(BaseModel):
    product_ids: list[str]


async def _collect_with_plan_decision(runtime, prompt, outcome, *, feedback=None, partial=None):
    events = []
    approval_seen = False

    async for event in runtime.query(prompt, thread_id="thread-smoke"):
        events.append(event)
        if event["type"] == "approval-required":
            approval_seen = True
            await runtime.respond_to_approval(
                event["data"]["id"],
                outcome,
                feedback=feedback,
                partial=partial,
            )

    assert approval_seen, "expected the plan gate to request approval"
    return events


def _tool_prompt(tool, args):
    return json.dumps({"tool": tool, "args": args})


def _plan_body(message="record the approved action", *, references=None):
    action = {
        "kind": "record_counter",
        "message": message,
    }
    if references is not None:
        action["references"] = references
    return {
        "rationale": "deterministic smoke test plan",
        "actions": [action],
    }


def _coordinator_script(plan_id=PLAN_ID, *, body=None):
    planner_prompt = _tool_prompt(
        "storePlan",
        {
            "specName": PLAN_SPEC,
            "planId": plan_id,
            "body": body or _plan_body(),
        },
    )
    executor_prompt = _tool_prompt("executePlan", {"planId": plan_id})
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


def _runtime(action_dispatches, *, dispatch_actions=None, references=None):
    scenario_plan = define_plan(
        PLAN_SPEC,
        {
            "type": "object",
            "required": ["rationale", "actions"],
            "properties": {
                "rationale": {"type": "string"},
                "actions": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["kind"],
                        "properties": {
                            "kind": {"type": "string"},
                            "message": {"type": "string"},
                            "references": {"type": "array"},
                        },
                    },
                },
            },
        },
    )

    coordinator = define_coordinator(
        "coordinator",
        model="claude-sonnet-4-6",
        prompt="Coordinate by delegating to the planner and executor.",
        routes=["planner", "executor"],
    )
    planner = define_planner(
        "planner",
        model="claude-sonnet-4-6",
        prompt="Store exactly one typed plan.",
        plan=scenario_plan,
    )
    executor = define_executor(
        "executor",
        model="claude-sonnet-4-6",
        prompt="Execute the requested plan.",
        plan=scenario_plan,
    )

    if dispatch_actions is None:

        def dispatch_actions(actions, ctx):
            action_dispatches.append((actions, ctx))
            return {
                "entitiesAffected": len(actions),
                "summary": f"executed {len(actions)} action(s)",
                "details": None,
            }

    return create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[coordinator, planner, executor],
            references=references or [],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        action_dispatcher=dispatch_actions,
        interpreter="scripted",
    )


def _agent_event_names(events):
    return [
        event["agentName"]
        for event in events
        if event["type"] == "tool-agent" and event["state"] == "call"
    ]


def _tool_result(events, tool_name):
    for event in events:
        if (
            event["type"] == "tool-invocation"
            and event["toolName"] == tool_name
            and event["state"] == "result"
        ):
            return event["result"]
    raise AssertionError(f"missing result event for {tool_name}")


def test_coordinator_planner_executor_approval_path_runs_actions_after_approval():
    action_dispatches = []
    runtime = _runtime(action_dispatches)

    async def run_flow():
        stream = runtime.query(_coordinator_script(), thread_id="thread-smoke")
        events = []
        async for event in stream:
            events.append(event)
            if event["type"] == "approval-required":
                assert action_dispatches == []
                await runtime.respond_to_approval(
                    event["data"]["id"],
                    "approve",
                    feedback="approved by smoke test",
                )
        return events

    events = asyncio.run(run_flow())

    assert _agent_event_names(events)[:3] == ["coordinator", "planner", "executor"]
    stored_plan = _tool_result(events, "storePlan")
    assert stored_plan["id"] == PLAN_ID
    assert stored_plan["status"] == "draft"
    assert any(event["type"] == "approval-decision" for event in events)
    assert action_dispatches == [
        (
            [
                {
                    "kind": "record_counter",
                    "payload": {"message": "record the approved action"},
                    "references": [],
                }
            ],
            {"resolved_refs": {}},
        )
    ]
    assert any(event["type"] == "finish" for event in events)


def test_action_dispatcher_receives_resolved_refs_grouped_by_kind_and_id():
    product_set = define_reference("ProductSet", ProductSetPayload)
    action_dispatches = []

    def dispatch_actions(actions, ctx):
        action_dispatches.append((actions, ctx))
        ref = actions[0]["references"][0]
        product_ids = ctx["resolved_refs"][ref["kind"]][ref["id"]]["product_ids"]
        return {
            "entitiesAffected": len(product_ids),
            "summary": f"executed {len(product_ids)} referenced products",
            "details": None,
        }

    runtime = _runtime(
        action_dispatches,
        dispatch_actions=dispatch_actions,
        references=[product_set],
    )

    async def run_flow():
        ref = await runtime.create_reference(
            product_set,
            ProductSetPayload(product_ids=["sku-1", "sku-2", "sku-3"]),
        )
        events = []
        prompt = _coordinator_script(
            "demo-plan-with-ref",
            body=_plan_body(references=[{"kind": ref["kind"], "id": ref["id"]}]),
        )
        async for event in runtime.query(prompt, thread_id="thread-ref-smoke"):
            events.append(event)
            if event["type"] == "approval-required":
                await runtime.respond_to_approval(
                    event["data"]["id"],
                    "approve",
                    feedback="approved by smoke test",
                )
        return ref, events

    ref, events = asyncio.run(run_flow())

    assert _tool_result(events, "executePlan")["entitiesAffected"] == 3
    assert action_dispatches == [
        (
            [
                {
                    "kind": "record_counter",
                    "payload": {"message": "record the approved action"},
                    "references": [{"kind": ref["kind"], "id": ref["id"]}],
                }
            ],
            {
                "resolved_refs": {
                    "ProductSet": {
                        ref["id"]: {"product_ids": ["sku-1", "sku-2", "sku-3"]}
                    }
                }
            },
        )
    ]


def test_action_dispatcher_return_shape_is_validated_with_clear_error():
    action_dispatches = []

    def dispatch_actions(actions, ctx):
        action_dispatches.append((actions, ctx))
        return {"summary": "missing entitiesAffected"}

    runtime = _runtime(action_dispatches, dispatch_actions=dispatch_actions)

    events = asyncio.run(
        _collect_with_plan_decision(
            runtime,
            _coordinator_script(),
            "approve",
            feedback="approved by smoke test",
        )
    )

    result = _tool_result(events, "executePlan")
    assert "action_dispatcher must return None or an object" in result["error"]
    assert "entitiesAffected" in result["error"]


@pytest.mark.parametrize(
    ("outcome", "feedback", "partial"),
    [
        ("reject", "do not execute", None),
        ("revise", "make the action safer", {"requestedChange": "make the action safer"}),
    ],
)
def test_coordinator_planner_executor_non_approve_paths_do_not_run_actions(
    outcome,
    feedback,
    partial,
):
    action_dispatches = []
    runtime = _runtime(action_dispatches)

    events = asyncio.run(
        _collect_with_plan_decision(
            runtime,
            _coordinator_script(),
            outcome,
            feedback=feedback,
            partial=partial,
        )
    )

    decisions = [
        event["data"]["outcome"]["outcome"]
        for event in events
        if event["type"] == "approval-decision"
    ]
    assert decisions == [outcome]
    assert action_dispatches == []

    if outcome == "revise":
        result = _tool_result(events, "executePlan")
        assert result["should_revise"] is True
        assert result["partial"] == partial


def test_tool_level_always_approval_blocks_python_handler_until_approved():
    calls = []

    @define_tool("increment_counter", {"amount": int}, approval="always")
    async def increment_counter(args, ctx):
        calls.append((args, ctx["tool_use_id"]))
        return {"count": len(calls)}

    specialist = define_specialist(
        "counter",
        model="claude-sonnet-4-6",
        prompt="Use the requested counter tool.",
        tools=[increment_counter],
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        interpreter="scripted",
    )

    async def run_flow():
        stream = runtime.run_specialist(
            "counter",
            _tool_prompt("increment_counter", {"amount": 1}),
            thread_id="thread-tool-approval",
        )
        events = []
        async for event in stream:
            events.append(event)
            if event["type"] == "approval-required":
                assert calls == []
                await runtime.respond_to_approval(event["data"]["id"], "approve")
        return events

    events = asyncio.run(run_flow())

    assert calls == [({"amount": 1}, "scripted-tool-1")]
    assert _tool_result(events, "increment_counter") == {"count": 1}
