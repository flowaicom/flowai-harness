# Tools

Tools are explicit model-callable capabilities attached to agents. They define
what an agent may call, what input shape the runtime validates, what output
shape is documented, and whether the call needs approval.

Use custom tools for application-specific work such as lookups, search,
read-only analysis, previews, or reference creation. Runtime-owned built-in
toolkits expose capabilities such as agent routing, plan storage, references,
catalog search, and SQL execution.

`define_tool(...)` creates a [`ToolSpec`](#flowai_harness.tools.ToolSpec). It
is callable, so the returned spec can also be used as a decorator to attach a
Python handler.

Tool input and output schemas are normalized by
[`normalize_schema(...)`](schema-utilities.md#flowai_harness._schema.normalize_schema),
which accepts JSON Schema dictionaries, Pydantic models, simple type maps, and
other Pydantic-exportable type hints.

For the full tool model, approval guidance, and built-in toolkit list, see the
[Tools concept](../concepts/tools.md).

::: flowai_harness.tools.define_tool

::: flowai_harness.tools.ToolSpec
    options:
      members:
        - bind
        - __call__
