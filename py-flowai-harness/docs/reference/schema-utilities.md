# Schema and utilities

Small public helpers used by multiple authoring APIs.

`normalize_schema(...)` is the shared schema normalizer behind tools, plans,
and references. Use it when you need to inspect the JSON Schema that Flow AI
will send to the Rust runtime before creating a `ToolSpec`, `PlanSpec`, or
`ReferenceSpec`.

`normalize_data_environment(...)` validates the same data-environment mapping
accepted by `create_runtime(...)` and Studio workspace bindings, without
constructing a runtime. It returns the camelCase wire mapping consumed by the
Rust runtime, or `None` when the environment is empty.

::: flowai_harness._schema.normalize_schema

::: flowai_harness.runtime.normalize_data_environment
