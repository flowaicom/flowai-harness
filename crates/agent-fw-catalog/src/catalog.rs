//! DataCatalog and CatalogWriter — read and write traits for the catalog.
//!
//! # DataCatalog Laws
//!
//! L1 (Existence): `get_by_id(id)` returns `Some` iff id was indexed
//! L2 (Determinism): Same inputs → same outputs (within snapshot)
//! L3 (Exact name lookup): `get_by_name(kind, name)` returns only exact
//! name matches for the requested kind
//! L4 (Relationship Integrity): `get_related(id)` returns only existing items
//!
//! # CatalogWriter Laws
//!
//! L1 (Roundtrip): `save_items(items); get_by_ids(ids)` returns the items
//! L2 (Delete): `delete_items(ids); get_by_ids(ids)` returns empty
//! L3 (Transaction): `save_in_transaction` partial failure rolls back all

use async_trait::async_trait;

use crate::entry::{CatalogEntry, CatalogError, CatalogKind, JoinPath};
use crate::semantic::CatalogRef;

const DEFAULT_EXACT_LOOKUP_SCAN_LIMIT: usize = 10_000;

/// Read-only catalog access (lookup, listing, graph traversal).
#[async_trait]
pub trait DataCatalog: Send + Sync {
    /// Get a single entry by its ID.
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError>;

    /// Get multiple entries by their IDs.
    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError>;

    /// Get a single entry by exact qualified name and kind.
    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(self
            .list_by_type(kind, DEFAULT_EXACT_LOOKUP_SCAN_LIMIT)
            .await?
            .into_iter()
            .find(|entry| entry.qualified_name.as_deref() == Some(qualified_name)))
    }

    /// Get entries by exact name and kind.
    ///
    /// Storage-backed interpreters should override this with an indexed exact
    /// query. The default remains exact-only for lightweight test catalogs and
    /// compatibility adapters.
    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .list_by_type(kind, DEFAULT_EXACT_LOOKUP_SCAN_LIMIT)
            .await?
            .into_iter()
            .filter(|entry| entry.name == name)
            .collect())
    }

    /// Resolve a user/system catalog reference to a concrete entry.
    async fn resolve_ref(
        &self,
        reference: &CatalogRef,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        match reference {
            CatalogRef::Id(id) => self.get_by_id(id).await,
            CatalogRef::QualifiedName {
                kind: Some(kind),
                qualified_name,
            } => self.get_by_qualified_name(*kind, qualified_name).await,
            CatalogRef::QualifiedName {
                kind: None,
                qualified_name,
            } => {
                for kind in exact_ref_kinds() {
                    if let Some(entry) = self.get_by_qualified_name(kind, qualified_name).await? {
                        return Ok(Some(entry));
                    }
                }
                Ok(None)
            }
            CatalogRef::Name { kind, name, schema } => {
                if let Some(schema) = schema {
                    let qualified_name = format!("{schema}.{name}");
                    return self.get_by_qualified_name(*kind, &qualified_name).await;
                }
                Ok(self.get_by_name(*kind, name).await?.into_iter().next())
            }
        }
    }

    /// List entries of a specific catalog kind.
    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError>;

    /// Get entries related to a given entry (optionally filtered by relation type).
    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError>;

    /// Get entries that point to a given entry.
    async fn get_related_reverse(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(vec![])
    }

    /// Find a join path between two tables.
    async fn find_join_path(
        &self,
        from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError>;

    /// List all table entries.
    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError>;

    /// Get column entries for a table.
    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError>;

    /// Get enum values for a categorical column.
    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError>;

    /// Check that the catalog backend is healthy.
    async fn health_check(&self) -> Result<(), CatalogError> {
        Ok(())
    }
}

fn exact_ref_kinds() -> [CatalogKind; 8] {
    [
        CatalogKind::Table,
        CatalogKind::Column,
        CatalogKind::Relationship,
        CatalogKind::Enum,
        CatalogKind::Metric,
        CatalogKind::Document,
        CatalogKind::Knowledge,
        CatalogKind::DataQualityFinding,
    ]
}

/// Write operations for the catalog (save, delete, transactional save).
#[async_trait]
pub trait CatalogWriter: Send + Sync {
    /// Save catalog entries. Returns the IDs of saved entries.
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError>;

    /// Delete catalog entries by ID. Returns the number of entries deleted.
    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError>;

    /// Save entries in a transaction (all-or-nothing).
    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ExactCatalog {
        entries: Vec<CatalogEntry>,
    }

    #[async_trait]
    impl DataCatalog for ExactCatalog {
        async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
            Ok(self.entries.iter().find(|entry| entry.id == id).cloned())
        }

        async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(ids
                .iter()
                .filter_map(|id| self.entries.iter().find(|entry| &entry.id == id).cloned())
                .collect())
        }

        async fn list_by_type(
            &self,
            kind: CatalogKind,
            limit: usize,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(self
                .entries
                .iter()
                .filter(|entry| entry.kind == kind)
                .take(limit)
                .cloned()
                .collect())
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
            self.list_by_type(CatalogKind::Table, usize::MAX).await
        }

        async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
            Ok(vec![])
        }
    }

    fn public_orders_catalog() -> ExactCatalog {
        ExactCatalog {
            entries: vec![CatalogEntry {
                id: "table:public.orders".into(),
                kind: CatalogKind::Table,
                name: "orders".into(),
                qualified_name: Some("public.orders".into()),
                content: "Orders table".into(),
                tags: vec![],
                links: vec![],
                metadata: serde_json::json!({}),
            }],
        }
    }

    #[tokio::test]
    async fn schema_qualified_name_miss_does_not_fallback_to_unqualified_name() {
        let catalog = public_orders_catalog();

        let resolved = catalog
            .resolve_ref(&CatalogRef::Name {
                kind: CatalogKind::Table,
                name: "orders".into(),
                schema: Some("archive".into()),
            })
            .await
            .unwrap();

        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn kindless_exact_ref_does_not_resolve_reserved_special_entries() {
        let catalog = ExactCatalog {
            entries: vec![CatalogEntry {
                id: "special:internal".into(),
                kind: CatalogKind::Special,
                name: "internal".into(),
                qualified_name: Some("internal.reserved".into()),
                content: "Reserved internal entry".into(),
                tags: vec![],
                links: vec![],
                metadata: serde_json::json!({}),
            }],
        };

        let resolved = catalog
            .resolve_ref(&CatalogRef::QualifiedName {
                kind: None,
                qualified_name: "internal.reserved".into(),
            })
            .await
            .unwrap();

        assert!(resolved.is_none());
    }
}
