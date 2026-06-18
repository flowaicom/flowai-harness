//! Workspace lifecycle service — create, update, delete with compensating cleanup.
//!
//! Composes [`DatabaseProvisioner`], [`WorkspaceEntityStore`], and [`KVStore`]
//! with the `compensating` pattern for saga-style resource safety.
//!
//! # Architecture
//!
//! ```text
//! create_workspace:
//!   validate(input)     → pure
//!   provision(slug)     → effectful
//!   store(ws)           → effectful, compensating-protected
//!   assemble(ws)        → pure
//!
//! delete_workspace:
//!   load(id)            → effectful
//!   remove(id)          → effectful (authoritative state change)
//!   deprovision(env)    → effectful, best-effort
//!   cleanup(kv)         → effectful, best-effort
//! ```
//!
//! The `compensating` combinator from `agent-fw-algebra::resource` ensures that
//! provisioned environments are deprovisioned *only if* a later step fails.
//! Unlike `bracket` (which always releases), `compensating` preserves the
//! provisioned resource on success — correct for saga-style workflows.
//!
//! # Example
//!
//! ```ignore
//! use agent_fw_workspace::lifecycle;
//!
//! let ws = lifecycle::create_workspace(
//!     CreateWorkspaceInput { name: "Demo".into(), description: None },
//!     &tenant_id,
//!     provisioner.as_ref(),
//!     store.as_ref(),
//!     None, // parent_id
//!     "analyst",
//! ).await?;
//! ```

use std::time::Duration;

use chrono::Utc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use agent_fw_algebra::kv_store::KVStore;
use agent_fw_algebra::resource::compensating;
use agent_fw_catalog::{
    DatabaseProvisioner, EnvironmentId, EnvironmentName, ProvisionRequest, ProvisioningError,
};
use agent_fw_core::{TenantId, WorkspaceId};

use crate::store::{WorkspaceEntityStore, WorkspaceError};
use crate::workspace::{slugify, DatabaseConfig, Workspace, WorkspaceModelConfig};

// =============================================================================
// Input / Error Types
// =============================================================================

/// Input for creating a new workspace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub description: Option<String>,
}

/// Input for updating an existing workspace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceInput {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Errors from workspace lifecycle operations.
#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Provisioning failed: {0}")]
    Provisioning(#[from] ProvisioningError),

    #[error("Store error: {0}")]
    Store(#[from] WorkspaceError),

    #[error("Workspace not found: {0}")]
    NotFound(String),

    #[error("KV store error: {0}")]
    KVStore(String),
}

// =============================================================================
// Pure Functions
// =============================================================================

/// Validate workspace creation input.
///
/// # Laws
/// - Name must be non-empty after trimming.
/// - Slug must be non-"unnamed" (i.e., name must produce a valid slug).
fn validate_workspace_input(input: &CreateWorkspaceInput) -> Result<String, LifecycleError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(LifecycleError::Validation(
            "Workspace name must not be empty".into(),
        ));
    }

    let slug = slugify(name);
    if slug == "unnamed" {
        return Err(LifecycleError::Validation(
            "Workspace name must produce a valid slug".into(),
        ));
    }

    Ok(slug)
}

/// Assemble a Workspace entity from validated components.
fn assemble_workspace(
    id: WorkspaceId,
    name: &str,
    slug: &str,
    description: Option<&str>,
    db_config: DatabaseConfig,
) -> Workspace {
    let now = Utc::now();
    Workspace {
        id,
        name: name.trim().to_string(),
        slug: slug.to_string(),
        description: description.map(|s| s.to_string()),
        database_config: db_config,
        model_config: WorkspaceModelConfig::default(),
        created_at: now,
        updated_at: now,
    }
}

// =============================================================================
// Effectful Operations
// =============================================================================

/// Create a new workspace with optional database provisioning.
///
/// Uses `compensating` for saga-style cleanup: if workspace store creation
/// fails after provisioning, the provisioned environment is deprovisioned.
/// On success, the provisioned environment persists (unlike `bracket` which
/// always releases).
///
/// # Arguments
///
/// - `input` — Name and description for the workspace.
/// - `tenant` — Tenant context.
/// - `provisioner` — Optional database provisioner. If `None`, creates a
///   workspace with `DatabaseConfig::Default`.
/// - `store` — Workspace entity store.
/// - `parent_id` — Optional parent environment for branch-based provisioning.
/// - `role` — Database role name for the provisioned connection.
pub async fn create_workspace(
    input: CreateWorkspaceInput,
    tenant: &TenantId,
    provisioner: Option<&dyn DatabaseProvisioner>,
    store: &dyn WorkspaceEntityStore,
    parent_id: Option<EnvironmentId>,
    role: &str,
) -> Result<Workspace, LifecycleError> {
    // Phase 1: Pure validation
    let slug = validate_workspace_input(&input)?;

    // Phase 1b: Slug uniqueness check — prevents duplicate workspace names
    let existing = store.list_workspaces(tenant).await?;
    if existing.iter().any(|ws| ws.slug == slug) {
        return Err(LifecycleError::Validation(format!(
            "Workspace with slug '{slug}' already exists"
        )));
    }

    let ws_id = WorkspaceId::new_unchecked(Uuid::new_v4().to_string());
    debug!(ws_id = %ws_id, slug = %slug, "Creating workspace");

    // Phase 2: Provision environment (if provisioner available)
    let (db_config, env_id) = match provisioner {
        Some(prov) => {
            let env_name = EnvironmentName::new(format!("ws-{slug}"));
            info!(env_name = %env_name, "Provisioning database environment");

            let env = prov
                .provision(ProvisionRequest {
                    name: env_name,
                    parent_id,
                    expires_at: None,
                })
                .await?;

            let config = DatabaseConfig::Managed {
                branch_id: env.id.to_string(),
                branch_name: env.name.to_string(),
                host: env.host.clone(),
                target_url: format!("postgresql://{}@{}/target", role, env.host),
                catalog_url: format!("postgresql://{}@{}/catalog", role, env.host),
                embeddings_url: format!("postgresql://{}@{}/embeddings", role, env.host),
            };

            info!(env_id = %env.id, host = %env.host, "Environment provisioned");
            (config, Some(env.id))
        }
        None => {
            debug!("No provisioner — using DatabaseConfig::Default");
            (DatabaseConfig::Default, None)
        }
    };

    // Phase 3: Pure assembly
    let workspace = assemble_workspace(
        ws_id,
        &input.name,
        &slug,
        input.description.as_deref(),
        db_config,
    );

    // Phase 4: Store — wrapped in `compensating` to deprovision on failure.
    // `compensating` only runs cleanup on Err; on Ok the environment persists.
    compensating(store.create_workspace(tenant, &workspace), || async {
        if let Some(ref eid) = env_id {
            if let Some(prov) = provisioner {
                warn!(env_id = %eid, "Store failed — compensating: deprovisioning environment");
                let cleanup = prov.deprovision(eid);
                match tokio::time::timeout(Duration::from_secs(10), cleanup).await {
                    Ok(Ok(())) => info!(env_id = %eid, "Compensating deprovision succeeded"),
                    Ok(Err(e)) => {
                        warn!(env_id = %eid, error = %e, "Compensating deprovision failed")
                    }
                    Err(_) => warn!(env_id = %eid, "Compensating deprovision timed out"),
                }
            }
        }
    })
    .await?;

    info!(ws_id = %workspace.id, slug = %workspace.slug, "Workspace created");
    Ok(workspace)
}

/// Delete a workspace, deprovisioning any managed databases.
///
/// # Ordering Rationale
///
/// Store deletion happens *before* deprovisioning. If deprovisioning fails
/// (network, timeout), we prefer an orphaned environment (discoverable via
/// `provisioner.list_environments()`) over an orphaned workspace record
/// pointing to a nonexistent environment (which would cause errors on every
/// user interaction).
pub async fn delete_workspace(
    tenant: &TenantId,
    workspace_id: &str,
    provisioner: Option<&dyn DatabaseProvisioner>,
    store: &dyn WorkspaceEntityStore,
    kv: Option<&dyn KVStore>,
) -> Result<(), LifecycleError> {
    debug!(workspace_id = %workspace_id, "Deleting workspace");

    // Load workspace (capture db config before deletion)
    let workspace = store
        .get_workspace(tenant, workspace_id)
        .await?
        .ok_or_else(|| LifecycleError::NotFound(workspace_id.to_string()))?;

    let db_config = workspace.database_config.clone();

    // Remove from store first — this is the authoritative state change.
    // If this fails, nothing has changed and we return the error.
    store.delete_workspace(tenant, workspace_id).await?;

    // Deprovision managed databases (best-effort, after store deletion)
    if let DatabaseConfig::Managed { branch_id, .. } = &db_config {
        if let Some(prov) = provisioner {
            let env_id = EnvironmentId::new(branch_id);
            info!(env_id = %env_id, "Deprovisioning managed environment");
            match prov.deprovision(&env_id).await {
                Ok(()) => info!(env_id = %env_id, "Deprovision succeeded"),
                Err(ProvisioningError::NotFound(_)) => {
                    debug!(env_id = %env_id, "Environment already gone")
                }
                Err(e) => {
                    warn!(env_id = %env_id, error = %e, "Deprovision failed (best-effort)")
                }
            }
        }
    }

    // Cleanup KV data (best-effort, prefix-based)
    // Uses list_keys to discover all keys under the workspace prefix,
    // rather than hardcoding patterns that would silently miss new key types.
    if let Some(kv) = kv {
        let tenant_str = tenant.as_str();
        let prefix = format!("ws:{}:", workspace_id);
        match kv.list_keys(tenant_str, &prefix).await {
            Ok(keys) => {
                for key in &keys {
                    let _ = kv.delete(tenant_str, key).await;
                }
                if !keys.is_empty() {
                    debug!(workspace_id = %workspace_id, keys = keys.len(), "Cleaned up KV data");
                }
            }
            Err(e) => {
                warn!(workspace_id = %workspace_id, error = %e, "KV key listing failed (best-effort)");
            }
        }
    }

    info!(workspace_id = %workspace_id, "Workspace deleted");
    Ok(())
}

/// Update an existing workspace's name and/or description.
pub async fn update_workspace(
    tenant: &TenantId,
    workspace_id: &str,
    input: UpdateWorkspaceInput,
    store: &dyn WorkspaceEntityStore,
) -> Result<Workspace, LifecycleError> {
    let mut workspace = store
        .get_workspace(tenant, workspace_id)
        .await?
        .ok_or_else(|| LifecycleError::NotFound(workspace_id.to_string()))?;

    if let Some(name) = &input.name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(LifecycleError::Validation(
                "Workspace name must not be empty".into(),
            ));
        }
        workspace.name = trimmed.to_string();
        workspace.slug = slugify(trimmed);
    }

    if let Some(desc) = input.description {
        workspace.description = Some(desc);
    }

    workspace.updated_at = Utc::now();

    store.update_workspace(tenant, &workspace).await?;

    Ok(workspace)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn validate_good_input() {
        let input = CreateWorkspaceInput {
            name: "My Workspace".into(),
            description: None,
        };
        let slug = validate_workspace_input(&input).unwrap();
        assert_eq!(slug, "my-workspace");
    }

    #[test]
    fn validate_empty_name_fails() {
        let input = CreateWorkspaceInput {
            name: "".into(),
            description: None,
        };
        assert!(validate_workspace_input(&input).is_err());
    }

    #[test]
    fn validate_whitespace_name_fails() {
        let input = CreateWorkspaceInput {
            name: "   ".into(),
            description: None,
        };
        assert!(validate_workspace_input(&input).is_err());
    }

    #[test]
    fn assemble_workspace_has_correct_fields() {
        let ws = assemble_workspace(
            WorkspaceId::new_unchecked("ws-1"),
            "Demo",
            "demo",
            Some("A demo workspace"),
            DatabaseConfig::Default,
        );
        assert_eq!(ws.name, "Demo");
        assert_eq!(ws.slug, "demo");
        assert_eq!(ws.description, Some("A demo workspace".to_string()));
        assert!(matches!(ws.database_config, DatabaseConfig::Default));
    }

    #[tokio::test]
    async fn create_workspace_without_provisioner() {
        use crate::kv_store::KVWorkspaceStore;
        use agent_fw_interpreter::DashMapKVStore;

        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv);
        let tenant = TenantId::new_unchecked("test-tenant");

        let input = CreateWorkspaceInput {
            name: "Test Workspace".into(),
            description: Some("A test".into()),
        };

        let ws = create_workspace(input, &tenant, None, &store, None, "analyst")
            .await
            .expect("create should succeed");

        assert_eq!(ws.name, "Test Workspace");
        assert_eq!(ws.slug, "test-workspace");
        assert!(matches!(ws.database_config, DatabaseConfig::Default));

        // Verify it was stored
        let loaded = store.get_workspace(&tenant, ws.id.as_str()).await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn create_duplicate_slug_fails() {
        use crate::kv_store::KVWorkspaceStore;
        use agent_fw_interpreter::DashMapKVStore;

        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv);
        let tenant = TenantId::new_unchecked("test-tenant");

        // Create first workspace
        let input = CreateWorkspaceInput {
            name: "My Workspace".into(),
            description: None,
        };
        create_workspace(input, &tenant, None, &store, None, "analyst")
            .await
            .expect("first create should succeed");

        // Attempt duplicate slug
        let input2 = CreateWorkspaceInput {
            name: "My Workspace".into(),
            description: Some("duplicate".into()),
        };
        let result = create_workspace(input2, &tenant, None, &store, None, "analyst").await;
        assert!(
            matches!(result, Err(LifecycleError::Validation(ref msg)) if msg.contains("already exists")),
            "Expected Validation error for duplicate slug, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn update_workspace_changes_name() {
        use crate::kv_store::KVWorkspaceStore;
        use agent_fw_interpreter::DashMapKVStore;

        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv);
        let tenant = TenantId::new_unchecked("test-tenant");

        // Create first
        let input = CreateWorkspaceInput {
            name: "Original".into(),
            description: None,
        };
        let ws = create_workspace(input, &tenant, None, &store, None, "analyst")
            .await
            .unwrap();

        // Update
        let update = UpdateWorkspaceInput {
            name: Some("Renamed".into()),
            description: Some("Now with description".into()),
        };
        let updated = update_workspace(&tenant, ws.id.as_str(), update, &store)
            .await
            .unwrap();

        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.slug, "renamed");
        assert_eq!(
            updated.description,
            Some("Now with description".to_string())
        );
    }

    #[tokio::test]
    async fn delete_workspace_removes_from_store() {
        use crate::kv_store::KVWorkspaceStore;
        use agent_fw_interpreter::DashMapKVStore;

        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv.clone());
        let tenant = TenantId::new_unchecked("test-tenant");

        // Create
        let input = CreateWorkspaceInput {
            name: "To Delete".into(),
            description: None,
        };
        let ws = create_workspace(input, &tenant, None, &store, None, "analyst")
            .await
            .unwrap();

        // Delete
        delete_workspace(&tenant, ws.id.as_str(), None, &store, Some(kv.as_ref()))
            .await
            .unwrap();

        // Verify gone
        let loaded = store.get_workspace(&tenant, ws.id.as_str()).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        use crate::kv_store::KVWorkspaceStore;
        use agent_fw_interpreter::DashMapKVStore;

        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new(kv);
        let tenant = TenantId::new_unchecked("test-tenant");

        let result = delete_workspace(&tenant, "nonexistent", None, &store, None).await;
        assert!(matches!(result, Err(LifecycleError::NotFound(_))));
    }

    // =========================================================================
    // Compensating integration test
    // =========================================================================

    /// A WorkspaceEntityStore that always fails on `create_workspace`.
    /// Used to verify the `compensating` combinator deprovisioning behavior.
    struct FailingCreateStore;

    #[async_trait::async_trait]
    impl WorkspaceEntityStore for FailingCreateStore {
        async fn list_workspaces(
            &self,
            _tenant: &TenantId,
        ) -> Result<Vec<crate::workspace::Workspace>, WorkspaceError> {
            Ok(vec![])
        }

        async fn get_workspace(
            &self,
            _tenant: &TenantId,
            _workspace_id: &str,
        ) -> Result<Option<crate::workspace::Workspace>, WorkspaceError> {
            Ok(None)
        }

        async fn create_workspace(
            &self,
            _tenant: &TenantId,
            _workspace: &crate::workspace::Workspace,
        ) -> Result<(), WorkspaceError> {
            Err(WorkspaceError::Db("simulated store failure".into()))
        }

        async fn update_workspace(
            &self,
            _tenant: &TenantId,
            _workspace: &crate::workspace::Workspace,
        ) -> Result<(), WorkspaceError> {
            Ok(())
        }

        async fn delete_workspace(
            &self,
            _tenant: &TenantId,
            _workspace_id: &str,
        ) -> Result<(), WorkspaceError> {
            Ok(())
        }
    }

    /// Verifies L2 of `compensating`: when store.create_workspace fails after
    /// provisioning, the provisioned environment is deprovisioned (cleaned up).
    #[tokio::test]
    async fn create_workspace_compensates_on_store_failure() {
        use agent_fw_interpreter::MockProvisioner;

        let provisioner = MockProvisioner::new();
        let store = FailingCreateStore;
        let tenant = TenantId::new_unchecked("test-tenant");

        let input = CreateWorkspaceInput {
            name: "Doomed Workspace".into(),
            description: None,
        };

        // Provision should succeed, then store should fail, then compensating
        // should deprovision the environment.
        let result =
            create_workspace(input, &tenant, Some(&provisioner), &store, None, "analyst").await;

        assert!(result.is_err(), "create should fail (store failure)");
        assert!(
            matches!(result, Err(LifecycleError::Store(_))),
            "error should be Store variant"
        );

        // The compensating combinator should have deprovisioned the environment.
        let envs = provisioner.list_environments().await.unwrap();
        assert!(
            envs.is_empty(),
            "L2 violated: provisioned environment should be deprovisioned after store failure, but found {} environments",
            envs.len()
        );
    }
}
