# Overview

Flow AI agents are assembled from a small set of harness components: tenants,
catalogs, agents, plans, tools, action dispatchers, references, prompts,
approvals, and the runtime.

These are specialized configuration surfaces with defaults tuned for routed agent roles, typed planning, approval-before-action flows, reference-backed context, and observable execution.

Together they let you describe how an agent system should behave while the
harness handles validation, plan lifecycle, tool wiring, approval pauses, and
runtime events.

## The basic lifecycle

```text
User request
  -> coordinator decides what kind of work is needed
  -> catalog tools ground data and knowledge context
  -> planner creates a typed plan
  -> approval gate pauses risky work
  -> executor calls executePlan
  -> action dispatcher applies approved writes
  -> references carry large or sensitive data between steps
  -> runtime streams events, traces, approvals, and results
```

This lifecycle is useful when an agent needs to decide what should happen
before any sensitive work happens.

## Main harness components

- **Tenant**: scopes runtime identity and tenant-specific state.
- **Catalog**: stores scoped data and knowledge metadata that agents can search,
  hydrate, inspect, and use for read-only data workflows.
- **Agent**: a configured agent type, such as a coordinator, planner, executor,
  or specialist, with defaults for that role.
- **Plan**: a typed container for actions that can be reviewed, approved,
  safely executed, and passed between sub-agents through an efficient context
  handoff.
- **Tool**: a callable capability exposed to an agent.
- **Action dispatcher**: the host callback that applies approved plan actions
  to your platform.
- **Reference**: a handle to large or sensitive data stored outside the prompt.
- **Glimpse**: a small summary of a referenced value.
- **Approval**: a runtime gate for sensitive tools or plans that include write
  actions.
- **Runtime**: the running harness handle that validates definitions, routes
  agents, manages plans and approvals, dispatches tools, handles references,
  and streams events.
- **Prompt**: layered instructions that shape each agent's behavior.

## What to read next

- New to `flowai-harness`? Start with
  [Multi-agent architectures](execution-model.md).
- Building data or knowledge agents? Read [Catalog](catalog.md).
- Building a multi-agent system? Read [Agents](agents.md).
- Plans and actions? Read [Plans](plans.md).
- Applying approved writes? Read [Action dispatcher](action-dispatcher.md).
- Passing large data? Read [References & glimpses](references.md).
- Gating actions and/or tools? Read [Approvals](approvals.md).
- Running the system? Read [Runtime](runtime.md).
