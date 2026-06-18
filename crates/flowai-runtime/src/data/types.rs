use std::collections::BTreeMap;
use std::path::PathBuf;

use agent_fw_core::{TenantId, WorkspaceId};

use crate::storage::DataEnvironmentConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Estimate token / cost / duration for a profiling run.
#[derive(Debug, Clone)]
pub struct ProfilingEstimateCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tenant_id: Option<TenantId>,
    pub workspace_id: Option<WorkspaceId>,
    pub database_id: String,
    pub schema_name: Option<String>,
    pub tables: Vec<String>,
    pub model_id: Option<String>,
    pub sample_size: Option<usize>,
}

/// Profile one table and persist catalog artifacts into the configured catalog backend.
#[derive(Debug, Clone)]
pub struct ProfileTableCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tenant_id: Option<TenantId>,
    pub workspace_id: Option<WorkspaceId>,
    pub database_id: String,
    pub schema_name: Option<String>,
    pub table_name: String,
    pub model_id: Option<String>,
    pub sample_size: Option<usize>,
}

/// Profile a full database or a selected subset of tables.
#[derive(Debug, Clone)]
pub struct ProfileDatabaseCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tenant_id: Option<TenantId>,
    pub workspace_id: Option<WorkspaceId>,
    pub database_id: String,
    pub schema_name: Option<String>,
    pub tables: Vec<String>,
    pub model_id: Option<String>,
    pub sample_size: Option<usize>,
}

/// Supported harness knowledge-ingestion source descriptors.
#[derive(Debug, Clone)]
pub enum KnowledgeSourceSpec {
    LocalDirectory {
        path: PathBuf,
        extensions: Vec<String>,
    },
}

/// Ingest knowledge documents into the harness-owned knowledge pipeline.
#[derive(Debug, Clone)]
pub struct IngestKnowledgeCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tenant_id: String,
    pub workspace_id: Option<WorkspaceId>,
    pub database_id: String,
    pub source: KnowledgeSourceSpec,
    pub extract_knowledge: bool,
}

/// Export the durable catalog entries under a scope as a portable artifact.
///
/// This reads an already-profiled catalog backend (`sqlite`/`postgres`, or an
/// `inline` catalog) and does not connect to or re-profile the target database.
#[derive(Debug, Clone)]
pub struct ExportCatalogCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tenant_id: Option<TenantId>,
    pub workspace_id: Option<WorkspaceId>,
}

/// Summary of a catalog export, suitable for CLI / Python adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogExportSummary {
    pub tenant_id: String,
    pub workspace_id: String,
    pub entries_written: usize,
    pub counts_by_kind: BTreeMap<String, usize>,
}

/// Search the configured data catalog from the harness Connect surface.
#[derive(Debug, Clone)]
pub struct SearchCatalogCommand {
    pub data_environment: DataEnvironmentConfig,
    pub query: String,
    pub mode: Option<String>,
    pub limit: Option<usize>,
}

/// List metric definitions from the configured catalog.
#[derive(Debug, Clone)]
pub struct ListMetricsCommand {
    pub data_environment: DataEnvironmentConfig,
    pub query: Option<String>,
    pub limit: Option<usize>,
}

/// Execute one supported catalog/toolkit primitive.
#[derive(Debug, Clone)]
pub struct ExecuteCatalogToolCommand {
    pub data_environment: DataEnvironmentConfig,
    pub tool_id: String,
    pub input: Value,
}

/// Frontend-facing catalog search response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchResult {
    pub items: Vec<CatalogSearchItem>,
    pub total_count: usize,
    pub query_time_ms: u64,
    pub mode: Option<String>,
}

/// Normalized catalog hit for Studio search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchItem {
    pub id: String,
    pub name: String,
    pub item_type: String,
    pub description: String,
    pub qualified_name: Option<String>,
    pub tags: Vec<String>,
    pub score: f64,
    pub match_field: Option<String>,
    pub metadata: Value,
}

/// Supported catalog tool descriptor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogToolSummary {
    pub tool_id: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub parameters: Value,
}

/// Tool registry response for Studio Connect.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogToolList {
    pub tools: Vec<CatalogToolSummary>,
}

/// Result of executing a supported catalog tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogToolExecutionResult {
    pub tool_id: String,
    pub success: bool,
    pub data: Value,
    pub count: Option<usize>,
    pub error: Option<String>,
}

/// Metric listing response for Studio Connect.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricListResult {
    pub metrics: Vec<MetricSummary>,
    pub total_count: usize,
}

/// Normalized metric summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub metric_type: Option<String>,
    pub tags: Vec<String>,
    pub metadata: Value,
    pub score: Option<f64>,
}
