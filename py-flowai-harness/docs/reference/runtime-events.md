# Runtime events

Runtime events are the observable stream emitted while a run is happening.
They let an application or Studio UI render text, reasoning, tool calls,
sub-agent handoffs, approval gates, plan status changes, traces, usage, and
errors without waiting for the whole run to finish.

`runtime.query(...)` and `runtime.run_specialist(...)` return async iterators
over these event dictionaries. Applications commonly forward them to their own
transport, such as SSE or WebSockets, and use the `type` field to decide how to
render each event.

For implementation guidance, see [Streaming events](../guides/streaming.md).

## Common event kinds

### `text`

Incremental text token.

```python
{"type": "text", "text": "Hello"}
```

### `reasoning`

Reasoning text, emitted only by models that surface it.

```python
{"type": "reasoning", "text": "..."}
```

### `step-start`

Boundary marker before output for one logical step.

```python
{"type": "step-start"}
```

### `tool-invocation`

Emitted for tool calls and results.

```python
{
    "type": "tool-invocation",
    "toolInvocationId": "scripted-tool-1",
    "toolName": "echo",
    "args": {"value": "hello"},
    "state": "call",
}
```

```python
{
    "type": "tool-invocation",
    "toolInvocationId": "scripted-tool-1",
    "toolName": "echo",
    "args": {"value": "hello"},
    "state": "result",
    "result": {"echo": "hello"},
}
```

### `tool-progress`

Progress milestone for a long-running tool.

```python
{
    "type": "tool-progress",
    "toolName": "buildPlan",
    "label": "Resolving products",
    "phaseIndex": 1,
    "totalPhases": 4,
    "milestone": {"matched": 142},
}
```

### `tool-agent`

Sub-agent invocation event, emitted in call/result pairs.

```python
{
    "type": "tool-agent",
    "agentName": "planner",
    "state": "call",
}
```

### `data-tool-agent`

Sub-agent completion with usage metrics.

```python
{
    "type": "data-tool-agent",
    "data": {"agentName": "planner", "model": "claude-haiku-4-5", "usage": {}},
}
```

### `approval-required`

The runtime is waiting for a host decision.

```python
{
    "type": "approval-required",
    "data": {
        "id": "apr-1234",
        "kind": "plan",
        "target": "demo-plan-1",
        "payload": {},
        "resourceId": "acme",
        "threadId": "thread-1",
    },
}
```

For tool approvals, `kind` is `"tool"` and `target` is the tool name.

### `approval-decision`

Emitted after `runtime.respond_to_approval(...)` is processed.

```python
{
    "type": "approval-decision",
    "data": {
        "id": "apr-1234",
        "outcome": {"outcome": "approve"},
        "feedback": "approved by smoke test",
    },
}
```

### `plan-status-change`

Plan lifecycle transition.

```python
{
    "type": "plan-status-change",
    "data": {"planId": "demo-plan-1", "from": "draft", "to": "pending_approval"},
}
```

### `data-file-registered`

A file produced by a tool becomes available for download.

```python
{"type": "data-file-registered", "data": {}}
```

### `data-cost-summary` and `data-latency-summary`

Aggregated cost and latency metrics for the stream.

```python
{"type": "data-cost-summary", "data": {}}
{"type": "data-latency-summary", "data": {}}
```

### `finish`

Terminal event for a successful stream.

```python
{
    "type": "finish",
    "finishReason": "stop",
    "usage": {
        "promptTokens": 12,
        "completionTokens": 8,
        "totalTokens": 20,
    },
}
```

### `error`

Non-recoverable error. No further events follow.

```python
{"type": "error", "error": {"message": "...", "code": "..."}}
```

### `custom`

Domain-specific event from a tool or product layer.

```python
{"type": "custom", "event_type": "acme-forecast-refresh", "data": {"runId": "fr-42"}}
```

## Ordering notes

- `step-start` precedes output for that step.
- Tool calls emit call events before result events.
- Approval-gated tools emit approval events between call and result.
- `finish` is terminal for successful streams, though aggregate summaries may
  follow.
- After `error`, no further events are emitted.
