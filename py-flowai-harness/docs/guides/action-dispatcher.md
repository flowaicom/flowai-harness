# Execute approved actions

The action dispatcher is the host adapter that applies approved plan actions to
your platform.

Use it when a plan contains actions that should only run after approval. Tools
help the model inspect, search, simulate, or preview state. The action
dispatcher performs the approved writes.

## When to use an action dispatcher

Use an action dispatcher when:

- the planner produces typed actions
- the executor should run those actions only after approval
- your platform write API should not be directly model-callable
- you need one place to validate and apply approved changes

By the end of this guide, your runtime should call your dispatcher only after an
approved `executePlan` flow.

## Tools vs action dispatcher

Tools are model-callable.

The action dispatcher is not model-callable.

Use tools for search, inspection, simulation, and previews. Use the dispatcher
for approved writes.

```text
planner -> preview tools -> typed plan -> approval -> executor -> dispatcher
```

## Define typed plan actions

The plan should describe executable intent in a small set of action variants.

```python
from typing import Literal

from pydantic import BaseModel

from flowai_harness import TaggedUnion, define_plan


class PriceChange(BaseModel):
    kind: Literal["price_change"]
    product_id: str
    new_price: float


class AvailabilityChange(BaseModel):
    kind: Literal["availability_change"]
    product_id: str
    available: bool


CommercialAction = TaggedUnion(PriceChange, AvailabilityChange)


class CommercialPlan(BaseModel):
    rationale: str
    actions: list[CommercialAction]


commercial_plan = define_plan("CommercialPlan", CommercialPlan)
```

## Implement the dispatcher

The dispatcher receives normalized actions and returns a small execution result.

```python
class CommerceApi:
    def create_action(self, *, action_type, payload):
        return {"id": f"action-{action_type.lower()}", "payload": payload}


api = CommerceApi()


def dispatch_actions(actions, ctx):
    created = []

    for action in actions:
        payload = action["payload"]

        if action["kind"] == "price_change":
            created.append(
                api.create_action(
                    action_type="PRICE_CHANGE",
                    payload={
                        "productId": payload["product_id"],
                        "newPrice": payload["new_price"],
                    },
                )
            )
        elif action["kind"] == "availability_change":
            created.append(
                api.create_action(
                    action_type="AVAILABILITY_CHANGE",
                    payload={
                        "productId": payload["product_id"],
                        "available": payload["available"],
                    },
                )
            )
        else:
            raise ValueError(f"unsupported action kind: {action['kind']}")

    return {
        "entitiesAffected": len(created),
        "summary": f"Created {len(created)} platform action(s)",
        "details": {"createdActions": created},
    }
```

For the exact action and return shapes, see the
[Action dispatcher reference](../reference/action-dispatcher.md).

## Wire the dispatcher into the runtime

Pass the dispatcher to `create_runtime(...)`.

```python
runtime = create_runtime(
    define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, planner, executor],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    ),
    action_dispatcher=dispatch_actions,
)
```

Plan approvals are configured separately. In production flows, pair the
dispatcher with a plan approval policy:

```python
coordinator = define_coordinator(
    "coordinator",
    model="claude-sonnet-4-6",
    routes=["planner", "executor"],
    approval={"plans": "always", "tools": "never"},
    prompt="Route planning and execution.",
)
```

## Share platform dependencies

Tools receive `services` through the tool context:

```python
@define_tool("preview_price_change", {"product_id": str, "new_price": float})
async def preview_price_change(args, ctx):
    return ctx.platform.preview_price_change(args["product_id"], args["new_price"])
```

The dispatcher context is different. It carries execution data such as hydrated
references, not the `services` mapping. Close over platform clients from the
runtime factory instead.

```python
def build_runtime(tenant_id: str):
    api = CommerceApi()

    def dispatch_actions(actions, ctx):
        return apply_actions(api, actions)

    return create_runtime(
        define_runtime(
            tenant=define_tenant(tenant_id, "v1"),
            agents=[coordinator, planner, executor],
        ),
        services={"platform": api},
        action_dispatcher=dispatch_actions,
    )
```

## Verify that writes only happen after approval

In a scripted test, count dispatcher calls and approve inside the stream loop.

```python
calls = []


def dispatch_actions(actions, ctx):
    calls.append(actions)
    return {"entitiesAffected": len(actions)}


async for event in runtime.query(scripted_prompt, thread_id="thread-1"):
    assert calls == []
    if event["type"] == "approval-required":
        await runtime.respond_to_approval(event["data"]["id"], "approve")

assert len(calls) == 1
```

## Common errors

| Error | Fix |
| --- | --- |
| Dispatcher expects `ctx.platform` | Use closure or runtime factory scope for platform clients; dispatcher context is not tool context. |
| Writes happen during planning | Keep write APIs out of model-callable tools. Use tools only for preview or simulation. |
| Dispatcher sees an unknown action kind | Add a handler for every plan action variant, or reject unsupported variants explicitly. |
| `executePlan` fails after approval | Check the dispatcher return shape and put domain metadata under `details`. |

## See also

- [Action dispatcher concept](../concepts/action-dispatcher.md)
- [Require approvals](approvals.md)
- [Plans concept](../concepts/plans.md)
- [Tools concept](../concepts/tools.md)
- [Action dispatcher reference](../reference/action-dispatcher.md)
