use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_algebra::{KVStore, KVStoreExt, TargetDatabase};
use agent_fw_catalog::{
    decode_metadata, CatalogEntry, CatalogKind, CatalogScope, CatalogSearchBackend,
    CatalogSearchHealth, CatalogSearchRequest, CatalogToolEnvironmentExt, ColumnMetadata,
    DataCatalog, DataQualityFindingMetadata, DocumentItem, DocumentMetadata, EnumValueMetadata,
    JoinHop, JoinPath, KnowledgeItem, KnowledgeMetadata, MetricMetadata, RelationshipMetadata,
    SemanticEntityKind, TableMetadata,
};
use agent_fw_core::{WorkspaceContext, WorkspaceId};
use agent_fw_tool::{ToolEnvironment, ToolExtensionManifest, ToolSchema};
use async_trait::async_trait;

use crate::{tier1_discovery, tool_metadata, CatalogToolError};

use super::graph::RELATIONSHIP_SCAN_LIMIT;
use super::{
    CatalogEntityAssembler, CatalogEntityKind, CatalogFilterResolver, CatalogFilters,
    CatalogGraphEdge, CatalogGraphService, CatalogRef, CatalogRefResolver, CatalogRelationPath,
    CatalogRelationPathStep, ExecuteQueryInput, ExecuteQueryOutput, FacetValue,
    GetCatalogEntitiesInput, GetCatalogEntitiesOutput, GetCatalogRelationsInput,
    GetCatalogRelationsOutput, GetRelationPathsBetweenInput, GetRelationPathsBetweenOutput,
    ListSchemaFieldsInput, ListSchemaFieldsOutput, MatchDiagnostics, OutputPolicyRegistry,
    Pagination, PathType, RelationDirection, SampleTableDataInput, SampleTableDataOutput,
    SchemaFieldsForTable, SearchCatalogDiagnostics, SearchCatalogFacets, SearchCatalogInput,
    SearchCatalogOutput,
};

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;
const SEARCH_OVERFETCH_MULTIPLIER: usize = 3;
const MAX_SEARCH_WINDOW: usize = 200;
const MAX_SEARCH_ROUND_TRIPS: usize = 3;
const MAX_ENTITY_REFS: usize = 50;
const MAX_TABLE_REFS: usize = 10;
const DEFAULT_FIELDS_LIMIT: usize = 200;
const MAX_FIELDS_LIMIT: usize = 200;
const DEFAULT_SAMPLE_LIMIT: usize = 10;
const MAX_SAMPLE_LIMIT: usize = 20;
const MAX_PATH_DEPTH: usize = 6;
const DOCUMENT_KV_PREFIX: &str = "data:document:";
const KNOWLEDGE_KV_PREFIX: &str = "data:knowledge:";

pub struct SearchCatalogHandler;
pub struct GetCatalogEntitiesHandler;
pub struct ListSchemaFieldsHandler;
pub struct GetCatalogRelationsHandler;
pub struct GetRelationPathsBetweenHandler;
pub struct SampleTableDataHandler;
pub struct ExecuteQueryHandler;

pub fn surface_handlers() -> Vec<Arc<dyn ToolHandler>> {
    vec![
        Arc::new(SearchCatalogHandler),
        Arc::new(GetCatalogEntitiesHandler),
        Arc::new(ListSchemaFieldsHandler),
        Arc::new(GetCatalogRelationsHandler),
        Arc::new(GetRelationPathsBetweenHandler),
        Arc::new(SampleTableDataHandler),
        Arc::new(ExecuteQueryHandler),
    ]
}

fn invalid_input(tool_use_id: &str, err: impl std::fmt::Display) -> ToolCallResult {
    ToolCallResult::error(tool_use_id, format!("Invalid input: {err}"))
}

fn serialize_ok<T: serde::Serialize>(tool_use_id: &str, value: T) -> ToolCallResult {
    match serde_json::to_value(value) {
        Ok(value) => ToolCallResult::success(tool_use_id, value),
        Err(error) => ToolCallResult::error(
            tool_use_id,
            format!("Failed to serialize tool output: {error}"),
        ),
    }
}

fn catalog_scope_from_env(env: &ToolEnvironment) -> CatalogScope {
    if let Some(context) = env.maybe_ext::<WorkspaceContext>() {
        return CatalogScope::new(context.base_tenant_id.clone(), context.workspace_id.clone());
    }

    CatalogScope::new(
        env.tenant().resource_id().clone(),
        WorkspaceId::default_workspace(),
    )
}

async fn ensure_search_ready(
    backend: &dyn CatalogSearchBackend,
    scope: &CatalogScope,
) -> Result<(), CatalogToolError> {
    match backend.health(scope).await? {
        CatalogSearchHealth::Ready { .. } => Ok(()),
        CatalogSearchHealth::Stale { reason, .. } => Err(CatalogToolError::Validation(format!(
            "Catalog search index is stale: {reason}. Rebuild the catalog search index before retrying search_catalog."
        ))),
        CatalogSearchHealth::Unavailable { reason } => Err(CatalogToolError::Validation(format!(
            "Catalog search index is unavailable: {reason}. Rebuild or attach the catalog search index before retrying search_catalog."
        ))),
    }
}

macro_rules! surface_handler {
    (
        $handler:ident,
        $metadata:expr,
        $input:ty,
        $func:ident,
        [$($required:ty => $reason:literal),* $(,)?]
    ) => {
        #[async_trait]
        impl ToolHandler for $handler {
            fn definition(&self) -> ToolDefinition {
                let metadata = $metadata;
                ToolDefinition {
                    name: metadata.name.to_string(),
                    description: metadata.description.to_string(),
                    input_schema: <$input>::json_schema(),
                }
            }

            fn extension_manifest(&self) -> ToolExtensionManifest {
                let manifest = ToolExtensionManifest::new();
                $(let manifest = manifest.requires::<$required>($reason);)*
                manifest
            }

            async fn handle(
                &self,
                tool_use_id: &str,
                input: serde_json::Value,
                env: &ToolEnvironment,
            ) -> ToolCallResult {
                let parsed: $input = match serde_json::from_value(input) {
                    Ok(value) => value,
                    Err(error) => return invalid_input(tool_use_id, error),
                };
                match $func(env, parsed).await {
                    Ok(output) => serialize_ok(tool_use_id, output),
                    Err(error) => ToolCallResult::error(tool_use_id, error.to_string()),
                }
            }
        }
    };
}

surface_handler!(
    SearchCatalogHandler,
    tool_metadata::SEARCH_CATALOG,
    SearchCatalogInput,
    search_catalog_from_env,
    [
        dyn DataCatalog => "DataCatalog for search_catalog hydration",
        dyn CatalogSearchBackend => "CatalogSearchBackend for search_catalog lexical retrieval",
    ]
);

surface_handler!(
    GetCatalogEntitiesHandler,
    tool_metadata::GET_CATALOG_ENTITIES,
    GetCatalogEntitiesInput,
    get_catalog_entities_from_env,
    [dyn DataCatalog => "DataCatalog for get_catalog_entities"]
);

surface_handler!(
    ListSchemaFieldsHandler,
    tool_metadata::LIST_SCHEMA_FIELDS,
    ListSchemaFieldsInput,
    list_schema_fields_from_env,
    [dyn DataCatalog => "DataCatalog for list_schema_fields"]
);

surface_handler!(
    GetCatalogRelationsHandler,
    tool_metadata::GET_CATALOG_RELATIONS,
    GetCatalogRelationsInput,
    get_catalog_relations_from_env,
    [dyn DataCatalog => "DataCatalog for get_catalog_relations"]
);

surface_handler!(
    GetRelationPathsBetweenHandler,
    tool_metadata::GET_RELATION_PATHS_BETWEEN,
    GetRelationPathsBetweenInput,
    get_relation_paths_between_from_env,
    [dyn DataCatalog => "DataCatalog for get_relation_paths_between"]
);

surface_handler!(
    SampleTableDataHandler,
    tool_metadata::SAMPLE_TABLE_DATA,
    SampleTableDataInput,
    sample_table_data_from_env,
    [
        dyn DataCatalog => "DataCatalog for sample_table_data table and column validation",
        dyn TargetDatabase => "TargetDatabase for sample_table_data row access",
    ]
);

surface_handler!(
    ExecuteQueryHandler,
    tool_metadata::EXECUTE_QUERY,
    ExecuteQueryInput,
    execute_query_from_env,
    [dyn TargetDatabase => "TargetDatabase for execute_query"]
);

async fn search_catalog_from_env(
    env: &ToolEnvironment,
    input: SearchCatalogInput,
) -> Result<SearchCatalogOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    let backend = require_search_backend_for_result(env)?;
    let scope = catalog_scope_from_env(env);
    search_catalog(catalog.as_ref(), backend.as_ref(), &scope, input).await
}

async fn get_catalog_entities_from_env(
    env: &ToolEnvironment,
    input: GetCatalogEntitiesInput,
) -> Result<GetCatalogEntitiesOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    let scope = catalog_scope_from_env(env);
    get_catalog_entities_with_kv(catalog.as_ref(), Some(env.kv().as_ref()), &scope, input).await
}

async fn list_schema_fields_from_env(
    env: &ToolEnvironment,
    input: ListSchemaFieldsInput,
) -> Result<ListSchemaFieldsOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    list_schema_fields(catalog.as_ref(), input).await
}

async fn get_catalog_relations_from_env(
    env: &ToolEnvironment,
    input: GetCatalogRelationsInput,
) -> Result<GetCatalogRelationsOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    get_catalog_relations(catalog.as_ref(), input).await
}

async fn get_relation_paths_between_from_env(
    env: &ToolEnvironment,
    input: GetRelationPathsBetweenInput,
) -> Result<GetRelationPathsBetweenOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    get_relation_paths_between(catalog.as_ref(), input).await
}

async fn sample_table_data_from_env(
    env: &ToolEnvironment,
    input: SampleTableDataInput,
) -> Result<SampleTableDataOutput, CatalogToolError> {
    let catalog = require_catalog_for_result(env)?;
    let target_db = require_target_db_for_result(env)?;
    sample_table_data(catalog.as_ref(), target_db.as_ref(), input).await
}

async fn execute_query_from_env(
    env: &ToolEnvironment,
    input: ExecuteQueryInput,
) -> Result<ExecuteQueryOutput, CatalogToolError> {
    let target_db = require_target_db_for_result(env)?;
    execute_query(target_db.as_ref(), input).await
}

fn require_catalog_for_result(
    env: &ToolEnvironment,
) -> Result<&Arc<dyn DataCatalog>, CatalogToolError> {
    env.try_catalog()
        .map_err(|error| CatalogToolError::Validation(error.message().to_string()))
}

fn require_search_backend_for_result(
    env: &ToolEnvironment,
) -> Result<&Arc<dyn CatalogSearchBackend>, CatalogToolError> {
    env.try_catalog_search_backend().map_err(|error| {
        CatalogToolError::Validation(format!(
            "{}. Rebuild or attach the catalog search index before retrying search_catalog.",
            error.message()
        ))
    })
}

fn require_target_db_for_result(
    env: &ToolEnvironment,
) -> Result<&Arc<dyn TargetDatabase>, CatalogToolError> {
    env.try_target_db()
        .map_err(|error| CatalogToolError::Validation(error.message().to_string()))
}

pub async fn search_catalog(
    catalog: &dyn DataCatalog,
    backend: &dyn CatalogSearchBackend,
    scope: &CatalogScope,
    input: SearchCatalogInput,
) -> Result<SearchCatalogOutput, CatalogToolError> {
    let query = input.query.trim().to_string();
    if query.is_empty() {
        return Err(CatalogToolError::Validation(
            "search_catalog.query must not be empty".to_string(),
        ));
    }
    let requested_limit = input
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let assembler = assembler_for("search_catalog", requested_limit);
    let limit = requested_limit.min(assembler.policy().max_entities);
    let kinds = search_kinds(&input.kinds)?;
    ensure_search_ready(backend, scope).await?;

    let filter_input = input.filters.unwrap_or_default();
    let resolver = CatalogRefResolver::new(catalog);
    let resolved_filters = CatalogFilterResolver::new(resolver)
        .resolve(&filter_input)
        .await?;
    let filter_resolution = resolved_filters.resolution.clone();

    let mut results = Vec::new();
    let mut warnings = Vec::new();
    if requested_limit > limit {
        warnings.push(format!(
            "search_catalog limit reduced from {requested_limit} to {limit} by output policy {}",
            assembler.policy().id
        ));
    }
    let mut next_cursor = input.cursor.map(agent_fw_catalog::CatalogSearchCursor::new);
    // The opaque resume cursor of the last candidate we examined from any window.
    // The returned page cursor encodes this (the last *consumed* candidate), not
    // the backend window-end cursor, so the next page neither skips nor
    // re-returns ids when we over-fetch a window but emit only `limit` survivors.
    let mut consumed_resume_cursor: Option<agent_fw_catalog::CatalogSearchCursor> = None;
    // True when the per-hit loop stopped because the page filled to `limit`,
    // which means the current window still held candidates we never examined.
    let mut page_filled = false;
    let mut has_more = false;
    let mut facets = SearchCatalogFacets::default();
    let mut candidate_count = 0usize;
    let mut hydrated_count = 0usize;
    let mut dropped_by_recheck = 0usize;
    let mut round_trips = 0usize;
    let fetch_limit = (limit * SEARCH_OVERFETCH_MULTIPLIER).min(MAX_SEARCH_WINDOW);

    while results.len() < limit && round_trips < MAX_SEARCH_ROUND_TRIPS {
        round_trips += 1;
        let backend_results = backend
            .search(
                scope,
                CatalogSearchRequest {
                    query: query.clone(),
                    kinds: kinds.clone(),
                    filters: resolved_filters.backend_filters.clone(),
                    limit: fetch_limit,
                    cursor: next_cursor.clone(),
                },
            )
            .await?;
        if round_trips == 1 {
            facets = map_facets(backend_results.facets);
        }
        warnings.extend(backend_results.warnings);
        has_more = backend_results.has_more;
        next_cursor = backend_results.next_cursor.clone();

        let hit_by_id: Vec<_> = backend_results.hits;
        let ids: Vec<String> = hit_by_id.iter().map(|hit| hit.entry_id.clone()).collect();
        let entries = catalog.get_by_ids(&ids).await?;
        hydrated_count += entries.len();
        let entries_by_id: HashMap<String, CatalogEntry> = entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();

        for hit in hit_by_id {
            candidate_count += 1;
            // The consumed unit is the candidate: record its resume cursor for
            // every hit we examine (including ones dropped by the recheck), so a
            // page that fills mid-window still resumes strictly after the last
            // candidate it touched.
            consumed_resume_cursor = hit.resume_cursor.clone();
            let Some(entry) = entries_by_id.get(&hit.entry_id).cloned() else {
                dropped_by_recheck += 1;
                continue;
            };
            if !entry.kind.is_public_searchable()
                || !matches_public_kinds(&entry, &input.kinds)
                || !entry_matches_filters(&entry, &filter_resolution.applied)
            {
                dropped_by_recheck += 1;
                continue;
            }
            let mut diagnostics = MatchDiagnostics::from(&hit);
            diagnostics.rank = results.len() + 1;
            let assembly = assembler.assemble_with_match(entry, Some(diagnostics));
            warnings.extend(assembly.warnings);
            if let Some(entity) = assembly.entity {
                results.push(entity);
                if results.len() == limit {
                    page_filled = true;
                    break;
                }
            }
        }

        if !has_more || next_cursor.is_none() {
            break;
        }
    }

    if dropped_by_recheck > 0 {
        warnings.push(format!(
            "dropped {dropped_by_recheck} stale or non-matching search candidates after catalog hydration"
        ));
    }

    // Resume after the last *consumed* candidate, not at the backend window end.
    // More candidates remain to deliver whenever the page filled to `limit`
    // (so the current window still held unexamined hits) or the backend reported
    // additional windows beyond what we fetched. Only when both are false has the
    // candidate stream been fully drained and consumed.
    let page_has_more = consumed_resume_cursor.is_some() && (page_filled || has_more);
    let page_next_cursor = if page_has_more {
        consumed_resume_cursor
    } else {
        None
    };

    let returned = results.len();
    Ok(SearchCatalogOutput {
        query,
        results,
        suggested_filters: suggested_filters(&facets),
        facets,
        filter_resolution,
        pagination: Pagination {
            limit,
            returned,
            has_more: page_has_more,
            next_cursor: page_next_cursor.map(|cursor| cursor.as_str().to_string()),
        },
        warnings,
        diagnostics: SearchCatalogDiagnostics {
            search_mode: "lexical".to_string(),
            backend: "runtime_internal".to_string(),
            hydrated_count,
            candidate_count,
            dropped_by_recheck: Some(dropped_by_recheck),
            round_trips: Some(round_trips),
        },
    })
}

pub async fn get_catalog_entities(
    catalog: &dyn DataCatalog,
    input: GetCatalogEntitiesInput,
) -> Result<GetCatalogEntitiesOutput, CatalogToolError> {
    get_catalog_entities_with_kv(catalog, None, &CatalogScope::legacy_unscoped(), input).await
}

async fn get_catalog_entities_with_kv(
    catalog: &dyn DataCatalog,
    kv: Option<&dyn KVStore>,
    scope: &CatalogScope,
    input: GetCatalogEntitiesInput,
) -> Result<GetCatalogEntitiesOutput, CatalogToolError> {
    if input.refs.is_empty() || input.refs.len() > MAX_ENTITY_REFS {
        return Err(CatalogToolError::Validation(format!(
            "get_catalog_entities.refs must contain 1..={MAX_ENTITY_REFS} refs"
        )));
    }
    let assembler = assembler_for("get_catalog_entities", input.refs.len());
    let resolver = CatalogRefResolver::new(catalog);
    let mut entities = Vec::new();
    let mut missing = Vec::new();
    let mut warnings = Vec::new();
    let mut resolved_refs = Vec::new();

    for reference in input.refs {
        let resolution = resolver.resolve_exact(&reference).await?;
        let Some(resolved) = resolution.resolved else {
            if !resolution.ambiguous.is_empty() {
                warnings.push(format!(
                    "ambiguous catalog ref {} matched {} entities",
                    reference.display_input(),
                    resolution.ambiguous.len()
                ));
            }
            missing.push(reference);
            continue;
        };
        resolved_refs.push(resolved);
    }

    let ids = resolved_refs
        .iter()
        .map(|resolved| resolved.id.clone())
        .collect::<Vec<_>>();
    let entries = catalog.get_by_ids(&ids).await?;
    let entries_by_id: HashMap<String, CatalogEntry> = entries
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();

    for resolved in resolved_refs {
        let Some(entry) = entries_by_id.get(&resolved.id).cloned() else {
            missing.push(resolved.catalog_ref());
            continue;
        };
        let entry = hydrate_body_from_kv(entry, kv, scope, &mut warnings).await?;
        let assembly = assembler.assemble(entry);
        warnings.extend(assembly.warnings);
        if let Some(entity) = assembly.entity {
            entities.push(entity);
        }
    }

    Ok(GetCatalogEntitiesOutput {
        entities,
        missing,
        warnings,
    })
}

async fn hydrate_body_from_kv(
    mut entry: CatalogEntry,
    kv: Option<&dyn KVStore>,
    scope: &CatalogScope,
    warnings: &mut Vec<String>,
) -> Result<CatalogEntry, CatalogToolError> {
    let Some(kv) = kv else {
        return Ok(entry);
    };
    let workspace_tenant_id =
        WorkspaceContext::from_ids(scope.tenant_id.clone(), Some(scope.workspace_id.as_str()))
            .workspace_tenant_id()
            .to_string();
    match entry.kind {
        CatalogKind::Document => {
            let metadata = match decode_metadata::<DocumentMetadata>(&entry) {
                Ok(metadata) => metadata,
                Err(_) => return Ok(entry),
            };
            let key = format!("{DOCUMENT_KV_PREFIX}{}", metadata.source_document_id);
            match kv.get::<DocumentItem>(&workspace_tenant_id, &key).await {
                Ok(Some(document)) if !document.content.trim().is_empty() => {
                    entry.content = document.content;
                }
                Ok(_) if metadata.content_available => warnings.push(format!(
                    "document body for {} was marked available but was not found in KV",
                    entry.id
                )),
                Ok(_) => {}
                Err(error) => warnings.push(format!(
                    "failed to hydrate document body for {} from KV: {error}",
                    entry.id
                )),
            }
        }
        CatalogKind::Knowledge => {
            let metadata = match decode_metadata::<KnowledgeMetadata>(&entry) {
                Ok(metadata) => metadata,
                Err(_) => return Ok(entry),
            };
            let Some(source_knowledge_id) = metadata
                .source_knowledge_id
                .as_deref()
                .or_else(|| entry.id.strip_prefix("knowledge:"))
            else {
                return Ok(entry);
            };
            let key = format!("{KNOWLEDGE_KV_PREFIX}{source_knowledge_id}");
            match kv.get::<KnowledgeItem>(&workspace_tenant_id, &key).await {
                Ok(Some(item)) if !item.description.trim().is_empty() => {
                    entry.content = item.description;
                }
                Ok(_) => warnings.push(format!(
                    "knowledge body for {} was not found in KV",
                    entry.id
                )),
                Err(error) => warnings.push(format!(
                    "failed to hydrate knowledge body for {} from KV: {error}",
                    entry.id
                )),
            }
        }
        _ => {}
    }
    Ok(entry)
}

pub async fn list_schema_fields(
    catalog: &dyn DataCatalog,
    input: ListSchemaFieldsInput,
) -> Result<ListSchemaFieldsOutput, CatalogToolError> {
    if input.tables.is_empty() || input.tables.len() > MAX_TABLE_REFS {
        return Err(CatalogToolError::Validation(format!(
            "list_schema_fields.tables must contain 1..={MAX_TABLE_REFS} refs"
        )));
    }
    let filter_input = input.filters.unwrap_or_default();
    let filter_resolution = CatalogFilterResolver::new(CatalogRefResolver::new(catalog))
        .resolve(&filter_input)
        .await?
        .resolution;
    let limit = input
        .limit_per_table
        .unwrap_or(DEFAULT_FIELDS_LIMIT)
        .clamp(1, MAX_FIELDS_LIMIT);
    let assembler = assembler_for("list_schema_fields", input.tables.len());
    let resolver = CatalogRefResolver::new(catalog);
    let mut output_tables = Vec::new();
    let mut warnings = Vec::new();

    for reference in input.tables {
        let resolution = resolver.resolve_exact(&reference).await?;
        let Some(resolved) = resolution.resolved else {
            warnings.push(format!(
                "could not resolve table ref {}",
                reference.display_input()
            ));
            continue;
        };
        let Some(table_entry) = catalog.get_by_id(&resolved.id).await? else {
            warnings.push(format!("resolved table {} disappeared", resolved.id));
            continue;
        };
        if table_entry.kind != CatalogKind::Table {
            warnings.push(format!("{} is not a table", table_entry.id));
            continue;
        }
        let table_assembly = assembler.assemble(table_entry.clone());
        let Some(table) = table_assembly.entity else {
            warnings.extend(table_assembly.warnings);
            continue;
        };
        let mut table_warnings = table_assembly.warnings;
        let columns = catalog_columns(catalog, &table_entry).await?;
        let mut fields = Vec::new();
        for column in columns {
            if !column_matches_filters(&column, &filter_resolution.applied) {
                continue;
            }
            let assembly = assembler.assemble(column);
            table_warnings.extend(assembly.warnings);
            if let Some(entity) = assembly.entity {
                fields.push(entity);
            }
        }
        let has_more = fields.len() > limit;
        fields.truncate(limit);
        let returned = fields.len();
        output_tables.push(SchemaFieldsForTable {
            table,
            fields,
            pagination: Pagination {
                limit,
                returned,
                has_more,
                next_cursor: None,
            },
            warnings: table_warnings,
        });
    }

    Ok(ListSchemaFieldsOutput {
        tables: output_tables,
        filter_resolution,
        warnings,
    })
}

pub async fn get_catalog_relations(
    catalog: &dyn DataCatalog,
    input: GetCatalogRelationsInput,
) -> Result<GetCatalogRelationsOutput, CatalogToolError> {
    let assembler = assembler_for("get_catalog_relations", input.refs.len());
    CatalogGraphService::new(catalog, CatalogRefResolver::new(catalog), assembler)
        .get_relations(input)
        .await
}

pub async fn get_relation_paths_between(
    catalog: &dyn DataCatalog,
    input: GetRelationPathsBetweenInput,
) -> Result<GetRelationPathsBetweenOutput, CatalogToolError> {
    let path_type = input.path_type.unwrap_or_default();
    let max_depth = input
        .max_depth
        .unwrap_or(MAX_PATH_DEPTH)
        .clamp(1, MAX_PATH_DEPTH);
    let assembler = assembler_for("get_relation_paths_between", input.to.len().max(1));
    let resolver = CatalogRefResolver::new(catalog);
    let mut warnings = Vec::new();
    let from_resolution = resolver.resolve_exact(&input.from_ref).await?;
    let Some(from_ref) = from_resolution.resolved else {
        return Ok(GetRelationPathsBetweenOutput {
            paths: Vec::new(),
            warnings: vec![format!(
                "could not resolve path source {}",
                input.from_ref.display_input()
            )],
        });
    };
    let Some(from_entry) = catalog.get_by_id(&from_ref.id).await? else {
        return Ok(GetRelationPathsBetweenOutput {
            paths: Vec::new(),
            warnings: vec![format!("resolved path source {} disappeared", from_ref.id)],
        });
    };
    let Some(from_entity) = assembler.assemble(from_entry.clone()).entity else {
        return Ok(GetRelationPathsBetweenOutput {
            paths: Vec::new(),
            warnings: vec![format!("path source {} cannot be emitted", from_entry.id)],
        });
    };

    // Load the relationship vertex set ONCE for the whole request and share it
    // across every BFS node of every target, instead of re-scanning the catalog
    // once per dequeued node (M7).
    let relationships = Arc::new(
        catalog
            .list_by_type(CatalogKind::Relationship, RELATIONSHIP_SCAN_LIMIT)
            .await?,
    );

    let mut paths = Vec::new();
    for target_ref in input.to {
        let target_resolution = resolver.resolve_exact(&target_ref).await?;
        let Some(to_ref) = target_resolution.resolved else {
            warnings.push(format!(
                "could not resolve path target {}",
                target_ref.display_input()
            ));
            continue;
        };
        let Some(to_entry) = catalog.get_by_id(&to_ref.id).await? else {
            warnings.push(format!("resolved path target {} disappeared", to_ref.id));
            continue;
        };
        let Some(to_entity) = assembler.assemble(to_entry.clone()).entity else {
            warnings.push(format!("path target {} cannot be emitted", to_entry.id));
            continue;
        };

        let mut truncation_seen = false;
        let path = match path_type {
            PathType::Join => join_path(catalog, &assembler, &from_entry, &to_entry).await?,
            PathType::Semantic => {
                semantic_path(
                    catalog,
                    &assembler,
                    &from_entry,
                    &to_entry,
                    max_depth,
                    &relationships,
                    &mut truncation_seen,
                )
                .await?
            }
            // M6: `any` tries the join-only (materialized FK) path first, then
            // falls back to the semantic BFS. `join_path` returns `Ok(None)` for
            // non-table endpoints, so the fallback still covers those.
            PathType::Any => match join_path(catalog, &assembler, &from_entry, &to_entry).await? {
                Some(steps) => Some(steps),
                None => {
                    semantic_path(
                        catalog,
                        &assembler,
                        &from_entry,
                        &to_entry,
                        max_depth,
                        &relationships,
                        &mut truncation_seen,
                    )
                    .await?
                }
            },
        };
        let found = path.is_some();
        let steps = path.unwrap_or_default();
        let length = steps.len();
        // M5: a `found == false` that coincided with adjacency truncation at a
        // hub node is reported as a path that *may* exist but was not explored,
        // so the caller can distinguish it from a definitive "no path".
        let path_warnings = if !found && truncation_seen {
            vec![
                "path search truncated adjacency at one or more hub nodes (limit_per_ref reached); a path may exist but was not explored".to_string(),
            ]
        } else {
            Vec::new()
        };
        paths.push(CatalogRelationPath {
            from_entity: from_entity.clone(),
            to: to_entity,
            found,
            path_type,
            steps,
            length,
            warnings: path_warnings,
        });
    }

    Ok(GetRelationPathsBetweenOutput { paths, warnings })
}

pub async fn sample_table_data(
    catalog: &dyn DataCatalog,
    target_db: &dyn TargetDatabase,
    input: SampleTableDataInput,
) -> Result<SampleTableDataOutput, CatalogToolError> {
    let resolver = CatalogRefResolver::new(catalog);
    let resolution = resolver.resolve_exact(&input.table).await?;
    let Some(resolved) = resolution.resolved else {
        return Err(CatalogToolError::NotFound(format!(
            "table not found: {}",
            input.table.display_input()
        )));
    };
    let Some(table_entry) = catalog.get_by_id(&resolved.id).await? else {
        return Err(CatalogToolError::NotFound(format!(
            "resolved table disappeared: {}",
            resolved.id
        )));
    };
    if table_entry.kind != CatalogKind::Table {
        return Err(CatalogToolError::Validation(format!(
            "{} is not a table",
            table_entry.id
        )));
    }
    let table_name = target_table_name(&table_entry)?;
    if !crate::is_valid_table_name(&table_name) {
        return Err(CatalogToolError::Validation(format!(
            "Invalid table name: {table_name}"
        )));
    }

    let columns = catalog_columns(catalog, &table_entry).await?;
    let selected_columns = selected_column_names(&columns, &input.columns)?;
    let limit = input
        .limit
        .unwrap_or(DEFAULT_SAMPLE_LIMIT)
        .clamp(1, MAX_SAMPLE_LIMIT);
    let rows = target_db.sample_table(&table_name, limit).await?;
    // When the projection is implicit (empty `input.columns`), the rows are
    // projected to the catalog-known columns. A column that exists in the real
    // target row but is absent from the catalog would otherwise be silently
    // dropped, so surface a data-quality warning before projection strips it.
    let warnings = if input.columns.is_empty() {
        stale_catalog_column_warnings(&rows, &selected_columns)
    } else {
        Vec::new()
    };
    let rows = project_rows(rows, &selected_columns);
    let row_count = rows.len();
    let table = assembler_for("sample_table_data", 1)
        .assemble(table_entry)
        .entity
        .ok_or_else(|| {
            CatalogToolError::Validation("sample_table_data table cannot be emitted".to_string())
        })?;

    Ok(SampleTableDataOutput {
        table,
        columns: if selected_columns.is_empty() {
            infer_columns(&rows)
        } else {
            selected_columns
        },
        rows,
        row_count,
        sample_note: None,
        warnings,
    })
}

/// Detect columns present in the raw target rows but missing from the
/// catalog-known projection, and emit a single data-quality warning naming
/// them. Empty when every observed column is catalog-known.
fn stale_catalog_column_warnings(
    rows: &[serde_json::Value],
    selected_columns: &[String],
) -> Vec<String> {
    use std::collections::BTreeSet;

    let known: HashSet<&str> = selected_columns.iter().map(String::as_str).collect();
    let mut omitted: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if !known.contains(key.as_str()) {
                    omitted.insert(key.clone());
                }
            }
        }
    }
    if omitted.is_empty() {
        return Vec::new();
    }
    let names = omitted.into_iter().collect::<Vec<_>>().join(", ");
    let count = names.matches(", ").count() + 1;
    vec![format!(
        "sample omitted {count} column(s) present in the table but missing from the catalog ({names}); catalog may be stale or incomplete"
    )]
}

pub async fn execute_query(
    target_db: &dyn TargetDatabase,
    input: ExecuteQueryInput,
) -> Result<ExecuteQueryOutput, CatalogToolError> {
    if let Some(purpose) = input
        .purpose
        .as_deref()
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
    {
        tracing::info!(purpose, "execute_query purpose");
    }
    let output = tier1_discovery::execute_query(
        Some(target_db),
        tier1_discovery::ExecuteQueryInput {
            sql: input.sql,
            params: if input.params.is_empty() {
                None
            } else {
                Some(input.params)
            },
            limit: input.limit,
        },
    )
    .await?;

    Ok(ExecuteQueryOutput {
        columns: output.columns,
        rows: output.rows,
        row_count: output.row_count,
        truncated: output.truncated,
        warnings: Vec::new(),
    })
}

fn search_kinds(kinds: &[CatalogEntityKind]) -> Result<Vec<SemanticEntityKind>, CatalogToolError> {
    let public_kinds: Vec<CatalogEntityKind> = if kinds.is_empty() {
        CatalogEntityKind::PUBLIC_SEARCHABLE.to_vec()
    } else {
        kinds.to_vec()
    };
    if public_kinds.iter().any(|kind| !kind.is_public_searchable()) {
        return Err(CatalogToolError::Validation(
            "special catalog kind is reserved and cannot be searched".to_string(),
        ));
    }
    Ok(public_kinds
        .into_iter()
        .map(SemanticEntityKind::from)
        .collect())
}

fn matches_public_kinds(entry: &CatalogEntry, kinds: &[CatalogEntityKind]) -> bool {
    kinds.is_empty() || kinds.contains(&CatalogEntityKind::from(entry.kind))
}

fn assembler_for(tool_name: &str, requested_count: usize) -> CatalogEntityAssembler {
    let registry = OutputPolicyRegistry::default();
    CatalogEntityAssembler::new(registry.policy_for_tool(tool_name, requested_count).clone())
}

fn map_facets(facets: agent_fw_catalog::CatalogSearchFacets) -> SearchCatalogFacets {
    SearchCatalogFacets {
        kinds: map_facet_values(facets.kinds),
        schemas: map_facet_values(facets.schemas),
        tables: map_facet_values(facets.tables),
        tags: map_facet_values(facets.tags),
    }
}

fn map_facet_values(values: Vec<agent_fw_catalog::CatalogFacetValue>) -> Vec<FacetValue> {
    values
        .into_iter()
        .map(|value| FacetValue {
            value: value.value,
            count: value.count,
        })
        .collect()
}

fn suggested_filters(facets: &SearchCatalogFacets) -> Vec<CatalogFilters> {
    let mut suggestions = Vec::new();
    if let Some(schema) = facets.schemas.first() {
        suggestions.push(CatalogFilters {
            schema: Some(schema.value.clone()),
            ..CatalogFilters::default()
        });
    }
    if let Some(tag) = facets.tags.first() {
        suggestions.push(CatalogFilters {
            tags: vec![tag.value.clone()],
            ..CatalogFilters::default()
        });
    }
    suggestions
}

fn entry_matches_filters(entry: &CatalogEntry, filters: &CatalogFilters) -> bool {
    if !tags_filter_matches(&filters.tags, &entry.tags) {
        return false;
    }
    match entry.kind {
        CatalogKind::Table => {
            let Ok(metadata) = decode_metadata::<TableMetadata>(entry) else {
                return filters.schema.is_none()
                    && filters.database_id.is_none()
                    && filters.preferred_query_surface.is_none()
                    && filters.table.is_none();
            };
            string_filter_matches(&filters.database_id, &metadata.database_id)
                && string_filter_matches(&filters.schema, &metadata.schema_name)
                && filters
                    .preferred_query_surface
                    .is_none_or(|expected| metadata.preferred_query_surface == expected)
                && ref_filter_matches(&filters.table, entry)
        }
        CatalogKind::Column => column_matches_filters(entry, filters),
        CatalogKind::Knowledge => {
            let Ok(metadata) = decode_metadata::<KnowledgeMetadata>(entry) else {
                return filters.knowledge_type.is_none()
                    && filters.table.is_none()
                    && filters.column.is_none()
                    && filters.database_id.is_none()
                    && filters.schema.is_none();
            };
            filters.database_id.is_none()
                && filters.schema.is_none()
                && optional_string_filter_matches(
                    &filters.knowledge_type,
                    metadata.knowledge_type.as_deref(),
                )
                && scoped_filter_matches(&filters.table, &metadata.scope_tables)
                && scoped_filter_matches(&filters.column, &metadata.scope_columns)
        }
        CatalogKind::Relationship => {
            let Ok(metadata) = decode_metadata::<RelationshipMetadata>(entry) else {
                return filters.relation_kind.is_none()
                    && filters.source_table.is_none()
                    && filters.target_table.is_none()
                    && filters.database_id.is_none()
                    && filters.schema.is_none();
            };
            string_filter_matches(&filters.database_id, &metadata.database_id)
                && string_filter_matches(&filters.schema, &metadata.source_schema)
                && optional_string_filter_matches(
                    &filters.relation_kind,
                    Some(metadata.relationship_kind.as_str()),
                )
                && scoped_filter_matches(&filters.source_table, &[metadata.source_table_id])
                && scoped_filter_matches(&filters.target_table, &[metadata.target_table_id])
        }
        CatalogKind::Enum => {
            let Ok(metadata) = decode_metadata::<EnumValueMetadata>(entry) else {
                return filters.database_id.is_none()
                    && filters.schema.is_none()
                    && filters.table.is_none()
                    && filters.column.is_none()
                    && filters.low_cardinality_enum.is_none();
            };
            string_filter_matches(&filters.database_id, &metadata.database_id)
                && string_filter_matches(&filters.schema, &metadata.schema_name)
                && filters.low_cardinality_enum.is_none_or(|expected| expected)
                && ref_filter_matches(&filters.column, entry)
                && enum_table_filter_matches(&filters.table, &metadata)
        }
        CatalogKind::Metric => {
            let Ok(metadata) = decode_metadata::<MetricMetadata>(entry) else {
                return filters.database_id.is_none()
                    && filters.schema.is_none()
                    && filters.table.is_none()
                    && filters.column.is_none();
            };
            filters.database_id.is_none()
                && filters.schema.is_none()
                && scoped_filter_matches(&filters.table, &metadata.source_tables)
                && scoped_filter_matches(&filters.column, &metadata.source_columns)
        }
        CatalogKind::Document => {
            filters.database_id.is_none()
                && filters.schema.is_none()
                && filters.table.is_none()
                && filters.column.is_none()
        }
        CatalogKind::DataQualityFinding => {
            let Ok(metadata) = decode_metadata::<DataQualityFindingMetadata>(entry) else {
                return filters.database_id.is_none()
                    && filters.schema.is_none()
                    && filters.table.is_none()
                    && filters.column.is_none();
            };
            string_filter_matches(&filters.database_id, &metadata.database_id)
                && string_filter_matches(&filters.schema, &metadata.schema_name)
                && data_quality_table_filter_matches(&filters.table, &metadata)
                && optional_scoped_filter_matches(&filters.column, metadata.column_name.as_deref())
        }
        CatalogKind::Special => false,
    }
}

fn column_matches_filters(entry: &CatalogEntry, filters: &CatalogFilters) -> bool {
    let Ok(metadata) = decode_metadata::<ColumnMetadata>(entry) else {
        return filters.schema.is_none()
            && filters.database_id.is_none()
            && filters.table.is_none()
            && filters.column.is_none()
            && filters.data_type.is_none()
            && filters.semantic_type.is_none()
            && filters.low_cardinality_enum.is_none();
    };
    string_filter_matches(&filters.database_id, &metadata.database_id)
        && string_filter_matches(&filters.schema, &metadata.schema_name)
        && string_filter_matches(&filters.data_type, &metadata.data_type)
        && optional_string_filter_matches(&filters.semantic_type, metadata.semantic_type.as_deref())
        && filters
            .low_cardinality_enum
            .is_none_or(|expected| metadata.low_cardinality_enum == expected)
        && ref_filter_matches(&filters.column, entry)
        && table_filter_matches(&filters.table, &metadata)
}

fn string_filter_matches(filter: &Option<String>, actual: &str) -> bool {
    filter
        .as_ref()
        .is_none_or(|expected| expected.eq_ignore_ascii_case(actual))
}

fn optional_string_filter_matches(filter: &Option<String>, actual: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|expected| actual.is_some_and(|actual| expected.eq_ignore_ascii_case(actual)))
}

fn ref_filter_matches(filter: &Option<super::CatalogFilterRef>, entry: &CatalogEntry) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    match filter {
        super::CatalogFilterRef::Ref(reference) => {
            reference.id.as_deref() == Some(entry.id.as_str())
                || reference.qualified_name.as_deref() == entry.qualified_name.as_deref()
                || reference.name.as_deref() == Some(entry.name.as_str())
        }
        super::CatalogFilterRef::String(value) => {
            value == &entry.id
                || entry.qualified_name.as_deref() == Some(value.as_str())
                || value == &entry.name
        }
    }
}

fn table_filter_matches(
    filter: &Option<super::CatalogFilterRef>,
    metadata: &ColumnMetadata,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let qualified = format!("{}.{}", metadata.schema_name, metadata.table_name);
    match filter {
        super::CatalogFilterRef::Ref(reference) => {
            reference.qualified_name.as_deref() == Some(qualified.as_str())
                || reference.name.as_deref() == Some(metadata.table_name.as_str())
        }
        super::CatalogFilterRef::String(value) => {
            value == &qualified || value == &metadata.table_name
        }
    }
}

fn enum_table_filter_matches(
    filter: &Option<super::CatalogFilterRef>,
    metadata: &EnumValueMetadata,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let qualified = format!("{}.{}", metadata.schema_name, metadata.table_name);
    match filter {
        super::CatalogFilterRef::Ref(reference) => {
            reference.qualified_name.as_deref() == Some(qualified.as_str())
                || reference.name.as_deref() == Some(metadata.table_name.as_str())
        }
        super::CatalogFilterRef::String(value) => {
            value == &qualified || value == &metadata.table_name
        }
    }
}

fn data_quality_table_filter_matches(
    filter: &Option<super::CatalogFilterRef>,
    metadata: &DataQualityFindingMetadata,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let qualified = format!("{}.{}", metadata.schema_name, metadata.table_name);
    match filter {
        super::CatalogFilterRef::Ref(reference) => {
            reference.qualified_name.as_deref() == Some(qualified.as_str())
                || reference.name.as_deref() == Some(metadata.table_name.as_str())
        }
        super::CatalogFilterRef::String(value) => {
            value == &qualified || value == &metadata.table_name
        }
    }
}

fn optional_scoped_filter_matches(
    filter: &Option<super::CatalogFilterRef>,
    actual: Option<&str>,
) -> bool {
    let Some(actual) = actual else {
        return filter.is_none();
    };
    scoped_filter_matches(filter, &[actual.to_string()])
}

fn scoped_filter_matches(filter: &Option<super::CatalogFilterRef>, scopes: &[String]) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    match filter {
        super::CatalogFilterRef::Ref(reference) => {
            reference
                .qualified_name
                .as_ref()
                .is_some_and(|value| scopes.contains(value))
                || reference
                    .id
                    .as_ref()
                    .is_some_and(|value| scopes.contains(value))
                || reference
                    .name
                    .as_ref()
                    .is_some_and(|value| scopes.contains(value))
        }
        super::CatalogFilterRef::String(value) => scopes.contains(value),
    }
}

fn tags_filter_matches(expected: &[String], actual: &[String]) -> bool {
    expected.iter().all(|tag| {
        actual
            .iter()
            .any(|actual| actual.eq_ignore_ascii_case(tag.trim()))
    })
}

async fn catalog_columns(
    catalog: &dyn DataCatalog,
    table: &CatalogEntry,
) -> Result<Vec<CatalogEntry>, CatalogToolError> {
    let mut columns = Vec::new();
    let mut seen = HashSet::new();
    for key in [
        Some(table.id.as_str()),
        table.qualified_name.as_deref(),
        Some(table.name.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        for column in catalog.get_columns(key).await? {
            if seen.insert(column.id.clone()) {
                columns.push(column);
            }
        }
        if !columns.is_empty() {
            break;
        }
    }
    Ok(columns)
}

fn target_table_name(table: &CatalogEntry) -> Result<String, CatalogToolError> {
    if let Some(qualified_name) = table
        .qualified_name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(qualified_name.clone());
    }
    let metadata = decode_metadata::<TableMetadata>(table)?;
    Ok(format!("{}.{}", metadata.schema_name, metadata.table_name))
}

fn selected_column_names(
    columns: &[CatalogEntry],
    requested: &[String],
) -> Result<Vec<String>, CatalogToolError> {
    if requested.is_empty() {
        return Ok(columns.iter().map(|column| column.name.clone()).collect());
    }
    let mut selected = Vec::new();
    for requested_column in requested {
        if !crate::is_valid_table_name(requested_column) {
            return Err(CatalogToolError::Validation(format!(
                "invalid column identifier for sample_table_data: {requested_column}"
            )));
        }
        let Some(column) = columns.iter().find(|column| {
            column.name == *requested_column
                || column.qualified_name.as_deref() == Some(requested_column.as_str())
        }) else {
            return Err(CatalogToolError::Validation(format!(
                "unknown column for sample_table_data: {requested_column}"
            )));
        };
        selected.push(column.name.clone());
    }
    Ok(selected)
}

fn project_rows(
    rows: Vec<serde_json::Value>,
    selected_columns: &[String],
) -> Vec<serde_json::Value> {
    if selected_columns.is_empty() {
        return rows;
    }
    rows.into_iter()
        .map(|row| {
            let Some(object) = row.as_object() else {
                return row;
            };
            let mut projected = serde_json::Map::new();
            for column in selected_columns {
                if let Some(value) = object.get(column) {
                    projected.insert(column.clone(), value.clone());
                }
            }
            serde_json::Value::Object(projected)
        })
        .collect()
}

fn infer_columns(rows: &[serde_json::Value]) -> Vec<String> {
    rows.first()
        .and_then(|row| row.as_object())
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

async fn join_path(
    catalog: &dyn DataCatalog,
    assembler: &CatalogEntityAssembler,
    from: &CatalogEntry,
    to: &CatalogEntry,
) -> Result<Option<Vec<CatalogRelationPathStep>>, CatalogToolError> {
    if from.kind != CatalogKind::Table || to.kind != CatalogKind::Table {
        return Ok(None);
    }
    let Some(path) = catalog.find_join_path(&from.id, &to.id).await? else {
        return Ok(None);
    };
    Ok(Some(join_steps(path, assembler)))
}

fn join_steps(path: JoinPath, assembler: &CatalogEntityAssembler) -> Vec<CatalogRelationPathStep> {
    // The interpreter's `steps[0]` is always the `from` source table, and
    // `hops[i]` describes the edge `steps[i] -> steps[i + 1]`. The surface
    // contract reports steps beginning at the FIRST hop target, so we skip the
    // source step and zip each remaining step with its hop metadata.
    let JoinPath { steps, hops, .. } = path;
    let mut hops = hops.into_iter();
    steps
        .into_iter()
        .skip(1)
        .filter_map(|entry| {
            // Pull the hop describing this step regardless of whether the entity
            // assembles, so steps and hops stay aligned.
            let hop = hops.next();
            assembler
                .assemble(entry)
                .entity
                .map(|entity| CatalogRelationPathStep {
                    entity,
                    via_relation: hop.map(via_relation_from_hop),
                })
        })
        .collect()
}

fn via_relation_from_hop(hop: JoinHop) -> serde_json::Value {
    let mut value = serde_json::json!({
        "relation_kind": hop.relation_kind,
        "description": hop.description,
    });
    // Only emit the `join` object when concrete join columns are known (a
    // relationship vertex backed the hop); non-FK table links omit it.
    if hop.from_column.is_some() || hop.to_column.is_some() {
        value["join"] = serde_json::json!({
            "from_column": hop.from_column,
            "to_column": hop.to_column,
            "join_type": hop.join_type,
        });
    }
    value
}

#[allow(clippy::too_many_arguments)]
async fn semantic_path(
    catalog: &dyn DataCatalog,
    assembler: &CatalogEntityAssembler,
    from: &CatalogEntry,
    to: &CatalogEntry,
    max_depth: usize,
    relationships: &Arc<Vec<CatalogEntry>>,
    truncation_seen: &mut bool,
) -> Result<Option<Vec<CatalogRelationPathStep>>, CatalogToolError> {
    // Reuse the request-wide relationship vertex set across every dequeued node
    // instead of re-scanning the catalog per node (M7). The dedupe/edge
    // semantics are unchanged — only the data source for relationship edges is
    // hoisted out of the per-node scan.
    let graph = CatalogGraphService::new_with_relationships(
        catalog,
        CatalogRefResolver::new(catalog),
        assembler.clone(),
        relationships.clone(),
    );

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    queue.push_back((from.id.clone(), Vec::<CatalogGraphEdge>::new()));
    visited.insert(from.id.clone());

    while let Some((current_id, path)) = queue.pop_front() {
        if path.len() >= max_depth {
            continue;
        }
        let Some(current) = catalog.get_by_id(&current_id).await? else {
            continue;
        };
        let relations = graph
            .get_relations(GetCatalogRelationsInput {
                refs: vec![CatalogRef::id(current.id.clone())],
                direction: Some(RelationDirection::Both),
                relation_kinds: Vec::new(),
                target_kinds: Vec::new(),
                limit_per_ref: Some(100),
            })
            .await?;
        let Some(first) = relations.results.into_iter().next() else {
            continue;
        };
        // M5: if this hub had more adjacent edges than the per-node limit, a
        // real edge toward `to` may have been truncated away. Remember it so a
        // `found == false` result can be flagged as possibly-truncated rather
        // than definitively no-path.
        if first.pagination.has_more {
            *truncation_seen = true;
        }
        for edge in first.relations {
            if !visited.insert(edge.target.id.clone()) {
                continue;
            }
            let mut next_path = path.clone();
            next_path.push(edge.clone());
            if edge.target.id == to.id {
                return Ok(Some(
                    next_path
                        .into_iter()
                        .map(|edge| CatalogRelationPathStep {
                            entity: edge.target,
                            via_relation: Some(serde_json::json!({
                                "relation_kind": edge.relation_kind,
                                "description": edge.description,
                            })),
                        })
                        .collect(),
                ));
            }
            queue.push_back((edge.target.id.clone(), next_path));
        }
    }

    Ok(None)
}
