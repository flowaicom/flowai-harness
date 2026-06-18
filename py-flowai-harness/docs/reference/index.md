# Reference

API reference for the `flowai_harness` public surface. Most pages render the
exported classes, functions, and type aliases directly from their docstrings;
some pages add contract notes where the raw generated signature needs more
context.

Common entry points:

| Task | Start here |
| --- | --- |
| Define a tenant and runtime spec | [Tenant](tenant.md), [Runtime](runtime.md), [Agents](agents.md) |
| Add typed plans, references, tools, and prompts | [Plans](plans.md), [References](references.md), [Tools](tools.md), [Prompts](prompts.md) |
| Run or stream the runtime | [Runtime](runtime.md), [Runtime events](runtime-events.md), [Action dispatcher](action-dispatcher.md) |
| Author and score evals | [Evals](evals.md) |
| Serve tools over MCP | [MCP](mcp.md) |
| Serve local Studio | [Studio](studio.md) |
| Normalize schemas and data environments | [Schema and utilities](schema-utilities.md) |

For task-oriented guides see [Require approvals](../guides/approvals.md),
[Configure a data environment](../guides/data-environment.md),
[Studio](../guides/studio.md),
[Expose Tools Over MCP](../guides/mcp.md),
[Test agents without provider calls](../guides/testing.md), and
[Streaming events](../guides/streaming.md).

Besides `dev` / `serve` (see [Studio](studio.md)), the CLI also provides
`flowai-harness docs export` to generate the docs content artifact and
`flowai-harness mcp python MODULE:OBJECT` to serve a Python runtime object
over MCP (see [MCP](mcp.md)).

- [Runtime](runtime.md) — `Runtime`, `create_runtime`, `define_runtime`, `RuntimeSpec`, and the testing / data-environment configs.
- [Studio](studio.md) — `define_app`, workspace bindings, and the local Studio command surface.
- [Evals](evals.md) — eval configs, test cases, structured action ground truth, scorer presets, artifacts, and event envelopes.
- [MCP](mcp.md) — stdio and Streamable HTTP helpers for exposing runtime tools to MCP clients.
- [Agents](agents.md) — `define_coordinator`, `define_planner`, `define_executor`, `define_specialist`, and `AgentSpec`.
- [Plans](plans.md) — `define_plan`, `PlanSpec`, `PlanDisplayAlias`.
- [References](references.md) — `define_reference`, `ReferenceSpec`.
- [Tools](tools.md) — `define_tool`, `ToolSpec`.
- [Prompts](prompts.md) — `layered_prompt`, `LayeredPrompt`.
- [Tenant](tenant.md) — `define_tenant`, `TenantIdentity`.
- [Unions](unions.md) — `TaggedUnion`.
- [Glimpse](glimpse.md) — `glimpse`.
- [Runtime events](runtime-events.md) — stream event kinds and payload examples.
- [Action dispatcher](action-dispatcher.md) — approved plan action callback contract.
- [Schema and utilities](schema-utilities.md) — `normalize_schema` and `normalize_data_environment`.
