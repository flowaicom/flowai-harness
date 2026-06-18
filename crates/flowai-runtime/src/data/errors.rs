use crate::storage::StorageConfigError;

/// Harness-level error surface for data commands.
#[derive(Debug, thiserror::Error)]
pub enum DataCommandError {
    #[error(transparent)]
    Storage(#[from] StorageConfigError),
    #[error(transparent)]
    Catalog(#[from] agent_fw_catalog::CatalogError),
    #[error("{0}")]
    Execution(String),
    #[error("{0}")]
    Invalid(String),
}
