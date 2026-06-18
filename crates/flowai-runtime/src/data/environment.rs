use std::sync::Arc;

use agent_fw_algebra::TargetDatabase;
use agent_fw_catalog::CatalogScope;

use crate::storage::{
    build_target_database_from_environment, catalog_scope_from_data_environment,
    open_writable_catalog_from_environment_for_scope, DataEnvironmentConfig, OpenedCatalog,
};

use super::DataCommandError;

/// Opened dependencies for profiling commands.
///
/// Profiling needs a read-only target database plus a durable writable catalog
/// sink. This struct binds those two runtime-opened dependencies together at
/// the harness command boundary.
pub struct OpenedProfilingEnvironment {
    pub target_database: Arc<dyn TargetDatabase>,
    pub catalog: OpenedCatalog,
}

impl OpenedProfilingEnvironment {
    pub async fn open(config: &DataEnvironmentConfig) -> Result<Self, DataCommandError> {
        let scope = catalog_scope_from_data_environment(
            config,
            agent_fw_core::TenantId::new_unchecked(super::DEFAULT_DATA_TENANT_ID),
        );
        Self::open_for_scope(config, scope).await
    }

    pub async fn open_for_scope(
        config: &DataEnvironmentConfig,
        scope: CatalogScope,
    ) -> Result<Self, DataCommandError> {
        Ok(Self {
            target_database: build_target_database_from_environment(config).await?,
            catalog: open_writable_catalog_from_environment_for_scope(config, scope).await?,
        })
    }
}
