# Action dispatcher

The action dispatcher is the host-side write boundary for approved plan
actions. It is where Flow AI hands normalized, validated actions back to your
application so your code can update databases, call APIs, enqueue jobs, or
apply other side effects.

The dispatcher is not a model-callable tool. A planner first stores a typed
plan, the runtime validates it, approval may pause execution, and an executor
calls `executePlan`. Only after that flow does the runtime invoke
`action_dispatcher`.

This page records the exact callback and return-value contract. For the task
flow, see [Execute approved actions](../guides/action-dispatcher.md). For the
conceptual model, see [Action dispatcher](../concepts/action-dispatcher.md).

## Callback shape

```python
def dispatch_actions(actions, ctx):
    ...
```

`actions` is a JSON-serializable list of normalized harness actions. Flat plan
actions are converted to the canonical shape before dispatch:

```python
[
    {
        "kind": "price_change",
        "payload": {"product_id": "sku-1", "new_price": 12.5},
        "references": [],
    }
]
```

`ctx["resolved_refs"]` contains action references hydrated by the runtime,
grouped by reference kind and id:

```python
{
    "ProductSet": {
        "ref-123": {"product_ids": ["sku-1", "sku-2"]}
    }
}
```

## Return value

The dispatcher may return `None`, which is treated as an empty execution result.
Otherwise it must return:

```python
{
    "entitiesAffected": 3,
    "summary": "Updated 3 products",
    "details": {"platformJobId": "job-123"},
}
```

Accepted top-level fields:

- `entitiesAffected`: required non-negative integer
- `summary`: optional string or `None`
- `details`: optional JSON value

Put domain-specific metadata under `details`. Invalid dispatcher returns fail
`executePlan`, and the plan is marked failed.
