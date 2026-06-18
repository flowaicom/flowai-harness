# Flow AI Harness

[![Website](https://img.shields.io/badge/website-flow--ai.com-blue)](https://flow-ai.com)
[![Docs](https://img.shields.io/badge/docs-flow--ai.com%2Fdocs-blue)](https://flow-ai.com/docs)

Build production-grade data agents with typed plans and actions, tool execution,
human approval, and observable runtime events.

flowai-harness is an opinionated agent harness for building agents that operate
on complex data, use tools, and execute multi-step plans safely on the Flow AI
runtime. You define the multi-agent system in plain Python; flowai-harness runs
the loop, validates inputs, manages the plan lifecycle, and emits structured
runtime events.

The public surface is `flowai-harness`: a Python API and CLI for defining agent
topologies, typed plans, tools, references, approvals, tests, evals, data
environments, and local Studio apps. Under that Python facade, a native Rust
runtime powers orchestration, provider routing, streaming, approval gates, plan
lifecycle, built-in data tooling, and execution state.

For source checkout installs, use the flow below. It builds the local Studio
UI, packages it into the Python harness, and installs the harness into `.venv`.

Current alpha release: `v0.1.0-alpha.1`.

The Python package version for this tag is `0.1.0a1`, which is the PEP 440 form
of the same prerelease version.

## Source Checkout Install

From the repository root:

```bash
./scripts/check-env.sh
./scripts/install.sh
./.venv/bin/flowai-harness --version
```

If the environment check reports a missing or incompatible dependency, run the
pinned setup script and retry:

```bash
./scripts/setup-env.sh
./scripts/check-env.sh
./scripts/install.sh
```

You can also install the dependencies manually if you want to manage install
locations yourself. Install compatible versions, make sure they are on `PATH`,
then rerun `./scripts/check-env.sh` before `./scripts/install.sh`.

The source install toolchain versions are:

| Tool         | Setup script installs | `check-env.sh` accepts |
| ------------ | --------------------- | ---------------------- |
| Rust / Cargo | 1.94.0                | 1.88.0 or newer        |
| Python       | 3.12.13               | any Python 3.12.x      |
| uv           | 0.10.9                | 0.10.9 or newer        |
| Bun          | 1.3.5                 | 1.3.5 or newer         |

`setup-env.sh` installs missing compatible versions for the current user and
sets the Rust override for this checkout. `install.sh` stops before building if
the environment is not compatible.

## First Commands

After installation, use the harness from the local virtual environment:

```bash
./.venv/bin/flowai-harness --help
```

For agent projects managed with `uv`, you can also install this checkout into
the project environment and run the CLI through `uv run`.

The detailed package docs are published at https://flow-ai.com/docs. Start with:

- [Quickstart](https://flow-ai.com/docs/quickstart) for a complete agent
  walkthrough.
- [Runtime concepts](https://flow-ai.com/docs/concepts/runtime) for the runtime
  mental model.
- [Agents](https://flow-ai.com/docs/concepts/agents), [plans](https://flow-ai.com/docs/concepts/plans),
  [tools](https://flow-ai.com/docs/concepts/tools), and
  [references](https://flow-ai.com/docs/concepts/references) for the main
  building blocks.
- [API reference](https://flow-ai.com/docs/reference) for the exported
  Python symbols.

The public docs use this same source-checkout install flow.

## Examples

The repo ships two customer-facing examples that show full agent topologies,
typed plans, data environments, and Studio workflows. Start with
[examples/README.md](examples/README.md) for the overview and run steps, then
dive into each example:

- [Coordinator-Planner-Executor](examples/coordinator_planner_executor/) — a
  Flow AI Harness app with coordinator, planner, executor, and data analyst
  agents, typed plans, seeded SQLite demo data, Studio eval seed cases, and an
  offline smoke check.
- [Inventory Scenario](examples/inventory_scenario/) — the flagship
  data-environment example with a published SQLite artifact, local
  catalog/KV/search state, a mutable mock platform, and
  coordinator/planner/executor/insights agents.

## What It Is

flowai-harness helps AI engineering teams build agents that do more than
analyze data. It gives you opinionated configuration surfaces for:

- **Agents** — define coordinator, planner, executor, and specialist roles and
  routing.
- **Plans** — create auditable plans as typed actions built on Pydantic models
  and tagged unions.
- **Tools** — connect the runtime to customer services, databases, and APIs to
  safely execute actions.
- **Approvals** — gate side-effecting actions behind human approval before they
  run.
- **References and glimpses** — pass large customer-owned values as compact
  handles outside the context window, instead of stuffing prompts.
- **Streaming events** — emit structured runtime events for debugging and
  observability.

Around those surfaces, the harness also gives you **tenants** to scope runtime
state, references, approvals, traces, and evals; **tests and evals** for
planner, executor, specialist, and sequential flows; and **data environments**
that attach target databases, catalog storage, catalog search, and knowledge
files to built-in data tools.

flowai-harness is for engineering teams building customer-facing agents that
work reliably with business data and execute write actions — analytical agents
over databases, warehouses, catalogs, or internal APIs; workflow agents that
plan and execute multiple steps; agents with approval gates before sensitive
actions; and multi-agent systems that need traces, evals, and repeatable
debugging from day one.

Task-focused docs:

- [Approvals](https://flow-ai.com/docs/guides/approvals)
- [Data environments](https://flow-ai.com/docs/guides/data-environment)
- [Catalog profiling](https://flow-ai.com/docs/guides/catalog-profiling)
- [Knowledge ingest](https://flow-ai.com/docs/guides/knowledge)
- [Testing](https://flow-ai.com/docs/guides/testing)
- [Streaming events](https://flow-ai.com/docs/guides/streaming)
- [Expose tools over MCP](https://flow-ai.com/docs/guides/mcp)

## Architecture

The harness is intentionally split between a Python authoring layer and a Rust
execution layer.

```text
Customer Python app
  - define_runtime(...)
  - define_tool(...)
  - define_app(...)
  - service callbacks
        |
        v
flowai-harness Python facade
  - validates specs with Pydantic
  - adapts Python callbacks
  - exposes the CLI and Studio app definitions
        |
        v
Native Flow AI runtime
  - agent orchestration
  - provider routing
  - plan lifecycle
  - approval gates
  - event streaming
  - built-in data and MCP tooling
        |
        v
Customer systems
  - models
  - databases
  - APIs
  - services
```

The Python layer is the customer contract. The native runtime is embedded behind
that contract so customers can write normal Python apps while still getting a
high-performance, strongly typed execution kernel.

## Studio

Studio is the local browser interface for a `flowai-harness` app. It runs next
to your Python runtime and lets you chat with the app, inspect agents, browse
attached data sources, create tests, run evals, and inspect runs and traces.

`install.sh` builds the Studio frontend and stages it inside the Python
package. Normal Studio usage does not require running a separate frontend
server.

```bash
./.venv/bin/flowai-harness dev --app my_agent.studio_app:app
```

See the [Studio guide](https://flow-ai.com/docs/guides/studio) and
[Studio reference](https://flow-ai.com/docs/reference/studio) for app
definition, run modes, stored state, smoke checks, and UI behavior.

## Repository Layout

```text
py-flowai-harness/      Python package, CLI, docs, tests, and native extension
crates/                 Rust runtime and supporting execution crates
studio/                 Shared Studio React UI source
scripts/                Source install, setup, and packaging scripts
examples/               Customer-facing coordinator-planner-executor example
contracts/              Harness and Studio integration contracts
```

Customer-facing package docs are published at https://flow-ai.com/docs.

## Maintainer Checks

For the source install scripts:

```bash
cd py-flowai-harness
uv run --extra dev pytest tests/test_preview_scripts.py -q
cd ..
bash -n scripts/check-env.sh scripts/setup-env.sh scripts/install.sh
```

For broader harness changes, run the relevant package tests:

```bash
cd py-flowai-harness
uv run --extra dev pytest tests -q
```

Studio frontend changes should be checked from `studio/`:

```bash
bun install
bun run build
bun run typecheck
```

## License

Private preview use is governed by the
[Flow AI Private Preview License](LICENSE.md). The license permits internal
non-production evaluation during the evaluation period. Production use,
commercial use, redistribution, and hosted or third-party-facing use require a
separate written agreement with Flow AI.
