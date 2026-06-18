//! Harness-native catalog/search/tool command surface for Studio Connect.
//!
//! These commands keep Connect backed by the same `flowai-runtime::data`
//! boundary as profiling and knowledge ingestion. They intentionally expose
//! supported read/query primitives only; browser file import and mutable metric
//! APIs are deferred to a later import/ETL milestone.

use std::sync::Arc;
use std::time::Instant;

use agent_fw_agent::{ComposedDispatcher, ToolDispatcher};
use agent_fw_algebra::KVStore;
use agent_fw_catalog::{CatalogEntry, CatalogKind, CatalogToolEnvironmentExt, DataCatalog};
use agent_fw_catalog_tools::{surface_handlers, tool_metadata};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{TenantId, WorkspaceContext};
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_tool::ToolEnvironment;
use serde_json::{json, Map, Value};

use crate::storage::{
    build_catalog_for_scope, build_catalog_search_backend, build_kv_store_from_environment,
    build_target_database_from_environment, catalog_scope_from_data_environment,
};

use super::errors::DataCommandError;
use super::types::{
    CatalogSearchItem, CatalogSearchResult, CatalogToolExecutionResult, CatalogToolList,
    CatalogToolSummary, ExecuteCatalogToolCommand, ListMetricsCommand, MetricListResult,
    MetricSummary, SearchCatalogCommand,
};

const CATALOG_SCAN_LIMIT: usize = 10_000;

const SUPPORTED_TOOL_IDS: &[&str] = &[
    "search_catalog",
    "get_catalog_entities",
    "list_schema_fields",
    "get_catalog_relations",
    "get_relation_paths_between",
    "sample_table_data",
    "execute_query",
];

/// Search the configured catalog.
pub async fn search_catalog(
    command: SearchCatalogCommand,
) -> Result<CatalogSearchResult, DataCommandError> {
    let query = command.query.trim();
    if query.is_empty() {
        return Err(DataCommandError::Invalid(
            "data search requires a non-empty query".to_string(),
        ));
    }

    let started = Instant::now();
    let limit = command.limit.unwrap_or(25).clamp(1, 100);
    let catalog_config = command.data_environment.catalog.clone().ok_or_else(|| {
        DataCommandError::Invalid("data search requires data_environment.catalog".to_string())
    })?;
    let scope = catalog_scope_from_data_environment(
        &command.data_environment,
        TenantId::new_unchecked(super::DEFAULT_DATA_TENANT_ID),
    );
    let catalog = build_catalog_for_scope(catalog_config, scope).await?;

    let items =
        search_catalog_reader(catalog.as_ref(), query, command.mode.as_deref(), limit).await?;
    Ok(CatalogSearchResult {
        total_count: items.len(),
        items,
        query_time_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        mode: command.mode,
    })
}

/// List supported catalog tools.
pub fn list_catalog_tools() -> CatalogToolList {
    let definitions = surface_handlers()
        .into_iter()
        .map(|handler| {
            let definition = handler.definition();
            (definition.name.clone(), definition)
        })
        .collect::<std::collections::HashMap<_, _>>();

    let tools = tool_metadata::SURFACE_TOOLS
        .into_iter()
        .filter_map(|metadata| {
            definitions
                .get(metadata.name)
                .map(|definition| CatalogToolSummary {
                    tool_id: metadata.name.to_string(),
                    id: metadata.name.to_string(),
                    name: metadata.name.replace('_', " "),
                    description: definition.description.clone(),
                    parameters: definition.input_schema.clone(),
                    input_schema: definition.input_schema.clone(),
                })
        })
        .collect();
    CatalogToolList { tools }
}

/// Execute one supported catalog tool against the configured catalog/database.
pub async fn execute_catalog_tool(
    command: ExecuteCatalogToolCommand,
) -> Result<CatalogToolExecutionResult, DataCommandError> {
    let (tool_id, input) = normalize_tool_request(&command.tool_id, command.input.clone());
    if !SUPPORTED_TOOL_IDS.contains(&tool_id.as_str()) {
        return Err(DataCommandError::Invalid(format!(
            "unsupported catalog tool '{}'",
            command.tool_id
        )));
    }

    let env = build_catalog_tool_environment(&command, &tool_id).await?;
    let handler = surface_handlers()
        .into_iter()
        .find(|handler| handler.definition().name == tool_id)
        .ok_or_else(|| DataCommandError::Execution(format!("missing handler for '{tool_id}'")))?;
    let dispatcher = ComposedDispatcher::new(env)
        .with_handler(handler)
        .try_build()
        .map_err(|err| {
            DataCommandError::Execution(format!("failed to build catalog tool dispatcher: {err}"))
        })?;
    let result = dispatcher
        .dispatch(&tool_id, "studio-catalog-tool-call", input)
        .await;
    let count = extract_count(&result.content);
    let error = if result.is_error {
        extract_error_message(&result.content)
    } else {
        None
    };
    Ok(CatalogToolExecutionResult {
        tool_id,
        success: !result.is_error,
        data: result.content,
        count,
        error,
    })
}

/// List metrics from the configured catalog.
pub async fn list_metrics(
    command: ListMetricsCommand,
) -> Result<MetricListResult, DataCommandError> {
    let limit = command.limit.unwrap_or(50).clamp(1, 250);
    let catalog_config = command.data_environment.catalog.clone().ok_or_else(|| {
        DataCommandError::Invalid("metric listing requires data_environment.catalog".to_string())
    })?;
    let scope = catalog_scope_from_data_environment(
        &command.data_environment,
        TenantId::new_unchecked(super::DEFAULT_DATA_TENANT_ID),
    );
    let catalog = build_catalog_for_scope(catalog_config, scope).await?;

    let metrics = if let Some(query) = command
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        search_catalog_reader(catalog.as_ref(), query, Some("metrics"), limit)
            .await?
            .into_iter()
            .map(metric_from_item)
            .collect::<Vec<_>>()
    } else {
        catalog
            .list_by_type(CatalogKind::Metric, limit)
            .await?
            .into_iter()
            .map(metric_from_entry)
            .collect::<Vec<_>>()
    };

    Ok(MetricListResult {
        total_count: metrics.len(),
        metrics,
    })
}

async fn build_catalog_tool_environment(
    command: &ExecuteCatalogToolCommand,
    tool_id: &str,
) -> Result<ToolEnvironment, DataCommandError> {
    let catalog_config = command.data_environment.catalog.clone().ok_or_else(|| {
        DataCommandError::Invalid("tool execution requires data_environment.catalog".to_string())
    })?;
    let scope = catalog_scope_from_data_environment(
        &command.data_environment,
        TenantId::new_unchecked(super::DEFAULT_DATA_TENANT_ID),
    );
    let catalog = build_catalog_for_scope(catalog_config, scope.clone()).await?;
    let kv: Arc<dyn KVStore> = if command.data_environment.kv.is_some() {
        build_kv_store_from_environment(&command.data_environment).await?
    } else {
        Arc::new(DashMapKVStore::new())
    };
    let workspace_context =
        WorkspaceContext::from_ids(scope.tenant_id.clone(), Some(scope.workspace_id.as_str()));
    let mut env = ToolEnvironment::builder()
        .kv_arc(kv)
        .tenant_context(TenantContext::new(scope.tenant_id.clone()))
        .build()
        .with_catalog(catalog)
        .with_ext::<WorkspaceContext>(Arc::new(workspace_context));

    if tool_needs_catalog_search_backend(tool_id) {
        env = env
            .with_catalog_search_backend(build_catalog_search_backend(&command.data_environment)?);
    }
    if tool_needs_target_database(tool_id) {
        env = env.with_target_db(
            build_target_database_from_environment(&command.data_environment).await?,
        );
    }

    Ok(env)
}

fn tool_needs_catalog_search_backend(tool_id: &str) -> bool {
    tool_id == "search_catalog"
}

fn tool_needs_target_database(tool_id: &str) -> bool {
    matches!(tool_id, "sample_table_data" | "execute_query")
}

fn normalize_tool_id(tool_id: &str) -> String {
    match tool_id {
        "fuzzyTableSearch"
        | "fuzzy_table_search"
        | "fuzzyColumnSearch"
        | "fuzzy_column_search"
        | "fuzzyEnumSearch"
        | "fuzzy_enum_search"
        | "resolveTerm"
        | "resolve_term"
        | "searchMetrics"
        | "search_metrics" => "search_catalog",
        "getTableInfo"
        | "get_table_info"
        | "getColumnInfo"
        | "get_column_info"
        | "getEntriesByIds"
        | "get_entries_by_ids"
        | "getFullTableContext"
        | "get_full_table_context" => "get_catalog_entities",
        "getTableColumns" | "get_table_columns" => "list_schema_fields",
        "getRelatedTables" | "get_related_tables" => "get_catalog_relations",
        "findJoinPath" | "find_join_path" => "get_relation_paths_between",
        "sampleTableData" => "sample_table_data",
        "executeQuery" => "execute_query",
        other => other,
    }
    .to_string()
}

fn normalize_tool_request(tool_id: &str, input: Value) -> (String, Value) {
    let normalized_tool_id = normalize_tool_id(tool_id);
    let input = normalize_tool_input(tool_id, &normalized_tool_id, input);
    (normalized_tool_id, input)
}

fn normalize_tool_input(original_tool_id: &str, tool_id: &str, input: Value) -> Value {
    let Value::Object(mut map) = input else {
        return input;
    };

    alias(&mut map, "tableName", "table");
    alias(&mut map, "table_id", "table");
    alias(&mut map, "queryText", "query");
    alias(&mut map, "fromTable", "from_table");
    alias(&mut map, "toTable", "to_table");
    alias(&mut map, "limitPerRef", "limit_per_ref");
    alias(&mut map, "sampleLimit", "limit");

    match tool_id {
        "search_catalog" => normalize_search_input(original_tool_id, &mut map),
        "get_catalog_entities" => normalize_entities_input(&mut map),
        "list_schema_fields" => normalize_schema_fields_input(&mut map),
        "get_catalog_relations" => normalize_relations_input(original_tool_id, &mut map),
        "get_relation_paths_between" => normalize_paths_input(&mut map),
        "sample_table_data" => normalize_sample_input(&mut map),
        "execute_query" => {
            alias(&mut map, "query", "sql");
        }
        _ => {}
    }
    Value::Object(map)
}

fn normalize_search_input(original_tool_id: &str, map: &mut Map<String, Value>) {
    alias(map, "term", "query");
    if let Some(kind) = legacy_search_kind(original_tool_id) {
        map.entry("kinds".to_string())
            .or_insert_with(|| json!([kind]));
    }
}

fn normalize_entities_input(map: &mut Map<String, Value>) {
    if map.contains_key("refs") {
        return;
    }
    if let Some(Value::Array(ids)) = map.get("ids") {
        let refs = ids
            .iter()
            .filter_map(Value::as_str)
            .map(|id| json!({ "id": id }))
            .collect::<Vec<_>>();
        if !refs.is_empty() {
            map.insert("refs".to_string(), Value::Array(refs));
            return;
        }
    }
    if let Some(Value::String(id)) = map.get("id") {
        map.insert("refs".to_string(), json!([{ "id": id }]));
        return;
    }
    if let Some(table) = map.get("table").and_then(catalog_table_ref) {
        map.insert("refs".to_string(), json!([table]));
    }
}

fn normalize_schema_fields_input(map: &mut Map<String, Value>) {
    if map.contains_key("tables") {
        return;
    }
    if let Some(table) = map.get("table").and_then(catalog_table_ref) {
        map.insert("tables".to_string(), json!([table]));
    }
}

fn normalize_relations_input(original_tool_id: &str, map: &mut Map<String, Value>) {
    if !map.contains_key("refs") {
        if let Some(table) = map.get("table").and_then(catalog_table_ref) {
            map.insert("refs".to_string(), json!([table]));
        }
    }
    if matches!(original_tool_id, "getRelatedTables" | "get_related_tables") {
        map.entry("target_kinds".to_string())
            .or_insert_with(|| json!(["table"]));
    }
}

fn normalize_paths_input(map: &mut Map<String, Value>) {
    if !map.contains_key("from") {
        if let Some(from) = map.get("from_table").and_then(catalog_table_ref) {
            map.insert("from".to_string(), from);
        }
    }
    if !map.contains_key("to") {
        if let Some(to) = map.get("to_table").and_then(catalog_table_ref) {
            map.insert("to".to_string(), json!([to]));
        }
    }
}

fn normalize_sample_input(map: &mut Map<String, Value>) {
    if let Some(table) = map.get("table").and_then(catalog_table_ref) {
        map.insert("table".to_string(), table);
    }
}

fn legacy_search_kind(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "fuzzyTableSearch" | "fuzzy_table_search" => Some("table"),
        "fuzzyColumnSearch" | "fuzzy_column_search" => Some("column"),
        "fuzzyEnumSearch" | "fuzzy_enum_search" => Some("enum_value"),
        "searchMetrics" | "search_metrics" => Some("metric"),
        _ => None,
    }
}

fn catalog_table_ref(value: &Value) -> Option<Value> {
    match value {
        Value::String(table) if !table.trim().is_empty() => {
            let table = table.trim();
            if table.contains('.') {
                Some(json!({ "qualified_name": table, "kind": "table" }))
            } else {
                Some(json!({ "name": table, "kind": "table" }))
            }
        }
        Value::Object(_) => Some(value.clone()),
        _ => None,
    }
}

fn alias(map: &mut Map<String, Value>, from: &str, to: &str) {
    if !map.contains_key(to) {
        if let Some(value) = map.get(from).cloned() {
            map.insert(to.to_string(), value);
        }
    }
}

async fn search_catalog_reader(
    catalog: &dyn DataCatalog,
    query: &str,
    mode: Option<&str>,
    limit: usize,
) -> Result<Vec<CatalogSearchItem>, DataCommandError> {
    let mut matches = Vec::new();
    for kind in search_kinds_for_mode(mode) {
        for entry in catalog.list_by_type(kind, CATALOG_SCAN_LIMIT).await? {
            if let Some((score, match_field)) = score_catalog_entry(&entry, query) {
                matches.push(ScoredCatalogEntry {
                    entry,
                    score,
                    match_field,
                });
            }
        }
    }
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.entry.name.cmp(&right.entry.name))
    });
    matches.truncate(limit);
    Ok(matches.into_iter().map(scored_entry_to_item).collect())
}

fn search_kinds_for_mode(mode: Option<&str>) -> Vec<CatalogKind> {
    match mode.unwrap_or_default().to_ascii_lowercase().as_str() {
        "table" | "tables" => vec![CatalogKind::Table],
        "column" | "columns" => vec![CatalogKind::Column],
        "enum" | "enums" | "enum_value" | "enum_values" => vec![CatalogKind::Enum],
        "metric" | "metrics" => vec![CatalogKind::Metric],
        "knowledge" => vec![CatalogKind::Knowledge],
        "document" | "documents" => vec![CatalogKind::Document],
        "relationship" | "relationships" => vec![CatalogKind::Relationship],
        "data_quality" | "data_quality_finding" | "data_quality_findings" => {
            vec![CatalogKind::DataQualityFinding]
        }
        _ => vec![
            CatalogKind::Table,
            CatalogKind::Column,
            CatalogKind::Relationship,
            CatalogKind::Enum,
            CatalogKind::Metric,
            CatalogKind::Knowledge,
            CatalogKind::Document,
            CatalogKind::DataQualityFinding,
        ],
    }
}

struct ScoredCatalogEntry {
    entry: CatalogEntry,
    score: f64,
    match_field: Option<String>,
}

fn score_catalog_entry(entry: &CatalogEntry, query: &str) -> Option<(f64, Option<String>)> {
    let query = query.trim().to_ascii_lowercase();
    if query == "*" {
        return Some((0.1, None));
    }

    let mut best: Option<(f64, Option<String>)> = None;
    consider_match(&mut best, &entry.name, &query, "name", 1.0);
    if let Some(qualified_name) = &entry.qualified_name {
        consider_match(&mut best, qualified_name, &query, "qualified_name", 0.95);
    }
    consider_match(&mut best, &entry.content, &query, "content", 0.7);
    for tag in &entry.tags {
        consider_match(&mut best, tag, &query, "tags", 0.8);
    }
    consider_match(
        &mut best,
        &entry.metadata.to_string(),
        &query,
        "metadata",
        0.5,
    );
    best
}

fn consider_match(
    best: &mut Option<(f64, Option<String>)>,
    haystack: &str,
    query: &str,
    field: &str,
    base_score: f64,
) {
    let haystack = haystack.to_ascii_lowercase();
    let score = if haystack == query {
        Some(base_score)
    } else if haystack.contains(query) {
        Some(base_score * 0.8)
    } else {
        None
    };
    if let Some(score) = score {
        if best
            .as_ref()
            .map(|(current_score, _)| score > *current_score)
            .unwrap_or(true)
        {
            *best = Some((score, Some(field.to_string())));
        }
    }
}

fn scored_entry_to_item(scored: ScoredCatalogEntry) -> CatalogSearchItem {
    CatalogSearchItem {
        id: scored.entry.id,
        name: scored.entry.name,
        item_type: scored.entry.kind.as_str().to_string(),
        description: scored.entry.content,
        qualified_name: scored.entry.qualified_name,
        tags: scored.entry.tags,
        score: scored.score,
        match_field: scored.match_field,
        metadata: scored.entry.metadata,
    }
}

fn metric_from_item(item: CatalogSearchItem) -> MetricSummary {
    let score = item.score;
    let mut metric = metric_from_parts(
        item.id,
        item.name,
        item.description,
        item.tags,
        item.metadata,
    );
    metric.score = Some(score);
    metric
}

fn metric_from_entry(entry: CatalogEntry) -> MetricSummary {
    metric_from_parts(
        entry.id,
        entry.name,
        entry.content,
        entry.tags,
        entry.metadata,
    )
}

fn metric_from_parts(
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    metadata: Value,
) -> MetricSummary {
    let metric_type = metadata
        .get("metricType")
        .or_else(|| metadata.get("metric_type"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    MetricSummary {
        id,
        name,
        description,
        metric_type,
        tags,
        metadata,
        score: None,
    }
}

fn extract_count(value: &Value) -> Option<usize> {
    if let Some(count) = value
        .get("count")
        .or_else(|| value.get("rowCount"))
        .or_else(|| value.get("row_count"))
        .or_else(|| value.get("totalMatches"))
        .or_else(|| value.get("total_matches"))
        .or_else(|| value.get("totalCount"))
        .or_else(|| value.get("total_count"))
        .and_then(Value::as_u64)
    {
        return Some(count as usize);
    }
    if let Some(returned) = value
        .get("pagination")
        .and_then(|pagination| pagination.get("returned"))
        .and_then(Value::as_u64)
    {
        return Some(returned as usize);
    }
    ["results", "rows", "entities", "relations", "paths"]
        .into_iter()
        .find_map(|field| value.get(field).and_then(Value::as_array).map(Vec::len))
}

fn extract_error_message(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_catalog_tools_exposes_supported_tool_ids() {
        let tools = list_catalog_tools();
        let ids = tools
            .tools
            .iter()
            .map(|tool| tool.tool_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "search_catalog",
                "get_catalog_entities",
                "list_schema_fields",
                "get_catalog_relations",
                "get_relation_paths_between",
                "sample_table_data",
                "execute_query",
            ]
        );
    }

    #[test]
    fn normalize_tool_id_accepts_legacy_ui_names() {
        assert_eq!(normalize_tool_id("fuzzyTableSearch"), "search_catalog");
        assert_eq!(normalize_tool_id("fuzzyColumnSearch"), "search_catalog");
        assert_eq!(normalize_tool_id("resolveTerm"), "search_catalog");
        assert_eq!(
            normalize_tool_id("getRelatedTables"),
            "get_catalog_relations"
        );
    }

    #[test]
    fn normalize_legacy_search_input_adds_current_kind_filter() {
        let (tool_id, input) = normalize_tool_request(
            "fuzzyTableSearch",
            json!({
                "query": "orders",
                "limit": 5
            }),
        );

        assert_eq!(tool_id, "search_catalog");
        assert_eq!(input["query"], "orders");
        assert_eq!(input["kinds"], json!(["table"]));
    }
}
