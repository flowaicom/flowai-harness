//! Mock DataCatalog + CatalogWriter — in-memory implementation for testing.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogWriter, DataCatalog, JoinPath,
};

/// In-memory catalog for testing.
pub struct MockCatalog {
    entries: Arc<RwLock<Vec<CatalogEntry>>>,
}

impl MockCatalog {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Pre-load entries for testing.
    pub async fn load(&self, entries: Vec<CatalogEntry>) {
        *self.entries.write().await = entries;
    }
}

impl Default for MockCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataCatalog for MockCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries.iter().find(|e| e.id == id).cloned())
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect())
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|entry| entry.kind == kind && entry.name == name)
            .cloned()
            .collect())
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        let source = entries.iter().find(|e| e.id == id);
        match source {
            None => Ok(vec![]),
            Some(entry) => {
                let related_ids: Vec<&str> = entry
                    .links
                    .iter()
                    .filter(|r| relation_type.is_none() || Some(r.kind.as_str()) == relation_type)
                    .map(|r| r.target_id.as_str())
                    .collect();
                Ok(entries
                    .iter()
                    .filter(|e| related_ids.contains(&e.id.as_str()))
                    .cloned()
                    .collect())
            }
        }
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        // Simple mock: always returns None (no join path found)
        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|e| e.kind == CatalogKind::Table)
            .cloned()
            .collect())
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|e| {
                e.kind == CatalogKind::Column
                    && e.qualified_name
                        .as_ref()
                        .is_some_and(|qn| qn.starts_with(table_name))
            })
            .cloned()
            .collect())
    }

    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
        let entries = self.entries.read().await;
        let entry = entries
            .iter()
            .find(|e| e.id == column_id && e.kind == CatalogKind::Enum);
        match entry {
            Some(e) => {
                // Parse values from metadata
                if let Some(values) = e.metadata.get("values") {
                    if let Ok(vals) = serde_json::from_value::<Vec<String>>(values.clone()) {
                        return Ok(vals);
                    }
                }
                Ok(vec![])
            }
            None => Ok(vec![]),
        }
    }
}

#[async_trait]
impl CatalogWriter for MockCatalog {
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let mut entries = self.entries.write().await;
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        for item in items {
            // Upsert: remove existing with same id, then push
            entries.retain(|e| e.id != item.id);
            entries.push(item);
        }
        Ok(ids)
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|e| !ids.contains(&e.id));
        Ok((before - entries.len()) as u32)
    }

    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        // Mock: same as save_items (no real transaction)
        self.save_items(items).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::CatalogRelation;

    fn test_entry(id: &str, name: &str, kind: CatalogKind) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            kind,
            name: name.into(),
            qualified_name: None,
            content: format!("{name} description"),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn get_by_id_found_and_missing() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![test_entry("t1", "users", CatalogKind::Table)])
            .await;

        assert!(catalog.get_by_id("t1").await.unwrap().is_some());
        assert!(catalog.get_by_id("t99").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_by_name_filters_exact_name_and_kind() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                test_entry("t1", "users", CatalogKind::Table),
                test_entry("t2", "user_roles", CatalogKind::Table),
                test_entry("c1", "users", CatalogKind::Column),
            ])
            .await;

        let hits = catalog
            .get_by_name(CatalogKind::Table, "users")
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "t1");
    }

    #[tokio::test]
    async fn save_and_delete() {
        let catalog = MockCatalog::new();

        let ids = catalog
            .save_items(vec![
                test_entry("a", "alpha", CatalogKind::Table),
                test_entry("b", "beta", CatalogKind::Column),
            ])
            .await
            .unwrap();
        assert_eq!(ids, vec!["a", "b"]);

        let deleted = catalog.delete_items(&["a".into()]).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(catalog.get_by_id("a").await.unwrap().is_none());
        assert!(catalog.get_by_id("b").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn list_tables_filters_by_kind() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                test_entry("t1", "users", CatalogKind::Table),
                test_entry("c1", "id", CatalogKind::Column),
            ])
            .await;

        let tables = catalog.list_tables().await.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "users");
    }

    #[tokio::test]
    async fn get_related_follows_links() {
        let catalog = MockCatalog::new();
        let mut entry = test_entry("t1", "users", CatalogKind::Table);
        entry.links.push(CatalogRelation {
            target_id: "c1".into(),
            kind: "has_column".into(),
            description: None,
        });

        catalog
            .load(vec![
                entry,
                test_entry("c1", "id", CatalogKind::Column),
                test_entry("c2", "name", CatalogKind::Column),
            ])
            .await;

        let related = catalog.get_related("t1", None).await.unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, "c1");
    }
}
