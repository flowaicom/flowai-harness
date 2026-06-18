# Debug system prompts

Use this guide when an agent receives the wrong system prompt, no prompt, or a
stale prompt.

For everyday prompt authoring, start with the
[Prompts concept](../concepts/prompts.md) and the
[`layered_prompt`](../reference/prompts.md#flowai_harness.prompts.layered_prompt)
reference.

## Common symptoms

- the rendered prompt looks right, but the model behaves as if instructions are
  missing
- Studio or a test appears to use an older prompt
- the wrong agent responds to the request
- a tool appears in prompt text but the agent cannot call it

## Check the prompt definition

Confirm the agent is built with the prompt you expect. If you use
`layered_prompt(...)`, inspect the layer inputs before passing the prompt to
`define_coordinator(...)`, `define_planner(...)`, `define_executor(...)`, or
`define_specialist(...)`.

```python
prompt = layered_prompt(
    identity="You create typed plans.",
    operational_rules=["Do not execute plans."],
)

print(str(prompt))
```

## Check the rendered prompt

If you pass a `LayeredPrompt`, the text is rendered before it is attached to the
agent. Empty sections are omitted and structured sections render deterministically.

```python
agent = define_planner(
    name="planner",
    model="claude-sonnet-4-6",
    plan=scenario_plan,
    prompt=prompt,
)

assert "Do not execute plans." in agent.system_prompt
```

## Check the runtime spec

Make sure the runtime is created from the agent spec you just inspected. A stale
`RuntimeSpec`, module-level singleton, or Studio process can make prompt changes
look like they were ignored.

```python
runtime_spec = define_runtime(
    tenant=tenant,
    agents=[coordinator, planner, executor],
)

planner_spec = next(agent for agent in runtime_spec.agents if agent.name == "planner")
assert "Do not execute plans." in planner_spec.system_prompt
```

Restart Studio after changing the exported `FlowAIApp`.

## Check which agent is invoked

`runtime.query(...)` invokes the coordinator. `runtime.run_specialist(...)`
invokes the named specialist directly.

A correct prompt on the wrong agent looks like a prompt bug. Confirm the
entrypoint and routes match the behavior you are testing.

## Check tools separately

Tool descriptions in prompt text are not executable bindings. If a tool appears
in the prompt but cannot be called, inspect the agent's `tools=[...]` and
`toolkits=[...]` configuration.

See [Tool descriptions versus executable tools](../concepts/prompts.md#tool-descriptions-versus-executable-tools).

## Verify it works

Run a deterministic or scripted test that asserts the expected agent receives
the expected prompt-sensitive behavior. For live-provider debugging, confirm the
runtime is using a provider-backed interpreter rather than a deterministic test
path.

## Common causes

| Symptom | Likely cause |
| --- | --- |
| Prompt text changed but Studio still behaves the same | Studio server needs a restart, or the exported app still points at the old spec. |
| Tool is described but unavailable | The tool was rendered in prompt text but not attached to the agent or toolkit. |
| Coordinator instructions affect specialist behavior | The specialist has its own prompt; update the role-specific prompt. |
| Live behavior does not match deterministic tests | Check whether the test uses `testing={...}` or `interpreter="scripted"` while Studio uses a live provider. |

## See also

- [Prompts concept](../concepts/prompts.md)
- [Agents concept](../concepts/agents.md)
- [Test agents without provider calls](testing.md)
- [`layered_prompt` reference](../reference/prompts.md#flowai_harness.prompts.layered_prompt)
