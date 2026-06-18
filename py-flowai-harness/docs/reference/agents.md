# Agents

Role-specific agent constructors. Each returns an
[`AgentSpec`](runtime.md#flowai_harness.runtime.AgentSpec) that
`define_runtime(...)` composes into a [`RuntimeSpec`](runtime.md#flowai_harness.runtime.RuntimeSpec).

Role constructors add role-default prompt tools during runtime assembly without
serializing those defaults into `AgentSpec.toolkits`.

- Coordinators document route hand-off tools.
- Planners document `storePlan` and `getPlan`.
- Executors document `getPlan`, `executePlan`, and reference tools.

Pass `toolkits=` only for additional explicit built-in toolkit ids such as
`catalog`.

::: flowai_harness.agents.define_coordinator

::: flowai_harness.agents.define_planner

::: flowai_harness.agents.define_executor

::: flowai_harness.agents.define_specialist
