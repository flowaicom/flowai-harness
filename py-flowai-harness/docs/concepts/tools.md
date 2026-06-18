# Tools

Tools are model-callable capabilities exposed to agents.

They are how agents inspect systems, search application data, preview changes,
create references, call read APIs, or delegate work through built-in runtime
capabilities.

For side-effecting plan execution, keep one distinction clear:

```text
Tool = something the model can call
Action dispatcher = host callback used by executePlan to apply approved actions
```

## Why tools exist

Models should not invent access to data or mutate systems directly.

Tools make capabilities explicit. They define what an agent can call, what input
shape the runtime validates, and which calls require approval.

Use tools for:

- looking up application data
- searching or inspecting catalogs
- previewing a change before it becomes a plan action
- creating or resolving references
- running read-only analysis
- routing to another agent

Use the action dispatcher for approved writes from a plan.

## How to define tools

Define Python tools with `define_tool(...)`. The returned `ToolSpec` can be used
as a decorator, then attached to an agent with `tools=[...]`.

```python
from flowai_harness import define_tool


@define_tool(
    name="lookup_order",
    description="Look up an order by id.",
    input_schema={"order_id": str},
    approval="never",
)
async def lookup_order(args, ctx):
    order = await ctx.orders.get(args["order_id"])
    return {
        "orderId": order.id,
        "status": order.status,
    }
```

Tool handlers receive:

- `args`: the validated JSON input from the model
- `ctx`: runtime context for the call

Host services passed to `create_runtime(..., services={...})` are available on
`ctx` by key and attribute. For example, `services={"orders": order_service}`
makes `ctx.orders` and `ctx["orders"]` available to the handler.

```python
runtime = create_runtime(
    runtime_spec,
    services={"orders": order_service},
)
```

Attach the tool to only the agents that should be able to call it:

```python
support_agent = define_specialist(
    name="support_agent",
    model="claude-sonnet-4-6",
    tools=[lookup_order],
    prompt="Answer support questions from order data.",
)
```

## Tool approval

Tool approval controls whether a model-callable tool can run immediately.

Use `approval="never"` for safe lookup, preview, and read-only tools. Use
`approval="always"` or a dynamic approval policy for sensitive model-callable
capabilities, such as sending external messages or running expensive operations.

Plan approval is separate. If a write is part of a typed plan, prefer plan
approval plus an action dispatcher instead of exposing the write API as a direct
model-callable tool.

## Built-in toolkits

Some tools are built into the runtime. You do not define Python handlers for
these. The runtime wires them to internal registries, agent routing, plan
storage, references, catalog metadata, and the target database.

| Toolkit | Tools | How it is attached |
| --- | --- | --- |
| `agents` | `call_agent` | Added for coordinators with `routes=[...]`. |
| `plans` | `storePlan`, `getPlan`, `executePlan` | Added implicitly for planner and executor roles, scoped by role. Can also be selected by other roles for `getPlan`. |
| `references` | `resolveRef`, `glimpseRef` | Added implicitly for executors. Can also be selected with `toolkits=["references"]`. |
| `catalog` | `search_catalog`, `get_catalog_entities`, `list_schema_fields`, `get_catalog_relations`, `get_relation_paths_between`, `sample_table_data`, `execute_query` | Selected explicitly with `toolkits=["catalog"]`; requires a `data_environment`. |

Toolkit tools are still model-callable tools. The difference is ownership:
Python tools are callbacks you define, while built-in toolkit tools are backed
by runtime-owned services.

## Plans and references are role defaults

Planner and executor agents get plan tools automatically from their role.

- Planners get `storePlan` and `getPlan`.
- Executors get `getPlan` and `executePlan`.
- Executors also get `resolveRef` and `glimpseRef`.

You should not define replacement Python tools named `storePlan`, `getPlan`,
`executePlan`, `resolveRef`, or `glimpseRef`. Declare the plan and reference
schemas instead; the runtime provides the tools.

```python
planner = define_planner(
    name="change_planner",
    model="claude-sonnet-4-6",
    plan=change_plan,
    tools=[lookup_order],
    prompt="Create typed change plans.",
)

executor = define_executor(
    name="change_executor",
    model="claude-sonnet-4-6",
    plan=change_plan,
    prompt="Execute approved change plans.",
)
```

In this setup, `lookup_order` is a user-defined tool available to the planner.
`storePlan`, `getPlan`, `executePlan`, `resolveRef`, and `glimpseRef` are
runtime tools supplied by the harness.

## Catalog tools

The `catalog` toolkit is built in, but it is not attached to every agent by
default. Add it to agents that should inspect catalog metadata or run read-only
data queries.

```python
analyst = define_specialist(
    name="data_analyst",
    model="claude-sonnet-4-6",
    toolkits=["catalog"],
    prompt="Use catalog tools to answer data questions.",
)
```

Catalog tools need data dependencies from
`create_runtime(..., data_environment=...)`. The common dependencies are:

- `catalog` for metadata and entity hydration
- `catalog_search` for `search_catalog`
- `target_database` or `target_database_url` for `sample_table_data` and
  `execute_query`

Use catalog tools for discovery, schema inspection, relation context, sampling,
and read-only SQL. They do not perform platform writes.

## Action dispatcher

The action dispatcher is the write boundary for approved plan actions. It is not
a model-callable tool; it is the host callback the runtime invokes after
`executePlan` passes validation and approval.

Use it for API calls that patch or mutate your platform: update a price, apply a
subscription change, create a campaign, approve an invoice, or send a committed
external notification.

See [Action dispatcher](action-dispatcher.md) for the concept and
[Execute approved actions](../guides/action-dispatcher.md) for the
implementation walkthrough.

## Tool results

Return small, structured results when possible.

If the result is large, sensitive, or expensive to place in the prompt, return a
[reference with a glimpse](references.md) instead. The executor can resolve
references later through the built-in reference tools or receive hydrated action
references through the dispatcher context.

## Common mistakes

Do not expose write APIs directly as ordinary tools when the write should be
reviewed as part of a plan. Model the write as a typed plan action and execute it
through the action dispatcher.

Do not define Python versions of built-in plan or reference tools. Declare
plans, references, and toolkits; let the runtime supply those tools.

Do not return large datasets directly from tools. Store the full value behind a
reference and return a small glimpse.

## See also

- [Plans](plans.md) for typed actions and plan lifecycle.
- [Action dispatcher](action-dispatcher.md) for approved plan writes.
- [References & glimpses](references.md) for passing large values safely.
- [Approvals](approvals.md) for gating sensitive tools and plans.
- [Runtime](runtime.md) for how applications provide tool callbacks and
  services.
- [Configure a data environment](../guides/data-environment.md) for catalog
  toolkit dependencies.
- [Execute approved actions](../guides/action-dispatcher.md) for the approved
  write boundary.
- [Expose tools over MCP](../guides/mcp.md) for external MCP clients.
- [`define_tool` reference](../reference/tools.md#flowai_harness.tools.define_tool).
