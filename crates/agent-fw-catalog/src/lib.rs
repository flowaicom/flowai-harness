//! Data catalog types, traits, and domain model.
//!
//! This crate provides the complete catalog algebra — types for representing
//! database schema, column profiles, semantic enrichments, knowledge items,
//! and the traits for reading/writing the catalog, enriching with LLMs,
//! and provisioning database environments.
//!
//! # Modules
//!
//! - [`entry`] — CatalogEntry, CatalogKind, CatalogRelation
//! - [`knowledge`] — KnowledgeItem, DocumentItem, MetricItem
//! - [`discovery`] — PhysicalTable, ColumnInfo, TableInfo
//! - [`profiling`] — ColumnProfile, TableProfile, SemanticType
//! - [`enrichment`] — SemanticEnricher trait, SemanticTableProfile
//! - [`provisioner`] — DatabaseProvisioner trait, EnvironmentId
//! - [`datasource`] — DatabaseType, DataSource, ConnectionTestResult
//! - [`ingestion`] — IngestionStatus state machine, IngestionSummary monoid
//! - [`catalog`] — DataCatalog and CatalogWriter traits
//! - [`search_backend`] — storage-agnostic lexical catalog retrieval contract
//! - [`diagnostics`] — catalog consistency diagnostics
//! - [`composer`] — CatalogComposer tagless final algebra (Markdown + XML interpreters)
//! - [`scope`] — CatalogScope tenant/workspace access boundary
//! - [`semantic`] — Ontology-lite typed semantic model and relation vocabulary
//! - [`tool_env`] — Catalog-specific `ToolEnvironment` ergonomics

pub mod api;
pub mod artifact;
mod catalog;
pub mod composer;
pub mod datasource;
pub mod diagnostics;
pub mod discovery;
pub mod enrichment;
pub mod entry;
pub mod identifier;
pub mod ingestion;
pub mod knowledge;
pub mod profiling;
pub mod provisioner;
pub mod scope;
pub mod search_backend;
pub mod semantic;
pub mod table_role;
pub mod tool_env;

// Re-export key types at crate root

pub use api::{FindJoinPathRequest, ProfileDatabaseRequest, ProfileTableRequest};

// Catalog artifact export ordering
pub use artifact::order_entries_for_artifact;

// Entry types
pub use entry::{
    CatalogEntry, CatalogEntryKind, CatalogError, CatalogKind, CatalogRelation, JoinHop, JoinPath,
};

// Knowledge types
pub use knowledge::{
    AggregationType, DocumentItem, ExtractionStatus, KnowledgeItem, KnowledgeType,
    LlmKnowledgeItem, MetricItem, OutputFormat,
};

// Discovery types
pub use discovery::{
    ColumnInfo, ConstraintInfo, ConstraintKind, ForeignKeyEdge, ForeignKeyRef, IndexInfo,
    PhysicalTable, TableInfo, TableType,
};

// Profiling types
pub use profiling::{CategoryValue, ColumnProfile, SemanticType, TableProfile, TypeSpecificStats};

// Enrichment types + trait
pub use enrichment::{
    CachedEnrichmentEntry, ColumnDescriptions, EnrichmentError, EnrichmentResult, EnrichmentSource,
    InferredRelationship, JoinPair, KnowledgeExtractionRequest, QualityNote, RelationshipKind,
    SemanticEnricher, SemanticTableProfile, TableEnrichmentRequest,
};

// Provisioner types + trait
pub use provisioner::{
    DatabaseProvisioner, EnvironmentId, EnvironmentName, EnvironmentSummary, ProvisionRequest,
    ProvisionedConnection, ProvisionedEnvironment, ProvisioningError,
};

// Data source types
pub use datasource::{
    ConnectionTestResult, CreateDataSourceRequest, DataSource, DataSourceStatus, DatabaseType,
    UpdateDataSourceRequest,
};

// Catalog diagnostics
pub use diagnostics::{
    diagnose_catalog_relations, CatalogRelationDiagnostic, CatalogRelationDiagnostics,
    CatalogRelationIssue,
};

// Ingestion types
pub use ingestion::{IngestionEvent, IngestionStatus, IngestionSummary, IngestionTransitionError};

// SQL identifier newtypes
pub use identifier::{ColumnName, IdentifierError, QualifiedTable, SchemaName, TableName};

// Table role classification
pub use table_role::{classify_table_role, TableClassificationInput, TableRole};

// Catalog traits
pub use catalog::{CatalogWriter, DataCatalog};
pub use scope::CatalogScope;
pub use search_backend::{
    CatalogFacetValue, CatalogSearchBackend, CatalogSearchCursor, CatalogSearchFacets,
    CatalogSearchFilters, CatalogSearchHealth, CatalogSearchHitRef, CatalogSearchRequest,
    CatalogSearchResults,
};
pub use semantic::{
    decode_metadata, provenance_origin, relation_kind, Cardinality, CatalogProvenance, CatalogRef,
    ColumnMetadata, DataQualityFindingMetadata, DocumentMetadata, EnumValueMetadata,
    ForeignKeyMetadata, KnowledgeMetadata, MetricMetadata, RelationshipMetadata,
    ResolvedCatalogRef, SemanticEntity, SemanticEntityKind, TableMetadata,
};
pub use tool_env::CatalogToolEnvironmentExt;

// Composer algebra + interpreters
pub use composer::{
    compose, compose_many, CatalogComposer, ComposerContext, ComposerVariant, Markdown,
    ModelFamily, Xml,
};
