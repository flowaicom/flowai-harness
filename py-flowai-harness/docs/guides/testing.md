# Test agents without provider calls

The harness ships two deterministic interpreters so you can unit-test your
agent topology without a live model provider:

- `testing={"mock_response": "..."}` — the deterministic testing interpreter:
  a no-network path that emits a fixed text response and a `finish` event.
- `interpreter="scripted"` — drives the runtime from a JSON script you embed
  in the user prompt. Agent routing, tool calls, and plan storage all happen
  end-to-end; only the LLM's decisions are replaced with the script.

Tool handlers, action dispatchers, and approval predicates are still invoked
for real in both modes, so you can assert end-to-end behavior while keeping the
model response deterministic.

This page covers the interpreters themselves, tool context in tests, approval
flows, and dispatcher validation. For scoring agent behavior — trajectories,
actions, and response text — see [Evals](evals.md) and
[Final-response judge evals](judge-evals.md).

## Which testing mode should I use?

| Need | Use |
| --- | --- |
| Simple smoke test | `testing={"mock_response": "..."}` |
| Exact multi-agent or tool flow | `interpreter="scripted"` |
| Approval flow test | `interpreter="scripted"` |
| Action dispatcher validation | `interpreter="scripted"` |
| Real model behavior | Provider-backed runtime plus [evals](evals.md) |

By the end of this guide, you should have a test that passes without provider
credentials.

## Mocked text response

`TestingConfig` is a `TypedDict` with one supported key, `mock_response`. The
runtime returns it as a single `text` event followed by `finish`.

```python
import asyncio

from flowai_harness import (
    AgentSpec,
    create_runtime,
    define_tenant,
    define_runtime,
    define_specialist,
)


def test_coordinator_emits_mock_response():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a minimal response.",
        routes=["worker"],
    )
    worker = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Handle delegated test requests.",
    )

    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[coordinator, worker],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked runtime response"},
    )

    events = asyncio.run(_collect(runtime.query("hello", thread_id="thread-1")))
    assert any(
        event["type"] == "text" and "mocked runtime response" in event["text"]
        for event in events
    )


async def _collect(stream):
    return [event async for event in stream]
```

!!! note
    `testing` cannot be combined with a non-default `interpreter`. Pass one or
    the other.

## Scripted interpreter

Pass `interpreter="scripted"` and encode the script as a JSON object in the
user prompt. The script is a list of tool calls; the runtime replays them as if
the LLM had emitted them.

```python
import json

prompt = json.dumps({"tool": "echo", "args": {"value": "hello"}})
```

For a coordinator-driven flow, wrap a list of `call_agent` invocations. The
nested `prompt` for each routed agent is itself a JSON script in the same
`{"tool": ..., "args": ...}` shape:

```python
planner_prompt = json.dumps({"tool": "echo", "args": {"value": "plan step"}})
executor_prompt = json.dumps({"tool": "echo", "args": {"value": "execute step"}})

prompt = json.dumps(
    {
        "script": [
            {"tool": "call_agent", "args": {"agent": "planner",  "prompt": planner_prompt}},
            {"tool": "call_agent", "args": {"agent": "executor", "prompt": executor_prompt}},
        ]
    }
)
```

For a full coordinator -> planner -> executor script that stores and executes a
plan, see [Scripted runtime eval](evals.md#scripted-runtime-eval).

The example below uses a specialist with a Python tool handler, invoked through
`runtime.run_specialist(...)`:

```python
import asyncio
import json

from flowai_harness import (
    create_runtime,
    define_tenant,
    define_runtime,
    define_specialist,
    define_tool,
)


def test_python_tool_handler_runs_under_scripted_interpreter():
    calls = []

    @define_tool("echo", {"value": str}, approval="never")
    async def echo(args, ctx):
        calls.append((args, ctx["tool_use_id"]))
        return {"echo": args["value"]}

    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        tools=[echo],
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        interpreter="scripted",
    )

    prompt = json.dumps({"tool": "echo", "args": {"value": "hello"}})
    events = asyncio.run(_collect(runtime.run_specialist("worker", prompt, thread_id="thread-1")))

    assert calls == [({"value": "hello"}, "scripted-tool-1")]
    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "echo"
        and event["state"] == "result"
        and event["result"] == {"echo": "hello"}
        for event in events
    )


async def _collect(stream):
    return [event async for event in stream]
```

## Tool context fields

Inside a tool handler under either interpreter, `ctx` is a mapping with at
least:

- `tool_use_id` — the runtime-assigned id of the current call. Under the
  scripted interpreter this is `"scripted-tool-1"`, `"scripted-tool-2"`, and
  so on, which makes assertions deterministic.
- `services` — the mapping passed to `create_runtime(..., services=...)`.
  Valid service names are also available via `ctx.<name>` and `ctx["<name>"]`,
  so tests can assert service-backed tools without a live model.
- For dynamic-approval predicates, `ctx` also carries `target` (the tool name)
  and `kind` (always `"tool"` for tool approvals).

## Approval flows under scripted

The scripted interpreter respects coordinator and tool approval policy. To
test a gated flow, intercept `approval-required` in your async loop and call
`runtime.respond_to_approval(...)`:

```python
async def run_flow():
    stream = runtime.query(_coordinator_script(), thread_id="thread-smoke")
    events = []
    async for event in stream:
        events.append(event)
        if event["type"] == "approval-required":
            await runtime.respond_to_approval(
                event["data"]["id"],
                "approve",
                feedback="approved by smoke test",
            )
    return events
```

The plan executor only dispatches actions after `"approve"`. For `"reject"`
and `"revise"` the action dispatcher is not invoked at all; for `"revise"`,
the executor's `executePlan` tool result carries `{"should_revise": True,
"partial": {...}}`.

Action dispatcher return values are validated in the same scripted path. A
dispatcher may return `None`, or an object with required `entitiesAffected`,
optional `summary`, and optional `details`. Missing or mistyped envelope fields
surface as an `executePlan` error, so tests can assert bad host adapters fail
before malformed execution results are stored.

## Verify it works

Run the test without provider credentials. A deterministic test should pass with
`providers={"anthropic": {"apiKey": "unused"}}` or another unused placeholder,
as long as the selected testing mode does not call a live provider.

## Common errors

`testing` enforces its shape eagerly:

- Unknown keys raise `ValueError`.
- A missing `mock_response` raises `ValueError`.
- A non-string `mock_response` raises `TypeError`.
- Combining `testing=` with `interpreter="scripted"` (or `"anthropic"`)
  raises `ValueError`.

This makes wrong fixtures fail at the call site rather than mid-stream.

## Scoring agent behavior

Both deterministic interpreters pair naturally with the eval helpers:

- [Evals](evals.md) covers offline scoring with `score_sample(...)`, score
  semantics, trajectory modes and projection, action payload matching,
  specialist scoring, and runtime-backed evals (including running them against
  the testing or scripted interpreter for CI).
- [Final-response judge evals](judge-evals.md) covers judge-backed scoring of
  the final user-facing response, including how to unit-test judge weighting
  offline with precomputed verdicts.

## See also

- [Streaming events](streaming.md) for consuming runtime events in tests.
- [Require approvals](approvals.md) for approval configuration and outcomes.
- [Evals reference](../reference/evals.md) for DTOs, scorer presets, artifacts, and events.
- [`TestingConfig` reference](../reference/runtime.md#flowai_harness.runtime.TestingConfig)
- [`create_runtime` reference](../reference/runtime.md#flowai_harness.runtime.create_runtime)
