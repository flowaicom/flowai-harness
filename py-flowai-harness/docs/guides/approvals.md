# Require approvals

Approvals let the runtime pause sensitive work until your application approves
or rejects it.

Use this guide to gate plans or tools before they affect real systems. For the
mental model, start with the [Approvals concept](../concepts/approvals.md).

## When to use approvals

Use approvals when an agent can propose work that should be reviewed before it
continues:

- plan execution that changes customer data
- tools that send messages or trigger workflows
- expensive operations
- dynamic policies where the host application decides per call

By the end of this guide, your stream should emit an `approval-required` event
before the gated work runs.

## Understand the hierarchy

Approval policy is hierarchical across runtime -> agent -> tool:

1. `define_runtime(..., approval_policies=...)` sets the runtime floor.
2. If no explicit runtime floor is passed, coordinator `approval={...}`
   becomes the runtime floor.
3. Agent `approval={...}` overrides the runtime floor for that agent.
4. `define_tool(..., approval=...)` supplies the default policy for agents that
   bind that tool.
5. Agent `tool_approvals={...}` overrides one tool under that agent.

Missing agent channels inherit from the runtime floor. In the `define_*`
helpers, use `"default"` when you want to leave one channel inherited:

```python
approval={"plans": "default", "tools": "always"}
```

The most specific matching rule wins. This means an agent or tool can either
tighten a broader `"never"` policy to `"always"` or relax a broader `"always"`
policy to `"never"`. If both `define_tool(..., approval=...)` and
`tool_approvals={...}` apply to the same agent and tool, `tool_approvals`
wins for that agent.

## Configure plan approvals

Plan approval is usually the runtime floor. Configure it explicitly when you
want the runtime spec to show the approval boundary:

```python
runtime_spec = define_runtime(
    tenant=tenant,
    agents=[coordinator, planner, executor],
    approval_policies={
        "plans": "always",
        "tools": "never",
    },
)
```

If `approval_policies` is omitted, the coordinator can provide the same floor
with `approval={...}`. This keeps the policy visible at the entry point for a
coordinator -> planner -> executor flow:

```python
coordinator = define_coordinator(
    name="coordinator",
    model="claude-sonnet-4-6",
    routes=["planner", "executor"],
    approval={"plans": "always", "tools": "never"},
    prompt="Route planning work to the planner and approved work to the executor.",
)
```

Use `"never"` only when plan execution is safe to continue without a gate.

## Configure agent overrides

Use an agent override when one agent should differ from the runtime floor. This
is common for executors after the plan has already been approved:

```python
executor = define_executor(
    name="executor",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    approval={"plans": "never", "tools": "never"},
    prompt="Execute approved plans.",
)
```

This override applies only to `executor`. Other agents continue to inherit the
runtime floor.

## Configure tool approvals

Use tool approval when a specific capability is sensitive, even outside a plan.
Python tools default to `approval="never"`, so set a tool approval explicitly
when the direct tool call should be gated.

```python
@define_tool("send_message", {"recipient": str, "body": str}, approval="always")
async def send_message(args, ctx):
    return await ctx.messaging.send(args["recipient"], args["body"])
```

Dynamic approval predicates can inspect the tool arguments and context:

```python
def needs_approval(args, ctx):
    return args.get("amount", 0) > 10_000


@define_tool("post_journal_entry", {"account": str, "amount": float}, approval=needs_approval)
async def post_journal_entry(args, ctx):
    return await ctx.ledger.post(args)
```

You can also register a dynamic predicate by id when constructing the runtime:

```python
tool = define_tool(
    name="post_journal_entry",
    input_schema={"account": str, "amount": float},
    approval={"kind": "dynamic", "value": "needs_approval"},
)

runtime = create_runtime(
    runtime_spec,
    approval_predicates={"needs_approval": needs_approval},
)
```

Per-agent tool overrides are useful when the same tool is safe for one agent
and gated for another:

```python
executor = define_executor(
    name="executor",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    tool_approvals={"execute_query": "always"},
    prompt="Execute approved plans.",
)
```

That override is scoped to `executor`. Another agent using `execute_query`
continues to use its own tool policy.

## Listen for approval events

`runtime.query(...)` pauses the stream when approval is required. Your
application should display the approval request or pass it to a policy service.

```python
async for event in runtime.query("Draft and execute a scenario.", thread_id="thread-1"):
    if event["type"] == "approval-required":
        data = event["data"]
        print(f"approval required for {data['kind']}: {data['target']}")
```

See [Runtime events](../reference/runtime-events.md) for the event payload
shape.

## Respond to an approval

Call `runtime.respond_to_approval(...)` with the approval id and an outcome.

```python
await runtime.respond_to_approval(
    approval_id,
    "approve",
    feedback="Approved by the host application.",
)
```

Outcomes:

- `"approve"` continues the gated work.
- `"reject"` stops the gated work.
- `"revise"` asks the planner to revise a plan. Tool approvals treat revise as
  a rejection.

## Minimal runnable flow

The example below uses the scripted interpreter so no provider call is needed.
The coordinator routes to a planner and executor, the executor asks to run the
stored plan, and the runtime pauses before the action dispatcher runs.

```python
import asyncio
import json

from flowai_harness import (
    create_runtime,
    define_coordinator,
    define_executor,
    define_plan,
    define_planner,
    define_runtime,
    define_tenant,
)

plan = define_plan(
    "DemoPlan",
    {
        "type": "object",
        "required": ["actions"],
        "properties": {
            "actions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["kind", "message"],
                    "properties": {
                        "kind": {"type": "string"},
                        "message": {"type": "string"},
                    },
                },
            },
        },
    },
)

coordinator = define_coordinator(
    "coordinator",
    model="claude-sonnet-4-6",
    routes=["planner", "executor"],
    approval={"plans": "always", "tools": "never"},
    prompt="Route to the planner, then the executor.",
)
planner = define_planner("planner", model="claude-sonnet-4-6", plan=plan, prompt="Store a plan.")
executor = define_executor("executor", model="claude-sonnet-4-6", plan=plan, prompt="Execute a plan.")


def dispatch_actions(actions, ctx):
    return {"entitiesAffected": len(actions), "summary": "executed approved actions"}


runtime = create_runtime(
    define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, planner, executor],
        providers={"anthropic": {"apiKey": "unused"}},
    ),
    action_dispatcher=dispatch_actions,
    interpreter="scripted",
)

planner_prompt = json.dumps({
    "tool": "storePlan",
    "args": {
        "specName": "DemoPlan",
        "planId": "demo-plan-1",
        "body": {"actions": [{"kind": "record_counter", "message": "approved"}]},
    },
})
executor_prompt = json.dumps({"tool": "executePlan", "args": {"planId": "demo-plan-1"}})
coordinator_prompt = json.dumps({
    "script": [
        {"tool": "call_agent", "args": {"agent": "planner", "prompt": planner_prompt}},
        {"tool": "call_agent", "args": {"agent": "executor", "prompt": executor_prompt}},
    ]
})


async def main():
    async for event in runtime.query(coordinator_prompt, thread_id="thread-1"):
        if event["type"] == "approval-required":
            await runtime.respond_to_approval(event["data"]["id"], "approve")


asyncio.run(main())
```

## Verify it works

Check that:

- the stream emits `approval-required` before the gated action runs
- the stream does not finish until your application responds
- `approval-decision` appears after you respond
- the action dispatcher runs only after `"approve"`

For dispatcher-specific verification, see
[Execute approved actions](action-dispatcher.md).

## Common errors

| Error | Fix |
| --- | --- |
| No `approval-required` event appears | Check the resolved runtime, agent, and tool policy. A narrower `"never"` can override a broader gate. |
| The stream appears paused | Respond with `runtime.respond_to_approval(...)`; gated work intentionally waits. |
| The wrong tool is gated | Tool overrides are scoped by agent, so check which agent owns the current tool call. |
| A tool `revise` acts like rejection | `revise` is plan-only. Use reject plus feedback for tool calls. |

## See also

- [Approvals concept](../concepts/approvals.md)
- [Execute approved actions](action-dispatcher.md)
- [Streaming events](streaming.md)
- [Runtime events reference](../reference/runtime-events.md)
