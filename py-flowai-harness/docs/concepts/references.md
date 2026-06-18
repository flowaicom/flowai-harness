# References & glimpses

Agents frequently need to pass around large, expensive, or sensitive data
between tools, planner agents, executor agents, action dispatchers, and future
turns.

A reference keeps the full payload outside the prompt and gives the agent a
typed handle. A glimpse gives the agent a small, safe summary of what the
handle points to.

Together they let agents reason over lightweight summaries most of the time and
only hydrate the full payload when a tool, executor, or host application truly
needs it.

## The core problem

Without references, large values get copied through every agent hop. A tool may
return 10,000 product ids, then a planner, executor, and later tool call may all
copy the same ids into prompt context again.

This creates four problems.

| Problem | Why it matters |
| --- | --- |
| Context bloat | Large payloads consume tokens, slow down prompts, increase cost, and leave less room for reasoning. |
| Repeated serialization | The same dataset may be embedded many times across planner, executor, tool, and future-turn boundaries. |
| Sensitive data leakage | Customer data can enter prompts unnecessarily, and every agent that touches the workflow may see the full payload. |
| Poor planning efficiency | Agents often need to know what a dataset contains before deciding whether to fetch it. Loading the full value just to inspect it is wasteful. |

References solve the copying problem. Glimpses solve the reasoning problem.

## Reference

A reference is a typed pointer to a stored value.

The runtime stores the full value once and returns a `{kind, id}` handle.
Agents can pass the handle through tool results, plans, approvals, executor
prompts, and later turns without embedding the full payload.

Reference ids are tenant-scoped and content-addressed by the stored value. The
same payload in the same tenant resolves to the same stable handle, while
different tenants remain isolated.

## Glimpse

A pure pointer is compact, but it is not enough for planning. The agent cannot
tell whether the reference points to three products, 30,000 products,
enterprise accounts, an empty result, or a sensitive segment.

A glimpse is the small summary stored beside the reference, such as a count,
short preview, segment flag, or aggregate.

The glimpse is metadata for reasoning. It should help the agent decide whether
to continue with the handle, ask for clarification, create a plan action, or
resolve the full value.

For example, the agent can decide to create a plan over a large product set
without loading every id, or decide to resolve the reference because a later
calculation needs detailed product attributes.

Keep glimpses small and safe. Do not put secrets, full customer records, or the
same large payload into the glimpse.

## The design pattern

The pattern is similar to database access:

| Database | Flow AI |
| --- | --- |
| Row id | Reference |
| Query planner statistics | Glimpse |
| Full row fetch | `resolveRef` |

The key rule is: reason over the glimpse, pass the reference, and resolve only
when needed.

This keeps prompts small, reduces cost, improves privacy, and lets workflows
operate over datasets that would not fit in an LLM context window.

## Define a reference type

Declare reference types in the runtime spec with `define_reference(...)`.

`schema` describes the full payload. `glimpse` is a Python callback that derives
the compact summary before the value is stored. `ttl_ms` can expire stored
payloads when references should be temporary.

For the implementation steps, see
[Work with references and glimpses](../guides/references-and-glimpses.md).

## Create references from tools

Pointer-producing tools should return the reference handle and its glimpse, not
the full payload.

The model sees enough to plan from the handle and glimpse. The full list stays
in reference storage.

For tool code, see
[Create references inside tools](../guides/references-and-glimpses.md#create-references-inside-tools).

## Use references in plans

Plans should carry reference handles when an action depends on a large or
sensitive payload.

The planner can create a compact, reviewable action without copying every
product id into the plan. The executor can call `executePlan` with the plan id.
When the action dispatcher runs, the runtime can hydrate referenced values
outside the model context and pass them to host code.

## Resolve only when needed

Resolving a reference loads the full value.

Executors get `resolveRef` and `glimpseRef` by default. Other agents can select
the `references` toolkit when they need those tools explicitly.

Use `glimpseRef` when the agent only needs to inspect the summary. Use
`resolveRef` when the full payload is required for a calculation, detailed
answer, or tool call.

Host code can also create, resolve, and inspect references through runtime
methods. In many flows, the model never needs the full payload. The host
application or action dispatcher can hydrate it at the boundary where concrete
side effects or API calls happen.

## When to use references

Use references for:

- large query results
- customer records or sensitive datasets
- product sets, account sets, cohorts, or segments
- documents, reports, or generated artifacts
- intermediate datasets shared across multiple agent steps
- payloads that a planner should mention but an executor or dispatcher should
  hydrate later

Do not use references for tiny scalar values that are already safe and useful
in context, such as a single status string or a short id.

## Common mistakes

- Returning a large payload directly from a tool because the model might need it
  later.
- Putting the full payload into the glimpse.
- Creating a reference but omitting the glimpse, leaving the agent unable to
  reason about the handle.
- Resolving references inside the model loop before the full value is actually
  needed.
- Copying referenced data into plan actions instead of storing only the
  reference handle.
- Treating references as an approval boundary. References reduce prompt
  exposure, but sensitive actions still need plan or tool approvals.

## See also

- [Work with references and glimpses](../guides/references-and-glimpses.md)
  for implementation examples.
- [Tools](tools.md) for returning references from tool handlers.
- [Plans](plans.md) for compact actions that carry reference handles.
- [Action dispatcher](action-dispatcher.md) for hydrated references at the
  execution boundary.
- [Runtime](runtime.md) for host-side reference creation and resolution.
- [Configure a data environment](../guides/data-environment.md) for data
  dependencies used by built-in tools.
- [Knowledge and documents](../guides/knowledge.md) for retrieval-backed
  context.
- [`define_reference` reference](../reference/references.md#flowai_harness.references.define_reference).
- [`glimpse` helper reference](../reference/glimpse.md).
