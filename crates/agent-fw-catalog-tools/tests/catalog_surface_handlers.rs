use std::sync::{Arc, Mutex};

use agent_fw_agent::{ComposedDispatcher, ToolDispatcher, ToolHandler};
use agent_fw_algebra::{
    DbError, DbRow, KVStore, KVStoreExt, QueryParam, ReadOnlyQuery, TargetDatabase,
};
use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogRelation, CatalogScope, CatalogSearchBackend,
    CatalogSearchCursor, CatalogSearchFacets as BackendFacets, CatalogSearchHealth,
    CatalogSearchHitRef, CatalogSearchRequest, CatalogSearchResults, CatalogToolEnvironmentExt,
    DataCatalog, DocumentItem, ExtractionStatus, JoinHop, JoinPath,
};
use agent_fw_catalog_tools::surface::handlers::{
    surface_handlers, ExecuteQueryHandler as SurfaceExecuteQueryHandler,
    GetCatalogEntitiesHandler as SurfaceGetCatalogEntitiesHandler,
    GetRelationPathsBetweenHandler as SurfaceGetRelationPathsBetweenHandler,
    SampleTableDataHandler as SurfaceSampleTableDataHandler,
    SearchCatalogHandler as SurfaceSearchCatalogHandler,
};
use agent_fw_core::{DatabaseType, TenantId, WorkspaceContext};
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use serde_json::json;

#[test]
fn surface_handlers_expose_exact_seven_contract_tools() {
    let handlers = surface_handlers();
    let names: Vec<String> = handlers
        .iter()
        .map(|handler| handler.definition().name)
        .collect();

    assert_eq!(
        names,
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

#[tokio::test]
async fn search_catalog_requires_catalog_search_backend() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let env = base_env().with_catalog(catalog);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "products"}), &env)
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("CatalogSearchBackend"));
}

#[tokio::test]
async fn search_catalog_uses_backend_hits_and_hydrates_through_catalog() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "table:products".to_string(),
        score: 0.91,
        rank: 1,
        match_signals: vec!["name".to_string()],
        matched_fields: vec!["name_text".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend.clone();
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle(
            "call-search",
            json!({"query": "products", "limit": 10}),
            &env,
        )
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    assert_eq!(result.content["results"][0]["id"], "table:products");
    assert_eq!(result.content["results"][0]["kind"], "table");
    assert_eq!(result.content["results"][0]["match"]["rank"], 1);
    assert_eq!(result.content["diagnostics"]["backend"], "runtime_internal");
    assert_eq!(backend.requests().len(), 1);
}

#[tokio::test]
async fn search_catalog_paginates_without_skip_or_duplicate() {
    // Five matching entries with a page limit of 2. The handler over-fetches a
    // window of `fetch_limit = min(2 * 3, 200) = 6` candidates, so the entire set
    // comes back in ONE backend window while the handler emits only 2 survivors
    // per page. Before the fix the handler forwarded the backend window-end
    // cursor (here `None`, since the whole set fits in one window), so page 1
    // returned [p0, p1] and reported the search exhausted — silently losing
    // p2..p4. After the fix page 1 carries forward the resume cursor of the last
    // consumed candidate (p1), so page 2 resumes precisely at p2.
    let entries: Vec<CatalogEntry> = (0..5).map(paged_table_entry).collect();
    let hits: Vec<CatalogSearchHitRef> = (0..5).map(paged_hit).collect();
    let expected_ids: Vec<String> = (0..5).map(|i| format!("table:p{i}")).collect();

    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(entries));
    let backend = Arc::new(RecordingSearchBackend::ready(hits));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend.clone();
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let mut collected: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for page in 0..4 {
        let mut input = json!({"query": "p", "limit": 2});
        if let Some(cursor) = &cursor {
            input["cursor"] = json!(cursor);
        }
        let result = SurfaceSearchCatalogHandler
            .handle("call-search", input, &env)
            .await;
        assert!(!result.is_error, "page {page} errored: {result:?}");

        for id in result.content["results"].as_array().unwrap() {
            collected.push(id["id"].as_str().unwrap().to_string());
        }

        let pagination = &result.content["pagination"];
        if pagination["has_more"].as_bool().unwrap() {
            cursor = Some(pagination["next_cursor"].as_str().unwrap().to_string());
        } else {
            assert!(pagination["next_cursor"].is_null());
            break;
        }
        assert!(page < 3, "pagination did not terminate within four pages");
    }

    assert_eq!(
        collected, expected_ids,
        "paginated ids must be the full ordered set with no skip and no duplicate"
    );
}

#[tokio::test]
async fn search_catalog_rechecks_non_table_filters_after_hydration() {
    let enum_entry = enum_value_entry(
        "enum:channel.supermarket",
        "Supermarket",
        "warehouse",
        "public",
        "channels",
        "channel_name",
    );
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![enum_entry]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "enum:channel.supermarket".to_string(),
        score: 0.9,
        rank: 1,
        match_signals: vec!["metadata".to_string()],
        matched_fields: vec!["enum_value".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend;
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle(
            "call-search",
            json!({
                "query": "Supermarket",
                "kinds": ["enum_value"],
                "filters": {
                    "database_id": "other-warehouse",
                    "schema": "archive",
                    "tags": ["not-present"]
                }
            }),
            &env,
        )
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    assert!(
        result.content["results"].as_array().unwrap().is_empty(),
        "enum_value hits from a stale index must be dropped when hydrated metadata fails filters"
    );
    assert!(result.content["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap_or_default().contains("dropped 1")));
}

#[tokio::test]
async fn search_catalog_reports_response_local_rank_and_consumed_candidates() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![paged_table_entry(0)]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![
        CatalogSearchHitRef {
            entry_id: "missing:stale".to_string(),
            score: 1.0,
            rank: 1,
            match_signals: vec!["name".to_string()],
            matched_fields: vec!["name_text".to_string()],
            raw_score: None,
            snippet: None,
            resume_cursor: None,
        },
        CatalogSearchHitRef {
            entry_id: "table:p0".to_string(),
            score: 0.9,
            rank: 2,
            match_signals: vec!["name".to_string()],
            matched_fields: vec!["name_text".to_string()],
            raw_score: None,
            snippet: None,
            resume_cursor: None,
        },
        CatalogSearchHitRef {
            entry_id: "table:p1".to_string(),
            score: 0.8,
            rank: 3,
            match_signals: vec!["name".to_string()],
            matched_fields: vec!["name_text".to_string()],
            raw_score: None,
            snippet: None,
            resume_cursor: None,
        },
    ]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend;
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "p", "limit": 1}), &env)
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    assert_eq!(result.content["results"][0]["id"], "table:p0");
    assert_eq!(
        result.content["results"][0]["match"]["rank"], 1,
        "rank should be local to returned results after stale candidates are dropped"
    );
    assert_eq!(
        result.content["diagnostics"]["candidate_count"], 2,
        "diagnostics should count consumed candidates, not the full over-fetched window"
    );
}

#[tokio::test]
async fn search_catalog_bubbles_invalid_metadata_warnings_to_top_level() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![CatalogEntry {
        id: "table:broken".to_string(),
        kind: CatalogKind::Table,
        name: "broken".to_string(),
        qualified_name: Some("public.broken".to_string()),
        content: "Broken catalog table.".to_string(),
        tags: vec!["catalog".to_string()],
        links: vec![],
        metadata: json!({"schemaName": "public"}),
    }]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "table:broken".to_string(),
        score: 0.9,
        rank: 1,
        match_signals: vec!["name".to_string()],
        matched_fields: vec!["name_text".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend;
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "broken"}), &env)
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    assert!(result.content["results"][0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning
            .as_str()
            .unwrap_or_default()
            .contains("invalid table metadata")));
    assert!(result.content["results"][0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning
            .as_str()
            .unwrap_or_default()
            .contains("invalid table metadata")));
}

#[tokio::test]
async fn search_catalog_compact_output_truncates_long_catalog_content() {
    let long_description = format!(
        "{} SECRET_SENTINEL_AFTER_COMPACT_CAP",
        "Catalog body. ".repeat(40)
    );
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![CatalogEntry {
        id: "document:long-body".to_string(),
        kind: CatalogKind::Document,
        name: "Long catalog body".to_string(),
        qualified_name: Some("docs.long_body".to_string()),
        content: long_description,
        tags: vec!["docs".to_string()],
        links: vec![],
        metadata: json!({
            "sourceDocumentId": "doc-long-body",
            "contentAvailable": true,
            "contentSource": "catalog",
            "extractionStatus": "processed",
            "extractedKnowledgeIds": []
        }),
    }]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "document:long-body".to_string(),
        score: 0.9,
        rank: 1,
        match_signals: vec!["description".to_string()],
        matched_fields: vec!["description_text".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend;
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "catalog body"}), &env)
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    let description = result.content["results"][0]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        description.len() <= 240,
        "compact search description should be capped, got {} chars",
        description.len()
    );
    assert!(
        !description.contains("SECRET_SENTINEL_AFTER_COMPACT_CAP"),
        "compact search output leaked full catalog content: {description}"
    );
    assert!(result.content["results"][0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap_or_default().contains("description")));
}

#[tokio::test]
async fn search_catalog_compact_table_output_omits_schema_blocks() {
    let mut table = table_entry();
    table.content = concat!(
        "Short: Product master data used for merchandising.\n",
        "Columns:\n",
        "- sku text primary key\n",
        "- name text\n",
        "- category text\n",
        "Row count: 125000"
    )
    .to_string();
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "table:products".to_string(),
        score: 0.95,
        rank: 1,
        match_signals: vec!["name".to_string()],
        matched_fields: vec!["name_text".to_string()],
        raw_score: Some(12.0),
        snippet: Some("products".to_string()),
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend;
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle(
            "call-search",
            json!({"kinds": ["table"], "query": "products"}),
            &env,
        )
        .await;

    assert!(!result.is_error, "search_catalog errored: {result:?}");
    let entity = &result.content["results"][0];
    let description = entity["description"].as_str().unwrap_or_default();
    assert!(description.contains("Product master data"));
    assert!(
        !description.contains("Columns:"),
        "compact table search output leaked schema block: {description}"
    );
    assert!(
        !description.contains("sku text"),
        "compact table search output leaked column list: {description}"
    );
    assert!(
        !description.contains("Row count"),
        "compact table search output leaked row-count prose: {description}"
    );
    assert_eq!(entity["details"]["schema_name"], "public");
    assert_eq!(entity["details"]["table_name"], "products");
    assert!(entity["match"]["match_signals"].as_array().is_some());
    assert!(entity["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning
            .as_str()
            .unwrap_or_default()
            .contains("schema-heavy table sections")));
}

#[tokio::test]
async fn search_catalog_rejects_stale_index_before_backend_search() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let backend = Arc::new(RecordingSearchBackend {
        hits: vec![CatalogSearchHitRef {
            entry_id: "table:products".to_string(),
            score: 0.9,
            rank: 1,
            match_signals: vec!["name".to_string()],
            matched_fields: vec!["name_text".to_string()],
            raw_score: None,
            snippet: None,
            resume_cursor: None,
        }],
        health: CatalogSearchHealth::Stale {
            indexed_entries: 1,
            projection_version: 1,
            reason: "catalog write-through failed".to_string(),
        },
        requests: Mutex::new(Vec::new()),
    });
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend.clone();
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "products"}), &env)
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("Catalog search index is stale"));
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("Rebuild the catalog search index before retrying search_catalog"));
    assert_eq!(
        backend.requests().len(),
        0,
        "stale health must short-circuit before backend.search"
    );
}

#[tokio::test]
async fn search_catalog_rejects_empty_query_before_backend_search() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let backend = Arc::new(RecordingSearchBackend::ready(vec![CatalogSearchHitRef {
        entry_id: "table:products".to_string(),
        score: 0.9,
        rank: 1,
        match_signals: vec!["name".to_string()],
        matched_fields: vec!["name_text".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }]));
    let backend_ext: Arc<dyn CatalogSearchBackend> = backend.clone();
    let env = base_env()
        .with_catalog(catalog)
        .with_catalog_search_backend(backend_ext);

    let result = SurfaceSearchCatalogHandler
        .handle("call-search", json!({"query": "   "}), &env)
        .await;

    assert!(result.is_error);
    assert_eq!(
        backend.requests().len(),
        0,
        "invalid empty query must not reach backend.search"
    );
}

fn paged_table_entry(index: usize) -> CatalogEntry {
    CatalogEntry {
        id: format!("table:p{index}"),
        kind: CatalogKind::Table,
        name: format!("p{index}"),
        qualified_name: Some(format!("public.p{index}")),
        content: "Paged catalog table.".to_string(),
        tags: vec!["catalog".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": format!("p{index}"),
            "relationType": "table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    }
}

fn paged_hit(index: usize) -> CatalogSearchHitRef {
    CatalogSearchHitRef {
        entry_id: format!("table:p{index}"),
        // Strictly descending so the candidate ordering is deterministic and the
        // test isolates the cursor offset defect (C1) from the tie-break (M1).
        score: 1.0 - (index as f64) * 0.1,
        rank: index + 1,
        match_signals: vec!["name".to_string()],
        matched_fields: vec!["name_text".to_string()],
        raw_score: None,
        snippet: None,
        resume_cursor: None,
    }
}

fn enum_value_entry(
    id: &str,
    value: &str,
    database_id: &str,
    schema_name: &str,
    table_name: &str,
    column_name: &str,
) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Enum,
        name: value.to_string(),
        qualified_name: Some(format!("{schema_name}.{table_name}.{column_name}.{value}")),
        content: format!("{value} enum value."),
        tags: vec!["channel".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": database_id,
            "schemaName": schema_name,
            "tableName": table_name,
            "columnName": column_name,
            "columnId": format!("column:{schema_name}.{table_name}.{column_name}"),
            "value": value,
            "normalizedValue": value.to_lowercase(),
            "displayValue": value,
            "frequency": 1,
            "frequencyPercentage": 10.0,
            "rank": 1,
            "synonyms": []
        }),
    }
}

#[tokio::test]
async fn sample_table_data_rejects_unknown_columns_before_sampling() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![
        table_entry(),
        column_entry("sku", "text", false),
    ]));
    let target_db = Arc::new(RecordingTargetDb::default());
    let env = base_env()
        .with_catalog(catalog)
        .with_target_db(target_db.clone());

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "table:products"}, "columns": ["missing"]}),
            &env,
        )
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("unknown column"));
    assert_eq!(target_db.sample_calls(), 0);
}

#[tokio::test]
async fn sample_table_data_rejects_non_table_ref_before_sampling() {
    let catalog: Arc<dyn DataCatalog> =
        Arc::new(TestCatalog::new(vec![column_entry("sku", "text", false)]));
    let target_db = Arc::new(RecordingTargetDb::default());
    let env = base_env()
        .with_catalog(catalog)
        .with_target_db(target_db.clone());

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "column:products.sku"}}),
            &env,
        )
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("is not a table"));
    assert_eq!(
        target_db.sample_calls(),
        0,
        "non-table refs must not reach target sampling"
    );
}

#[tokio::test]
async fn sample_table_data_clamps_limit_to_20() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let target_db = Arc::new(RecordingTargetDb::with_sample_rows(vec![json!({
        "sku": "P-1",
        "name": "Widget"
    })]));
    let env = base_env()
        .with_catalog(catalog)
        .with_target_db(target_db.clone());

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "table:products"}, "limit": 99}),
            &env,
        )
        .await;

    assert!(
        !result.is_error,
        "sample_table_data should clamp high limits, got {result:?}"
    );
    assert_eq!(target_db.sample_calls(), 1);
    assert_eq!(
        target_db.sample_limits(),
        vec![20],
        "sample_table_data must clamp target DB sampling to 20 rows"
    );
}

#[tokio::test]
async fn sample_table_data_projects_requested_columns() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![
        table_entry(),
        column_entry("sku", "text", false),
        column_entry("name", "text", false),
    ]));
    let target_db = Arc::new(RecordingTargetDb::with_sample_rows(vec![json!({
        "sku": "P-1",
        "name": "Widget"
    })]));
    let env = base_env().with_catalog(catalog).with_target_db(target_db);

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"qualified_name": "public.products", "kind": "table"}, "columns": ["sku"], "limit": 99}),
            &env,
        )
        .await;

    assert!(!result.is_error, "sample_table_data errored: {result:?}");
    assert_eq!(result.content["columns"], json!(["sku"]));
    assert_eq!(result.content["rows"][0], json!({"sku": "P-1"}));
    assert_eq!(result.content["row_count"], 1);
}

fn named_table_entry(table: &str) -> CatalogEntry {
    CatalogEntry {
        id: format!("table:public.{table}"),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(format!("public.{table}")),
        content: format!("{table} table"),
        tags: vec!["catalog".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table,
            "relationType": "table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    }
}

fn join_hop(from_column: &str, to_column: &str, target_table: &str) -> JoinHop {
    JoinHop {
        relation_kind: agent_fw_catalog::relation_kind::REFERENCES_TABLE.to_string(),
        from_column: Some(from_column.to_string()),
        to_column: Some(to_column.to_string()),
        join_type: Some("inner".to_string()),
        description: Some(format!("-> {target_table}.{to_column}")),
        relationship_id: Some(format!("relationship:{target_table}")),
    }
}

#[tokio::test]
async fn join_path_drops_source_step_and_attaches_join_metadata() {
    // M3: single-hop join path must report steps beginning at the FIRST hop
    // target (NOT the `from` table) with length == hop count.
    // M4: each step carries via_relation.join.{from_column,to_column,join_type}.
    let fact = named_table_entry("fact_scenario");
    let dim_products = named_table_entry("dim_products");
    let dim_segments = named_table_entry("dim_segments");

    // Multi-hop JoinPath as the interpreters now produce: steps[0] is `from`,
    // hops[i] describes steps[i] -> steps[i+1].
    let join_path = JoinPath {
        steps: vec![fact.clone(), dim_products.clone(), dim_segments.clone()],
        length: 3,
        hops: vec![
            join_hop("product_id", "product_id", "dim_products"),
            join_hop("segment_id", "segment_id", "dim_segments"),
        ],
    };

    let catalog: Arc<dyn DataCatalog> = Arc::new(
        TestCatalog::new(vec![fact, dim_products, dim_segments]).with_join_path(join_path),
    );
    let env = base_env().with_catalog(catalog);

    let result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-path",
            json!({
                "from": {"qualified_name": "public.fact_scenario", "kind": "table"},
                "to": [
                    {"qualified_name": "public.dim_products", "kind": "table"},
                    {"qualified_name": "public.dim_segments", "kind": "table"}
                ],
                "path_type": "join"
            }),
            &env,
        )
        .await;

    assert!(!result.is_error, "join path errored: {result:?}");
    let paths = result.content["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);

    // First target: single hop. steps must start at dim_products, length 1.
    let single = &paths[0];
    assert_eq!(single["found"], true);
    assert_eq!(single["length"], 1);
    let steps = single["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["entity"]["id"], "table:public.dim_products");
    assert_ne!(
        steps[0]["entity"]["id"], "table:public.fact_scenario",
        "the `from` source table must NOT appear as a step"
    );
    let join = &steps[0]["via_relation"]["join"];
    assert_eq!(join["from_column"], "product_id");
    assert_eq!(join["to_column"], "product_id");
    assert_eq!(join["join_type"], "inner");
    assert_eq!(
        steps[0]["via_relation"]["relation_kind"],
        "references_table"
    );

    // Second target: two hops, length 2, steps [dim_products, dim_segments].
    let multi = &paths[1];
    assert_eq!(multi["length"], 2);
    let multi_steps = multi["steps"].as_array().unwrap();
    assert_eq!(multi_steps.len(), 2);
    assert_eq!(multi_steps[0]["entity"]["id"], "table:public.dim_products");
    assert_eq!(multi_steps[1]["entity"]["id"], "table:public.dim_segments");
    assert_eq!(
        multi_steps[1]["via_relation"]["join"]["from_column"],
        "segment_id"
    );
}

fn relationship_vertex_entry(
    id: &str,
    source_table_id: &str,
    target_table_id: &str,
    source_table: &str,
    target_table: &str,
) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Relationship,
        name: format!("{source_table}_to_{target_table}"),
        qualified_name: None,
        content: format!("{source_table} references {target_table}"),
        tags: vec!["relationship".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": "warehouse",
            "sourceTableId": source_table_id,
            "targetTableId": target_table_id,
            "sourceSchema": "public",
            "sourceTable": source_table,
            "sourceColumn": "fk_id",
            "targetSchema": "public",
            "targetTable": target_table,
            "targetColumn": "id",
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0
        }),
    }
}

#[tokio::test]
async fn any_path_type_finds_join_only_table_path_then_falls_back() {
    // M6: a join-only (materialized FK) table->table path with NO semantic edge.
    // `path_type=any` must try the join path FIRST and report it found, while
    // `path_type=semantic` on the same fixture finds nothing (no graph edge).
    let fact = named_table_entry("fact_scenario");
    let dim_products = named_table_entry("dim_products");
    let join_path = JoinPath {
        steps: vec![fact.clone(), dim_products.clone()],
        length: 2,
        hops: vec![join_hop("product_id", "product_id", "dim_products")],
    };
    // No relationship vertex and no links: the semantic BFS sees no edges.
    let catalog: Arc<dyn DataCatalog> =
        Arc::new(TestCatalog::new(vec![fact, dim_products]).with_join_path(join_path));
    let env = base_env().with_catalog(catalog);

    let any_result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-any",
            json!({
                "from": {"qualified_name": "public.fact_scenario", "kind": "table"},
                "to": [{"qualified_name": "public.dim_products", "kind": "table"}],
                "path_type": "any"
            }),
            &env,
        )
        .await;
    assert!(!any_result.is_error, "any path errored: {any_result:?}");
    let any_path = &any_result.content["paths"][0];
    assert_eq!(any_path["found"], true, "any must find the join-only path");
    assert_eq!(
        any_path["steps"][0]["via_relation"]["join"]["from_column"], "product_id",
        "join-first fallback must carry the typed join metadata"
    );

    let semantic_result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-semantic",
            json!({
                "from": {"qualified_name": "public.fact_scenario", "kind": "table"},
                "to": [{"qualified_name": "public.dim_products", "kind": "table"}],
                "path_type": "semantic"
            }),
            &env,
        )
        .await;
    assert!(!semantic_result.is_error);
    assert_eq!(
        semantic_result.content["paths"][0]["found"], false,
        "semantic must NOT find the join-only path (no graph edge)"
    );
}

#[tokio::test]
async fn semantic_path_scans_relationships_once_per_request() {
    // M7: a multi-node BFS must load the relationship vertex set ONCE per
    // request, not once per dequeued node. Build a 3-table chain connected by
    // relationship vertices so the BFS dequeues several nodes.
    let t_a = named_table_entry("a");
    let t_b = named_table_entry("b");
    let t_c = named_table_entry("c");
    let rel_ab = relationship_vertex_entry(
        "relationship:a_b",
        "table:public.a",
        "table:public.b",
        "a",
        "b",
    );
    let rel_bc = relationship_vertex_entry(
        "relationship:b_c",
        "table:public.b",
        "table:public.c",
        "b",
        "c",
    );

    let catalog = Arc::new(TestCatalog::new(vec![t_a, t_b, t_c, rel_ab, rel_bc]));
    let catalog_dyn: Arc<dyn DataCatalog> = catalog.clone();
    let env = base_env().with_catalog(catalog_dyn);

    let result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-chain",
            json!({
                "from": {"qualified_name": "public.a", "kind": "table"},
                "to": [{"qualified_name": "public.c", "kind": "table"}],
                "path_type": "semantic",
                "max_depth": 4
            }),
            &env,
        )
        .await;

    assert!(!result.is_error, "semantic chain errored: {result:?}");
    assert_eq!(
        result.content["paths"][0]["found"], true,
        "a -> b -> c must be found"
    );
    assert!(
        catalog.relationship_scans() <= 1,
        "relationship vertices must be scanned at most once per request, got {}",
        catalog.relationship_scans()
    );
}

#[tokio::test]
async fn semantic_path_warns_when_hub_adjacency_is_truncated() {
    // M5: a hub node whose adjacency exceeds the per-node limit (100) and whose
    // edge toward the target is beyond the truncation point. The result is
    // found=false but the path MAY exist, so a truncation warning is emitted.
    let mut entries = vec![named_table_entry("hub"), named_table_entry("target")];
    // 120 distinct column edges out of `hub` (relation_kind=has_column). These
    // appear BEFORE the relationship-vertex edges, so the relationship edge to
    // `target` is pushed past the 100-edge truncation point.
    for index in 0..120 {
        entries.push(CatalogEntry {
            id: format!("column:hub.c{index}"),
            kind: CatalogKind::Column,
            name: format!("c{index}"),
            qualified_name: Some(format!("public.hub.c{index}")),
            content: "filler column".to_string(),
            tags: vec![],
            metadata: json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "hub",
                "columnName": format!("c{index}"),
                "dataType": "text",
                "nullable": true,
                "primaryKey": false,
                "foreignKey": null,
                "semanticType": null,
                "lowCardinalityEnum": false
            }),
            links: vec![CatalogRelation {
                target_id: format!("column:hub.c{index}"),
                kind: agent_fw_catalog::relation_kind::HAS_COLUMN.to_string(),
                description: None,
            }],
        });
    }
    // The only edge to `target` is via a relationship vertex (emitted last).
    entries.push(relationship_vertex_entry(
        "relationship:hub_target",
        "table:public.hub",
        "table:public.target",
        "hub",
        "target",
    ));
    // Attach the 120 has_column links to the hub itself.
    let hub_links: Vec<CatalogRelation> = (0..120)
        .map(|index| CatalogRelation {
            target_id: format!("column:hub.c{index}"),
            kind: agent_fw_catalog::relation_kind::HAS_COLUMN.to_string(),
            description: None,
        })
        .collect();
    entries[0].links = hub_links;

    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(entries));
    let env = base_env().with_catalog(catalog);

    let result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-truncated",
            json!({
                "from": {"qualified_name": "public.hub", "kind": "table"},
                "to": [{"qualified_name": "public.target", "kind": "table"}],
                "path_type": "semantic",
                "max_depth": 4
            }),
            &env,
        )
        .await;

    assert!(!result.is_error, "truncated path errored: {result:?}");
    let path = &result.content["paths"][0];
    assert_eq!(path["found"], false, "the target edge was truncated away");
    let warnings = path["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().unwrap_or_default().contains("truncated")),
        "found=false coinciding with truncation must carry a truncation warning, got {warnings:?}"
    );
}

#[tokio::test]
async fn semantic_path_no_truncation_warning_on_small_graph() {
    // Control for M5: a small graph where the path is found carries no warning.
    let t_a = named_table_entry("a");
    let t_b = named_table_entry("b");
    let rel_ab = relationship_vertex_entry(
        "relationship:a_b",
        "table:public.a",
        "table:public.b",
        "a",
        "b",
    );
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![t_a, t_b, rel_ab]));
    let env = base_env().with_catalog(catalog);

    let result = SurfaceGetRelationPathsBetweenHandler
        .handle(
            "call-small",
            json!({
                "from": {"qualified_name": "public.a", "kind": "table"},
                "to": [{"qualified_name": "public.b", "kind": "table"}],
                "path_type": "semantic"
            }),
            &env,
        )
        .await;

    assert!(!result.is_error);
    let path = &result.content["paths"][0];
    assert_eq!(path["found"], true);
    assert!(
        path["warnings"]
            .as_array()
            .map(|warnings| warnings.is_empty())
            .unwrap_or(true),
        "found path must not carry truncation warnings: {:?}",
        path["warnings"]
    );
}

#[tokio::test]
async fn sample_table_data_warns_when_target_row_has_columns_absent_from_catalog() {
    // M9: with an empty `columns` projection the handler projects to the
    // catalog-known columns. A column present in the real target row but missing
    // from the (stale) catalog is silently dropped today; the handler must now
    // surface a data-quality warning that names the omitted column.
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![
        table_entry(),
        column_entry("sku", "text", false),
        column_entry("name", "text", false),
    ]));
    let target_db = Arc::new(RecordingTargetDb::with_sample_rows(vec![json!({
        "sku": "P-1",
        "name": "Widget",
        "extra_col": "ghost"
    })]));
    let env = base_env().with_catalog(catalog).with_target_db(target_db);

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "table:products"}}),
            &env,
        )
        .await;

    assert!(!result.is_error, "sample_table_data errored: {result:?}");
    // Projection behavior is unchanged: extra_col is still dropped from rows.
    assert_eq!(
        result.content["rows"][0],
        json!({"sku": "P-1", "name": "Widget"})
    );
    let warnings = result.content["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().unwrap_or_default().contains("extra_col")),
        "expected a stale-catalog warning naming extra_col, got {warnings:?}"
    );
}

#[tokio::test]
async fn sample_table_data_no_warning_when_all_target_columns_are_known() {
    // Control for M9: when every target row key is catalog-known, no warning.
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![
        table_entry(),
        column_entry("sku", "text", false),
        column_entry("name", "text", false),
    ]));
    let target_db = Arc::new(RecordingTargetDb::with_sample_rows(vec![json!({
        "sku": "P-1",
        "name": "Widget"
    })]));
    let env = base_env().with_catalog(catalog).with_target_db(target_db);

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "table:products"}}),
            &env,
        )
        .await;

    assert!(!result.is_error, "sample_table_data errored: {result:?}");
    assert!(
        result.content["warnings"]
            .as_array()
            .map(|warnings| warnings.is_empty())
            .unwrap_or(true),
        "no warning expected when all columns are catalog-known: {:?}",
        result.content["warnings"]
    );
}

#[tokio::test]
async fn sample_table_data_explicit_columns_does_not_warn_on_missing_catalog_column() {
    // Control for M9: an explicit column selection is the agent's choice; the
    // stale-catalog omission warning only applies to the implicit (empty
    // `columns`) projection path.
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![
        table_entry(),
        column_entry("sku", "text", false),
        column_entry("name", "text", false),
    ]));
    let target_db = Arc::new(RecordingTargetDb::with_sample_rows(vec![json!({
        "sku": "P-1",
        "name": "Widget",
        "extra_col": "ghost"
    })]));
    let env = base_env().with_catalog(catalog).with_target_db(target_db);

    let result = SurfaceSampleTableDataHandler
        .handle(
            "call-sample",
            json!({"table": {"id": "table:products"}, "columns": ["sku"]}),
            &env,
        )
        .await;

    assert!(!result.is_error, "sample_table_data errored: {result:?}");
    assert!(
        result.content["warnings"]
            .as_array()
            .map(|warnings| warnings.is_empty())
            .unwrap_or(true),
        "explicit column selection must not warn about catalog omissions: {:?}",
        result.content["warnings"]
    );
}

#[tokio::test]
async fn get_catalog_entities_hydrates_document_body_from_workspace_kv() {
    let document_entry = CatalogEntry {
        id: "document:slow-movers".to_string(),
        kind: CatalogKind::Document,
        name: "Slow mover playbook".to_string(),
        qualified_name: Some("docs.slow_movers".to_string()),
        content: "Catalog summary only.".to_string(),
        tags: vec!["ops".to_string()],
        links: vec![],
        metadata: json!({
            "sourceDocumentId": "doc-slow",
            "contentAvailable": true,
            "contentSource": "kv",
            "extractionStatus": "processed",
            "extractedKnowledgeIds": []
        }),
    };
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![document_entry]));
    let kv = Arc::new(DashMapKVStore::new());
    kv.put(
        "tenant-1::workspace:workspace-1",
        "data:document:doc-slow",
        &DocumentItem {
            id: "doc-slow".to_string(),
            name: "Slow mover playbook".to_string(),
            content: "Full body text from KV.".to_string(),
            target_database_id: None,
            extraction_status: ExtractionStatus::Processed,
            extracted_knowledge_ids: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
        None,
    )
    .await
    .unwrap();
    let kv_ext: Arc<dyn KVStore> = kv;
    let env = base_env().with_catalog(catalog).with_kv(kv_ext);

    let result = SurfaceGetCatalogEntitiesHandler
        .handle(
            "call-entities",
            json!({"refs": [{"id": "document:slow-movers"}]}),
            &env,
        )
        .await;

    assert!(!result.is_error, "get_catalog_entities errored: {result:?}");
    assert_eq!(
        result.content["entities"][0]["description"],
        "Full body text from KV."
    );
}

#[tokio::test]
async fn get_catalog_relations_enforces_refs_bound_server_side() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let env = base_env().with_catalog(catalog);
    let refs: Vec<_> = (0..21)
        .map(|index| json!({"id": format!("table:{index}")}))
        .collect();

    let result = agent_fw_catalog_tools::surface::handlers::GetCatalogRelationsHandler
        .handle("call-relations", json!({"refs": refs}), &env)
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("1..=20"));
}

#[tokio::test]
async fn get_catalog_entities_rejects_empty_refs() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let env = base_env().with_catalog(catalog);

    let result = SurfaceGetCatalogEntitiesHandler
        .handle("call-entities", json!({"refs": []}), &env)
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("1..=50"));
}

#[tokio::test]
async fn list_schema_fields_rejects_empty_tables() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let env = base_env().with_catalog(catalog);

    let result = agent_fw_catalog_tools::surface::handlers::ListSchemaFieldsHandler
        .handle("call-fields", json!({"tables": []}), &env)
        .await;

    assert!(result.is_error);
    assert!(result.content["error"]
        .as_str()
        .unwrap_or_default()
        .contains("1..=10"));
}

#[tokio::test]
async fn execute_query_rejects_mutations_before_target_database_query() {
    let cases = [
        ("drop", "DROP TABLE products"),
        ("insert", "INSERT INTO products (sku) VALUES ('P-1')"),
        ("update", "UPDATE products SET name = 'Widget'"),
        ("create", "CREATE TABLE scratch_products (sku text)"),
        ("alter", "ALTER TABLE products ADD COLUMN temp text"),
    ];

    for (label, sql) in cases {
        let target_db = Arc::new(RecordingTargetDb::default());
        let env = base_env().with_target_db(target_db.clone());

        let result = SurfaceExecuteQueryHandler
            .handle("call-query", json!({"sql": sql}), &env)
            .await;

        assert!(result.is_error, "{label} SQL should be rejected");
        assert_eq!(
            target_db.query_calls(),
            0,
            "{label} SQL must not reach target query execution"
        );
    }
}

#[tokio::test]
async fn execute_query_rejects_multi_statement_before_target_database_query() {
    let target_db = Arc::new(RecordingTargetDb::default());
    let env = base_env().with_target_db(target_db.clone());

    let result = SurfaceExecuteQueryHandler
        .handle(
            "call-query",
            json!({"sql": "SELECT * FROM products; SELECT * FROM orders"}),
            &env,
        )
        .await;

    assert!(result.is_error);
    assert_eq!(
        target_db.query_calls(),
        0,
        "multi-statement SQL must not reach target query execution"
    );
}

#[tokio::test]
async fn surface_dispatcher_does_not_include_legacy_tool_names() {
    let catalog: Arc<dyn DataCatalog> = Arc::new(TestCatalog::new(vec![table_entry()]));
    let backend: Arc<dyn CatalogSearchBackend> =
        Arc::new(RecordingSearchBackend::ready(Vec::new()));
    let target_db: Arc<dyn TargetDatabase> = Arc::new(RecordingTargetDb::default());
    let dispatcher = ComposedDispatcher::new(
        base_env()
            .with_catalog(catalog)
            .with_catalog_search_backend(backend)
            .with_target_db(target_db),
    )
    .with_handlers(surface_handlers());

    let names: std::collections::HashSet<String> = dispatcher
        .tool_definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();

    assert!(names.contains("search_catalog"));
    assert!(!names.contains("fuzzy_table_search"));
    assert!(!names.contains("resolve_term"));
    assert!(!names.contains("list_tables"));
    assert!(!names.contains("search_knowledge"));
}

fn base_env() -> ToolEnvironment {
    ToolEnvironment::builder()
        .kv(agent_fw_algebra::testing::NullKVStore)
        .tenant_context(agent_fw_core::tenant::TenantContext::new(
            TenantId::new_unchecked("tenant-1"),
        ))
        .build()
        .with_ext::<WorkspaceContext>(Arc::new(WorkspaceContext::from_ids(
            TenantId::new_unchecked("tenant-1"),
            Some("workspace-1"),
        )))
}

fn table_entry() -> CatalogEntry {
    CatalogEntry {
        id: "table:products".to_string(),
        kind: CatalogKind::Table,
        name: "products".to_string(),
        qualified_name: Some("public.products".to_string()),
        content: "Product master data.".to_string(),
        tags: vec!["catalog".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "products",
            "relationType": "table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    }
}

fn column_entry(name: &str, data_type: &str, low_cardinality_enum: bool) -> CatalogEntry {
    CatalogEntry {
        id: format!("column:products.{name}"),
        kind: CatalogKind::Column,
        name: name.to_string(),
        qualified_name: Some(format!("public.products.{name}")),
        content: format!("Product {name}."),
        tags: vec![],
        links: vec![CatalogRelation {
            target_id: "table:products".to_string(),
            kind: agent_fw_catalog::relation_kind::BELONGS_TO.to_string(),
            description: None,
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "products",
            "columnName": name,
            "dataType": data_type,
            "nullable": false,
            "primaryKey": name == "sku",
            "foreignKey": null,
            "semanticType": null,
            "lowCardinalityEnum": low_cardinality_enum
        }),
    }
}

struct TestCatalog {
    entries: Vec<CatalogEntry>,
    join_path: Option<JoinPath>,
    relationship_scans: Mutex<usize>,
}

impl TestCatalog {
    fn new(entries: Vec<CatalogEntry>) -> Self {
        Self {
            entries,
            join_path: None,
            relationship_scans: Mutex::new(0),
        }
    }

    fn with_join_path(mut self, join_path: JoinPath) -> Self {
        self.join_path = Some(join_path);
        self
    }

    fn relationship_scans(&self) -> usize {
        *self.relationship_scans.lock().unwrap()
    }
}

#[async_trait]
impl DataCatalog for TestCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(self.entries.iter().find(|entry| entry.id == id).cloned())
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(ids
            .iter()
            .filter_map(|id| self.entries.iter().find(|entry| &entry.id == id).cloned())
            .collect())
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .find(|entry| {
                entry.kind == kind && entry.qualified_name.as_deref() == Some(qualified_name)
            })
            .cloned())
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        if kind == CatalogKind::Relationship {
            *self.relationship_scans.lock().unwrap() += 1;
        }
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let Some(source) = self.entries.iter().find(|entry| entry.id == id) else {
            return Ok(Vec::new());
        };
        Ok(source
            .links
            .iter()
            .filter(|link| relation_type.is_none_or(|kind| kind == link.kind))
            .filter_map(|link| {
                self.entries
                    .iter()
                    .find(|entry| entry.id == link.target_id)
                    .cloned()
            })
            .collect())
    }

    async fn get_related_reverse(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.links.iter().any(|link| {
                    link.target_id == id && relation_type.is_none_or(|kind| kind == link.kind)
                })
            })
            .cloned()
            .collect())
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        let Some(full) = self.join_path.clone() else {
            return Ok(None);
        };
        // Truncate the configured path at the requested target so multi-hop and
        // single-hop targets resolve to their own sub-paths (mirrors a real BFS,
        // which stops at the destination).
        let Some(target_index) = full.steps.iter().position(|entry| {
            entry.qualified_name.as_deref() == Some(to_table)
                || entry.name == to_table
                || entry.id == to_table
        }) else {
            return Ok(None);
        };
        let steps = full.steps[..=target_index].to_vec();
        let hops = full.hops[..target_index].to_vec();
        let length = steps.len();
        Ok(Some(JoinPath {
            steps,
            length,
            hops,
        }))
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.list_by_type(CatalogKind::Table, usize::MAX).await
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Column)
            .filter(|entry| {
                entry
                    .metadata
                    .get("tableName")
                    .and_then(|value| value.as_str())
                    == Some(table_name)
                    || entry
                        .qualified_name
                        .as_deref()
                        .is_some_and(|qualified_name| {
                            qualified_name.starts_with(&format!("{table_name}."))
                                || qualified_name.starts_with(&format!("public.{table_name}."))
                        })
            })
            .cloned()
            .collect())
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Ok(Vec::new())
    }
}

struct RecordingSearchBackend {
    hits: Vec<CatalogSearchHitRef>,
    health: CatalogSearchHealth,
    requests: Mutex<Vec<CatalogSearchRequest>>,
}

impl RecordingSearchBackend {
    fn ready(hits: Vec<CatalogSearchHitRef>) -> Self {
        Self {
            hits,
            health: CatalogSearchHealth::Ready {
                indexed_entries: 1,
                projection_version: 1,
            },
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<CatalogSearchRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl CatalogSearchBackend for RecordingSearchBackend {
    async fn search(
        &self,
        _scope: &CatalogScope,
        request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogError> {
        self.requests.lock().unwrap().push(request.clone());
        // The mock cursor scheme is a plain absolute offset string into `hits`,
        // mirroring how the real Tantivy backend mints offset cursors. A `None`
        // cursor starts at offset 0.
        let offset = request
            .cursor
            .as_ref()
            .map(|cursor| cursor.as_str().parse::<usize>().expect("offset cursor"))
            .unwrap_or(0);
        let hits: Vec<CatalogSearchHitRef> = self
            .hits
            .iter()
            .enumerate()
            .skip(offset)
            .take(request.limit)
            .map(|(position, hit)| CatalogSearchHitRef {
                // Resume strictly after this hit (offset of the next candidate).
                resume_cursor: Some(CatalogSearchCursor::new((position + 1).to_string())),
                ..hit.clone()
            })
            .collect();
        let returned = hits.len();
        let has_more = offset + returned < self.hits.len();
        Ok(CatalogSearchResults {
            hits,
            facets: BackendFacets::default(),
            has_more,
            next_cursor: has_more
                .then(|| CatalogSearchCursor::new((offset + returned).to_string())),
            candidate_count: self.hits.len(),
            warnings: Vec::new(),
        })
    }

    async fn health(&self, _scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogError> {
        Ok(self.health.clone())
    }
}

#[derive(Default)]
struct RecordingTargetDb {
    rows: Vec<serde_json::Value>,
    sample_calls: Mutex<usize>,
    sample_limits: Mutex<Vec<usize>>,
    query_calls: Mutex<usize>,
}

impl RecordingTargetDb {
    fn with_sample_rows(rows: Vec<serde_json::Value>) -> Self {
        Self {
            rows,
            sample_calls: Mutex::new(0),
            sample_limits: Mutex::new(Vec::new()),
            query_calls: Mutex::new(0),
        }
    }

    fn sample_calls(&self) -> usize {
        *self.sample_calls.lock().unwrap()
    }

    fn sample_limits(&self) -> Vec<usize> {
        self.sample_limits.lock().unwrap().clone()
    }

    fn query_calls(&self) -> usize {
        *self.query_calls.lock().unwrap()
    }
}

#[async_trait]
impl TargetDatabase for RecordingTargetDb {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn query(
        &self,
        _query: &ReadOnlyQuery,
        _params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        *self.query_calls.lock().unwrap() += 1;
        Ok(Vec::new())
    }

    async fn health_check(&self) -> Result<(), DbError> {
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        Ok(Vec::new())
    }

    async fn get_table_columns(&self, _table_name: &str) -> Result<Vec<DbRow>, DbError> {
        Ok(Vec::new())
    }

    async fn sample_table(
        &self,
        _table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        *self.sample_calls.lock().unwrap() += 1;
        self.sample_limits.lock().unwrap().push(limit);
        Ok(self.rows.clone())
    }
}
