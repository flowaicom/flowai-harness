# Prompts

Prompts define how each agent should behave.

Flow AI encourages layered prompts so different kinds of instructions stay
separate and easier to maintain.

## Why layered prompts?

Agent prompts often mix many concerns:

- identity
- communication style
- routing rules
- tool-use rules
- domain knowledge
- safety rules
- output format
- examples

Layering keeps those concerns explicit.

## Common layers

- **Identity**: who the agent is.
- **Communication**: how the agent should speak.
- **Operational rules**: what the agent should do.
- **Tools**: how and when to use tools.
- **Domain knowledge**: business-specific context.
- **Safety**: what the agent must avoid.
- **Output format**: expected response shape.
- **Examples**: demonstrations of desired behavior.

```python
from flowai_harness import layered_prompt

planner_prompt = layered_prompt(
    identity="You create typed pricing scenario plans.",
    operational_rules=[
        "Create plans only; do not execute them.",
        "Use references for large product sets.",
    ],
    domain_knowledge={
        "entities": ["product", "segment", "channel"],
        "metrics": ["revenue", "margin"],
    },
    safety=["Never propose changing prices without approval."],
)
```

## Agent-specific prompts

Different roles need different instructions.

Coordinator prompts should focus on routing.

Planner prompts should focus on creating valid plans.

Executor prompts should focus on executing approved plans.

Specialist prompts should focus on a narrow capability.

## Tool descriptions versus executable tools

Tool text in a prompt helps the model understand what a tool is for. It does not
make that tool executable by itself.

Executable tools are attached to agents and made available through the runtime.
Keep the prompt description aligned with the actual tool binding so the model
does not try to call a capability the agent does not have.

## Tenant vs domain knowledge

A tenant identifies the runtime scope.

Domain knowledge explains the business context.

Do not use tenants as a place to store prompt instructions.

## Common mistake

Avoid one giant prompt that mixes role, policy, domain knowledge, and output
format. Split those concerns into clear layers.

## See also

- [Agents](agents.md) for role-specific prompt guidance.
- [Tools](tools.md) for capabilities mentioned in prompts.
- [Debug system prompts](../guides/system-prompts.md) for troubleshooting.
- [`layered_prompt` reference](../reference/prompts.md#flowai_harness.prompts.layered_prompt).
