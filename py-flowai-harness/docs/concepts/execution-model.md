# Multi-agent architectures

The harness lets you choose how much structure each use case needs. A run can
be a direct specialist call, a coordinator routing to one or more agents, a
planner/executor flow with a persisted plan, or a tool-using agent with
approval gates around only the sensitive capabilities.

`flowai-harness` gives you runtime controls around model execution: agent
defaults, routed handoffs, typed plan containers, plan and tool approvals,
references, tool dispatch, streamed events, and eval scoring.

## What happens during a run

The exact path depends on the agents and policies you define. A typical run may
include these steps:

1. Your application starts a run with `runtime.query(...)` for a coordinator or
   `runtime.run_specialist(...)` for a direct specialist call.
2. The runtime sends the prompt to the selected agent with its model, system
   prompt, role defaults, tools, and built-in toolkits.
3. The agent can answer directly, call tools, create or resolve references, or
   hand work to routed agents.
4. If there is a planning phase, a planner stores a typed plan and the runtime
   validates and persists it.
5. If approval is required, the runtime pauses a plan or tool call until the
   host application responds. Plan approvals can be approved, rejected, or sent
   back for revision; tool approvals are approved or rejected.
6. An executor can load a plan and execute its actions through tools or the
   action dispatcher once the relevant approval policy is satisfied.
7. The runtime streams events, traces, approvals, usage, and results back to
   your application.

Many runs use only part of this path. For example, a specialist can answer a
question with tools and references without creating a plan.

## Why these controls exist

Raw model calls are hard to control when agents need to work with real data and
real actions.

Flow AI adds optimized controls around the model:

- agents provide role defaults for coordination, planning, execution, and
  specialist work
- plans make proposed actions reviewable, portable between agents, and safe to
  execute after approval
- tools make capabilities explicit
- approvals gate sensitive plans and tool calls
- references keep large or sensitive data outside prompts
- runtime events make execution observable
- eval scorers check trajectories, planned actions, executed actions, and final
  responses without forcing a specific agent architecture

## Choosing how much structure to add

Start with the least structure that protects the work:

- Use a specialist when the task is narrow and your application can select the
  agent directly.
- Add a coordinator when the model should decide where work goes.
- Add tools when the agent needs explicit access to data, APIs, or services.
- Add references when data is large, sensitive, or shared across steps.
- Add approvals when a tool call or plan could affect real systems.
- Add plans when actions should be inspectable, revisable, approved, and passed
  efficiently between planner and executor.

The coordinator-planner-approval-executor architecture is a strong default for
write-capable data agents, but it is not the only option.

## See also

- [Agents](agents.md) for role-specific defaults and routing.
- [Plans](plans.md) for typed action containers and plan lifecycle.
- [Approvals](approvals.md) for plan and tool gates.
- [Runtime](runtime.md) for how applications start runs and observe progress.
