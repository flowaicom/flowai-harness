use thiserror::Error;

use agent_fw_catalog::CatalogError;

#[derive(Debug, Error)]
pub enum CatalogIndexError {
    #[error("catalog index IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("catalog index error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("invalid catalog index query: {0}")]
    InvalidQuery(String),

    #[error("invalid catalog index cursor: {0}")]
    InvalidCursor(String),

    #[error("catalog entity kind is not indexed: {0}")]
    UnsupportedKind(String),

    #[error("catalog index unavailable: {0}")]
    Unavailable(String),
}

impl From<CatalogIndexError> for CatalogError {
    fn from(error: CatalogIndexError) -> Self {
        match error {
            CatalogIndexError::InvalidQuery(message)
            | CatalogIndexError::InvalidCursor(message)
            | CatalogIndexError::UnsupportedKind(message) => CatalogError::InvalidQuery(message),
            CatalogIndexError::Unavailable(message) => CatalogError::Unavailable(message),
            CatalogIndexError::Io(error) => CatalogError::Unavailable(error.to_string()),
            CatalogIndexError::Tantivy(error) => CatalogError::Unavailable(error.to_string()),
        }
    }
}
