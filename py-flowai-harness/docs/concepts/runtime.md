# Runtime

The runtime turns your Flow AI definitions into an executable agent system.

You define tenants, agents, plans, tools, references, and prompts. The runtime
validates those definitions, runs the agent loop, routes work between agents,
manages plans and approvals, and streams events back to your application.

## RuntimeSpec vs Runtime

A `RuntimeSpec` is the description of the system.

A `Runtime` is the running handle created from that spec.

```python
from flowai_harness import create_runtime, define_runtime, define_tenant

runtime_spec = define_runtime(
    tenant=define_tenant("acme", "v1"),
    agents=[coordinator, planner, executor],
    references=[ProductSet],
)

runtime = create_runtime(runtime_spec)
```

## What the runtime owns

- agent routing
- plan lifecycle
- approval gates
- reference handling
- tool dispatch
- event streaming
- execution state

## What your application owns

- tool callbacks
- provider credentials
- domain services
- UI
- persistence choices
- approval responses

## How applications interact with the runtime

Applications usually:

- start a run
- stream events
- display approvals
- respond to approvals
- inspect results and traces
- create or resolve references when host code needs the full value

```python
async for event in runtime.query("Draft a pricing scenario.", thread_id="thread-1"):
    if event["type"] == "approval-required":
        await runtime.respond_to_approval(event["data"]["id"], "approve")
```

## Common mistake

Do not treat the runtime as just a model wrapper. It is the execution boundary
for the full agent system.

## See also

- [Multi-agent architectures](execution-model.md) for common agent layouts.
- [Approvals](approvals.md) for runtime gates.
- [Reference API](../reference/runtime.md) for the full runtime surface.
- [Streaming events](../guides/streaming.md) for consuming runtime output.
- [Test agents without provider calls](../guides/testing.md) for deterministic
  runtime checks.
