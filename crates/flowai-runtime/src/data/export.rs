//! Harness-native catalog export command.
//!
//! Reads an already-profiled catalog backend under a tenant/workspace scope and
//! returns its entries deterministically ordered for a portable
//! `catalog.entries.json` artifact. Unlike the standalone `catalog-export`
//! utility, this does **not** connect to or re-profile the target database — it
//! is the read-side complement to `profile_table` / `profile_database`.

use std::collections::BTreeMap;

use agent_fw_catalog::{order_entries_for_artifact, CatalogEntry, CatalogKind, CatalogScope};
use agent_fw_core::{TenantId, WorkspaceId};

use super::{
    types::{CatalogExportSummary, ExportCatalogCommand},
    DataCommandError, DEFAULT_DATA_TENANT_ID,
};
use crate::storage::{build_catalog_for_scope, DataEnvironmentConfig};

/// Catalog entries plus a summary, ready to serialize into an export artifact.
#[derive(Debug, Clone)]
pub struct CatalogExport {
    /// Entries in deterministic artifact order (see [`order_entries_for_artifact`]).
    pub entries: Vec<CatalogEntry>,
    pub summary: CatalogExportSummary,
}

/// Catalog entry kinds enumerated for export, mirroring the catalog graph
/// builder's content set. `Table` is fetched via `list_tables`; `Enum` entries
/// are intentionally excluded (their values are carried inline on columns and
/// are dropped by [`order_entries_for_artifact`]).
const EXPORT_KINDS: &[CatalogKind] = &[
    CatalogKind::Column,
    CatalogKind::Relationship,
    CatalogKind::Metric,
    CatalogKind::Document,
    CatalogKind::Knowledge,
    CatalogKind::DataQualityFinding,
];

/// Read and order all durable catalog entries under the resolved scope.
pub async fn export_catalog(
    command: ExportCatalogCommand,
) -> Result<CatalogExport, DataCommandError> {
    let Some(catalog_config) = command.data_environment.catalog.clone() else {
        return Err(DataCommandError::Invalid(
            "catalog export requires data_environment.catalog".to_string(),
        ));
    };

    let scope = export_catalog_scope(
        &command.data_environment,
        command.tenant_id,
        command.workspace_id,
    );
    let catalog = build_catalog_for_scope(catalog_config, scope.clone()).await?;

    let mut entries = catalog.list_tables().await?;
    for kind in EXPORT_KINDS {
        entries.extend(catalog.list_by_type(*kind, usize::MAX).await?);
    }
    let entries = order_entries_for_artifact(entries);

    let mut counts_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        *counts_by_kind
            .entry(entry.kind.as_str().to_string())
            .or_default() += 1;
    }

    let summary = CatalogExportSummary {
        tenant_id: scope.tenant_id.to_string(),
        workspace_id: scope.workspace_id.to_string(),
        entries_written: entries.len(),
        counts_by_kind,
    };

    Ok(CatalogExport { entries, summary })
}

fn export_catalog_scope(
    config: &DataEnvironmentConfig,
    tenant_id: Option<TenantId>,
    workspace_id: Option<WorkspaceId>,
) -> CatalogScope {
    CatalogScope::new(
        tenant_id
            .or_else(|| config.tenant_id.clone())
            .unwrap_or_else(|| TenantId::new_unchecked(DEFAULT_DATA_TENANT_ID)),
        workspace_id
            .or_else(|| config.workspace_id.clone())
            .unwrap_or_else(WorkspaceId::default_workspace),
    )
}
