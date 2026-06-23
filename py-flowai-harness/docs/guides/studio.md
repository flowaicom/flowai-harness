# Studio

Studio is the local browser interface for a `flowai-harness` app. It runs next
to your Python runtime and lets you inspect agents, chat with the entrypoint,
browse attached data sources, create tests, run evals, and inspect runs and
traces.

Installed `flowai-harness` wheels include the Studio UI. You do not need Node,
Bun, or a separate frontend server for normal use.

!!! note "Prerequisites"
    Studio chat calls a real model, so export `ANTHROPIC_API_KEY` before
    starting the server. If your app attaches a data environment, also export
    `TARGET_DATABASE_URL` (or whichever URL your data environment reads) —
    the snippets below assume both are set.

## What you will have running

By the end of this guide, you will have a local Studio session where you can:

- chat with your agents
- inspect runs and traces
- browse attached data sources
- create tests
- run evals

## Define a Studio app

Create a module that exports a `FlowAIApp`. The CLI imports this module with
the `package.module:symbol` syntax.

```python
# my_agent/studio_app.py
from flowai_harness import (
    create_runtime,
    define_app,
    define_coordinator,
    define_runtime,
    define_specialist,
    define_tenant,
)


analyst = define_specialist(
    name="analyst",
    model="claude-haiku-4-5",
    prompt="Answer focused questions about the customer's data.",
)

coordinator = define_coordinator(
    name="coordinator",
    model="claude-sonnet-4-6",
    routes=["analyst"],
    prompt="Route analytical questions to the analyst.",
)

runtime_spec = define_runtime(
    tenant=define_tenant("acme", "v1"),
    agents=[coordinator, analyst],
    providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
)

app = define_app(
    name="acme-analytics",
    runtime_spec=runtime_spec,
    runtime_factory=lambda: create_runtime(runtime_spec, interpreter="anthropic"),
)
```

The `runtime_factory` line is what makes Studio chat call a model. Without
`interpreter="anthropic"`, `create_runtime(...)` defaults to the `noop` echo
interpreter: the server starts and the UI loads, but chat silently echoes
instead of calling the LLM, and the `providers=` entry is never exercised.
Keep the noop default only when you deliberately want an offline demo or a UI
smoke test with no model calls.

Run Studio from the project environment:

```bash
flowai-harness dev --app my_agent.studio_app:app
```

If you use `uv`, prefix the same command with `uv run`:

```bash
uv run flowai-harness dev --app my_agent.studio_app:app
```

Open:

```text
http://127.0.0.1:4111
```

The same server serves the Studio UI, `/api/...` routes, and the dynamic
`/__flowai_config.js` file.

Studio API authentication is enabled by default. The browser UI receives a
per-process token from `/__flowai_config.js` and adds it automatically to API
requests, so normal UI usage does not require any settings changes.

Direct API clients must send the token:

```bash
curl -H "X-FlowAI-Studio-Token: <token-from-__flowai_config.js>" \
  http://127.0.0.1:4111/api/status
```

For alpha-only local workflows that cannot set headers, disable the local API
token check explicitly:

```bash
flowai-harness serve --app my_agent.studio_app:app --no-api-auth
```

Only use `--no-api-auth` on trusted loopback development sessions.

## Attach a data environment

Pass the same data environment you use for runtime data tools. Studio uses it
to populate Connect pages such as Discovery, Search, and Tools.

```python
import os

from flowai_harness import define_app


data_environment = {
    # Requires TARGET_DATABASE_URL to be exported; os.environ.get(...) avoids
    # a KeyError at import time, but Connect pages need a real URL to work.
    "target_database_url": os.environ.get("TARGET_DATABASE_URL"),
    "catalog": {
        "kind": "sqlite",
        "url": "sqlite:.flowai/catalog.db",
        "ensure_schema": True,
    },
    "catalog_search": {
        "index_path": ".flowai/catalog-index",
        "rebuild_on_start": True,
    },
}

app = define_app(
    name="acme-analytics",
    runtime_spec=runtime_spec,
    data_environment=data_environment,
)
```

If you provide a custom `runtime_factory`, pass the same data environment to
`create_runtime(...)` inside that factory, and keep `interpreter="anthropic"`
so chat still reaches a model. This keeps the runtime tools and Studio's data
inspection routes pointed at the same resources.

```python
from flowai_harness import create_runtime, define_app


def build_runtime():
    return create_runtime(
        runtime_spec,
        data_environment=data_environment,
        services={"warehouse": warehouse_service},
        interpreter="anthropic",
    )


app = define_app(
    name="acme-analytics",
    runtime_spec=runtime_spec,
    runtime_factory=build_runtime,
    data_environment=data_environment,
)
```

## Run modes

Use `dev` while iterating on an agent:

```bash
flowai-harness dev --app my_agent.studio_app:app
```

Use `serve` when you want the same app as a stable local process, for example
for a demo or a local smoke test:

```bash
flowai-harness serve --app my_agent.studio_app:app
```

Both commands serve Studio on one port. The default is `127.0.0.1:4111`:

```bash
flowai-harness serve --app my_agent.studio_app:app --host 127.0.0.1 --port 4111
```

Use `--no-studio` only when you want API routes without the browser UI:

```bash
flowai-harness serve --app my_agent.studio_app:app --no-studio
```

## What Studio stores

Studio writes local development state under `.flowai/studio.db` in the
directory where you start the server. This database stores local chat threads,
tests, eval definitions, eval artifacts, run events, traces, and approval
references for the current app.

Keep this file as local development state unless your project intentionally
checks in demo fixtures.

## Verify it works

After the server starts, these endpoints should respond:

```bash
curl -H "X-FlowAI-Studio-Token: <token>" http://127.0.0.1:4111/api/status
curl -H "X-FlowAI-Studio-Token: <token>" http://127.0.0.1:4111/api/workspaces
curl -H "X-FlowAI-Studio-Token: <token>" http://127.0.0.1:4111/api/workspaces/default/agents
```

The root path should return Studio HTML:

```bash
curl http://127.0.0.1:4111/
```

## Using the UI

- **Playground** streams chat through the runtime entrypoint and persists the
  turn in the local thread store.
- **Connect** shows data sources, schemas, search, and data tools when the app
  has a data environment.
- **Tests** creates and edits test cases with prompts, expected trajectories,
  and structured ground truth.
- **Evals** runs saved test cases against planner, executor, specialist, or
  sequential eval modes.
- **Runs** shows persisted runtime activity, approvals, tool calls, sub-agent
  calls, and trace links.

The UI reads the app's workspace and agent metadata from your exported
`FlowAIApp`, so changes to the runtime spec take effect after restarting the
Studio server.

## Example

For a complete seeded Studio app with a local SQLite data environment, built-in
catalog tools, and a mutable mock platform, see the
[Inventory scenario example](../tutorials/inventory-scenario.md).

## Common errors

| Symptom | Fix |
| --- | --- |
| Studio loads but chat does not call a model | Make the runtime factory call `create_runtime(..., interpreter="anthropic")`. |
| Connect pages are empty | Pass the same `data_environment` to both `define_app(...)` and `create_runtime(...)`. |
| Server starts with a stale runtime spec | Restart Studio after changing the exported `FlowAIApp`. |
