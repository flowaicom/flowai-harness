# Tenants

A tenant identifies the customer, workspace, or environment the runtime is
operating for.

Tenants are how Flow AI keeps runtime state scoped. Plans, references,
approvals, events, and data access should belong to the tenant that created
them.

```python
from flowai_harness import define_tenant

tenant = define_tenant("acme", "v1")
```

## Why tenants exist

Agents often run in multi-customer or multi-workspace environments. Tenant
identity prevents one customer's state from leaking into another customer's run.

It also gives your application a stable way to create one runtime identity for a
customer, workspace, deployment, or test environment.

## What belongs in a tenant

Use a tenant for stable runtime identity:

- customer
- workspace
- environment
- deployment boundary

Pick the tenant id from trusted application state, such as auth or deployment
configuration. Do not derive it from a user prompt.

## Tenant identity versus shared configuration

A tenant is the scope key for runtime-owned state. It is not the place where you
define how an agent behaves.

Prompts, tools, plans, catalog context, and domain knowledge can be reused
across many tenants. For example, every workspace can use the same planner
prompt, while each tenant still gets its own references, plans, approvals,
events, and data-access scope.

Tenant-specific business context should come from explicit configuration,
retrieval, tools, or prompt layers that your application chooses for that
tenant. Do not hide those instructions inside the tenant id.

## How tenants fit with other concepts

- References are created inside a tenant scope.
- Plans belong to a tenant run.
- Approvals are resolved inside the same tenant context.
- Runtime events are emitted for tenant-scoped execution.

## Common mistake

Do not use tenant identity as hidden prompt content. If the model needs business
context, put that context in a prompt layer or retrieve it with a tool.

## See also

- [Runtime](runtime.md) for where the tenant attaches to the executable system.
- [References & glimpses](references.md) for tenant-scoped handles to stored
  values.
- [`define_tenant` reference](../reference/tenant.md).
