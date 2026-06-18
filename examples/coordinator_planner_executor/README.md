# Coordinator Planner Executor Example

Default local example for building a Flow AI Harness app with Python. It is
meant to be installed from this source checkout and run locally in Studio.

For the explanatory walkthrough, see the
[Coordinator Planner Executor tutorial](https://flow-ai.com/docs/tutorials/coordinator-planner-executor).

## What this example demonstrates

- A `coordinator` that routes work to a planner, executor, or data analyst.
- A `planner` that stores typed `CatalogActionPlan` plans with `storePlan`.
- An `executor` that executes stored plans with `executePlan` through a mock
  action dispatcher.
- A `data_analyst` specialist with read-only access to seeded SQLite product
  and order data.
- A Studio app with `acme` and `globex` workspaces.
- Seeded planner and executor eval cases for Studio.

## Prerequisites

Run the source checkout install from the repository root first:

```bash
./scripts/install.sh
```

That creates `./.venv`, builds the Studio UI, and installs
`flowai-harness==0.1.0a1` from this checkout.

## Quick start

From this example directory, install the example package into the repository
virtual environment:

```bash
cd examples/coordinator_planner_executor
uv pip install --python ../../.venv/bin/python -e .
```

Run the offline smoke check:

```bash
../../.venv/bin/python -m coordinator_planner_executor.smoke
```

Expected output:

```text
Coordinator-planner-executor smoke passed.
Observed agents: coordinator -> planner -> executor
```

Seed the bundled eval cases so Studio's Tests/Evals views are populated (they
start empty). This is idempotent and safe to re-run:

```bash
../../.venv/bin/python -m coordinator_planner_executor.seed_eval_tests --workspace acme
```

Expected output:

```text
seeded tc-planner-price-change in workspace acme
seeded tc-executor-price-change in workspace acme
```

## Run Studio

Create a local env file and set your Anthropic key:

```bash
cp .env.example .env
# Edit .env and set ANTHROPIC_API_KEY.
```

Start Studio:

```bash
../../.venv/bin/flowai-harness dev \
  --app coordinator_planner_executor.app:app \
  --port 4111
```

Open:

```text
http://127.0.0.1:4111
```

Studio serves the browser UI and API from the same process. Runtime state,
catalog data, and seeded SQLite target databases are written under
`.flowai/coordinator-planner-executor/` in this directory.

## Seed eval cases

Before starting Studio, seed planner and executor test cases into the local
Studio store:

```bash
../../.venv/bin/python -m coordinator_planner_executor.seed_eval_tests \
  --workspace acme
```

Seeded cases:

- `tc-planner-price-change` exercises planner mode and expects `storePlan`.
- `tc-executor-price-change` exercises a full coordinator flow and expects the
  planner's `storePlan` followed by the executor's `executePlan`.

## What to try

- Ask the `coordinator` about revenue, products, or orders. It should route to
  `data_analyst`.
- Ask the `coordinator` to plan and apply a price change. It should route to
  `planner` to create the plan, then `executor` to execute the stored plan.
- Run the seeded eval cases from Studio's Tests and Evals views.
- Inspect Runs to see tool calls, sub-agent calls, approvals, and traces.

## Common issues

| Symptom | Fix |
| --- | --- |
| `flowai-harness` is not found | Run `./scripts/install.sh` from the repository root, then use `../../.venv/bin/flowai-harness`. |
| The example imports the wrong harness version | Install the example into the repository venv with `uv pip install --python ../../.venv/bin/python -e .`. |
| `uv run` tries to resolve `flowai-harness` from a registry | Use `../../.venv/bin/python ...`, or run `uv run --active --no-sync ...` from an activated repository venv. |
| Studio starts but chat does not call a model | Set `ANTHROPIC_API_KEY` in `.env` and use the Studio command above. |
| Eval cases do not appear | Run the seed command after Studio has created `.flowai/studio.db`, or pass the expected `--workspace`. |

## Links

- [Tutorial walkthrough](https://flow-ai.com/docs/tutorials/coordinator-planner-executor)
- [Quickstart](https://flow-ai.com/docs/quickstart)
- [Studio guide](https://flow-ai.com/docs/guides/studio)
- [Test agents without provider calls](https://flow-ai.com/docs/guides/testing)
- [Evals](https://flow-ai.com/docs/guides/evals)
- [Agents](https://flow-ai.com/docs/concepts/agents)
- [Plans](https://flow-ai.com/docs/concepts/plans)
- [Require approvals](https://flow-ai.com/docs/guides/approvals)
