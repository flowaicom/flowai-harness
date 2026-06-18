# Work with references and glimpses

Use references when agents need to pass large, sensitive, or intermediate data
between tools, plans, executors, and host code without copying the full payload
into the prompt.

A reference is the pointer. A glimpse is the small planner-facing summary that
travels with the pointer. For the conceptual background, start with
[References & glimpses](../concepts/references.md).

By the end of this guide, you should have a typed reference spec, code that
creates references from host code and tools, and prompt rules that tell agents
when to use `glimpseRef` or `resolveRef`.

## Workflow

The core workflow is:

```text
Payload = real data
Reference = pointer to payload
Glimpse = small summary for reasoning
resolveRef = fetch full payload only when needed
```

Agents should reason from glimpses, pass references through plans and tool
results, and resolve only at the point where the full payload is genuinely
needed.

## Define a reference type

Reference types are declared with `define_reference(...)`.

```python
from pydantic import BaseModel

from flowai_harness import define_reference


class ProductSetPayload(BaseModel):
    product_ids: list[str]


ProductSet = define_reference(
    name="ProductSet",
    schema=ProductSetPayload,
    ttl_ms=60 * 60 * 1000,
    glimpse=lambda value: {
        "productCount": len(value.product_ids),
        "preview": value.product_ids[:3],
    },
)
```

This creates a typed, TTL-bounded reference spec. The full payload stays outside
the prompt. The agent workflow receives the handle:

```json
{
  "kind": "ProductSet",
  "id": "..."
}
```

and the glimpse:

```json
{
  "productCount": 3,
  "preview": ["sku-1", "sku-2", "sku-3"]
}
```

Register the reference type in the runtime spec:

```python
runtime_spec = define_runtime(
    tenant=tenant,
    agents=[planner, executor],
    references=[ProductSet],
)
```

`ttl_ms` is optional. Use it when referenced payloads should expire, such as
temporary search results or generated previews.

## Create references from host code

Host code can create references before starting or continuing an agent workflow.

```python
payload = ProductSetPayload(product_ids=["sku-1", "sku-2", "sku-3"])

ref = await runtime.create_reference(ProductSet, payload)
```

The returned value includes the reference handle and the stored glimpse:

```json
{
  "kind": "ProductSet",
  "id": "...",
  "glimpse": {
    "productCount": 3,
    "preview": ["sku-1", "sku-2", "sku-3"]
  }
}
```

Pass only `{kind, id}` when a plan action, tool argument, or host workflow needs
the pointer. Include the glimpse when the agent needs to reason about what the
pointer represents.

## Create references inside tools

Pointer-producing tools should store the full payload and return a handle plus a
glimpse. This is the most common pattern for search, selection, query, and
preview tools.

```python
from flowai_harness import define_tool


@define_tool("searchProducts", {"query": str}, approval="never")
async def search_products(args, ctx):
    products = await ctx.product_catalog.search_products(args["query"])

    payload = ProductSetPayload(
        product_ids=[product["id"] for product in products]
    )

    ref = await ctx.references.create(ProductSet, payload)

    return {
        "productSetRef": {
            "kind": ref["kind"],
            "id": ref["id"],
        },
        "glimpse": ref["glimpse"],
    }
```

The model sees enough to plan:

```json
{
  "productSetRef": {
    "kind": "ProductSet",
    "id": "..."
  },
  "glimpse": {
    "productCount": 3200,
    "preview": ["sku-1", "sku-2", "sku-3"]
  }
}
```

The full product list remains in runtime-owned reference storage.

## Tell agents how to use references

Put a compact rule in the relevant planner, executor, or specialist prompt:

```text
Have glimpse? Reason from glimpse.
Need compact preview later? Call glimpseRef.
Need full payload? Call resolveRef.
Executing an approved plan? References are normally hydrated automatically.
```

With that rule, an agent can say:

```text
I found 3,200 matching products. I will filter by enterprise segment before
resolving the full set.
```

instead of loading all 3,200 product ids into context.

Executors get `resolveRef` and `glimpseRef` by default. Other agents can select
the built-in reference toolkit when they need those tools directly:

```python
analyst = define_specialist(
    name="analyst",
    model="claude-sonnet-4-6",
    prompt="Use references for large result sets.",
    toolkits=["references"],
)
```

## Design domain-specific glimpses

The glimpse should contain the smallest set of fields the agent needs for
planning decisions.

```python
ProductSet = define_reference(
    name="ProductSet",
    schema=ProductSetPayload,
    glimpse=lambda value: {
        "productCount": len(value.product_ids),
        "preview": value.product_ids[:3],
        "hasEnterpriseSegment": any(
            pid.startswith("ENT-") for pid in value.product_ids
        ),
    },
)
```

Think of the glimpse as planner-facing metadata. Good glimpse fields answer
questions such as:

- how large is the payload?
- is the payload empty?
- what are a few representative ids or labels?
- does it contain a segment, status, risk flag, or aggregate the plan depends on?

Keep glimpses small and safe. Do not include secrets, full records, long lists,
or the original payload serialized under another key.

## Use references in plans

Plan actions should carry reference handles when the action depends on a large
or sensitive payload.

```json
{
  "kind": "discount_products",
  "reason": "Apply a promotion to the selected enterprise products.",
  "references": [
    {
      "kind": "ProductSet",
      "id": "..."
    }
  ]
}
```

The planner stores a compact action. The executor calls `executePlan`. When an
approved action reaches the action dispatcher, the runtime hydrates action
references outside the model context and passes them through dispatcher `ctx`.

## Verify it works

Check the behavior in a scripted or local runtime:

- tool results include `{kind, id}` plus a small `glimpse`
- large payload fields are absent from tool results and plan actions
- `runtime.reference_glimpse(ref)` returns the cached glimpse
- `runtime.resolve_reference(ref)` returns the full payload only when host code
  explicitly asks for it
- approved plan dispatch receives hydrated references through dispatcher `ctx`

## Common errors

| Error | Fix |
| --- | --- |
| Returning the full payload from the search tool | Store the payload with `ctx.references.create(...)` and return only the handle plus glimpse. |
| Putting the whole payload in `glimpse` | Keep the glimpse to counts, previews, booleans, labels, and aggregates. |
| The agent cannot use `resolveRef` | Executors get reference tools by default; specialists need `toolkits=["references"]`. |
| Reference creation fails | Ensure the reference spec is registered in `define_runtime(..., references=[...])`. |
| A plan contains product ids instead of a reference | Put `{kind, id}` under the action `references` field and let execution hydrate later. |

## See also

- [References & glimpses concept](../concepts/references.md)
- [Tools concept](../concepts/tools.md)
- [Plans concept](../concepts/plans.md)
- [Action dispatcher guide](action-dispatcher.md)
- [`define_reference` reference](../reference/references.md#flowai_harness.references.define_reference)
- [Runtime reference](../reference/runtime.md)
