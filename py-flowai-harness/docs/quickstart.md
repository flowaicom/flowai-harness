# Quickstart

This quickstart builds a simple multi-agent system that:

- scopes runtime state with a tenant identity
- stores a typed item-set reference with a small glimpse
- defines a typed change plan
- calls an item search tool while planning
- routes through coordinator, planner, and executor agents
- pauses on a plan approval gate
- dispatches approved actions and streams runtime events

No provider key is required. The example uses the scripted interpreter, so the
LLM decisions are deterministic JSON scripts while routing, tools, plan storage,
approval gates, reference hydration, and action dispatch all run through the
runtime.

## Before you begin

- Install Python 3.12 (any 3.12.x).
- You do not need Anthropic, OpenAI, or other provider credentials for this
  example.

## 1. Install

From the root of a clone of the source repository:

```bash
./scripts/check-env.sh
./scripts/install.sh
./.venv/bin/flowai-harness --version
```

`install.sh` verifies the toolchain, builds the Studio UI and native runtime, and
installs `flowai-harness` into `.venv`. Run the scripts in this guide with
`./.venv/bin/python`.

## 2. Create `quickstart.py`

Save the following as `quickstart.py`. It is one complete, runnable script:

```python
import asyncio
import json

from pydantic import BaseModel, Field

from flowai_harness import (
    TaggedUnion,
    create_runtime,
    define_coordinator,
    define_executor,
    define_plan,
    define_planner,
    define_reference,
    define_runtime,
    define_tenant,
    define_tool,
    glimpse,
    layered_prompt,
)


tenant = define_tenant("demo", "v1")

domain_knowledge = {
    "workspace": "A demo workspace with operational items.",
    "categories": ["operations", "customer"],
    "states": ["draft", "review", "approved"],
    "approved_actions": ["status_update", "owner_assignment"],
}


class ItemSetPayload(BaseModel):
    item_ids: list[str]


ItemSet = define_reference(
    name="ItemSet",
    schema=ItemSetPayload,
    ttl_ms=60 * 60 * 1000,
    glimpse=lambda value: {
        "itemCount": len(value.item_ids),
        "preview": value.item_ids[:3],
    },
)


class StatusUpdate(BaseModel):
    kind: str = "status_update"
    item_id: str
    status: str
    references: list[dict[str, str]] = Field(default_factory=list)


class OwnerAssignment(BaseModel):
    kind: str = "owner_assignment"
    item_ids: list[str]
    owner: str
    references: list[dict[str, str]] = Field(default_factory=list)


ChangeAction = TaggedUnion(StatusUpdate, OwnerAssignment)


class ChangePlanPayload(BaseModel):
    target_ref: str
    rationale: str
    actions: list[ChangeAction]


change_plan = define_plan(name="ChangePlan", schema=ChangePlanPayload)


class ItemDirectory:
    def __init__(self) -> None:
        self.items = [
            {
                "id": "item-onboarding-checklist",
                "name": "Onboarding checklist",
                "category": "operations",
            },
            {
                "id": "item-support-handoff",
                "name": "Support handoff",
                "category": "operations",
            },
            {
                "id": "item-renewal-review",
                "name": "Renewal review",
                "category": "customer",
            },
        ]

    async def search(self, query: str, *, limit: int) -> list[dict]:
        terms = query.lower().split()
        matches = [
            item
            for item in self.items
            if any(
                term in item["id"]
                or term in item["name"].lower()
                or term in item["category"]
                for term in terms
            )
        ]
        return matches[:limit]


@define_tool(
    name="search_items",
    description="Search demo items by query.",
    input_schema={"query": str, "limit": int},
    approval="never",
)
async def search_items(args, ctx):
    items = await ctx.directory.search(args["query"], limit=args["limit"])
    return {
        "items": items,
        "glimpse": glimpse(
            {
                "resultCount": len(items),
                "preview": [item["id"] for item in items],
            }
        ),
    }


coordinator = define_coordinator(
    name="change_coordinator",
    model="claude-sonnet-4-6",
    routes=["change_planner", "change_executor"],
    approval={"plans": "always", "tools": "never"},
    prompt=layered_prompt(
        identity="You coordinate structured change requests.",
        communication="Be concise and call out approval points.",
        operational_rules=[
            "Send plan creation to change_planner.",
            "Send approved execution to change_executor.",
        ],
        domain_knowledge=domain_knowledge,
        safety=["Never execute side-effecting work before approval."],
    ),
)

planner = define_planner(
    name="change_planner",
    model="claude-sonnet-4-6",
    plan=change_plan,
    tools=[search_items],
    prompt=layered_prompt(
        identity="You turn requests into typed change plans.",
        tools=[search_items],
        domain_knowledge=domain_knowledge,
        output_format="Store exactly one ChangePlan.",
    ),
)

executor = define_executor(
    name="change_executor",
    model="claude-sonnet-4-6",
    plan=change_plan,
    prompt=layered_prompt(
        identity="You execute approved ChangePlan actions.",
        domain_knowledge=domain_knowledge,
        safety=["Only execute plans after the runtime approval gate resolves."],
    ),
)


def dispatch_actions(actions, ctx):
    resolved_sets = ctx["resolved_refs"].get("ItemSet", {})
    affected_items = sum(len(value["item_ids"]) for value in resolved_sets.values())
    affected_items = affected_items or len(actions)

    print(
        f"dispatch: approved {len(actions)} action(s) "
        f"for {affected_items} item(s)"
    )

    return {
        "entitiesAffected": affected_items,
        "summary": f"Queued {len(actions)} approved change action(s).",
        "details": {
            "actions": actions,
            "resolvedRefs": ctx["resolved_refs"],
        },
    }


runtime = create_runtime(
    define_runtime(
        tenant=tenant,
        agents=[coordinator, planner, executor],
        references=[ItemSet],
        providers={"anthropic": {"apiKey": "unused"}},
    ),
    services={"directory": ItemDirectory()},
    action_dispatcher=dispatch_actions,
    interpreter="scripted",
)


def build_script(item_ref: dict) -> str:
    plan_id = "change-plan-1"
    ref_handle = {"kind": item_ref["kind"], "id": item_ref["id"]}

    planner_prompt = json.dumps(
        {
            "script": [
                {
                    "tool": "search_items",
                    "args": {"query": "operations handoff", "limit": 3},
                },
                {
                    "tool": "storePlan",
                    "args": {
                        "specName": "ChangePlan",
                        "planId": plan_id,
                        "body": {
                            "target_ref": item_ref["id"],
                            "rationale": (
                                "Apply a small reviewed change to the selected "
                                "operational items."
                            ),
                            "actions": [
                                {
                                    "kind": "owner_assignment",
                                    "item_ids": [
                                        "item-onboarding-checklist",
                                        "item-support-handoff",
                                    ],
                                    "owner": "ops-review",
                                    "references": [ref_handle],
                                }
                            ],
                        },
                    },
                },
            ]
        }
    )

    executor_prompt = json.dumps(
        {"tool": "executePlan", "args": {"planId": plan_id}}
    )

    return json.dumps(
        {
            "script": [
                {
                    "tool": "call_agent",
                    "args": {
                        "agent": "change_planner",
                        "prompt": planner_prompt,
                    },
                },
                {
                    "tool": "call_agent",
                    "args": {
                        "agent": "change_executor",
                        "prompt": executor_prompt,
                    },
                },
            ]
        }
    )


async def main() -> None:
    item_ref = await runtime.create_reference(
        ItemSet,
        ItemSetPayload(
            item_ids=[
                "item-onboarding-checklist",
                "item-support-handoff",
                "item-renewal-review",
            ]
        ),
    )
    item_glimpse = item_ref["glimpse"]
    print(
        "reference glimpse: "
        f"itemCount={item_glimpse['itemCount']} "
        f"preview={item_glimpse['preview']}"
    )

    seen_tool_results = set()
    async for event in runtime.query(
        build_script(item_ref),
        thread_id="quickstart-thread",
    ):
        if event["type"] == "tool-agent" and event["state"] == "call":
            print(f"agent call: {event['agentName']}")

        if (
            event["type"] == "tool-invocation"
            and event["state"] == "result"
            and event["toolName"] != "call_agent"
        ):
            result_key = (event["toolInvocationId"], event["toolName"])
            if result_key in seen_tool_results:
                continue
            seen_tool_results.add(result_key)
            print(f"tool result: {event['toolName']}")

        if event["type"] == "approval-required":
            data = event["data"]
            print(f"approval required: {data['kind']} {data['target']}")
            await runtime.respond_to_approval(
                data["id"],
                "approve",
                feedback="approved in quickstart",
            )

        if event["type"] == "approval-decision":
            print(f"approval decision: {event['data']['outcome']}")

    print("finish")


asyncio.run(main())
```

## 3. Run it

```bash
./.venv/bin/python quickstart.py
```

## 4. Expected output

Identifiers differ on every run, but the summary should look like this:

```text
reference glimpse: itemCount=3 preview=['item-onboarding-checklist', 'item-support-handoff', 'item-renewal-review']
agent call: change_coordinator
agent call: change_planner
tool result: search_items
tool result: storePlan
agent call: change_executor
approval required: plan change-plan-1
dispatch: approved 1 action(s) for 3 item(s)
approval decision: {'outcome': 'approve'}
tool result: executePlan
finish
```

## 5. Inspect what happened

The run is deterministic, but it exercises the harness behavior instead of only
mocking final text:

- `define_tenant("demo", "v1")` scopes runtime-owned references, plans,
  approvals, and telemetry.
- `ItemSet` stores the full item list outside the prompt and exposes only a
  small `glimpse` in the event flow.
- `ChangePlan` validates the planner's typed action payload before execution.
- `search_items` is a normal Python tool attached to the planner.
- The coordinator uses `call_agent` to route first to the planner, then to the
  executor.
- The planner calls `storePlan`; the executor calls `executePlan`.
- `approval={"plans": "always"}` makes `executePlan` pause on
  `approval-required`.
- `runtime.respond_to_approval(..., "approve")` resumes the stream.
- `dispatch_actions` receives normalized actions plus hydrated references only
  after approval.

The scripted interpreter replaces the model's choices with JSON scripts so the
quickstart works without credentials. With a live interpreter, the same runtime
topology lets the model decide when to call `search_items`, `storePlan`, and
`executePlan`.

## Where to next

- Read [Concepts](concepts/index.md) for the mental model behind tenants,
  agents, plans, references, tools, and runtime events.
- Read [Require approvals](guides/approvals.md) for approval outcomes and
  tool-level approval gates.
- Read [Execute approved actions](guides/action-dispatcher.md) for the action
  dispatcher pattern used by approved plans.
- Read [References & Glimpses](concepts/references.md) for large or sensitive
  values that should not be stuffed into every prompt.
- Open [Studio](guides/studio.md) when you want a browser UI for chat, runs,
  traces, tests, and evals.
- Continue with the [Coordinator planner executor tutorial](tutorials/coordinator-planner-executor.md)
  for the default full Studio app.
- Continue with the [Inventory scenario tutorial](tutorials/inventory-scenario.md)
  for a larger data-agent example with references, catalog tools, and approved
  side effects.
