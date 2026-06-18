# flowai-harness

Build production-grade data agents with typed plans and actions, tool execution, human approval, and observable runtime events.

`flowai-harness` is an opinionated agent harness for building agents that operate on complex data, use tools, and execute multi-step plans safely on the Flow AI runtime.

## What it is

`flowai-harness` helps AI engineering teams build agents that do more than analyzing data.

It gives you opinionated configuration surfaces for

- defining agent roles and routing
- creating auditable plans
- calling APIs to safely execute actions
- pausing for human approval
- passing large data through references outside the context window
- streaming execution events for debugging and observability

You define the multi-agent system in plain Python. `flowai-harness` runs the loop, validates inputs, manages the plan lifecycle, and emits structured runtime events.

## Who is this for

`flowai-harness` is for engineering teams building customer-facing agents that need to work reliably with business data and have to execute write actions.

Use it if you are building:

- analytical agents over databases, warehouses, catalogs, or internal APIs
- workflow agents that plan and execute multiple steps
- customer-facing agents that need approval gates before sensitive actions
- multi-agent systems with coordinators, planners, executors, and specialists
- agents that need traces, evals, and repeatable debugging from day one

## When to use it

Flow AI is for data-heavy multi-agent systems, not lightweight chat wrappers or NL2SQL only agents.

| Good fit                                                                                                                | Better alternatives                                           |
| ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| The agent coordinates multiple tool calls or specialist agents.                                                         | The app makes one model call and returns the answer.          |
| The agent creates, reviews, and follows a typed plan.                                                                   | The experience is a simple chatbot with no plan lifecycle.    |
| Actions may change data, call business APIs, or require approval before execution.                                      | There are no tools, approvals, or side effects to manage.     |
| Prompts need access to large or sensitive business data through references instead of stuffing everything into context. | The required context fits directly in the prompt.             |
| You need traces, runtime events, tests, or evals around agent behavior.                                                 | A prototype can tolerate raw SDK calls and ad hoc debugging.  |
| The agent is headed toward production, customer-facing, or operational use.                                             | You are exploring a prompt shape before designing the system. |

## Install

Install from a clone of the source repository. From the repository root:

```bash
./scripts/check-env.sh
./scripts/install.sh
./.venv/bin/flowai-harness --version
```

`install.sh` verifies the toolchain, builds the Studio UI and native runtime, and
installs `flowai-harness` into `.venv`. Run the scripts in this guide with
`./.venv/bin/python`.

## Before you begin

- Use Python 3.12 (any 3.12.x).
- Pin the alpha package version in production environments.
- The hello-world example below uses `TestingConfig`, so it does not require
  provider credentials. Live model runs need the provider environment variables
  referenced by your runtime spec, such as `ANTHROPIC_API_KEY`.

!!! warning "Alpha release"
Version `1.0.0a1` is the Python package version for the `v1.0.0-alpha.1`
alpha tag. The public API is stable enough to build against, but
breaking changes are still possible during the alpha cycle.

## Where to next

<div class="grid cards" markdown>

- **Quickstart**

  ***

  Build a structured data agent with references, tools, typed plans, approvals,
  action dispatch, and streamed events.

  [:octicons-arrow-right-24: Quickstart](quickstart.md)

- **Minimal runtime**

  ***

  Verify install, native runtime import, provider declaration, and deterministic
  event streaming without provider credentials.

  [:octicons-arrow-right-24: Minimal runtime](#minimal-runtime)

- **Coordinator planner executor**

  ***

  Build a coordinator-planner-executor Studio app with routed agents,
  seeded SQLite data, typed plans, traces, and eval cases.

  [:octicons-arrow-right-24: Coordinator planner executor](tutorials/coordinator-planner-executor.md)

- **Inventory scenario**

  ***

  Seed a static SQLite artifact, run local catalog tools, execute approved
  inventory actions, and inspect the app in Studio.

  [:octicons-arrow-right-24: Inventory scenario](tutorials/inventory-scenario.md)

- **Concepts**

  ***

  Mental model and focused concept guides: tenant identity, agents, plans, references, tools,
  prompts, and the runtime handle.

  [:octicons-arrow-right-24: Concepts](concepts/index.md)

- **Reference**

  ***

  Generated API reference for every public symbol, including Pydantic spec values and the
  native `Runtime` handle.

  [:octicons-arrow-right-24: Reference](reference/index.md)

- **Studio**

  ***

  Run a local browser UI for chat, data inspection, tests, evals, runs, and traces.

  [:octicons-arrow-right-24: Studio](guides/studio.md)

- **Source**

  ***

  Browse the source, file issues, and install from source with
  `./scripts/install.sh`.

  [:octicons-arrow-right-24: flowaicom/flowai-harness](https://github.com/flowaicom/flowai-harness)

</div>

## Minimal runtime

Verify installation, native runtime import, provider declaration, and event
streaming in five minutes.

This smoke test builds a minimal coordinator and specialist, runs the native
runtime with the deterministic testing interpreter, and prints the event stream.
It does not require provider credentials, and it does not exercise real routing,
planning, tools, references, or approval gates. For the structured agent
walkthrough, start with the [Quickstart](quickstart.md).

### Minimal runtime prerequisites

- Install Python 3.12 (any 3.12.x) and the Rust toolchain — `install.sh`
  builds the native runtime from source.
- No Anthropic, OpenAI, or other provider API key is required for this example.

### Minimal runtime install

From the repository root:

```bash
./scripts/check-env.sh
./scripts/install.sh
```

### What you build

You will build one runtime with:

- A tenant identity for runtime-owned state.
- A coordinator that receives the user request.
- A specialist that can be routed to by the coordinator.
- A deterministic no-network mock response for local testing.

### Create `hello_flowai.py`

Save the following as `hello_flowai.py`. It is one complete, runnable script:

```python
import asyncio

from flowai_harness import (
    TestingConfig,
    create_runtime,
    define_coordinator,
    define_runtime,
    define_specialist,
    define_tenant,
)


async def main() -> None:
    tenant = define_tenant("acme", "v1")

    specialist = define_specialist(
        name="greeter",
        model="claude-haiku-4-5",
        prompt="You greet the user politely.",
    )
    coordinator = define_coordinator(
        name="hello_coordinator",
        model="claude-sonnet-4-6",
        routes=["greeter"],
        prompt="Route greeting requests to the greeter specialist.",
    )

    runtime_spec = define_runtime(
        tenant=tenant,
        agents=[coordinator, specialist],
        providers={"anthropic": {"apiKey": "unused"}},
    )

    runtime = create_runtime(
        runtime_spec,
        testing=TestingConfig(mock_response="hello from the Rust runtime"),
    )

    async for event in runtime.query("Say hello", thread_id="thread-1"):
        print(event)


asyncio.run(main())
```

!!! note "Why `providers=` when no key is used"
    Every agent model resolves to a provider, and `create_runtime` validates
    that the provider is declared in `RuntimeSpec.providers` even when the
    deterministic testing interpreter never calls it. The placeholder
    `{"apiKey": "unused"}` satisfies validation without making any network
    request.

### Run it

```bash
./.venv/bin/python hello_flowai.py
```

### Expected output

The runtime prints a short stream of event dictionaries. Identifiers such as
`toolInvocationId` differ on every run, but the shape looks like this:

```text
{'agentName': 'hello_coordinator', 'state': 'call', 'toolInvocationId': 'inv-1955407c-815d-4f47-a49c-99f719900160', 'type': 'tool-agent'}
{'type': 'step-start'}
{'text': 'Received: Say hello\n\n', 'type': 'text'}
{'text': 'hello from the Rust runtime', 'type': 'text'}
{'data': {'hadTimeout': False, 'phases': {'llmCalls': 1, 'llmTimeMs': 0, 'subAgentTimeMs': 0, 'toolTimeMs': 0}, 'retryCount': 0, 'toolTimings': [], 'totalDurationMs': 0}, 'type': 'data-latency-summary'}
{'finishReason': 'stop', 'type': 'finish', 'usage': {'cacheCreationInputTokens': 0, 'cacheReadInputTokens': 0, 'completionTokens': 25, 'promptTokens': 50, 'totalTokens': 75}}
{'agentName': 'hello_coordinator', 'state': 'result', 'toolInvocationId': 'inv-1955407c-815d-4f47-a49c-99f719900160', 'type': 'tool-agent'}
```

### What happened

- `define_tenant("acme", "v1")` created the tenant identity that keys all
  runtime-owned state.
- `define_specialist` and `define_coordinator` built two validated Pydantic
  agent specs; the coordinator routes greeting requests to the specialist.
  Coordinators with `routes=[...]` receive the built-in `call_agent` tool by
  default; you do not need to list `toolkits=["agents"]`.
- `define_runtime` assembled the specs into a `RuntimeSpec`, including the
  provider declaration that every agent model resolves against.
- `create_runtime(..., testing=TestingConfig(...))` selected the deterministic
  testing interpreter, so no API key or network access was needed.
- `runtime.query(...)` streamed events from the embedded Rust runtime, ending
  with the mock response text.

The testing interpreter returns a fixed response, so routing and specialist
logic are stubbed: the events show `hello_coordinator`, not `greeter`. You are
verifying install and runtime wiring here, not agent behavior. Use the
[Quickstart](quickstart.md) to see tools, typed plans, references, approvals,
and action dispatch.

### Common errors

| Error | Fix |
| --- | --- |
| `ValueError: agent 'hello_coordinator' references provider 'anthropic' for model 'claude-sonnet-4-6', but no such provider is declared in RuntimeSpec.providers` | Add `providers={"anthropic": {"apiKey": "unused"}}` to `define_runtime(...)`. The testing interpreter never calls the provider, but the spec must declare it. |
| `ValueError: create_runtime accepts either testing or a non-default interpreter, not both` | Pass either `testing=TestingConfig(...)` or `interpreter="..."` to `create_runtime`, never both. They are mutually exclusive modes. |
| `ModuleNotFoundError: No module named 'flowai_harness'` | Install the package in the same virtual environment that runs the script. |
| Native extension import error | Use Python 3.11+ and reinstall the wheel for the active interpreter. |
| Agent route validation fails | Make sure every coordinator `routes=[...]` entry matches a registered agent name. |

### Next steps

- Build the structured [Quickstart](quickstart.md).
- Build the full [Coordinator planner executor tutorial](tutorials/coordinator-planner-executor.md).
- Run the [Inventory scenario tutorial](tutorials/inventory-scenario.md) for a
  seeded SQLite artifact, local catalog tools, a mock platform, and Studio.
- Read the [Concepts](concepts/index.md) section for the mental model.
- Browse the [Guides](guides/index.md) for task-focused how-tos.
