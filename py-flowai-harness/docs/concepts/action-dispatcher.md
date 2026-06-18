# Action dispatcher

The action dispatcher is the host-side function that executes approved plan
actions.

It is not a tool the model can call directly. The planner creates a typed plan,
the runtime validates and stores it, approval may be required, and the executor
calls `executePlan`. Only after that flow does the runtime invoke your
`action_dispatcher`.

In product terms, the action dispatcher is the bridge between Flow AI's typed
plan world and your application's side-effect world.

## The short version

```text
Planner
  -> creates typed plan actions

Runtime
  -> validates and stores the plan
  -> pauses for approval if needed

Executor
  -> calls executePlan

Action dispatcher
  -> maps approved actions to platform/API/database/job calls
```

Use the action dispatcher for work that changes real systems: updating prices,
creating records, sending committed messages, approving invoices, launching
campaigns, or enqueueing platform jobs.

## Why it is not a tool

Tools are model-callable. They are useful while the agent is deciding what
should happen.

Use tools for:

- search
- inspection
- preview
- simulation
- read-only analysis
- reference creation

The action dispatcher is different. It applies the final approved writes after
the plan has been validated and approved.

```text
Tool
  model-callable capability for planning-time work

Action dispatcher
  host callback for approved execution-time writes
```

This keeps dangerous write APIs out of the model's direct tool surface. The
model can propose structured actions, but the runtime controls validation,
approval, reference hydration, and dispatch.

## What the dispatcher receives

The dispatcher receives a list of normalized actions.

Planner output usually uses the flat action shape from your plan schema:

```json
{
  "kind": "price_change",
  "product_id": "sku-1",
  "new_price": 12.5
}
```

Before dispatch, the runtime normalizes each action into the harness action
shape:

```python
[
    {
        "kind": "price_change",
        "payload": {"product_id": "sku-1", "new_price": 12.5},
        "references": [],
    }
]
```

The dispatcher also receives `ctx`. If actions contain references, the runtime
hydrates them before calling the dispatcher and exposes them as
`ctx["resolved_refs"]`, grouped by reference kind and id:

```python
{
    "ProductSet": {
        "ref-123": {"product_ids": ["sku-1", "sku-2"]}
    }
}
```

Use hydrated references when an action points to a large or sensitive payload
instead of embedding everything directly in the plan action.

## Example

```python
def dispatch_actions(actions, ctx):
    created = []

    for action in actions:
        payload = action["payload"]

        if action["kind"] == "price_change":
            created.append(
                platform_api.create_price_change(
                    product_id=payload["product_id"],
                    new_price=payload["new_price"],
                )
            )
        else:
            raise ValueError(f"unsupported action kind: {action['kind']}")

    return {
        "entitiesAffected": len(created),
        "summary": f"Executed {len(created)} action(s)",
        "details": {"createdActions": created},
    }
```

Wire the dispatcher into the runtime:

```python
runtime = create_runtime(
    runtime_spec,
    action_dispatcher=dispatch_actions,
)
```

## Return value

The dispatcher returns an execution result:

```python
{
    "entitiesAffected": 3,
    "summary": "Updated 3 products",
    "details": {"platformJobId": "job-123"},
}
```

Use:

- `entitiesAffected` for a non-negative count of affected entities
- `summary` for a short human-readable result
- `details` for domain-specific metadata

The dispatcher may also return `None`, which the runtime treats as an empty
execution result. Invalid return values fail `executePlan`, and the plan is
marked failed.

## Platform dependencies

Python tools receive host services through tool context, such as `ctx.orders` or
`ctx["orders"]`.

The action dispatcher context is different. It carries execution data such as
`ctx["resolved_refs"]`; it is not the same service context used by Python tool
handlers. If the dispatcher needs platform clients, close over them from your
runtime factory:

```python
def build_runtime():
    platform_api = PlatformApi()

    def dispatch_actions(actions, ctx):
        return apply_actions(platform_api, actions, ctx)

    return create_runtime(
        runtime_spec,
        services={"platform": platform_api},
        action_dispatcher=dispatch_actions,
    )
```

## Design rule

If the model needs to inspect, search, preview, or simulate something, define a
tool.

If approved plan actions need to patch your API, update a database, send a
message, or trigger a job, use the action dispatcher.

```text
Flow AI action kind: "price_change"
        |
        v
Dispatcher
        |
        v
Your commerce API: create_action(type="PRICE_CHANGE", payload=...)
```

## See also

- [Plans](plans.md) for typed actions and plan lifecycle.
- [Tools](tools.md) for model-callable capabilities.
- [Approvals](approvals.md) for runtime approval gates.
- [Execute approved actions](../guides/action-dispatcher.md) for the
  implementation walkthrough.
- [Action dispatcher reference](../reference/action-dispatcher.md) for exact
  callback and return shapes.
