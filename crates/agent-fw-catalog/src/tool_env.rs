//! Catalog-specific `ToolEnvironment` ergonomics.
//!
//! `agent-fw-tool` cannot depend on `agent-fw-catalog` without creating a
//! crate cycle, so catalog accessors live here as an extension trait.

use std::sync::Arc;

use agent_fw_tool::{ToolEnvironment, ToolError};

use crate::catalog::DataCatalog;
use crate::search_backend::CatalogSearchBackend;

/// First-class catalog helpers for [`ToolEnvironment`].
pub trait CatalogToolEnvironmentExt {
    /// Register a data catalog as a common tool capability.
    fn with_catalog(self, catalog: Arc<dyn DataCatalog>) -> Self;

    /// Retrieve an optional data catalog capability.
    fn maybe_catalog(&self) -> Option<&Arc<dyn DataCatalog>>;

    /// Retrieve a required data catalog capability, panicking if missing.
    fn catalog(&self) -> &Arc<dyn DataCatalog>;

    /// Retrieve a required data catalog capability as a tool error.
    fn try_catalog(&self) -> Result<&Arc<dyn DataCatalog>, ToolError>;

    /// Register a catalog search backend as a common tool capability.
    fn with_catalog_search_backend(self, backend: Arc<dyn CatalogSearchBackend>) -> Self;

    /// Retrieve an optional catalog search backend capability.
    fn maybe_catalog_search_backend(&self) -> Option<&Arc<dyn CatalogSearchBackend>>;

    /// Retrieve a required catalog search backend capability as a tool error.
    fn try_catalog_search_backend(&self) -> Result<&Arc<dyn CatalogSearchBackend>, ToolError>;
}

impl CatalogToolEnvironmentExt for ToolEnvironment {
    fn with_catalog(self, catalog: Arc<dyn DataCatalog>) -> Self {
        self.with_ext::<dyn DataCatalog>(catalog)
    }

    fn maybe_catalog(&self) -> Option<&Arc<dyn DataCatalog>> {
        self.maybe_ext::<dyn DataCatalog>()
    }

    fn catalog(&self) -> &Arc<dyn DataCatalog> {
        self.expect_ext::<dyn DataCatalog>()
    }

    fn try_catalog(&self) -> Result<&Arc<dyn DataCatalog>, ToolError> {
        self.try_ext::<dyn DataCatalog>()
    }

    fn with_catalog_search_backend(self, backend: Arc<dyn CatalogSearchBackend>) -> Self {
        self.with_ext::<dyn CatalogSearchBackend>(backend)
    }

    fn maybe_catalog_search_backend(&self) -> Option<&Arc<dyn CatalogSearchBackend>> {
        self.maybe_ext::<dyn CatalogSearchBackend>()
    }

    fn try_catalog_search_backend(&self) -> Result<&Arc<dyn CatalogSearchBackend>, ToolError> {
        self.try_ext::<dyn CatalogSearchBackend>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_core::tenant::TenantContext;
    use async_trait::async_trait;

    use crate::{CatalogEntry, CatalogError, CatalogKind, JoinPath};

    struct StubCatalog;

    #[async_trait]
    impl DataCatalog for StubCatalog {
        async fn get_by_id(&self, _id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
            Ok(None)
        }

        async fn get_by_ids(&self, _ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn list_by_type(
            &self,
            _kind: CatalogKind,
            _limit: usize,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn get_related(
            &self,
            _id: &str,
            _relation_type: Option<&str>,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn find_join_path(
            &self,
            _from_table: &str,
            _to_table: &str,
        ) -> Result<Option<JoinPath>, CatalogError> {
            Ok(None)
        }

        async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
            Ok(vec![])
        }
    }

    fn test_env() -> ToolEnvironment {
        ToolEnvironment::builder()
            .kv(NullKVStore)
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant_context(TenantContext::new(
                agent_fw_core::id::TenantId::new_unchecked("test-tenant"),
            ))
            .build()
    }

    #[test]
    fn catalog_capability_round_trips() {
        let catalog: Arc<dyn DataCatalog> = Arc::new(StubCatalog);
        let env = test_env().with_catalog(Arc::clone(&catalog));

        assert!(Arc::ptr_eq(env.catalog(), &catalog));
        assert!(Arc::ptr_eq(env.try_catalog().unwrap(), &catalog));
        assert!(env.maybe_catalog().is_some());
    }
}
