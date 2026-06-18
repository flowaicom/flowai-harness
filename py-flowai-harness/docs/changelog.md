# Changelog

## 0.1.0a1 — 2026-06-18

!!! warning "Alpha"
    `0.1.0a1` is the Python package version for the `v0.1.0-alpha.1` public
    alpha tag. The public API is stable enough to build against, but breaking
    changes are still possible during the alpha cycle.

### Added

#### Package skeleton

- Initial `flowai-harness` package skeleton.
- Validated spec constructors for tenants, plans, references, and tools.
- `flowai_harness._internal` PyO3 extension module embedded privately in the wheel.

#### Public API surface

- Ergonomic Python public API for building role agents:
  `define_coordinator`, `define_planner`, `define_executor`, `define_specialist`.
- `layered_prompt(...)` with deterministic rendering and a SHA-256 cache key.
- Customer-configured glimpses on `define_reference(..., glimpse=...)` and the
  schema-neutral `glimpse(...)` helper.
- `TaggedUnion(...)` for tagged action unions with `kind` discriminator.

#### Native runtime handle

- `create_runtime(...)` returning a native Rust-backed `Runtime`.
- `Runtime.query(...)` async-iterable event streams.
- `Runtime.respond_to_approval(...)` for approval resume.
- `Runtime.run_specialist(...)` for direct specialist dispatch through the native handle.
- `Runtime.create_reference(...)`, `Runtime.resolve_reference(...)`, and
  `Runtime.reference_glimpse(...)` for host-created typed references, plus `ctx.references`
  for pointer-producing Python tools.
- `create_runtime(..., data_environment=...)` for attaching Rust-owned catalog and SQLite
  target database dependencies to built-in `catalog` toolkit dispatch.
- `create_runtime(..., services=...)` for injecting customer-owned Python services into
  custom tool callbacks as `ctx.<service>`, `ctx["<service>"]`, and `ctx.services`.
- `TestingConfig({"mock_response": ...})` deterministic no-network interpreter for unit tests.

#### MCP tool serving

- `flowai_harness.mcp` helpers for listing and serving runtime tools over MCP stdio and
  Streamable HTTP.
- `Runtime.list_mcp_tools(...)`, `Runtime.serve_mcp_stdio(...)`, and
  `Runtime.serve_mcp_http(...)` on the native runtime handle.
- `flowai-harness mcp python MODULE:OBJECT ...` for Python-defined callback servers.
- `flowai-harness mcp toolkit ...` for Rust-native toolkit-only servers.

### Changed

#### Tenant and prompt layer cleanup

- Replaced project-specific runtime identity with `TenantIdentity` and `define_tenant(...)`.
- `define_runtime(...)` now accepts `tenant=...`; runtime isolation is keyed by `resource_id`.
- `layered_prompt(...)` uses explicit prompt layers, including `domain_knowledge` and
  `operational_rules`, independently from runtime tenancy.

### Breaking

- The alpha API may still break before `1.0.0` is finalized.

### Migration

- No migration steps are required for the first pre-release.
