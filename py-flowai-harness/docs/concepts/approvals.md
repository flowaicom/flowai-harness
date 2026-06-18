# Approvals

Approvals are runtime gates that pause sensitive work until the host application
responds.

They let agents propose actions without immediately executing them.

## Why approvals exist

Agents may need to perform actions that affect real systems.

Examples:

- sending messages
- changing customer data
- updating prices
- launching campaigns
- triggering workflows
- running expensive operations

Approvals make those actions reviewable before they continue.

## What can require approval?

Approval policies have two runtime channels:

- plan execution
- tool calls

Use plan approvals when you want to inspect proposed work before execution.
Use tool approvals when a single capability is sensitive even outside a plan.

For action-specific review, model the work as typed plan actions and inspect
the plan before execution, or put a dynamic predicate on the sensitive tool.

## Approval hierarchy

Approval policy is resolved hierarchically across runtime -> agent -> tool.
The runtime floor sets the default. Agent policies can override that floor for
one agent. Tool policies can override the resolved tool policy for one tool
under one agent.

The most specific matching rule wins. A tool-level `"never"` can intentionally
relax a broader agent or runtime tool gate, and a tool-level `"always"` can
tighten a broader `"never"` floor.

### 1. Runtime floor

By default, the runtime floor is:

```python
{"plans": "always", "tools": "never"}
```

That default means plans pause before execution, while tools run immediately
unless a narrower tool policy says otherwise.

Set the floor explicitly with `define_runtime(...)` when you want the policy to
be obvious at the runtime boundary:

```python
runtime_spec = define_runtime(
    tenant=tenant,
    agents=[coordinator, planner, executor],
    approval_policies={
        "plans": "always",
        "tools": "never",
    },
)
```

### 2. Coordinator approval as an implicit floor

If `define_runtime(..., approval_policies=...)` is omitted, a coordinator
`approval` patch is applied to the runtime defaults and becomes the runtime
floor:

```python
coordinator = define_coordinator(
    name="coordinator",
    model="claude-sonnet-4-6",
    routes=["planner", "executor"],
    approval={"plans": "always", "tools": "never"},
    prompt="Route planning work to the planner and approved work to the executor.",
)
```

This is a convenience for coordinator -> planner -> executor applications. If
you pass `approval_policies` explicitly to `define_runtime(...)`, coordinator
approval is no longer promoted to the runtime floor; it is treated as a normal
coordinator-scoped agent override. Only one coordinator can provide the
implicit floor.

### 3. Agent-level override

Any agent can override the runtime floor for that agent:

```python
executor = define_executor(
    name="executor",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    approval={"plans": "never", "tools": "never"},
    prompt="Execute approved plans.",
)
```

Missing channels inherit from the runtime floor. In the `define_*` helpers,
`"default"` means "do not override this channel":

```python
approval={"plans": "default", "tools": "always"}
```

### 4. Tool definition policy

A Python tool can define its own approval policy:

```python
@define_tool(
    name="post_journal_entry",
    input_schema={"account": str, "amount": float},
    approval="always",
)
async def post_journal_entry(args, ctx):
    return await ctx.ledger.post(args)
```

Python tools default to `approval="never"`. If a bound custom tool should
follow a stricter runtime tool floor, set its approval policy explicitly or add
an agent `tool_approvals` override.

When that tool is bound to an agent, the policy is compiled as a tool override
for that agent's binding. If the same tool is bound to multiple agents, each
binding receives the tool definition's policy unless that agent overrides it.

Tool approval also supports dynamic predicates:

```python
def approve_expensive_writes(args, ctx):
    return args.get("amount", 0) > 10_000


@define_tool(
    name="post_journal_entry",
    input_schema={"account": str, "amount": float},
    approval=approve_expensive_writes,
)
async def post_journal_entry(args, ctx):
    return await ctx.ledger.post(args)
```

You can also reference a registered dynamic predicate by id and pass the
callable to `create_runtime(..., approval_predicates=...)`.

```python
tool = define_tool(
    name="post_journal_entry",
    input_schema={"account": str, "amount": float},
    approval={"kind": "dynamic", "value": "approve_expensive_writes"},
)

runtime = create_runtime(
    runtime_spec,
    approval_predicates={"approve_expensive_writes": approve_expensive_writes},
)
```

### 5. Agent-scoped tool override

An agent can override one specific tool under that agent:

```python
executor = define_executor(
    name="executor",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    approval={"plans": "never", "tools": "never"},
    tool_approvals={"execute_query": "always"},
    prompt="Execute approved plans.",
)
```

Tool overrides are scoped by agent. The same tool can require approval for one
agent and run without approval for another. If both `define_tool(...)` and
`tool_approvals` apply to the same agent and tool, the agent's
`tool_approvals` entry wins.

## Policy values

| Policy | Meaning | Applies to |
| --- | --- | --- |
| `"never"` | Run without approval. | Plans and tools |
| `"always"` | Always pause and emit an approval gate. | Plans and tools |
| `"default"` | Do not override the runtime floor for this channel. | Agent helper `approval` mappings |
| dynamic predicate | Decide per invocation from tool arguments and context. | Tools |

## How approvals fit into execution

1. Planner creates a plan.
2. Runtime detects that approval is required.
3. Runtime emits an `approval-required` event.
4. Host application shows the approval to a human or policy service.
5. Host responds with approve, reject, or revise.
6. Runtime continues or stops execution.

`"revise"` is meaningful for plan approvals. Tool approvals treat revise as a
rejection.

## Approval is not just UI

The UI may display the approval, but the approval is part of runtime execution.

The agent should not continue sensitive work until the approval is resolved.

## Common mistake

Do not rely on prompt instructions alone for sensitive actions. Use runtime
approvals for actions that need a real gate.

## Practical mental model

Use plan approval as the main human boundary:

```python
runtime_spec = define_runtime(
    tenant=tenant,
    agents=[coordinator, planner, executor],
    approval_policies={
        "plans": "always",
        "tools": "never",
    },
)
```

Then gate only the risky tools that are sensitive, expensive, or
side-effecting:

```python
executor = define_executor(
    name="scenario_executor",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    approval={"plans": "never", "tools": "never"},
    tool_approvals={"execute_query": "always"},
    prompt=executor_prompt,
)
```

In that shape, plans are normally reviewed before execution, while tools are
selectively gated only when their direct invocation needs an additional
approval boundary.

## See also

- [Plans](plans.md) for plan approval before execution.
- [Tools](tools.md) for approval-gated capabilities.
- [Runtime](runtime.md) for responding to approval events.
- [Require approvals](../guides/approvals.md) for policy setup and responses.
- [Execute approved actions](../guides/action-dispatcher.md) for applying
  approved plan actions.
