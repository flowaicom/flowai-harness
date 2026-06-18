# flowai-harness

`flowai-harness` is the Python package for building, running, testing, and
inspecting Flow AI data agents.

You define agents, tools, plans, approvals, tests, evals, and data environments
in Python. The harness validates those definitions, adapts your callbacks, and
runs them on the native Flow AI runtime.

## Install

Use Python 3.11 or newer.

For packaged releases:

```bash
pip install flowai-harness
```

If you are working from this source checkout, follow the repository-level
install instructions in the root `README.md`.

## Quick Start

This example uses the deterministic testing interpreter, so it does not need a
model provider API key.

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
        testing=TestingConfig(mock_response="hello from the Flow AI runtime"),
    )

    async for event in runtime.query("Say hello", thread_id="thread-1"):
        print(event)


asyncio.run(main())
```

For the complete walkthrough, see the
[Quickstart](https://flow-ai.com/docs/quickstart).

## Studio

Studio is the local browser interface for a harness app. It lets you chat with
agents, inspect tools and data connections, create tests, run evals, and review
runs and traces.

```bash
flowai-harness dev --app my_agent.app:app
```

See the [Studio guide](https://flow-ai.com/docs/guides/studio) for app setup,
run modes, stored state, and common workflows.

## What You Can Define

- **Tenants** for scoped runtime state.
- **Agents** for coordinator, planner, executor, and specialist roles.
- **Tools** for connecting agents to your services, databases, APIs, and
  workflows.
- **Plans** for typed, reviewable action sequences.
- **References and glimpses** for passing compact handles to larger values.
- **Approvals** for human-in-the-loop gates.
- **Tests and evals** for checking agent behavior.
- **Data environments** for connecting target databases, catalog storage,
  catalog search, and knowledge files.

## Documentation

The product docs are the source of truth for user-facing behavior:

- [Documentation home](https://flow-ai.com/docs)
- [Quickstart](https://flow-ai.com/docs/quickstart)
- [Runtime concepts](https://flow-ai.com/docs/concepts/runtime)
- [Agents](https://flow-ai.com/docs/concepts/agents)
- [Tools](https://flow-ai.com/docs/concepts/tools)
- [Plans](https://flow-ai.com/docs/concepts/plans)
- [Approvals](https://flow-ai.com/docs/guides/approvals)
- [Data environments](https://flow-ai.com/docs/guides/data-environment)
- [Testing](https://flow-ai.com/docs/guides/testing)
- [Evals](https://flow-ai.com/docs/guides/evals)
- [API reference](https://flow-ai.com/docs/reference)

## Package Layout

```text
flowai_harness/        Public Python API and CLI
src/lib.rs             Native extension boundary
tests/                 Python package tests
docs/                  Source for the published product docs
```

## Development

Run package-level tests from this directory:

```bash
uv run --extra dev pytest tests -q
```

For repository-wide setup, install, and frontend checks, use the root
`README.md`.
