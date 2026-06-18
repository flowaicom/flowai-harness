//! Harness-owned data command kernel.
//!
//! The generic profiling / ingestion algorithms live in framework crates such
//! as `agent-fw-ingest`. This module is the harness boundary where those
//! capabilities become callable through direct runtime storage descriptors
//! rather than Studio-specific `source_id` APIs.

mod catalog;
mod environment;
mod errors;
mod export;
mod knowledge;
mod profile;
mod types;

pub const DEFAULT_DATA_TENANT_ID: &str = "flowai-runtime-data";

pub use catalog::{execute_catalog_tool, list_catalog_tools, list_metrics, search_catalog};
pub use environment::OpenedProfilingEnvironment;
pub use errors::DataCommandError;
pub use export::{export_catalog, CatalogExport};
pub use knowledge::{
    ingest_knowledge, KnowledgeCommandDeps, KnowledgeIngestEvent, KnowledgeIngestSummary,
    KnowledgeIngestionRunHandle,
};
pub use profile::{
    estimate_profiling, profile_database, profile_table, ProfilingCommandDeps,
    ProfilingEstimateResult, ProfilingRunHandle,
};
pub use types::{
    CatalogExportSummary, CatalogSearchItem, CatalogSearchResult, CatalogToolExecutionResult,
    CatalogToolList, CatalogToolSummary, ExecuteCatalogToolCommand, ExportCatalogCommand,
    IngestKnowledgeCommand, KnowledgeSourceSpec, ListMetricsCommand, MetricListResult,
    MetricSummary, ProfileDatabaseCommand, ProfileTableCommand, ProfilingEstimateCommand,
    SearchCatalogCommand,
};
