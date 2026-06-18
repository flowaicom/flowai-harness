# Studio

Studio apps are local registries of one or more workspace runtime bindings.
The CLI imports a `FlowAIApp` and serves the Studio UI and API from the same
Python process.

## CLI

The Python app entrypoint uses the `package.module:symbol` import form. The
symbol can be a `FlowAIApp` value or a zero-argument factory returning one.

```bash
flowai-harness dev --app my_agent.studio_app:app
flowai-harness serve --app my_agent.studio_app:app
```

Both commands bind to `127.0.0.1:4111` by default and serve the packaged
Studio UI on the same port as the API.

Common options:

| Option | Description |
| --- | --- |
| `--app package.module:symbol` | Required Studio app import target. |
| `--host 127.0.0.1` | Host to bind. |
| `--port 4111` | Port for both Studio UI and API routes. |
| `--no-studio` | Serve API routes only. |

For Python harness apps, use `dev` or `serve`.

`dev` additionally accepts four flags for running the Studio frontend from
source instead of the packaged UI assets:

In most cases you will not need to build the Studio from source files, unless
you explicitly update or customize the Studio.

| Option | Description |
| --- | --- |
| `--no-frontend` | Do not launch the React Studio source frontend dev server. |
| `--frontend-host HOST` | Host for the React Studio source frontend dev server. Requires `--studio-dir`. Defaults to `127.0.0.1`. |
| `--frontend-port PORT` | Port for the React Studio source frontend dev server. Requires `--studio-dir`. Defaults to `3000`. |
| `--studio-dir PATH` | Path to the Studio frontend source directory containing `package.json`. When provided, `dev` starts Bun/Vite on a separate frontend port. |

These source-frontend flags cannot be combined with `--no-studio`, and
`--no-frontend` cannot be combined with the other three.

## HTTP surface

Studio serves a dynamic config file and workspace-scoped API routes:

| Route | Description |
| --- | --- |
| `/` | Packaged Studio browser UI. |
| `/__flowai_config.js` | Dynamic runtime config for the browser UI. |
| `/api/status` | Studio API version and implementation status. |
| `/api/workspaces` | Workspace list and default workspace key. |
| `/api/workspaces/{workspace_key}/runtime` | Runtime metadata. |
| `/api/workspaces/{workspace_key}/agents` | Agent metadata. |
| `/api/workspaces/{workspace_key}/data/...` | Data inspection routes when a data environment is attached. |
| `/api/workspaces/{workspace_key}/tests/...` | Test case management routes. |
| `/api/workspaces/{workspace_key}/evals/...` | Eval routes. |
| `/api/workspaces/{workspace_key}/runs/...` | Persisted run activity, events, approvals, and traces. |

For the complete generated route list, run the Studio server and open
`http://127.0.0.1:4111/api/docs` or
`http://127.0.0.1:4111/api/redoc`. The OpenAPI JSON is available at
`http://127.0.0.1:4111/api/openapi.json`.

`/__flowai_config.js` is generated on each request. The server does not mutate
installed Studio files.

## API reference

::: flowai_harness.studio.define_app

::: flowai_harness.studio.define_workspace_runtime

::: flowai_harness.studio.FlowAIApp
    options:
      show_bases: false

::: flowai_harness.studio.WorkspaceRuntimeBinding
    options:
      show_bases: false
