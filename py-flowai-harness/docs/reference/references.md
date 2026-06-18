# References

References are typed handles to stored values. They let agents pass large,
expensive, or sensitive payloads between tools, plans, executors, and host
callbacks without copying the full value into every prompt.

Use `define_reference(...)` to declare the payload shape behind a reference.
Customer code may also supply a `glimpse` callback that produces a small JSON
summary stored alongside the reference, so the model can reason about what the
handle points to before resolving the full value.

Reference schemas are normalized by
[`normalize_schema(...)`](schema-utilities.md#flowai_harness._schema.normalize_schema),
the same helper used by tools and plans.

For the full reference and glimpse model, see the
[References concept](../concepts/references.md).

::: flowai_harness.references.define_reference

::: flowai_harness.references.ReferenceSpec
