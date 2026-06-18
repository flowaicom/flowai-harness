//! Workspace-specific `ToolEnvironment` ergonomics.
//!
//! `agent-fw-tool` cannot depend on `agent-fw-workspace` without creating a
//! crate cycle, so workspace accessors live here as an extension trait.

use std::sync::Arc;

use agent_fw_tool::{ToolEnvironment, ToolError};

use crate::WorkspaceStore;

/// First-class workspace helpers for [`ToolEnvironment`].
pub trait WorkspaceToolEnvironmentExt {
    /// Register a workspace store as a common tool capability.
    fn with_workspace(self, workspace: Arc<dyn WorkspaceStore>) -> Self;

    /// Retrieve an optional workspace store capability.
    fn maybe_workspace(&self) -> Option<&Arc<dyn WorkspaceStore>>;

    /// Retrieve a required workspace store capability, panicking if missing.
    fn workspace(&self) -> &Arc<dyn WorkspaceStore>;

    /// Retrieve a required workspace store capability as a tool error.
    fn try_workspace(&self) -> Result<&Arc<dyn WorkspaceStore>, ToolError>;
}

impl WorkspaceToolEnvironmentExt for ToolEnvironment {
    fn with_workspace(self, workspace: Arc<dyn WorkspaceStore>) -> Self {
        self.with_ext::<dyn WorkspaceStore>(workspace)
    }

    fn maybe_workspace(&self) -> Option<&Arc<dyn WorkspaceStore>> {
        self.maybe_ext::<dyn WorkspaceStore>()
    }

    fn workspace(&self) -> &Arc<dyn WorkspaceStore> {
        self.expect_ext::<dyn WorkspaceStore>()
    }

    fn try_workspace(&self) -> Result<&Arc<dyn WorkspaceStore>, ToolError> {
        self.try_ext::<dyn WorkspaceStore>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_core::{id::TenantId, tenant::TenantContext};
    use agent_fw_interpreter::DashMapKVStore;

    use crate::KVWorkspaceStore;

    fn test_env() -> ToolEnvironment {
        ToolEnvironment::builder()
            .kv(NullKVStore)
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("test-tenant")))
            .build()
    }

    #[test]
    fn workspace_capability_round_trips() {
        let workspace: Arc<dyn WorkspaceStore> =
            Arc::new(KVWorkspaceStore::new(Arc::new(DashMapKVStore::new())));
        let env = test_env().with_workspace(Arc::clone(&workspace));

        assert!(Arc::ptr_eq(env.workspace(), &workspace));
        assert!(Arc::ptr_eq(env.try_workspace().unwrap(), &workspace));
        assert!(env.maybe_workspace().is_some());
    }

    // Ensure the trait object remains object-safe enough for storage in ToolEnvironment.
    #[allow(dead_code)]
    fn _workspace_object_safe(_: Arc<dyn WorkspaceStore>) {}
}
