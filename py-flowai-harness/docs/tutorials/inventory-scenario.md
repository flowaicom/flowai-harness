# Build an product inventory agent

The inventory agent example uses a published SQLite artifact, local catalog/KV/index
files, a mutable FastAPI mock platform, and a coordinator/planner/executor/
insights agent topology.

Use this page to understand the pattern and run the main flow. Use the
[example README](https://github.com/flowaicom/flowai-harness/tree/main/examples/inventory_scenario#readme)
as the local runbook for exact setup, verification, mock-platform, and
troubleshooting commands from a source checkout.

Use it when you want to see the harness working with:

- static artifact setup instead of direct warehouse access,
- the built-in `catalog` toolkit over local SQLite data,
- typed plan actions for inventory replenishment, safety stock, and promotion
  holdbacks,
- approved side effects through a local mock platform,
- Studio chat, Connect, and run inspection against a prepared app.

## What you will learn

The inventory domain is intentionally small. The important pattern is how an
agent can act on a large set of rows without copying every row through the
model context.

The flow is:

1. The planner discovers product-selection logic with catalog tools and
   exploratory read-only SQL.
2. The planner calls `resolveProductSet`, which runs the final SQL query and
   stores the full product ids as an `InventoryProductSet` reference.
3. The planner calls `storePlan` with compact actions that carry reference
   handles.
4. The coordinator hands execution off by plan id.
5. The executor calls `executePlan`.
6. The runtime hydrates referenced product sets outside the model context.
7. The Python action dispatcher applies approved actions through the mock
   platform.

This is the reference/glimpse pattern in a realistic data-agent workflow.

## Main concepts

This example keeps three concerns separate:

- **Reasoning:** agents inspect catalog metadata, write SQL, and draft plans.
- **Runtime state:** the runtime stores plans, approvals, and large referenced
  values that should not be copied through prompts.
- **Side effects:** approved actions are applied through a narrow Python
  dispatcher instead of through arbitrary model-generated code.

| Concept           | Where to look                   | What it means in this example                                                                                                          |
| ----------------- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime spec      | `inventory_scenario/runtime.py` | Declares the tenant, agents, plans, references, providers, catalog toolkit usage, and action dispatcher binding.                       |
| Agent topology    | `runtime.py`, `prompts.py`      | A coordinator routes work to a planner, executor, or read-only explorer specialist. Each role has a narrow job.                        |
| Data environment  | `support/data_environment.py`   | Describes local `target.db`, `catalog.db`, `kv.db`, and `catalog-index/` so catalog tools and runtime services know where state lives. |
| Catalog toolkit   | Built-in toolkit                | Lets agents discover schemas, profiles, knowledge documents, and sample data without mutating the target database.                     |
| Reference         | `plans.py`, `product_sets.py`   | A typed handle to a large value. `InventoryProductSet` stores selected product ids plus audit metadata.                                |
| Glimpse           | `plans.py`                      | A compact preview returned with a reference so the model can reason without seeing every selected product id.                          |
| Plan              | `plans.py`                      | A typed `InventoryScenarioPlan` containing small business actions and references, not raw product rows.                                |
| Approval boundary | `runtime.py`                    | Plans require approval before execution. The model can propose actions, but side effects happen only after approval.                   |
| Action dispatcher | `action_dispatcher.py`          | Python code receives hydrated references and calls the mock platform with concrete product ids.                                        |

## Architecture

```text
Studio or runtime
  -> coordinator
      -> planner
      -> explorer
      -> executor

planner and explorer
  -> catalog toolkit
      -> target.db          read-only inventory data
      -> catalog.db         schema/profile store
      -> kv.db              knowledge store
      -> catalog-index/     local search index

planner
  -> resolveProductSet
      -> final read-only SQL
      -> target.db
      -> runtime reference registry
      -> InventoryProductSet payload
      -> handle + compact glimpse
  -> storePlan
      -> InventoryScenarioPlan
      -> plan approval

executor
  -> executePlan(plan id)
      -> runtime hydrates references outside model context
      -> Python action dispatcher
      -> platform client
      -> platform.db mutable operational state
      -> optional FastAPI mock platform
```

The example has three important boundaries:

- **Analytical state:** `target.db` is the read-only inventory dataset. Catalog
  tools read it alongside `catalog.db`, `kv.db`, and `catalog-index/` so agents
  can inspect schemas, profiles, knowledge, and sample rows without mutating
  business state.
- **Runtime-owned references:** `resolveProductSet` runs the final read-only SQL
  against `target.db`, stores the complete product-id list as an
  `InventoryProductSet` reference, and returns only a reference handle plus a
  compact glimpse.
- **Operational state:** `platform.db` is the mutable mock platform. During
  `executePlan`, the runtime hydrates references outside the model context and
  passes full product ids to the Python action dispatcher.

## Why references matter here

Plan actions intentionally stay small. They carry business parameters plus a
reference handle, not the full product list:

```json
{
  "kind": "reorder_products",
  "name": "Reorder online low-stock products",
  "quantity": 25,
  "reason": "Products are below reorder point.",
  "references": [
    { "kind": "InventoryProductSet", "id": "ref-low-stock-online" }
  ]
}
```

The model sees a compact glimpse, such as product count and sample ids. The
deterministic Python dispatcher receives the full hydrated product list after
approval. That keeps prompts small while still giving the final write path the
complete data it needs.

## Implement the harness app

The complete source lives in `examples/inventory_scenario/`. This section
focuses on the harness-facing code: app exposure, contracts, agent assembly,
reference creation, and approved action execution. The seed scripts, artifact
download, mock-platform API, and CSS/HTML are support code, so they are covered
by the example README instead of repeated here.

### Expose the app import target

Studio loads the example through `inventory_scenario.app:runtime`. The app
target builds the local data environment, exposes a static runtime spec for
Studio metadata, and gives Studio a factory that creates the executable runtime:

```python
from flowai_harness import define_app

from inventory_scenario.runtime import build_runtime, build_runtime_spec
from inventory_scenario.support.data_environment import (
    build_data_environment,
    default_data_root,
)


def runtime():
    """Studio/MCP import target for the prepared local inventory scenario."""

    data_environment = build_data_environment(default_data_root())
    return define_app(
        name="inventory-scenario",
        description="Inventory scenario planning example with local catalog and mock platform.",
        runtime_spec=build_runtime_spec(),
        runtime_factory=lambda: build_runtime(data_environment=data_environment),
        data_environment=data_environment,
    )
```

The important harness detail is the split between `runtime_spec` and
`runtime_factory`. The spec describes the tenant, agents, plans, and references.
The factory binds the concrete local SQLite files and platform client used by a
running session.

### Define the reference contract

Product selections can be large, so the planner does not put every selected
product id into a plan. `plans.py` defines a typed reference payload and a
compact glimpse:

```python
class ProductSetPayload(DomainModel):
    product_ids: list[str]
    sql: str = Field(min_length=1)
    params: list[str | int | float | bool | None] = Field(default_factory=list)
    reason: str = Field(min_length=1)
    selection_summary: str | None = None
    sample: list[dict[str, Any]] = Field(default_factory=list)


def _product_set_glimpse(value: ProductSetPayload) -> dict[str, Any]:
    return {
        "productCount": len(value.product_ids),
        "previewProductIds": value.product_ids[:3],
        "selectionSummary": value.selection_summary,
        "sample": value.sample[:3],
    }


ProductSet = define_reference(
    name="InventoryProductSet",
    schema=ProductSetPayload,
    ttl_ms=60 * 60 * 1000,
    glimpse=_product_set_glimpse,
)
```

`define_reference(...)` registers a runtime-owned value type. When a tool
creates an `InventoryProductSet`, the runtime stores the full payload and
returns a handle plus the glimpse. The model can reason with the glimpse, while
the executor later receives the full hydrated payload.

### Define the plan contract

The plan schema keeps actions small and explicit. Each action carries one
`InventoryProductSet` reference instead of a copied product list:

```python
class ProductSetRef(DomainModel):
    kind: Literal["InventoryProductSet"] = "InventoryProductSet"
    id: str = Field(min_length=1)


class InventoryActionBase(DomainModel):
    name: str = Field(min_length=1)
    reason: str = Field(min_length=1)
    references: list[ProductSetRef] = Field(min_length=1, max_length=1)


class ReorderProductsAction(InventoryActionBase):
    kind: Literal["reorder_products"] = "reorder_products"
    quantity: int = Field(gt=0)


class HoldInventoryAction(InventoryActionBase):
    kind: Literal["hold_inventory"] = "hold_inventory"
    holdback_units: int = Field(ge=0)


InventoryScenarioAction = TaggedUnion(
    ReorderProductsAction,
    HoldInventoryAction,
)


class InventoryScenarioPlan(DomainModel):
    objective: str = Field(min_length=1)
    actions: list[InventoryScenarioAction] = Field(min_length=1)
    assumptions: list[str] = Field(default_factory=list)
```

Then `define_plan(...)` registers that Pydantic contract with the harness:

```python
inventory_scenario_plan = define_plan(
    "InventoryScenarioPlan",
    InventoryScenarioPlan,
    display_aliases={
        "draft": "Draft inventory plan",
        "approved": "Approved inventory plan",
        "executing": "Applying inventory actions",
        "executed": "Inventory plan applied",
        "failed": "Inventory plan failed",
    },
)
```

The planner must call `storePlan` with this shape. The executor later calls
`executePlan(planId)`, and the runtime validates and hydrates the stored actions
before dispatch.

### Teach the agents the protocol

The prompts are harness code too: they tell each role which tools and contracts
to use. The planner prompt is explicit about the reference-backed planning
lifecycle:

```python
_PLANNER_OPERATIONAL_RULES = """Your goal is to understand the user intent using catalog information and knowledge, resolve the required product sets, identify required actions and then propose a plan for execution.

Follow this high-level search workflow:
1. Use catalog tools for search and discovery
2. Once you've understood the user intent, the required actions and the product sets involve, resolve them with an accurate sql query
3. Store a plan with the required actions and references.

Use catalog tools and exploratory read-only SQL to identify the product set. Then call resolveProductSet once per product set. SQL is the authoritative product selection; filters are audit metadata only.

When calling storePlan:
- Use specName exactly `InventoryScenarioPlan`; do not use aliases such as `inventory`.
- The body must include `objective`, `actions`, and optionally `assumptions`.
- Each action must include `kind`, `name`, `reason`, and `references`.
- Valid action kinds are `reorder_products` with `quantity` and `hold_inventory` with `holdbackUnits`.
- Each action must reference resolved products as `references: [{"kind": "InventoryProductSet", "id": "<id from resolveProductSet>"}]`.
- Do not add any other product-selection fields to plan actions; the `references` array is the only allowed product selection link."""
```

The planner prompt also includes a concrete `storePlan` output shape generated
from the same `InventoryScenarioPlan` model used by `define_plan(...)`:

```python
PLAN_OUTPUT_FORMAT = {
    "tool": "storePlan",
    "args": {
        "specName": inventory_scenario_plan.name,
        "planId": "meaningful-unique-plan-id",
        "body": InventoryScenarioPlan.model_validate(
            {
                "objective": "Hold back inventory for a resolved product set.",
                "actions": [
                    {
                        "kind": "hold_inventory",
                        "name": "Hold inventory for selected products",
                        "holdbackUnits": 20,
                        "reason": "Reserve units for the requested inventory scenario.",
                        "references": [
                            {
                                "kind": "InventoryProductSet",
                                "id": "reference-id-from-resolveProductSet",
                            }
                        ],
                    }
                ],
                "assumptions": ["Use the current inventory snapshot."],
            }
        ).model_dump(mode="json"),
    },
}
```

The executor prompt is intentionally narrower. It tells the executor to use the
runtime's plan execution path instead of manually inspecting or reconstructing
product ids:

```python
_EXECUTOR_OPERATIONAL_RULES = """Your goal is to execute the tasks defined in the plan on the platform.

Call executePlan with the approved plan id. Do not resolve product ids manually; executePlan hydrates references outside the model context. Report the execution result and summarize hydrated references at a high level."""
```

Finally, `layered_prompt(...)` packages those instructions with the planner's
tool definitions. The same prompt module is imported by `runtime.py` when the
agents are assembled:

```python
PLANNER_PROMPT = layered_prompt(
    identity=_PLANNER_IDENTITY,
    communication=_COMMUNICATION_RULES,
    operational_rules=_PLANNER_OPERATIONAL_RULES,
    tools=PLANNER_TOOLS,
    domain_knowledge=_PLANNER_DOMAIN_KNOWLEDGE,
    output_format={
        "instructions": _PLANNER_OUTPUT_FORMAT,
        "storePlan": PLAN_OUTPUT_FORMAT,
    },
)
```

### Add the product-set resolver tool

`product_sets.py` is the bridge from model-discovered SQL to a runtime
reference. The input schema is intentionally small:

```python
class ResolveProductSetInput(ToolModel):
    sql: str = Field(min_length=1)
    params: list[SQL_PARAM] = Field(default_factory=list)
    reason: str = Field(min_length=1)
    selection_summary: str | None = None
```

The resolver accepts only read-only `SELECT` or `WITH` queries, executes the
query against the target SQLite database, and requires a `product_id` column:

```python
def _read_only_select_sql(sql: str) -> str:
    stripped = sql.strip()
    if stripped.endswith(";"):
        stripped = stripped[:-1].strip()
    if ";" in stripped:
        raise ValueError("resolveProductSet SQL must be a single read-only statement")
    first_token = stripped.split(None, 1)[0].lower() if stripped else ""
    if first_token not in {"select", "with"}:
        raise ValueError("resolveProductSet SQL must be a read-only SELECT or WITH query")
    if _FORBIDDEN_SQL.search(stripped):
        raise ValueError("resolveProductSet SQL must be read-only")
    return stripped
```

After the query runs, the tool creates the reference through the harness context:

```python
async def _resolve_product_set(
    args: dict[str, Any],
    ctx,
    *,
    resolver: _SqliteProductSetQueryResolver,
) -> dict[str, Any]:
    value = ResolveProductSetInput.model_validate(args)
    payload = await resolver.resolve(value)
    ref = await ctx.references.create(ProductSet, payload)
    return {
        "reference": {"kind": ref["kind"], "id": ref["id"]},
        "glimpse": ref["glimpse"],
    }
```

The tool definition is registered with `define_tool(...)` and attached to the
planner. It does not need approval because it only performs read-only selection
and reference storage:

```python
@define_tool(
    name="resolveProductSet",
    description=(
        "Run a read-only SQL product selection, store all product ids as an "
        "InventoryProductSet reference, and return only a handle plus glimpse."
    ),
    input_schema=ResolveProductSetInput,
    approval="never",
)
async def tool(args: dict[str, Any], ctx):
    if resolver is None:
        raise ValueError(
            "resolveProductSet must be bound with a data_environment before use"
        )
    return await _resolve_product_set(args, ctx, resolver=resolver)
```

### Assemble the agents

`runtime.py` declares the multi-agent architecture. The coordinator routes user
requests, the planner can use catalog tools plus `resolveProductSet`, the
executor executes stored plans, and the explorer handles read-only analysis:

```python
def build_runtime_spec(*, data_environment: dict[str, Any] | None = None):
    planner_tools = (
        [resolve_product_set_tool_for_data_environment(data_environment)]
        if data_environment is not None
        else PLANNER_TOOLS
    )
    coordinator = define_coordinator(
        name="coordinator",
        model="claude-opus-4-8",
        routes=["planner", "executor", "explorer"],
        approval={"plans": "always", "tools": "never"},
        prompt=COORDINATOR_PROMPT,
    )
    planner = define_planner(
        name="planner",
        model="claude-opus-4-8",
        plan=inventory_scenario_plan,
        tools=planner_tools,
        toolkits=["catalog"],
        prompt=PLANNER_PROMPT,
        max_turns=50,
    )
    executor = define_executor(
        name="executor",
        model="claude-sonnet-4-6",
        plan=inventory_scenario_plan,
        prompt=EXECUTOR_PROMPT,
    )
    explorer = define_specialist(
        name="explorer",
        model="claude-sonnet-4-6",
        toolkits=["catalog"],
        prompt=SPECIALIST_PROMPT,
    )
```

The same function returns the runtime spec with registered plans, references,
agents, tenant, and model provider:

```python
return define_runtime(
    tenant=define_tenant(TENANT_ID, "v2026-06"),
    agents=[coordinator, planner, executor, explorer],
    references=[ProductSet],
    plans=[inventory_scenario_plan],
    providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
)
```

The `references=[ProductSet]` line is what lets `ctx.references.create(...)`
persist an `InventoryProductSet`, and what lets `executePlan` hydrate that
reference for the dispatcher.

### Bind runtime services and dispatcher

`build_runtime(...)` turns the spec into an executable runtime. This is where
the example binds the local data environment, mock-platform service, and action
dispatcher:

```python
def build_runtime(
    *,
    data_environment: dict[str, Any],
    services: dict[str, Any] | None = None,
    interpreter: str | None = None,
    testing: TestingConfig | None = None,
):
    if services is None:
        platform = default_platform_client(data_environment)
        runtime_services = {"platform": platform}
    else:
        runtime_services = dict(services)
        if "platform" not in runtime_services:
            runtime_services["platform"] = default_platform_client(data_environment)
        platform = runtime_services["platform"]
    kwargs: dict[str, Any] = {
        "data_environment": data_environment,
        "services": runtime_services,
        "action_dispatcher": build_action_dispatcher(platform),
    }
    if testing is None:
        selected_interpreter = interpreter or DEFAULT_RUNTIME_INTERPRETER
        if selected_interpreter == DEFAULT_RUNTIME_INTERPRETER:
            _require_anthropic_api_key()
        kwargs["interpreter"] = selected_interpreter
    elif interpreter is not None:
        kwargs["interpreter"] = interpreter
    if testing is not None:
        kwargs["testing"] = testing
    return create_runtime(build_runtime_spec(data_environment=data_environment), **kwargs)
```

The `interpreter` and `testing` parameters let smoke tests use a scripted or
test runtime without requiring a live model. Studio uses the default live
interpreter.

### Execute approved actions

The dispatcher receives already-hydrated references in `ctx["resolved_refs"]`.
It extracts the product ids from the `InventoryProductSet` reference and calls a
narrow platform client method for each supported action:

```python
def build_action_dispatcher(platform: Any):
    async def dispatch_actions(
        actions: list[dict[str, Any]],
        ctx: dict[str, Any],
    ) -> dict[str, Any]:
        results = []
        for action in actions:
            product_ids = _product_ids_for_action(action, ctx)
            payload = dict(action.get("payload") or {})
            kind = action.get("kind")
            if kind == "reorder_products":
                result = await _maybe_await(
                    platform.replenishment(
                        {
                            "product_ids": product_ids,
                            "quantity": payload["quantity"],
                            "reason": payload["reason"],
                        }
                    )
                )
            elif kind == "hold_inventory":
                result = await _maybe_await(
                    platform.holdback(
                        {
                            "product_ids": product_ids,
                            "holdback_units": payload["holdbackUnits"],
                            "reason": payload["reason"],
                        }
                    )
                )
            else:
                raise ValueError(f"unsupported inventory action kind: {kind}")
            results.append(
                {
                    "kind": kind,
                    "name": payload.get("name"),
                    "productCount": len(product_ids),
                    "result": result,
                }
            )
        return {
            "entitiesAffected": len(results),
            "summary": f"Applied {len(results)} inventory action(s).",
            "details": {"actions": results},
        }

    return dispatch_actions
```

The helper that reads hydrated references is deliberately strict. If the plan
does not contain exactly one product-set reference, or if the runtime did not
hydrate it, execution fails before the platform is called:

```python
def _product_ids_for_action(action: dict[str, Any], ctx: dict[str, Any]) -> list[str]:
    references = action.get("references") or []
    product_refs = [
        ref
        for ref in references
        if isinstance(ref, dict) and ref.get("kind") == "InventoryProductSet"
    ]
    if len(product_refs) != 1:
        raise ValueError(
            "inventory actions must include exactly one InventoryProductSet reference"
        )
    ref_id = product_refs[0].get("id")
    resolved = ((ctx.get("resolved_refs") or {}).get("InventoryProductSet") or {}).get(
        ref_id
    )
    if not isinstance(resolved, dict):
        raise ValueError(f"missing hydrated InventoryProductSet reference: {ref_id}")
    product_ids = resolved.get("productIds") or resolved.get("product_ids")
    if not isinstance(product_ids, list):
        raise ValueError(f"InventoryProductSet {ref_id} did not hydrate product ids")
    return [str(product_id) for product_id in product_ids]
```

That is the core harness pattern in this example: the model proposes typed,
reference-backed actions; the runtime owns plan validation, approval, reference
hydration, and dispatch; Python service code performs the final side effect.

## Run it

From a source checkout:

```bash
./scripts/check-env.sh
./scripts/install.sh
cd examples/inventory_scenario
uv run inventory-scenario seed
uv run inventory-scenario data-env --out .data/inventory_scenario/data-environment.json
```

Seed creates:

```text
.data/inventory_scenario/target.db
.data/inventory_scenario/platform.db
.data/inventory_scenario/artifacts/
.data/inventory_scenario/data-environment.json
```

Then run the Flow AI data operations that own catalog, KV, and search-index
state:

```bash
export INVENTORY_DATA_ENV=.data/inventory_scenario/data-environment.json
PROFILE_TABLES=(
  --table dim_companies
  --table dim_brands
  --table dim_segments
  --table dim_subsegments
  --table dim_coordinates
  --table dim_sales_channels
  --table dim_time_periods
  --table dim_products
  --table dim_inventory
)

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output json \
  data profile estimate --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --schema main --sample-size 1 \
  "${PROFILE_TABLES[@]}"

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output ndjson \
  data profile database --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --schema main --sample-size 1 \
  "${PROFILE_TABLES[@]}" --schema-only

find data/knowledge -type f
uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output ndjson \
  data knowledge ingest --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --local-dir data/knowledge --ext md

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output json \
  data catalog index rebuild --tenant-id inventory_scenario --workspace-id default

uv run inventory-scenario smoke
```

The generated data environment disables search-index write-through. Profiling
and knowledge ingestion write `catalog.db` / `kv.db`; the explicit rebuild
creates or refreshes `catalog-index/` once after those writes. The default
profile scope intentionally excludes the large `fact_scenario` table and
`v_scenario_denormalized` view.

Those commands create:

```text
.data/inventory_scenario/catalog.db
.data/inventory_scenario/kv.db
.data/inventory_scenario/catalog-index/
```

These files are generated local state and should not be committed.

## Studio

After seeding and running the Flow AI data operations, start the optional mock
platform UI/API if you want the runtime to call it over HTTP:

```bash
uv run inventory-scenario platform --host 127.0.0.1 --port 8123
```

Then run Studio:

```bash
export ANTHROPIC_API_KEY
export INVENTORY_SCENARIO_PLATFORM_URL=http://127.0.0.1:8123
uv run flowai-harness dev --app inventory_scenario.app:runtime
```

Use prompts such as:

- "Which product categories have the lowest days of cover by channel?"
- "Draft a replenishment plan for products at stockout risk in online channels."
- "Execute the approved replenishment plan through the mock platform."
- "Show me what changed in the mock platform after execution."

See `examples/inventory_scenario/README.md` for the full
setup, artifact model, mock platform, troubleshooting, and verification
criteria.

## Key files

Start with these files in `examples/inventory_scenario/`:

- `inventory_scenario/app.py` exposes the Studio import target.
- `inventory_scenario/runtime.py` assembles coordinator, planner, executor, and
  explorer agents.
- `inventory_scenario/plans.py` defines typed plan and reference contracts.
- `inventory_scenario/product_sets.py` creates query-backed product-set
  references.
- `inventory_scenario/action_dispatcher.py` applies approved actions to the
  mock platform.
- `inventory_scenario/support/data_environment.py` creates the local
  data-environment descriptor.

## Next steps

- Read [References & glimpses](../concepts/references.md) for the underlying
  handle/glimpse pattern.
- Read [Configure a data environment](../guides/data-environment.md) for the
  runtime data dependencies used here.
- Read [Execute approved actions](../guides/action-dispatcher.md) for the
  dispatcher boundary.
- Use [Studio](../guides/studio.md) to inspect runs, approvals, traces, tests,
  and evals.
