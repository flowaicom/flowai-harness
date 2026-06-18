//! Tantivy-backed catalog retrieval index.

pub mod cursor;
pub mod error;
pub mod facets;
pub mod path;
pub mod projection;
pub mod schema;
pub mod tantivy_backend;

pub use error::CatalogIndexError;
pub use path::CatalogIndexPaths;
pub use projection::{CatalogDocumentProjection, PROJECTED_CATALOG_SCHEMA_VERSION};
pub use tantivy_backend::TantivyCatalogIndex;
