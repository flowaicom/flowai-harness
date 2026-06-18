# Tenant

A tenant identifies the customer, workspace, deployment, or test environment
that a runtime is executing for. Flow AI uses tenant identity to scope
runtime-owned state such as plans, references, approvals, events, and data
access.

Use `define_tenant(...)` to create the
[`TenantIdentity`](#flowai_harness.tenant.TenantIdentity) passed into
`define_runtime(...)`. Choose tenant ids from trusted application state such as
auth, workspace configuration, or deployment config. Do not use tenant identity
as hidden prompt context; put business instructions in prompts, tools, or
retrieved configuration instead.

For the full mental model, see the [Tenants concept](../concepts/tenant.md).

::: flowai_harness.tenant.define_tenant

::: flowai_harness.tenant.TenantIdentity
