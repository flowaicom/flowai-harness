//! DualCatalogWriter — product combinator for writing to two catalogs.
//!
//! # Design
//!
//! A dual writer is the **product** of two `CatalogWriter` algebras.
//! The primary is authoritative; the secondary is best-effort.
//!
//! ```text
//! let catalog = DualCatalogWriter::new(postgres_catalog, sqlite_catalog);
//! ```
//!
//! # Laws
//!
//! - L1 (Primary authoritative): Return value comes from primary
//! - L2 (Secondary best-effort): Secondary failure does not propagate
//! - L3 (DataCatalog delegation): All reads go through primary

use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogWriter, DataCatalog, JoinPath,
};
use async_trait::async_trait;

/// A `DataCatalog + CatalogWriter` that writes to two backends.
///
/// - All **reads** go through `primary`.
/// - All **writes** go through `primary` first (authoritative),
///   then best-effort through `secondary` (errors logged, not propagated).
pub struct DualCatalogWriter<P, S> {
    primary: P,
    secondary: S,
}

impl<P, S> DualCatalogWriter<P, S> {
    /// Create a dual writer from a primary and secondary catalog.
    pub fn new(primary: P, secondary: S) -> Self {
        Self { primary, secondary }
    }

    /// Access the primary catalog.
    pub fn primary(&self) -> &P {
        &self.primary
    }

    /// Access the secondary catalog.
    pub fn secondary(&self) -> &S {
        &self.secondary
    }
}

#[async_trait]
impl<P: DataCatalog, S: Send + Sync> DataCatalog for DualCatalogWriter<P, S> {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        self.primary.get_by_id(id).await
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.get_by_ids(ids).await
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        self.primary
            .get_by_qualified_name(kind, qualified_name)
            .await
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.get_by_name(kind, name).await
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.list_by_type(kind, limit).await
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.get_related(id, relation_type).await
    }

    async fn find_join_path(
        &self,
        from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        self.primary.find_join_path(from_table, to_table).await
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.list_tables().await
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.primary.get_columns(table_name).await
    }

    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
        self.primary.get_enum_values(column_id).await
    }

    async fn health_check(&self) -> Result<(), CatalogError> {
        self.primary.health_check().await
    }
}

#[async_trait]
impl<P: DataCatalog + CatalogWriter, S: CatalogWriter> CatalogWriter for DualCatalogWriter<P, S> {
    /// Save items to both catalogs. Primary is authoritative; secondary is best-effort.
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let ids = self.primary.save_items(items.clone()).await?;
        if let Err(e) = self.secondary.save_items(items).await {
            tracing::warn!("DualCatalogWriter: secondary save_items failed: {e}");
        }
        Ok(ids)
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        let count = self.primary.delete_items(ids).await?;
        if let Err(e) = self.secondary.delete_items(ids).await {
            tracing::warn!("DualCatalogWriter: secondary delete_items failed: {e}");
        }
        Ok(count)
    }

    /// Save in transaction. Primary is authoritative; secondary is best-effort.
    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        let ids = self.primary.save_in_transaction(items.clone()).await?;
        if let Err(e) = self.secondary.save_in_transaction(items).await {
            tracing::warn!("DualCatalogWriter: secondary save_in_transaction failed: {e}");
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockCatalog;
    use agent_fw_catalog::entry::CatalogKind;

    #[tokio::test]
    async fn reads_go_through_primary() {
        let primary = MockCatalog::new();
        let secondary = MockCatalog::new();

        // Seed primary only
        let entry = CatalogEntry {
            id: "t-1".into(),
            name: "primary_table".into(),
            kind: CatalogKind::Table,
            qualified_name: Some("public.primary_table".into()),
            content: "A table in primary".into(),
            tags: vec![],
            links: vec![],
            metadata: Default::default(),
        };
        primary.save_items(vec![entry]).await.unwrap();

        let dual = DualCatalogWriter::new(primary, secondary);
        let result = dual.get_by_id("t-1").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "primary_table");
    }

    #[tokio::test]
    async fn writes_go_to_both() {
        let primary = MockCatalog::new();
        let secondary = MockCatalog::new();
        let dual = DualCatalogWriter::new(primary, secondary);

        let entry = CatalogEntry {
            id: "t-1".into(),
            name: "dual_table".into(),
            kind: CatalogKind::Table,
            qualified_name: None,
            content: "Dual write".into(),
            tags: vec![],
            links: vec![],
            metadata: Default::default(),
        };
        let ids = dual.save_items(vec![entry]).await.unwrap();
        assert_eq!(ids, vec!["t-1"]);

        // Primary should have it (via reads)
        let result = dual.get_by_id("t-1").await.unwrap();
        assert!(result.is_some());

        // Secondary should also have it
        let result = dual.secondary().get_by_id("t-1").await.unwrap();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn secondary_failure_does_not_propagate() {
        let primary = MockCatalog::new();
        let secondary = crate::ErrorCatalog;
        let dual = DualCatalogWriter::new(primary, secondary);

        let entry = CatalogEntry {
            id: "t-1".into(),
            name: "resilient".into(),
            kind: CatalogKind::Table,
            qualified_name: None,
            content: "Resilient write".into(),
            tags: vec![],
            links: vec![],
            metadata: Default::default(),
        };
        // Should succeed despite secondary failure
        let ids = dual.save_items(vec![entry]).await.unwrap();
        assert_eq!(ids, vec!["t-1"]);
    }
}
