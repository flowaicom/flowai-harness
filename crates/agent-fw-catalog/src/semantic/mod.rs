//! Ontology-lite semantic model for catalog entries.
//!
//! The semantic model is a typed layer above the existing `CatalogEntry` and
//! `CatalogRelation` storage envelope. It defines built-in entity metadata,
//! controlled relation names, and reference parsing without introducing a new
//! backing service or treating access scope as JSON metadata.
//!
//! # Laws
//!
//! The ontology-lite layer is a pure contract over the catalog storage envelope:
//!
//! - `CatalogKind -> SemanticEntityKind -> CatalogKind` preserves the original
//!   catalog kind.
//! - `SemanticEntityKind -> CatalogKind -> SemanticEntityKind` preserves the
//!   original semantic kind.
//! - For valid kind-specific metadata, `SemanticEntity::try_from(entry)` returns
//!   an entity whose semantic kind maps back to `entry.kind`.
//! - For valid metadata, `SemanticEntity::try_from(entry.clone()).map(|entity|
//!   entity.into_entry())` preserves the original `CatalogEntry` storage
//!   envelope.
//! - `CatalogRef::parse_table` and `CatalogRef::parse_column` are deterministic
//!   and idempotent under surrounding whitespace trim.
//! - Representative semantic metadata serde roundtrips preserve explicit values
//!   and documented defaults.
//!
//! The reusable harness for these laws lives in
//! `agent_fw_test::semantic_laws`.

pub mod convert;
pub mod entity;
pub mod metadata;
pub mod reference;
pub mod relation;

#[cfg(test)]
mod tests;

pub use entity::{SemanticEntity, SemanticEntityKind};
pub use metadata::{
    decode_metadata, provenance_origin, CatalogProvenance, ColumnMetadata,
    DataQualityFindingMetadata, DocumentMetadata, EnumValueMetadata, ForeignKeyMetadata,
    KnowledgeMetadata, MetricMetadata, RelationshipMetadata, TableMetadata,
};
pub use reference::{CatalogRef, ResolvedCatalogRef};
pub use relation::{relation_kind, Cardinality};
