# Plans

Plans are typed, reviewable containers for the actions an agent intends to
take. In a planner/executor flow, a planner creates and stores a plan, the
runtime validates it against the plan schema and approval policy, and an
executor later loads and executes the approved actions.

Use `define_plan(...)` to declare the action payload schema shared by planner
and executor agents. It accepts JSON Schema, Pydantic models, or a
`{name: type}` shorthand and produces a frozen
[`PlanSpec`](#flowai_harness.plans.PlanSpec).

Plan schemas are normalized by
[`normalize_schema(...)`](schema-utilities.md#flowai_harness._schema.normalize_schema),
the same helper used by tools and references.

For the full lifecycle and execution model, see the
[Plans concept](../concepts/plans.md) and
[Action dispatcher](action-dispatcher.md) references.

::: flowai_harness.plans.define_plan

::: flowai_harness.plans.PlanSpec

::: flowai_harness.plans.PlanDisplayAlias
