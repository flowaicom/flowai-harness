# Harness Studio v1 Conformance

This document defines the reusable test strategy for every `harness-studio/v1` implementation.

Implementations:

- Python local Studio server in `py-flowai-harness`.
- Future TypeScript harness Studio server.
- Enterprise Studio API gateway/control plane.

## Required Checks

### Version Discovery

- `GET /api/status` returns `studioApiVersion: "harness-studio/v1"`.
- `supportedVersions` includes `harness-studio/v1`.
- A server does not advertise versions it does not pass.

### Workspace Scoping

- `GET /api/workspaces` returns at least one workspace and a valid `defaultWorkspaceKey`.
- Every workspace-scoped endpoint rejects unknown workspace keys with a stable error.
- Default aliases resolve to the configured default workspace.
- Data, tests, evals, runs, and threads are not shared across workspace keys unless explicitly configured by the app.

### REST DTOs

- Responses validate against `openapi.yaml`.
- Unknown additive fields are allowed, but required fields must remain stable.
- Secret-bearing fields expose only secret references or credential statuses.

### SSE Streams

- Events validate against the `StudioEvent` envelope.
- `seq` is monotonic per stream.
- Event `kind` values match documented fixture kinds.
- Reconnectable streams replay events when `Last-Event-ID` is supported by the implementation.
- Streams terminate with a terminal event for completed, failed, or cancelled runs.

### Capabilities

- Disabled modules are represented as disabled capabilities, not missing arbitrary routes.
- Enterprise-only capabilities are reported with `scope: "enterprise"`.
- The UI can decide visibility from capabilities without knowing the server implementation language.

### Security

- Raw provider API keys are never returned.
- Raw database passwords are never returned.
- Enterprise-only auth and RBAC claims are additive and do not change the core DTO shape.

## Fixture-Based Tests

The shared conformance harness should load:

```text
contracts/harness-studio/v1/openapi.yaml
contracts/harness-studio/v1/fixtures/*.json
contracts/harness-studio/v1/fixtures/sse/*.json
```

Then it should run:

1. JSON schema validation for REST examples.
2. JSON schema validation for SSE examples.
3. Golden response tests against a minimal local app.
4. Workspace isolation tests with two configured workspace keys.
5. Default alias tests.
6. Capability gating tests.
7. Secret redaction tests.

Until the full server conformance harness exists, run the local drift check:

```bash
python3 contracts/harness-studio/v1/validate_contract.py
```

The drift check validates JSON parsing, required endpoint strings, status version fields, required SSE event kinds, and obvious secret-shaped keys in fixtures.

## Minimum Local Test App

Each implementation should provide a test fixture app with:

- One default workspace.
- One coordinator agent.
- One specialist agent.
- One host callback tool.
- One toolkit-provided tool.
- One data source with no credential.
- One data source with a secret reference.
- One pending approval fixture.
- One test case and one eval run fixture.

This fixture is intentionally small but touches every M0 contract surface.
