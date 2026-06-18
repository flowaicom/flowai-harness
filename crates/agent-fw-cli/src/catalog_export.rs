use std::path::Path;
use std::sync::Mutex;

use agent_fw_catalog::{order_entries_for_artifact, CatalogEntry, CatalogError, CatalogWriter};
use async_trait::async_trait;

#[derive(Default)]
pub struct MemoryCatalogWriter {
    entries: Mutex<Vec<CatalogEntry>>,
}

impl MemoryCatalogWriter {
    pub async fn save_entries(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        self.save_in_transaction(items).await
    }

    pub fn entries(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        let entries = self.entries.lock().map_err(|_| {
            CatalogError::Unavailable("memory catalog writer lock poisoned".to_string())
        })?;
        Ok(entries.clone())
    }
}

#[async_trait]
impl CatalogWriter for MemoryCatalogWriter {
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let ids = items.iter().map(|item| item.id.clone()).collect();
        let mut entries = self.entries.lock().map_err(|_| {
            CatalogError::Unavailable("memory catalog writer lock poisoned".to_string())
        })?;
        entries.extend(items);
        Ok(ids)
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        let mut entries = self.entries.lock().map_err(|_| {
            CatalogError::Unavailable("memory catalog writer lock poisoned".to_string())
        })?;
        let before = entries.len();
        entries.retain(|item| !ids.contains(&item.id));
        Ok((before - entries.len()) as u32)
    }

    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        self.save_items(items).await
    }
}

pub async fn write_catalog_entries_json(
    writer: &MemoryCatalogWriter,
    out: &Path,
) -> Result<usize, CatalogExportError> {
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let entries = order_entries_for_artifact(writer.entries()?);

    let json = serde_json::to_vec_pretty(&entries)?;
    let written_count = entries.len();
    tokio::fs::write(out, json).await?;
    Ok(written_count)
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogExportError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
