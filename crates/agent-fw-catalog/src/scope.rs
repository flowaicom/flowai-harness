//! Catalog access scope.
//!
//! `CatalogScope` is an access-control boundary attached to a catalog
//! interpreter or read/write context. It is not a semantic catalog property and
//! must not be encoded into `CatalogEntry::metadata`.
//!
//! `database_id` identifies a target data source inside a workspace; it is not
//! an authorization boundary and must not substitute for this tenant/workspace
//! scope. Agent-facing and Studio-facing catalog instances are expected to be
//! scoped before insertion into a `ToolEnvironment`. Shared catalog backends
//! must persist `tenant_id` and `workspace_id` as first-class storage columns.

use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{TenantId, WorkspaceId};

/// Tenant/workspace boundary for catalog reads, writes, and graph traversal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogScope {
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
}

impl CatalogScope {
    pub fn new(tenant_id: TenantId, workspace_id: WorkspaceId) -> Self {
        Self {
            tenant_id,
            workspace_id,
        }
    }

    /// Compatibility scope for legacy callers that have not yet selected a
    /// tenant/workspace. Agent-facing code should pass an explicit scope.
    pub fn legacy_unscoped() -> Self {
        Self {
            tenant_id: TenantId::new_unchecked("legacy"),
            workspace_id: WorkspaceId::default_workspace(),
        }
    }

    pub fn tenant_context(&self) -> TenantContext {
        TenantContext::new(self.tenant_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_scope_tenant_context_preserves_resource_id() {
        let tenant_id = TenantId::new_unchecked("tenant-a");
        let scope = CatalogScope::new(tenant_id.clone(), WorkspaceId::new_unchecked("workspace-a"));

        assert_eq!(scope.tenant_context().resource_id(), &tenant_id);
    }

    #[test]
    fn semantic_scope_distinguishes_tenants_with_same_workspace() {
        let workspace_id = WorkspaceId::new_unchecked("shared-workspace");
        let scope_a = CatalogScope::new(TenantId::new_unchecked("tenant-a"), workspace_id.clone());
        let scope_b = CatalogScope::new(TenantId::new_unchecked("tenant-b"), workspace_id);

        assert_ne!(scope_a, scope_b);
    }
}
