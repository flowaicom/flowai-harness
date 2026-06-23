//! Harness-native knowledge ingestion commands.
//!
//! The framework owns the generic document-ingestion and extraction pipelines.
//! This module is the harness boundary that turns `DataEnvironmentConfig`
//! descriptors into foreground knowledge-ingestion runs callable from the CLI
//! and Python facade.

use std::sync::Arc;

use agent_fw_algebra::{CancellationToken, KVStore};
use agent_fw_catalog::SemanticEnricher;
use agent_fw_catalog::{
    decode_metadata, relation_kind, CatalogEntry, CatalogKind, CatalogRelation, CatalogScope,
    CatalogWriter, ColumnMetadata, DataCatalog, DataQualityFindingMetadata, DocumentItem,
    EnumValueMetadata, KnowledgeItem, RelationshipMetadata, TableMetadata,
};
use agent_fw_ingest::builder::{build_document_entry, build_knowledge_entries_with_namespaces};
use agent_fw_ingest::knowledge_extraction::KnowledgeExtractionService;
use agent_fw_ingest::knowledge_ingestion::{
    IngestEvent as FrameworkIngestEvent, IngestSummary as FrameworkIngestSummary,
    KnowledgeIngestionService, KnowledgeSourceSpec as FrameworkKnowledgeSourceSpec,
};
use agent_fw_ingest::knowledge_store;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::mpsc;
use uuid::Uuid;

use agent_fw_core::{TenantId, WorkspaceContext, WorkspaceId};

use super::{DataCommandError, IngestKnowledgeCommand, KnowledgeSourceSpec};

const EVENT_BUFFER: usize = 256;
/// Host-provided dependencies for harness knowledge-ingestion commands.
#[derive(Clone, Default)]
pub struct KnowledgeCommandDeps {
    pub enricher: Option<Arc<dyn SemanticEnricher>>,
}

impl KnowledgeCommandDeps {
    pub fn new() -> Self {
        Self { enricher: None }
    }

    pub fn with_enricher(mut self, enricher: Arc<dyn SemanticEnricher>) -> Self {
        self.enricher = Some(enricher);
        self
    }
}

/// Serializable summary for knowledge-ingestion completion.
pub type KnowledgeIngestSummary = FrameworkIngestSummary;

/// Serializable progress events for harness knowledge-ingestion runs.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum KnowledgeIngestEvent {
    #[serde(rename_all = "camelCase")]
    Discovered {
        total: usize,
    },
    #[serde(rename_all = "camelCase")]
    Ingesting {
        current: usize,
        total: usize,
        name: String,
    },
    Completed(KnowledgeIngestSummary),
    #[serde(rename_all = "camelCase")]
    Error {
        message: String,
    },
}

impl From<FrameworkIngestEvent> for KnowledgeIngestEvent {
    fn from(event: FrameworkIngestEvent) -> Self {
        match event {
            FrameworkIngestEvent::Discovered { total } => Self::Discovered { total },
            FrameworkIngestEvent::Ingesting {
                current,
                total,
                name,
            } => Self::Ingesting {
                current,
                total,
                name,
            },
            FrameworkIngestEvent::Completed(summary) => Self::Completed(summary),
        }
    }
}

/// Foreground knowledge-ingestion run handle.
pub struct KnowledgeIngestionRunHandle {
    pub job_id: String,
    pub events: mpsc::Receiver<KnowledgeIngestEvent>,
}

/// Ingest knowledge documents and optionally trigger extraction.
pub async fn ingest_knowledge(
    command: IngestKnowledgeCommand,
    deps: KnowledgeCommandDeps,
) -> Result<KnowledgeIngestionRunHandle, DataCommandError> {
    validate_source(&command.source)?;

    let catalog_scope = knowledge_catalog_scope(&command)?;
    let catalog_entity_database_id = knowledge_catalog_entity_database_id(&catalog_scope);
    let catalog_database_id = knowledge_catalog_database_id(&command)?;
    let kv = crate::storage::build_kv_store_from_environment(&command.data_environment).await?;
    let catalog = open_optional_catalog(&command.data_environment, catalog_scope).await?;
    let extraction = if command.extract_knowledge {
        let Some(enricher) = deps.enricher else {
            return Err(DataCommandError::Invalid(
                "knowledge extraction requires an enricher; configure Anthropic credentials or provide a runtime enricher"
                    .to_string(),
            ));
        };
        let target_database =
            crate::storage::build_target_database_from_environment(&command.data_environment)
                .await?;
        let extraction_policy = knowledge_store::workspace_extraction_persistence();
        Some(Arc::new(KnowledgeExtractionService::new_with_persistence(
            target_database,
            enricher,
            Arc::clone(&kv),
            extraction_policy,
        )))
    } else {
        None
    };
    let ingestion_policy = knowledge_store::workspace_ingestion_persistence();
    let service = Arc::new(KnowledgeIngestionService::new_with_persistence(
        kv.clone(),
        extraction,
        ingestion_policy,
    ));

    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let (inner_tx, mut inner_rx) = mpsc::channel::<FrameworkIngestEvent>(EVENT_BUFFER);
    let job_id = format!("knowledge-ingest-{}", Uuid::new_v4().simple());
    let tenant_id = knowledge_workspace_tenant_id(&command)?;
    let source = into_framework_source(command.source);

    let tx_forward = tx.clone();
    let forward_handle = tokio::spawn(async move {
        while let Some(event) = inner_rx.recv().await {
            match event {
                event @ (FrameworkIngestEvent::Discovered { .. }
                | FrameworkIngestEvent::Ingesting { .. }) => {
                    if tx_forward.send(event.into()).await.is_err() {
                        break;
                    }
                }
                FrameworkIngestEvent::Completed(_) => {
                    // The harness sends its public completion event only after
                    // post-ingest catalog projection succeeds.
                }
            }
        }
    });

    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        let result = service
            .ingest(&tenant_id, &source, &cancel, Some(&inner_tx))
            .await;
        match result {
            Ok(summary) => {
                drop(inner_tx);
                let _ = forward_handle.await;
                if !summary.errors.is_empty() {
                    let _ = tx
                        .send(KnowledgeIngestEvent::Error {
                            message: summary.errors.join("; "),
                        })
                        .await;
                    return;
                }
                if let Some(catalog) = catalog {
                    if let Err(err) = project_knowledge_corpus_to_catalog(
                        kv,
                        catalog.reader,
                        catalog.writer,
                        &tenant_id,
                        &catalog_entity_database_id,
                        &catalog_database_id,
                    )
                    .await
                    {
                        let _ = tx
                            .send(KnowledgeIngestEvent::Error {
                                message: err.to_string(),
                            })
                            .await;
                        return;
                    }
                }
                let _ = tx.send(KnowledgeIngestEvent::Completed(summary)).await;
            }
            Err(err) => {
                drop(inner_tx);
                let _ = forward_handle.await;
                let _ = tx
                    .send(KnowledgeIngestEvent::Error {
                        message: err.to_string(),
                    })
                    .await;
            }
        }
    });

    Ok(KnowledgeIngestionRunHandle { job_id, events: rx })
}

fn knowledge_workspace_tenant_id(
    command: &IngestKnowledgeCommand,
) -> Result<String, DataCommandError> {
    let tenant_id = TenantId::new(command.tenant_id.clone()).ok_or_else(|| {
        DataCommandError::Invalid("knowledge ingestion tenant_id must not be blank".to_string())
    })?;
    let workspace_id = command
        .workspace_id
        .clone()
        .or_else(|| command.data_environment.workspace_id.clone())
        .unwrap_or_else(WorkspaceId::default_workspace);
    Ok(
        WorkspaceContext::from_ids(tenant_id, Some(workspace_id.as_str()))
            .workspace_tenant_id()
            .to_string(),
    )
}

fn knowledge_catalog_scope(
    command: &IngestKnowledgeCommand,
) -> Result<CatalogScope, DataCommandError> {
    let tenant_id = TenantId::new(command.tenant_id.clone()).ok_or_else(|| {
        DataCommandError::Invalid("knowledge ingestion tenant_id must not be blank".to_string())
    })?;
    let workspace_id = command
        .workspace_id
        .clone()
        .or_else(|| command.data_environment.workspace_id.clone())
        .unwrap_or_else(WorkspaceId::default_workspace);
    Ok(CatalogScope::new(tenant_id, workspace_id))
}

fn knowledge_catalog_database_id(
    command: &IngestKnowledgeCommand,
) -> Result<String, DataCommandError> {
    let database_id = command.database_id.trim();
    if database_id.is_empty() {
        return Err(DataCommandError::Invalid(
            "knowledge ingestion database_id must not be blank".to_string(),
        ));
    }

    Ok(database_id.to_string())
}

fn knowledge_catalog_entity_database_id(scope: &CatalogScope) -> String {
    format!(
        "knowledge:{}:{}",
        scope.tenant_id.as_str(),
        scope.workspace_id.as_str()
    )
}

async fn open_optional_catalog(
    config: &crate::storage::DataEnvironmentConfig,
    scope: CatalogScope,
) -> Result<Option<crate::storage::OpenedCatalog>, DataCommandError> {
    if config.catalog.is_none() {
        return Ok(None);
    }
    let opened =
        crate::storage::open_writable_catalog_from_environment_for_scope(config, scope).await?;
    Ok(Some(opened))
}

async fn project_knowledge_corpus_to_catalog(
    kv: Arc<dyn KVStore>,
    catalog_reader: Arc<dyn DataCatalog>,
    catalog: Arc<dyn CatalogWriter>,
    tenant_id: &str,
    entity_database_id: &str,
    database_id: &str,
) -> Result<(), DataCommandError> {
    let documents = list_runtime_documents(kv.as_ref(), tenant_id).await?;
    let knowledge_items = list_runtime_knowledge_items(kv.as_ref(), tenant_id).await?;

    if documents.is_empty() && knowledge_items.is_empty() {
        return Ok(());
    }

    let mut entries: Vec<CatalogEntry> = documents
        .iter()
        .map(|document| build_document_entry(document, entity_database_id))
        .collect();

    let mut document_by_knowledge_id = BTreeMap::new();
    for document in &documents {
        for knowledge_id in &document.extracted_knowledge_ids {
            document_by_knowledge_id.insert(knowledge_id.clone(), document.id.clone());
        }
    }

    for item in &knowledge_items {
        let source_document_id = item
            .source_document_id
            .as_deref()
            .or_else(|| document_by_knowledge_id.get(&item.id).map(String::as_str));
        let scope_links =
            resolve_knowledge_scope_links(catalog_reader.as_ref(), item, database_id).await?;
        let mut knowledge_entries = build_knowledge_entries_with_namespaces(
            std::slice::from_ref(item),
            entity_database_id,
            database_id,
            source_document_id,
            None,
        );
        for entry in &mut knowledge_entries {
            entry
                .links
                .retain(|relation| relation.kind != relation_kind::KNOWLEDGE_APPLIES_TO);
            for link in &scope_links {
                push_unique_scope_link(&mut entry.links, link.clone());
            }
        }
        entries.extend(knowledge_entries);
    }

    validate_knowledge_scope_targets(catalog_reader.as_ref(), &entries, database_id).await?;

    catalog
        .save_items(entries)
        .await
        .map_err(|error| DataCommandError::Execution(error.to_string()))?;
    Ok(())
}

async fn resolve_knowledge_scope_links(
    catalog: &dyn DataCatalog,
    item: &KnowledgeItem,
    database_id: &str,
) -> Result<Vec<CatalogRelation>, DataCommandError> {
    let mut links = Vec::new();

    for scope in &item.scope_tables {
        let target = resolve_table_scope(catalog, database_id, scope).await?;
        push_unique_scope_link(
            &mut links,
            CatalogRelation {
                target_id: target.id,
                kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                description: Some(format!("Applies to table {scope}")),
            },
        );
    }

    for scope in &item.scope_columns {
        let target = resolve_column_scope(catalog, database_id, scope).await?;
        push_unique_scope_link(
            &mut links,
            CatalogRelation {
                target_id: target.id,
                kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                description: Some(format!("Applies to column {scope}")),
            },
        );
    }

    Ok(links)
}

async fn resolve_table_scope(
    catalog: &dyn DataCatalog,
    database_id: &str,
    scope: &str,
) -> Result<CatalogEntry, DataCommandError> {
    let parts = split_scope_parts(scope);
    match parts.as_slice() {
        [table] => {
            let candidates = scope_candidates_by_name(catalog, CatalogKind::Table, *table).await?;
            resolve_unique_scope_target(candidates, database_id, "table", scope, |entry| {
                table_target_matches(entry, database_id, None, *table)
            })
        }
        [schema, table] => {
            let candidates = scope_candidates_by_name(catalog, CatalogKind::Table, *table).await?;
            resolve_unique_scope_target(candidates, database_id, "table", scope, |entry| {
                table_target_matches(entry, database_id, Some(*schema), *table)
            })
        }
        _ => Err(invalid_scope_error(
            "table",
            scope,
            "`table` or `schema.table`",
        )),
    }
}

async fn resolve_column_scope(
    catalog: &dyn DataCatalog,
    database_id: &str,
    scope: &str,
) -> Result<CatalogEntry, DataCommandError> {
    let parts = split_scope_parts(scope);
    match parts.as_slice() {
        [table, column] => {
            let candidates =
                scope_candidates_by_name(catalog, CatalogKind::Column, *column).await?;
            resolve_unique_scope_target(candidates, database_id, "column", scope, |entry| {
                column_target_matches(entry, database_id, None, *table, *column)
            })
        }
        [schema, table, column] => {
            let candidates =
                scope_candidates_by_name(catalog, CatalogKind::Column, *column).await?;
            resolve_unique_scope_target(candidates, database_id, "column", scope, |entry| {
                column_target_matches(entry, database_id, Some(*schema), *table, *column)
            })
        }
        _ => Err(invalid_scope_error(
            "column",
            scope,
            "`table.column` or `schema.table.column`",
        )),
    }
}

async fn scope_candidates_by_name(
    catalog: &dyn DataCatalog,
    kind: CatalogKind,
    name: &str,
) -> Result<Vec<CatalogEntry>, DataCommandError> {
    catalog
        .get_by_name(kind, name)
        .await
        .map_err(|error| DataCommandError::Execution(error.to_string()))
}

fn resolve_unique_scope_target(
    candidates: Vec<CatalogEntry>,
    database_id: &str,
    target_label: &str,
    scope: &str,
    matches_target: impl Fn(&CatalogEntry) -> bool,
) -> Result<CatalogEntry, DataCommandError> {
    let candidates = candidates
        .into_iter()
        .filter(matches_target)
        .collect::<Vec<_>>();

    match candidates.as_slice() {
        [] => Err(missing_scope_target_error(database_id, target_label, scope)),
        [target] => Ok(target.clone()),
        _ => Err(ambiguous_scope_target_error(
            database_id,
            target_label,
            scope,
            &candidates,
        )),
    }
}

fn table_target_matches(
    entry: &CatalogEntry,
    database_id: &str,
    schema_name: Option<&str>,
    table_name: &str,
) -> bool {
    let Ok(metadata) = decode_metadata::<TableMetadata>(entry) else {
        return false;
    };

    metadata.database_id == database_id
        && metadata.table_name == table_name
        && schema_name.map_or(true, |schema_name| metadata.schema_name == schema_name)
}

fn column_target_matches(
    entry: &CatalogEntry,
    database_id: &str,
    schema_name: Option<&str>,
    table_name: &str,
    column_name: &str,
) -> bool {
    let Ok(metadata) = decode_metadata::<ColumnMetadata>(entry) else {
        return false;
    };

    metadata.database_id == database_id
        && metadata.table_name == table_name
        && metadata.column_name == column_name
        && schema_name.map_or(true, |schema_name| metadata.schema_name == schema_name)
}

fn split_scope_parts(scope: &str) -> Vec<&str> {
    scope.trim().split('.').map(str::trim).collect()
}

fn invalid_scope_error(target_label: &str, scope: &str, expected: &str) -> DataCommandError {
    DataCommandError::Invalid(format!(
        "knowledge catalog projection found invalid {target_label} scope target `{scope}`; expected {expected}"
    ))
}

fn missing_scope_target_error(
    database_id: &str,
    target_label: &str,
    scope: &str,
) -> DataCommandError {
    DataCommandError::Invalid(format!(
        "knowledge catalog projection found missing scope targets for database_id `{database_id}`; unresolved {target_label} scope `{scope}`"
    ))
}

fn ambiguous_scope_target_error(
    database_id: &str,
    target_label: &str,
    scope: &str,
    candidates: &[CatalogEntry],
) -> DataCommandError {
    let mut ids = candidates
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    let preview = ids.into_iter().take(5).collect::<Vec<_>>().join(", ");
    DataCommandError::Invalid(format!(
        "knowledge catalog projection found ambiguous scope target for database_id `{database_id}`; {target_label} scope `{scope}` matched multiple catalog targets: {preview}"
    ))
}

fn push_unique_scope_link(links: &mut Vec<CatalogRelation>, link: CatalogRelation) {
    if links
        .iter()
        .any(|existing| existing.kind == link.kind && existing.target_id == link.target_id)
    {
        return;
    }
    links.push(link);
}

async fn validate_knowledge_scope_targets(
    catalog: &dyn DataCatalog,
    entries: &[CatalogEntry],
    database_id: &str,
) -> Result<(), DataCommandError> {
    let mut target_ids = BTreeSet::new();
    for entry in entries {
        if entry.kind != CatalogKind::Knowledge {
            continue;
        }
        for relation in &entry.links {
            if relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO {
                target_ids.insert(relation.target_id.clone());
            }
        }
    }

    if target_ids.is_empty() {
        return Ok(());
    }

    let ids = target_ids.iter().cloned().collect::<Vec<_>>();
    let existing = catalog
        .get_by_ids(&ids)
        .await
        .map_err(|error| DataCommandError::Execution(error.to_string()))?;
    let existing_by_id = existing
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let existing_ids = existing_by_id.keys().cloned().collect::<BTreeSet<_>>();
    let missing = target_ids
        .difference(&existing_ids)
        .cloned()
        .collect::<Vec<_>>();

    if missing.is_empty() {
        let mut mismatched = Vec::new();
        for target_id in &target_ids {
            let Some(entry) = existing_by_id.get(target_id) else {
                continue;
            };
            match catalog_entry_database_id(entry)? {
                Some(actual) if actual == database_id => {}
                Some(actual) => mismatched.push(format!("{target_id} (database_id `{actual}`)")),
                None => mismatched.push(format!("{target_id} (no database_id)")),
            }
        }
        if mismatched.is_empty() {
            return Ok(());
        }
        let preview = mismatched
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DataCommandError::Invalid(format!(
            "knowledge catalog projection found {} scope targets outside database_id `{}`; mismatched target ids: {}",
            mismatched.len(),
            database_id,
            preview
        )));
    }

    let preview = missing
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Err(DataCommandError::Invalid(format!(
        "knowledge catalog projection found {} missing scope targets for database_id `{}`; missing target ids: {}",
        missing.len(),
        database_id,
        preview
    )))
}

fn catalog_entry_database_id(entry: &CatalogEntry) -> Result<Option<String>, DataCommandError> {
    match entry.kind {
        CatalogKind::Table => Ok(Some(
            decode_metadata::<TableMetadata>(entry)
                .map_err(invalid_catalog_metadata_error)?
                .database_id,
        )),
        CatalogKind::Column => Ok(Some(
            decode_metadata::<ColumnMetadata>(entry)
                .map_err(invalid_catalog_metadata_error)?
                .database_id,
        )),
        CatalogKind::Enum => Ok(Some(
            decode_metadata::<EnumValueMetadata>(entry)
                .map_err(invalid_catalog_metadata_error)?
                .database_id,
        )),
        CatalogKind::Relationship => Ok(Some(
            decode_metadata::<RelationshipMetadata>(entry)
                .map_err(invalid_catalog_metadata_error)?
                .database_id,
        )),
        CatalogKind::DataQualityFinding => Ok(Some(
            decode_metadata::<DataQualityFindingMetadata>(entry)
                .map_err(invalid_catalog_metadata_error)?
                .database_id,
        )),
        CatalogKind::Metric
        | CatalogKind::Document
        | CatalogKind::Knowledge
        | CatalogKind::Special => Ok(None),
    }
}

fn invalid_catalog_metadata_error(error: agent_fw_catalog::CatalogError) -> DataCommandError {
    DataCommandError::Invalid(format!(
        "knowledge catalog projection found invalid scope target metadata: {error}"
    ))
}

async fn list_runtime_documents(
    kv: &dyn KVStore,
    tenant_id: &str,
) -> Result<Vec<DocumentItem>, DataCommandError> {
    knowledge_store::list_documents(kv, tenant_id)
        .await
        .map_err(|error| DataCommandError::Execution(error.to_string()))
}

async fn list_runtime_knowledge_items(
    kv: &dyn KVStore,
    tenant_id: &str,
) -> Result<Vec<KnowledgeItem>, DataCommandError> {
    knowledge_store::list_knowledge_items(kv, tenant_id)
        .await
        .map_err(|error| DataCommandError::Execution(error.to_string()))
}

fn validate_source(source: &KnowledgeSourceSpec) -> Result<(), DataCommandError> {
    match source {
        KnowledgeSourceSpec::LocalDirectory { path, .. } => {
            let workspace_root = std::env::current_dir()
                .and_then(|path| path.canonicalize())
                .map_err(|error| {
                    DataCommandError::Execution(format!(
                        "failed to resolve workspace root for knowledge ingestion: {error}"
                    ))
                })?;
            let canonical_path = path.canonicalize().map_err(|error| {
                DataCommandError::Invalid(format!(
                    "knowledge source path cannot be resolved: {} ({error})",
                    path.display()
                ))
            })?;
            if !canonical_path.starts_with(&workspace_root) {
                return Err(DataCommandError::Invalid(format!(
                    "knowledge source path must be inside workspace root {}: {}",
                    workspace_root.display(),
                    path.display()
                )));
            }
            if !canonical_path.exists() {
                return Err(DataCommandError::Invalid(format!(
                    "knowledge source path does not exist: {}",
                    path.display()
                )));
            }
            if !canonical_path.is_dir() {
                return Err(DataCommandError::Invalid(format!(
                    "knowledge source path must be a directory: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn into_framework_source(source: KnowledgeSourceSpec) -> FrameworkKnowledgeSourceSpec {
    match source {
        KnowledgeSourceSpec::LocalDirectory { path, extensions } => {
            FrameworkKnowledgeSourceSpec::LocalDirectory { path, extensions }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_fw_catalog::{CatalogError, JoinPath};
    use async_trait::async_trait;
    use serde_json::json;

    struct ExactLookupOnlyCatalog {
        entries: Vec<CatalogEntry>,
    }

    #[async_trait]
    impl DataCatalog for ExactLookupOnlyCatalog {
        async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
            Ok(self.entries.iter().find(|entry| entry.id == id).cloned())
        }

        async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(self
                .entries
                .iter()
                .filter(|entry| ids.contains(&entry.id))
                .cloned()
                .collect())
        }

        async fn get_by_name(
            &self,
            kind: CatalogKind,
            name: &str,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(self
                .entries
                .iter()
                .filter(|entry| entry.kind == kind && entry.name == name)
                .cloned()
                .collect())
        }

        async fn list_by_type(
            &self,
            _kind: CatalogKind,
            _limit: usize,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            panic!("scope resolution must use exact-name lookup instead of type-wide scans")
        }

        async fn get_related(
            &self,
            _id: &str,
            _relation_type: Option<&str>,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(vec![])
        }

        async fn find_join_path(
            &self,
            _from_table: &str,
            _to_table: &str,
        ) -> Result<Option<JoinPath>, CatalogError> {
            Ok(None)
        }

        async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(self
                .entries
                .iter()
                .filter(|entry| entry.kind == CatalogKind::Table)
                .cloned()
                .collect())
        }

        async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
            Ok(self
                .entries
                .iter()
                .filter(|entry| {
                    entry.kind == CatalogKind::Column
                        && decode_metadata::<ColumnMetadata>(entry)
                            .map(|metadata| metadata.table_name == table_name)
                            .unwrap_or(false)
                })
                .cloned()
                .collect())
        }

        async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
            Ok(vec![])
        }
    }

    #[test]
    fn validate_source_rejects_local_directory_outside_workspace_root() {
        let outside = std::env::temp_dir().join(format!(
            "flowai-knowledge-outside-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: outside.clone(),
            extensions: Vec::new(),
        };

        let error = validate_source(&source).unwrap_err();

        assert!(error
            .to_string()
            .contains("knowledge source path must be inside workspace root"));
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn scope_resolution_uses_exact_name_lookup_without_type_scan() {
        let catalog = ExactLookupOnlyCatalog {
            entries: vec![
                table_entry("table-orders", "public", "orders"),
                column_entry(
                    "column-orders-velocity",
                    "public",
                    "orders",
                    "velocity_ratio",
                ),
                table_entry("table-other", "public", "other_table"),
            ],
        };
        let item = knowledge_item(
            vec!["orders".to_string()],
            vec!["orders.velocity_ratio".to_string()],
        );

        let links = resolve_knowledge_scope_links(&catalog, &item, "warehouse")
            .await
            .unwrap();

        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|link| link.target_id == "table-orders"));
        assert!(links
            .iter()
            .any(|link| link.target_id == "column-orders-velocity"));
    }

    #[tokio::test]
    async fn unqualified_scope_detects_ambiguity_from_all_exact_name_candidates() {
        let catalog = ExactLookupOnlyCatalog {
            entries: vec![
                table_entry("table-public-orders", "public", "orders"),
                table_entry("table-analytics-orders", "analytics", "orders"),
            ],
        };
        let item = knowledge_item(vec!["orders".to_string()], vec![]);

        let err = resolve_knowledge_scope_links(&catalog, &item, "warehouse")
            .await
            .err()
            .expect("ambiguous unqualified scope should fail");

        assert!(err.to_string().contains("ambiguous scope target"));
    }

    #[tokio::test]
    async fn knowledge_scope_validation_rejects_cross_database_links() {
        let mut other_database_table = table_entry("table-other-orders", "public", "orders");
        other_database_table.metadata["databaseId"] = json!("other_warehouse");
        let catalog = ExactLookupOnlyCatalog {
            entries: vec![other_database_table],
        };
        let knowledge = CatalogEntry {
            id: "knowledge:orders".to_string(),
            kind: CatalogKind::Knowledge,
            name: "Orders rule".to_string(),
            qualified_name: None,
            content: "Orders knowledge".to_string(),
            tags: vec![],
            links: vec![CatalogRelation {
                target_id: "table-other-orders".to_string(),
                kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                description: None,
            }],
            metadata: json!({}),
        };

        let err = validate_knowledge_scope_targets(&catalog, &[knowledge], "warehouse")
            .await
            .err()
            .expect("cross-database knowledge link should fail");

        assert!(err.to_string().contains("outside database_id `warehouse`"));
        assert!(err.to_string().contains("table-other-orders"));
        assert!(err.to_string().contains("other_warehouse"));
    }

    #[tokio::test]
    async fn knowledge_scope_validation_accepts_same_database_links() {
        let catalog = ExactLookupOnlyCatalog {
            entries: vec![table_entry("table-orders", "public", "orders")],
        };
        let knowledge = CatalogEntry {
            id: "knowledge:orders".to_string(),
            kind: CatalogKind::Knowledge,
            name: "Orders rule".to_string(),
            qualified_name: None,
            content: "Orders knowledge".to_string(),
            tags: vec![],
            links: vec![CatalogRelation {
                target_id: "table-orders".to_string(),
                kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                description: None,
            }],
            metadata: json!({}),
        };

        validate_knowledge_scope_targets(&catalog, &[knowledge], "warehouse")
            .await
            .expect("same-database knowledge links should validate");
    }

    fn knowledge_item(scope_tables: Vec<String>, scope_columns: Vec<String>) -> KnowledgeItem {
        KnowledgeItem {
            id: "knowledge-0".to_string(),
            name: "Knowledge".to_string(),
            description: "Knowledge description".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::Constraint,
            scope_tables,
            scope_columns,
            sql_expression: None,
            synonyms: vec![],
            source_document_id: None,
        }
    }

    fn table_entry(id: &str, schema_name: &str, table_name: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind: CatalogKind::Table,
            name: table_name.to_string(),
            qualified_name: Some(format!("{schema_name}.{table_name}")),
            content: String::new(),
            tags: vec![],
            links: vec![],
            metadata: json!({
                "databaseId": "warehouse",
                "schemaName": schema_name,
                "tableName": table_name,
                "relationType": "base_table",
                "rowCount": null,
                "columnCount": 1,
                "preferredQuerySurface": true,
                "source": {},
            }),
        }
    }

    fn column_entry(
        id: &str,
        schema_name: &str,
        table_name: &str,
        column_name: &str,
    ) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind: CatalogKind::Column,
            name: column_name.to_string(),
            qualified_name: Some(format!("{schema_name}.{table_name}.{column_name}")),
            content: String::new(),
            tags: vec![],
            links: vec![],
            metadata: json!({
                "databaseId": "warehouse",
                "schemaName": schema_name,
                "tableName": table_name,
                "columnName": column_name,
                "dataType": "numeric",
                "nullable": false,
                "primaryKey": false,
                "foreignKey": null,
                "semanticType": null,
                "distinctCount": null,
                "nullCount": null,
                "totalCount": null,
                "lowCardinalityEnum": false,
            }),
        }
    }
}
