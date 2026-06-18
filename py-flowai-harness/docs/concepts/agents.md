# Agents

Agents are configured harness roles with different defaults and built-in
capabilities.

Use the agent type that matches the job you want that part of the system to
perform. All agent types can have their own prompt, model, tools, approval
policy, and turn limits.

## Agent types

| Agent type      | Use it for                                                                             | Built-in capabilities and defaults                                                                                                      |
| --------------- | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **Coordinator** | User-facing routing and orchestration.                                                 | Requires `routes=[...]`; can call routed sub-agents through `call_agent`; stateful by default.                                          |
| **Planner**     | Creating typed plans from a user goal.                                                 | Requires a `plan`; gets `storePlan` and `getPlan`; stateful by default.                                                                 |
| **Executor**    | Running actions from an existing plan.                                                 | Requires a `plan`; gets `getPlan`, `executePlan`, `resolveRef`, and `glimpseRef`; stateless by default.                                 |
| **Specialist**  | Focused domain work such as data lookup, document analysis, or a narrow tool workflow. | No role-specific tools by default; can be called by a coordinator or directly with `runtime.run_specialist(...)`; stateless by default. |

Attach application tools to any agent with `tools=[...]`. Built-in tools are
added by the harness based on the agent type and selected toolkits.

## Common architecture

```text
Coordinator -> Planner -> Approval -> Executor -> Tools
```

This architecture keeps decisions and side effects separate. The coordinator
routes, the planner describes intent, the approval gate pauses sensitive work,
and the executor performs the approved actions through tools.

## Common mistake

Avoid putting all responsibilities into one agent. It usually makes the system
harder to control and harder to debug.

## See also

- [Multi-agent architectures](execution-model.md) for how agent roles fit together.
- [Plans](plans.md) for the contract between planner and executor.
- [Tools](tools.md) for capabilities agents can call.
- [`define_coordinator` reference](../reference/agents.md#flowai_harness.agents.define_coordinator).
