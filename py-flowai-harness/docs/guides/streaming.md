# Streaming events

Runtime events let your application observe agent execution as it happens.

Use this guide when you want to stream progress from a Flow AI run into your own
application UI, backend, Server-Sent Events endpoint, or WebSocket layer.

## When to use this guide

Use streaming when your application needs to:

- show incremental model text
- display tool calls and tool results
- pause and resume around approvals
- surface errors and completion
- persist a run trace for later inspection

By the end of this guide, your application should read events until `finish` or
`error` and handle approval pauses explicitly.

## Basic event loop

`runtime.query(prompt, thread_id=...)` and
`runtime.run_specialist(agent, prompt, thread_id=...)` both return async
iterators.

```python
async for event in runtime.query("Draft a pricing scenario.", thread_id="thread-1"):
    kind = event["type"]

    if kind == "text":
        print(event["text"], end="")
    elif kind == "tool-invocation":
        print(f"\n[{event['state']} {event['toolName']}]")
    elif kind == "finish":
        print("\nDone")
    elif kind == "error":
        raise RuntimeError(event["error"]["message"])
```

Dispatch by `event["type"]`. Do not assume text arrives before tools; tool-first
steps are valid.

## Adapting events to your app

Most applications translate runtime events into their own transport shape.

```python
async def app_events(runtime, prompt, thread_id):
    async for event in runtime.query(prompt, thread_id=thread_id):
        kind = event["type"]

        if kind == "text":
            yield {"event": "message.delta", "text": event["text"]}
        elif kind == "tool-invocation":
            yield {
                "event": "tool",
                "state": event["state"],
                "name": event["toolName"],
            }
        elif kind == "approval-required":
            yield {
                "event": "approval.required",
                "approval_id": event["data"]["id"],
                "target": event["data"]["target"],
            }
        elif kind == "finish":
            yield {"event": "run.finished", "usage": event.get("usage")}
```

Keep unknown event types non-fatal. Forward them to your trace store or ignore
them so product-specific events can pass through.

## Handling approval events

When the runtime emits `approval-required`, the stream is intentionally paused.
Your application should show the request to a human reviewer or policy service,
then respond with `runtime.respond_to_approval(...)`.

```python
async for event in runtime.query(prompt, thread_id="thread-1"):
    if event["type"] == "approval-required":
        approval_id = event["data"]["id"]
        decision = await ask_reviewer(event["data"])
        await runtime.respond_to_approval(
            approval_id,
            decision,
            feedback="reviewed in host app",
        )
```

See [Require approvals](approvals.md) for approval configuration and outcomes.

## Handling errors and completion

Read until a terminal event:

- `finish` means the run completed successfully.
- `error` means the run failed and no further events follow.

Cost, latency, and usage summaries may arrive near the end of the stream. If
your application needs those values, keep consuming until the stream is closed.

## Verify it works

Use the deterministic testing or scripted interpreter while developing stream
handling:

```python
events = []

async for event in runtime.query("hello", thread_id="thread-1"):
    events.append(event["type"])

assert "finish" in events or "error" in events
```

For approval flows, assert that `approval-required` appears before the gated
tool result or action dispatcher result.

## Common errors

| Symptom | Explanation |
| --- | --- |
| Text never appears before a tool call | Tool-first steps are valid. Dispatch by event type. |
| Stream stops at `approval-required` | Send `runtime.respond_to_approval(...)`; the runtime is waiting on purpose. |
| Usage metadata is missing mid-stream | Read through completion; aggregate summaries arrive at the end. |
| Unknown events break the UI | Preserve or ignore unknown event types instead of treating them as fatal. |

## See also

- [Runtime events reference](../reference/runtime-events.md)
- [Require approvals](approvals.md)
- [Test agents without provider calls](testing.md)
- [`Runtime` reference](../reference/runtime.md#flowai_harness.runtime.Runtime)
