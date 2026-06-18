# Plans

Plans make agent work explicit before it happens.

A **plan** is a typed container for the domain actions an agent intends to
perform. Instead of letting an agent reason and mutate systems in the same loop,
Flow AI lets the agent first produce a structured plan. The runtime can then
validate, persist, inspect, approve, execute, and evaluate that plan.

Use plans when agent work is consequential: updating records, changing prices,
approving requests, sending messages, triggering workflows, or modifying
customer data.

## The short version

A plan answers:

```text
What is the agent going to do?
Why is it doing it?
Which domain actions will be executed?
What data or scope does the plan apply to?
Has the plan been approved?
What happened during execution?
```

The normal flow is:

```text
User goal
  -> planner creates a typed plan
  -> runtime validates and stores the plan
  -> approval gate pauses risky execution
  -> executor runs the approved actions
  -> runtime records the result
```

## Plans, actions, and tools

The important distinction is:

```text
Plan   = container for the proposed work
Action = domain-specific operation inside the plan
Tool   = model-callable capability used for lookup, preview, or orchestration
```

For an inventory or pricing workflow, a plan might contain actions like:

- change product price
- launch promotion
- reorder inventory
- disable unavailable product
- notify account manager

These are business-level actions that describe what
should happen in the domain. The executor, runtime, and host application decide
which tools, APIs, database operations, or workflows are needed to apply them.

Prefer domain actions:

```text
Refund customer
Update subscription
Create support ticket
Send renewal email
Approve invoice
```

Avoid low-level implementation actions:

```text
Call POST /refunds
Run SQL UPDATE
Invoke send_email_tool
Call CRM API
```

## Define typed actions

Model each action as a typed schema. When a plan can contain more than one
action type, use `TaggedUnion(...)` so every action has a discriminator.

```python
from typing import Literal

from pydantic import BaseModel

from flowai_harness import TaggedUnion


class PriceChange(BaseModel):
    kind: Literal["price_change"]
    product_id: str
    new_price: float
    reason: str


class PromotionLaunch(BaseModel):
    kind: Literal["promotion_launch"]
    product_ids: list[str]
    discount_pct: float
    reason: str


class StakeholderNotification(BaseModel):
    kind: Literal["stakeholder_notification"]
    channel: str
    message: str
    reason: str


PricingAction = TaggedUnion(
    PriceChange,
    PromotionLaunch,
    StakeholderNotification,
)
```

`TaggedUnion(...)` defaults to the `kind` discriminator. It also supports the
harness shorthand `kind: str = "price_change"` if you prefer defaults over
`Literal[...]`.

## Define the plan body

The plan body is your application contract. It must contain a top-level
`actions` array, and it can include domain metadata such as `rationale`,
`scope_ref`, review notes, or requested constraints.

```python
from pydantic import BaseModel, Field

from flowai_harness import define_plan


class PricingPlan(BaseModel):
    scope_ref: str
    rationale: str
    actions: list[PricingAction] = Field(min_length=1)


pricing_plan = define_plan(
    name="PricingPlan",
    schema=PricingPlan,
)
```

`define_plan(...)` creates a frozen `PlanSpec`. The schema can come from a
Pydantic model, JSON Schema, a simple `{name: type}` mapping, or another type
hint Pydantic can export.

Planner output uses the flat action shape from your schema:

```json
{
  "scope_ref": "catalog:summer-products",
  "rationale": "Improve sell-through for slow-moving seasonal inventory.",
  "actions": [
    {
      "kind": "price_change",
      "product_id": "SKU-123",
      "new_price": 19.99,
      "reason": "The product is underperforming against similar items."
    },
    {
      "kind": "promotion_launch",
      "product_ids": ["SKU-123", "SKU-456"],
      "discount_pct": 15,
      "reason": "The products belong to the same seasonal campaign."
    }
  ]
}
```

When the planner calls `storePlan`, the runtime validates the whole body against
the `PlanSpec` schema. It then extracts `actions`, converts each flat action
into the canonical stored action shape, and keeps the other top-level fields as
plan context so they survive the round trip.

## Planner and executor

Planning and execution are separate roles.

The **planner** creates and stores plan instances. The **executor** loads an
existing plan and executes its approved actions.

Both roles should use the same `PlanSpec`:

```python
from flowai_harness import define_executor, define_planner


planner = define_planner(
    name="pricing_planner",
    model="claude-sonnet-4-6",
    plan=pricing_plan,
    prompt="Create safe, typed pricing plans.",
)

executor = define_executor(
    name="pricing_executor",
    model="claude-sonnet-4-6",
    plan=pricing_plan,
    tools=[search_products, preview_price_change],
    prompt="Execute approved pricing plans.",
)
```

The harness scopes built-in plan tools by role:

- planners get `storePlan` and `getPlan`
- executors get `getPlan` and `executePlan`
- other roles can read plans with `getPlan` when they use the plans toolkit

This prevents the planner from directly executing plans and keeps the executor
focused on approved stored work.

## Approval and execution

Plans are a safety boundary between reasoning and side effects.

Without a plan, an agent may mix reasoning and writes:

```text
Think -> call tool -> think -> call API -> update state
```

With a plan, the agent commits to a typed proposal first:

```text
Think -> store plan -> validate -> approve -> execute
```

Plan approval is configured through the runtime approval policy. When approval
is required, `executePlan` pauses before the write boundary. The plan remains in
`draft` while the approval gate is open; the runtime may emit a
`pending_approval` status-change event for display, but `pending_approval` is
not a stored plan status.

After approval, `executePlan` transitions the plan to execution and dispatches
the normalized actions through the host `action_dispatcher`.

```python
def dispatch_actions(actions, ctx):
    created = []

    for action in actions:
        if action["kind"] == "price_change":
            payload = action["payload"]
            created.append(
                commerce_api.update_price(
                    product_id=payload["product_id"],
                    new_price=payload["new_price"],
                )
            )
        else:
            raise ValueError(f"unsupported action kind: {action['kind']}")

    return {
        "entitiesAffected": len(created),
        "summary": f"Applied {len(created)} pricing action(s)",
        "details": {"createdActions": created},
    }
```

Tools are model-callable. The action dispatcher is not model-callable; it is the
host callback that applies approved plan actions to your platform. A real
dispatcher should handle every action kind your `PlanSpec` allows, or reject
unsupported variants explicitly.

## Lifecycle

Stored plans use a fixed lifecycle. Under the hood, each plan is a state
machine: it can only move through valid transitions, and terminal states cannot
be reopened.

```text
draft -> approved -> executing -> executed
                         |
                         v
                       failed
```

The runtime owns these transitions:

- `storePlan` creates a `draft` plan after schema validation.
- The approval gate records approval and moves the plan to `approved`.
- `executePlan` starts the plan, runs the dispatcher, and records `executed` or
  `failed`.

`executed` and `failed` are terminal statuses.

## Display labels

You can customize how fixed lifecycle statuses appear to users:

```python
pricing_plan = define_plan(
    name="PricingPlan",
    schema=PricingPlan,
    display_aliases={
        "draft": "Draft pricing proposal",
        "approved": "Approved pricing proposal",
        "executing": "Applying pricing changes",
        "executed": "Pricing changes applied",
        "failed": "Pricing update failed",
    },
)
```

Display aliases only affect presentation. The runtime still tracks the fixed
statuses `draft`, `approved`, `executing`, `executed`, and `failed`.

## When to use plans

Use plans when work needs structure, review, approval, or replayable evidence of
intent.

Good examples:

- changing prices
- launching promotions
- creating tickets
- updating customer records
- approving or rejecting requests
- running operational workflows
- sending external communications
- executing database-backed changes

Plans can also help with read-only workflows when structure matters:

- running an investigation
- generating a report
- comparing vendors
- preparing a migration proposal
- validating data quality

Plans are most valuable when the agent may modify state or trigger real-world
effects.

## Design guidelines

Good plan actions are:

- domain-specific
- typed
- inspectable
- executable
- easy to approve or reject
- stable enough to evaluate

Keep low-level mechanics in tools, dispatchers, and platform adapters. Keep the
plan close to business intent.

## Common mistakes

Do not use plans as free-form text summaries. A plan should describe executable
intent in a typed shape.

Do not put write APIs directly in planner tools. Let planners inspect and
propose; let executors and the host dispatcher apply approved actions.

## See also

- [Approvals](approvals.md) for pausing plans before execution.
- [Agents](agents.md) for planner and executor roles.
- [Tools](tools.md) for model-callable capabilities.
- [Action dispatcher](action-dispatcher.md) for applying approved plan actions.
- [Execute approved actions](../guides/action-dispatcher.md) for applying plan
  actions to your platform.
- [`define_plan` reference](../reference/plans.md#flowai_harness.plans.define_plan).
