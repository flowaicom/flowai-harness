# Runtime

A runtime is the executable harness instance created from a validated
`RuntimeSpec`. It is required whenever you want agents to do work: the spec is
pure configuration, while the runtime owns the live Rust orchestration engine,
provider routing, approval gates, plan lifecycle, reference storage, eval
execution, callbacks, and built-in toolkit dispatch.

The usual lifecycle is:

| Step | API | Result |
| --- | --- | --- |
| Describe the application | [`define_runtime(...)`](#flowai_harness.runtime.define_runtime) | A frozen `RuntimeSpec` value. |
| Attach live callbacks and storage | [`create_runtime(...)`](#flowai_harness.runtime.create_runtime) | A native [`Runtime`](#flowai_harness.runtime.Runtime) handle. |
| Run work | Runtime methods such as `query(...)`, `run_eval(...)`, and `serve_mcp_http(...)` | Streams, eval artifacts, references, approvals, traces, or MCP servers. |

Model constructors use Python field names, and Pydantic accepts both
`snake_case` and `camelCase` aliases unless noted. Fields marked
**Python-only** are used by the Python facade and excluded from the Rust wire
spec.

<span id="flowai_harness.runtime.Runtime"></span>

## Runtime Handle

`Runtime` is the public Python alias for the native handle returned by
`create_runtime(...)`. Do not construct it directly; build a `RuntimeSpec`,
then call `create_runtime(...)`.

| Method | Use | Related docs |
| --- | --- | --- |
| `query(prompt, thread_id, resume=None)` | Run a coordinator turn and receive an async runtime event stream. | [Runtime events](runtime-events.md), [Streaming events](../guides/streaming.md) |
| `run_specialist(specialist, prompt, thread_id=None)` | Dispatch one registered specialist directly, bypassing the coordinator. | [Agents](agents.md) |
| `run_eval(eval_request)` | Run an eval to completion and return an eval artifact. | [Evals](evals.md), [Write evals](../guides/evals.md) |
| `stream_eval(eval_request)` | Run an eval and stream progress event envelopes. | [Evals](evals.md) |
| `get_trace(trace_id)` / `list_traces(...)` | Inspect traces recorded by evals or runtime runs. | [Evals](evals.md) |
| `create_reference(...)`, `resolve_reference(...)`, `reference_glimpse(...)` | Store, resolve, and preview typed references. | [References](references.md), [References & glimpses](../guides/references-and-glimpses.md) |
| `respond_to_approval(...)` | Resolve a pending approval gate with approve, reject, or revise. | [Require approvals](../guides/approvals.md) |
| `list_mcp_tools(...)`, `serve_mcp_stdio(...)`, `serve_mcp_http(...)` | Expose one runtime agent's tools over MCP. | [MCP](mcp.md), [Expose tools over MCP](../guides/mcp.md) |

::: flowai_harness.runtime.define_runtime

::: flowai_harness.runtime.create_runtime

::: flowai_harness.runtime.normalize_data_environment

## Runtime Spec Models

These models are user-facing whenever you build specs directly or inspect the
objects returned by helper constructors. Helper functions such as
`define_runtime(...)`, `define_coordinator(...)`, and `define_specialist(...)`
create these same model objects for you.

::: flowai_harness.runtime.RuntimeSpec
    options:
      show_bases: false

::: flowai_harness.runtime.AgentSpec
    options:
      show_bases: false

::: flowai_harness.runtime.ModelSpec
    options:
      show_bases: false

::: flowai_harness.runtime.ToolkitSpec
    options:
      show_bases: false

::: flowai_harness.runtime.ApprovalPolicies
    options:
      show_bases: false

Approval rules accept `"never"`, `"always"`, or
`{"kind": "dynamic", "value": "<predicate_id>"}`. They normalize to the Rust
wire shape `{"kind": ...}`.

::: flowai_harness.runtime.ApprovalPolicyPatch
    options:
      show_bases: false

::: flowai_harness.runtime.ApprovalOverrides
    options:
      show_bases: false

::: flowai_harness.runtime.StorageFactorySpec
    options:
      show_bases: false

::: flowai_harness.runtime.StorageFactories
    options:
      show_bases: false

## Testing Config

<span id="flowai_harness.runtime.TestingConfig"></span>

`TestingConfig` is a `TypedDict` used only with
`create_runtime(..., testing=...)`.

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `mock_response` | `str` | yes | Text emitted by the deterministic mock interpreter for every model turn. Used with `create_runtime(..., testing={"mock_response": "..."})`. |

<span id="flowai_harness.runtime.DataEnvironmentConfig"></span>

## DataEnvironmentConfig

`DataEnvironmentConfig` is the optional `TypedDict` accepted by
`create_runtime(..., data_environment=...)` and
`normalize_data_environment(...)`. It attaches Rust-owned data dependencies
for built-in toolkit dispatch. All keys are optional and accept `snake_case`
or `camelCase` spelling.

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `tenant_id` / `tenantId` | `str` | no | Tenant id the environment is pinned to. When set, it must match the runtime tenant `resource_id`. |
| `workspace_id` / `workspaceId` | `str` | no | Workspace id used to scope stored data. |
| `kv` | descriptor | no | KV store descriptor. Supported kinds are `memory`, `sqlite`, `postgres`, and `redis`. |
| `target_database` / `targetDatabase` | descriptor | no | Target database descriptor for agent data queries. Supported kinds are `sqlite` and `postgres`. Mutually exclusive with `target_database_url`. |
| `target_database_url` / `targetDatabaseUrl` | `str` | no | Connection URL shorthand for the target database. |
| `target_database_schema` / `targetDatabaseSchema` | `str` | no | Schema name used with target database introspection. |
| `catalog` | descriptor | no | Data catalog store descriptor. Supported kinds are `empty`, `inline`, `sqlite`, and `postgres`. |
| `catalog_search` / `catalogSearch` | descriptor | no | Catalog fuzzy-search index configuration with `index_path` and optional rebuild/write-through flags. |

`normalize_data_environment(...)` returns `None` for an empty environment. For
a non-empty environment, it returns the camelCase wire dictionary passed to the
Rust runtime. The dictionary contains the normalized versions of the keys in
the table above.

## Data Environment Descriptors

The top-level `tenant_id`, when set, must match the runtime tenant
`resource_id`. `target_database` and `target_database_url` are mutually
exclusive.

### `kv`

| Kind | Required fields | Optional fields |
| --- | --- | --- |
| `memory` | none | none |
| `sqlite` | `url` | `ensure_schema` |
| `postgres` | `url` or `url_env` | `table`, `ensure_schema` |
| `redis` | `url` or `url_env` | `prefix` |

### `target_database`

| Kind | Required fields | Optional fields |
| --- | --- | --- |
| `sqlite` | `url` | none |
| `postgres` | `url` or `url_env` | `schema` |

`target_database_url` is a shorthand for a target database URL. Use
`target_database_schema` when the shorthand needs an explicit schema name.

### `catalog`

| Kind | Required fields | Optional fields |
| --- | --- | --- |
| `empty` | none | none |
| `inline` | none | `entries` |
| `sqlite` | `url` | `ensure_schema` |
| `postgres` | `url` or `url_env` | `ensure_schema` |

### `catalog_search`

| Required fields | Optional fields |
| --- | --- |
| `index_path` | `rebuild_on_start`, `write_through` |

For task-oriented setup, see
[Configure a data environment](../guides/data-environment.md) and
[Knowledge and documents](../guides/knowledge.md).
