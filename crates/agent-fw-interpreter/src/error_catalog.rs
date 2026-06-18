//! Error-returning DataCatalog + CatalogWriter — used when catalog is not configured.

use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogWriter, DataCatalog, JoinPath,
};
use async_trait::async_trait;

/// A DataCatalog + CatalogWriter implementation that always returns errors.
///
/// Used as a sentinel when no catalog backend is configured.
pub struct ErrorCatalog;

const NOT_CONFIGURED: &str = "catalog not configured";

#[async_trait]
impl DataCatalog for ErrorCatalog {
    async fn get_by_id(&self, _id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_by_ids(&self, _ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_by_qualified_name(
        &self,
        _kind: CatalogKind,
        _qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_by_name(
        &self,
        _kind: CatalogKind,
        _name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn list_by_type(
        &self,
        _kind: CatalogKind,
        _limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_related(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn health_check(&self) -> Result<(), CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }
}

#[async_trait]
impl CatalogWriter for ErrorCatalog {
    async fn save_items(&self, _items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn delete_items(&self, _ids: &[String]) -> Result<u32, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }

    async fn save_in_transaction(
        &self,
        _items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        Err(CatalogError::Unavailable(NOT_CONFIGURED.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lookup_returns_error() {
        let catalog = ErrorCatalog;
        let result = catalog
            .get_by_qualified_name(CatalogKind::Table, "public.products")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn save_returns_error() {
        let catalog = ErrorCatalog;
        let result = catalog.save_items(vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn health_check_returns_error() {
        let catalog = ErrorCatalog;
        assert!(catalog.health_check().await.is_err());
    }
}
