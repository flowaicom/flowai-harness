# Unions

Unions describe payloads that can take one of several typed shapes. In Flow AI
they are most often used for plan action schemas, where each action has a
discriminator such as `kind` and a variant-specific payload.

Use `TaggedUnion(...)` when a plan, event, or other schema accepts multiple
Pydantic model variants. It returns a Pydantic discriminated union over the
`kind` field by default, validates that every variant has a unique
discriminator value, and exports a schema the runtime can use for validation.

For plan-action examples, see [Plans concept](../concepts/plans.md#define-typed-actions).

::: flowai_harness.unions.TaggedUnion
