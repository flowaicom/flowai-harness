# Coordinator, planner, and executor tutorial

The
[`examples/coordinator_planner_executor`](https://github.com/flowaicom/flowai-harness/tree/main/examples/coordinator_planner_executor)
sample is a complete local Flow AI Harness app. This tutorial explains how the
sample is put together and why each piece exists. It is not a blank-file build
guide; the working code already lives in the example directory.

Use the sample
[`README`](https://github.com/flowaicom/flowai-harness/tree/main/examples/coordinator_planner_executor#readme)
when you need the exact local runbook. Use this page when you want to understand
the harness code behind the sample.

The app demonstrates a common multi-agent shape:

- A `coordinator` receives user requests and routes them to specialized agents.
- A `data_analyst` answers read-only product, order, and revenue questions.
- A `planner` creates a typed catalog action plan and stores it.
- An `executor` reads the stored plan and applies it through an action
  dispatcher.
- Studio exposes two local workspaces, `acme` and `globex`.
- Smoke and eval checks verify the planner/executor path without relying only
  on manual Studio testing.

## What to read first

Most of the sample code is in three files under
`examples/coordinator_planner_executor/src/coordinator_planner_executor/`:

- `app.py`
- `seed_eval_tests.py`
- `smoke.py`

The regression test for the smoke check lives in
`examples/coordinator_planner_executor/tests/test_smoke.py`.

`app.py` is the core harness definition. It contains the plan contract, agent
specs, local data environment, action dispatcher, runtime factory, Studio app,
and eval payloads.

`smoke.py` is the offline executable check. It uses the scripted interpreter to
exercise the coordinator, planner, and executor without a model API key.

`seed_eval_tests.py` inserts the sample eval cases into the local Studio store
so they can be run from Studio's Tests and Evals views.

## The runtime boundary

The most important function in the sample is `create_example_runtime(...)`.
It is the bridge between the declarative harness spec and an executable runtime:

```python
def create_example_runtime(resource_id: str = "acme", *, interpreter: str = "anthropic"):
    spec = _runtime_spec(resource_id)
    return create_runtime(
        spec,
        data_environment=_data_environment(resource_id),
        action_dispatcher=_dispatch_catalog_actions,
        interpreter=interpreter,
    )
```

That call combines four concerns:

- `_runtime_spec(resource_id)` describes the agents, tools, plan types, tenant,
  and model provider.
- `_data_environment(resource_id)` provides the target SQLite database plus the
  runtime-owned catalog, key-value, and search stores.
- `_dispatch_catalog_actions` is the side-effect boundary used by
  `executePlan`.
- `interpreter` chooses how the runtime executes model steps. Studio uses the
  Anthropic-backed interpreter; smoke tests use the scripted interpreter.

This split is what makes the same app usable from Studio, evals, and local
tests.

## The typed plan contract

Planner and executor agents communicate through a shared `CatalogActionPlan`.
The sample defines that contract with `define_plan(...)` in `app.py`.

The full schema is in the example, but the essential shape is:

```python
CATALOG_ACTION_PLAN = define_plan(
    "CatalogActionPlan",
    {
        "required": ["rationale", "actions"],
        "properties": {
            "rationale": {"type": "string"},
            "actions": {
                "type": "array",
                "items": {
                    "required": ["kind", "productId", "newPrice", "reason"],
                    "properties": {
                        "kind": {"const": "price_change"},
                        "productId": {"type": "string"},
                        "newPrice": {"type": "number"},
                        "reason": {"type": "string"},
                    },
                },
            },
        },
    },
)
```

The schema keeps the planner's output narrow. A stored plan cannot contain
free-form workflow steps, null prices, or invented action kinds. That matters
because the executor treats stored plan actions as executable inputs.

The constants below the schema define one stable price-change fixture:
product `p-001`, target price `6.49`, and plan id
`plan-eval-price-change`. The smoke check and eval cases reuse that fixture so
they all verify the same contract.

## The agent architecture

`_runtime_spec(resource_id)` defines the four-agent architecture. Each agent
has a specific job, and the coordinator is the only agent that routes between
them.

The coordinator declaration is the visible routing boundary:

```python
coordinator = define_coordinator(
    name="coordinator",
    model="claude-sonnet-4-6",
    prompt=(
        "Route data, database, product, order, and revenue questions to "
        "data_analyst. Route catalog changes that need a new plan to "
        "planner first, then route execution of an existing stored plan to "
        "executor."
    ),
    routes=["data_analyst", "planner", "executor"],
)
```

The data analyst opts into the catalog toolkit with `toolkits=["catalog"]`.
The planner and executor receive plan tools because they are declared with the
shared `CATALOG_ACTION_PLAN`:

- `define_planner(..., plan=CATALOG_ACTION_PLAN, ...)` gives the planner the
  ability to store typed plans.
- `define_executor(..., plan=CATALOG_ACTION_PLAN, ...)` gives the executor the
  ability to inspect and execute stored plans.

The runtime collects those agents and declares the catalog toolkit once:

```python
return define_runtime(
    tenant=define_tenant(resource_id, "0.1"),
    agents=[coordinator, planner, executor, analyst],
    toolkits=[
        {"id": "catalog", "config": {"tools": ["execute_query"]}},
    ],
    providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
)
```

The prompts in the sample are intentionally more explicit than a demo prompt
would normally be. The evals are checking concrete behavior: stable product
IDs, a numeric `newPrice`, no extra workflow actions in the stored plan, and an
executor path that uses a plan created by the planner.

## The local data environment

The `data_analyst` has a real target database to query. The sample seeds two
tables, `products` and `orders`, under `.flowai/coordinator-planner-executor/`
for each workspace.

The data environment distinguishes application data from runtime support
stores:

```python
return {
    "target_database": {"kind": "sqlite", "url": _sqlite_url(target_path)},
    "catalog": {
        "kind": "sqlite",
        "url": _sqlite_url(root / "catalog.db"),
        "ensure_schema": True,
    },
    "kv": {
        "kind": "sqlite",
        "url": _sqlite_url(root / "kv.db"),
        "ensure_schema": True,
    },
    "catalog_search": {
        "index_path": str(root / "catalog-index"),
        "rebuild_on_start": True,
    },
}
```

`target_database` is the read-only customer data source. `catalog`, `kv`, and
`catalog_search` are owned by the runtime and let the catalog toolkit profile,
index, search, and store metadata locally.

Because `_data_environment(resource_id)` is parameterized by workspace key,
`acme` and `globex` get the same schema and seed data but separate local files.

## The execution boundary

The executor does not update the database directly. It hands plan actions to an
action dispatcher:

```python
def _dispatch_catalog_actions(actions: Any, _ctx: Any) -> dict[str, Any]:
    count = len(actions) if hasattr(actions, "__len__") else 0
    return {
        "entitiesAffected": count,
        "summary": f"mock-dispatched {count} catalog action(s)",
        "details": {"dispatcher": EXAMPLE_APP_NAME, "mode": "mock"},
    }
```

The sample dispatcher is a mock on purpose. It makes the executor path visible
and testable without introducing a real pricing API. In a production app, this
function is where you would cross into a workflow engine, queue, internal
service, or other side-effecting system.

## The Studio app

Studio loads the exported `app` object from
`coordinator_planner_executor.app:app`. The app definition registers the
available workspaces and points each one at a runtime factory:

```python
app = define_app(
    name=EXAMPLE_APP_NAME,
    default_workspace="acme",
    workspaces={
        "acme": _workspace("acme", "ACME Demo"),
        "globex": _workspace("globex", "Globex Demo"),
    },
)
```

The workspace helper builds a workspace runtime with a display name, runtime
spec, runtime factory, data environment, and metadata. Studio uses that
definition to populate the workspace picker and to start conversations against
the right runtime.

Useful prompts to try in Playground:

- `List the products that you have.`
- `Which product has the highest revenue?`
- `Create a price-change plan for Sparkling Water 12pk.`
- `Execute the stored price-change plan.`

Runs will show the important internals: sub-agent calls, tool calls, plan
storage, execution, approvals, and traces.

## The offline smoke check

The smoke check proves the wiring works without calling a provider. It creates
the runtime with `interpreter="scripted"` and sends a scripted coordinator
prompt that calls the planner and then the executor.

Conceptually, the script does this:

```python
[
    {"tool": "call_agent", "args": {"agent": "planner", "prompt": planner_prompt}},
    {"tool": "call_agent", "args": {"agent": "executor", "prompt": executor_prompt}},
]
```

`smoke.py` then asserts that:

- the observed agent order starts with `coordinator -> planner -> executor`
- `storePlan` stored the expected plan id
- `executePlan` reported one affected entity
- the runtime emitted a finish event

This gives the sample a fast regression check for harness wiring, plan storage,
and execution dispatch.

## The eval cases

The eval payloads live in `app.py` next to the shared price-change fixture.
They test the same behavior as a user would expect in Studio, but with durable
assertions.

The planner eval expects a `storePlan` call and checks the planned action:

```python
PLANNER_EVAL_TEST_CASE = {
    "id": "tc-planner-price-change",
    "input": (
        "We need a catalog action plan for product p-001, Sparkling Water "
        "12pk. Lower the price to $6.49 because wholesale demand is strong."
    ),
    "expectedTrajectory": ["storePlan"],
    "trajectoryMode": "subsequence",
    "structuredGroundTruth": {
        "kind": "structured",
        "payload": {
            "kind": "flat",
            "payloadMatch": "subset",
            "plannedActions": [PRICE_CHANGE_EXPECTED_ACTION],
        },
    },
}
```

The executor eval asks for the plan to be created and applied, then expects
`storePlan` followed by `executePlan`.

The user-facing inputs stay natural. Tool names and JSON arguments belong in
the assertions, not in the user prompt. The prompt includes the canonical
product ID because the eval should catch an agent that invents a slug from a
product name.

`seed_eval_tests.py` validates each payload with `EvalTestCase` and upserts it
into the local Studio SQLite store for a workspace. After seeding, the cases
appear in Studio's Tests and Evals views.

## Run it locally

The source-checkout run path is:

```bash
./scripts/install.sh
cd examples/coordinator_planner_executor
uv pip install --python ../../.venv/bin/python -e .
../../.venv/bin/python -m coordinator_planner_executor.smoke
```

For Studio, create `.env` from `.env.example`, set `ANTHROPIC_API_KEY`, and
start the app:

```bash
../../.venv/bin/flowai-harness dev \
  --app coordinator_planner_executor.app:app \
  --port 4111
```

Then open:

```text
http://127.0.0.1:4111
```

Seed eval cases when you want them available in Studio:

```bash
../../.venv/bin/python -m coordinator_planner_executor.seed_eval_tests \
  --workspace acme
```

The sample README has the operational details and troubleshooting notes. This
tutorial's main goal is to make the code path clear enough that you can adapt
the sample to a real app.

## Next steps

- Run the [Inventory scenario tutorial](inventory-scenario.md) for the advanced
  data-agent pattern with references, catalog tools, and approved side effects.
- Read the [Studio guide](../guides/studio.md) for the browser workflow.
- Read [Configure a data environment](../guides/data-environment.md) for database and
  catalog configuration.
- Read [Test agents without provider calls](../guides/testing.md) and
  [Evals](../guides/evals.md) for authoring durable behavior checks.
- Read [Plans](../concepts/plans.md) for the typed plan lifecycle used by the
  planner and executor.
