//! Pure workspace context for multi-tenant runtime boundaries.
//!
//! This is a description value: it contains no IO and no authorization logic.
//! Interpreters decide how to apply it to request headers, KV tenants, or
//! database handles.

use serde::{Deserialize, Serialize};

use crate::{TenantId, WorkspaceId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContext {
    pub base_tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

impl WorkspaceContext {
    pub fn from_ids(base_tenant_id: TenantId, workspace_id: Option<&str>) -> Self {
        Self {
            base_tenant_id,
            workspace_id: normalize_workspace_id(workspace_id),
            profile_id: None,
            bundle_id: None,
        }
    }

    pub fn with_profile_id(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_bundle_id(mut self, bundle_id: impl Into<String>) -> Self {
        self.bundle_id = Some(bundle_id.into());
        self
    }

    pub fn is_default_workspace(&self) -> bool {
        self.workspace_id.is_default()
    }

    pub fn workspace_tenant_id(&self) -> TenantId {
        if self.is_default_workspace() {
            return self.base_tenant_id.clone();
        }
        TenantId::new_unchecked(format!(
            "{}::workspace:{}",
            escape_workspace_tenant_component(self.base_tenant_id.as_str()),
            escape_workspace_tenant_component(self.workspace_id.as_str())
        ))
    }

    pub fn workspace_header_value(&self) -> &str {
        self.workspace_id.as_str()
    }
}

fn escape_workspace_tenant_component(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '%' => escaped.push_str("%25"),
            ':' => escaped.push_str("%3A"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn normalize_workspace_id(workspace_id: Option<&str>) -> WorkspaceId {
    let Some(workspace_id) = workspace_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return WorkspaceId::default_workspace();
    };
    WorkspaceId::new(workspace_id).unwrap_or_else(WorkspaceId::default_workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_context_defaults_to_base_tenant_and_explicit_default_header() {
        let context = WorkspaceContext::from_ids(TenantId::new_unchecked("tenant-a"), Some("   "));

        assert_eq!(context.workspace_id.as_str(), "default");
        assert_eq!(context.workspace_tenant_id().as_str(), "tenant-a");
        assert_eq!(context.workspace_header_value(), "default");
        assert!(context.is_default_workspace());
    }

    #[test]
    fn workspace_context_derives_non_default_workspace_tenant() {
        let context =
            WorkspaceContext::from_ids(TenantId::new_unchecked("tenant-a"), Some(" customer-a "))
                .with_profile_id("profile-1")
                .with_bundle_id("bundle-1");

        assert_eq!(context.workspace_id.as_str(), "customer-a");
        assert_eq!(
            context.workspace_tenant_id().as_str(),
            "tenant-a::workspace:customer-a"
        );
        assert_eq!(context.workspace_header_value(), "customer-a");
        assert!(!context.is_default_workspace());
        assert_eq!(context.profile_id.as_deref(), Some("profile-1"));
        assert_eq!(context.bundle_id.as_deref(), Some("bundle-1"));
    }

    #[test]
    fn workspace_tenant_id_escapes_delimiters_to_avoid_pair_collisions() {
        let left = WorkspaceContext::from_ids(
            TenantId::new_unchecked("tenant-a::workspace:analytics"),
            Some("reports"),
        );
        let right = WorkspaceContext::from_ids(
            TenantId::new_unchecked("tenant-a"),
            Some("analytics::workspace:reports"),
        );

        assert_ne!(left.workspace_tenant_id(), right.workspace_tenant_id());
        assert!(left.workspace_tenant_id().as_str().contains("%3A"));
        assert!(right.workspace_tenant_id().as_str().contains("%3A"));
    }
}
