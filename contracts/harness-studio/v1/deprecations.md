# Harness Studio v1 Deprecation Policy

## Canonical Paths

New Harness Studio modules should use workspace-scoped paths:

```text
/api/workspaces/{workspace_key}/...
```

Default-workspace aliases are allowed for simple local apps and backwards-compatible getting-started examples:

```text
/api/runtime
/api/agents
/api/agents/{agent_id}/stream
```

Aliases must resolve to the configured `defaultWorkspaceKey`.

## Legacy `agent-fw` Studio Routes

Old routes from the framework Studio and Python `agent-fw` server are compatibility routes during migration. Examples include:

```text
/api/studio/status
/api/studio/project
/api/model-config
/api/chat-with-abort
/api/threads
/api/evals
/api/tests
/api/data/profile
```

They are not canonical Harness Studio API paths. New modules should not add dependencies on them.

## Compatibility Rules

- A deprecated route needs a documented replacement.
- A deprecated response field needs a replacement field or capability flag.
- Removal requires one migration window after the replacement exists.
- Compatibility routes may be implemented by a legacy adapter, but the shared UI should target `harness-studio/v1`.

## Breaking Changes

Breaking changes require a new version such as `harness-studio/v2`.

Breaking changes include:

- Removing a required field.
- Changing the meaning or type of a required field.
- Changing stream sequencing semantics.
- Changing route ownership from workspace-scoped to global.
- Returning raw secrets where v1 only returned references.

Additive fields, new event kinds, new capabilities, and new optional endpoints can remain in v1 when old clients can ignore them safely.
