use std::convert::TryFrom;

use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogRelation, CatalogScope, CatalogSearchBackend,
    CatalogSearchFilters, CatalogSearchHealth, CatalogSearchRequest, SemanticEntity,
    SemanticEntityKind,
};
use agent_fw_catalog_index::{
    CatalogDocumentProjection, TantivyCatalogIndex, PROJECTED_CATALOG_SCHEMA_VERSION,
};
use agent_fw_core::{TenantId, WorkspaceId};
use serde_json::json;

fn scope(name: &str) -> CatalogScope {
    CatalogScope::new(
        TenantId::new_unchecked(format!("tenant-{name}")),
        WorkspaceId::new_unchecked(format!("workspace-{name}")),
    )
}

fn entry(
    id: &str,
    kind: CatalogKind,
    name: &str,
    qualified_name: Option<&str>,
    content: &str,
    tags: Vec<&str>,
    metadata: serde_json::Value,
) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.map(str::to_string),
        content: content.to_string(),
        tags: tags.into_iter().map(str::to_string).collect(),
        links: Vec::<CatalogRelation>::new(),
        metadata,
    }
}

fn project(entry: CatalogEntry, body: Option<&str>) -> CatalogDocumentProjection {
    let entity = SemanticEntity::try_from(entry).unwrap();
    CatalogDocumentProjection::project(&entity, body.map(str::to_string)).unwrap()
}

fn table_entry(id: &str, table: &str, content: &str) -> CatalogEntry {
    entry(
        id,
        CatalogKind::Table,
        table,
        Some(&format!("public.{table}")),
        content,
        vec!["sales"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    )
}

fn column_entry() -> CatalogEntry {
    entry(
        "column:fact_sales.units",
        CatalogKind::Column,
        "units",
        Some("public.fact_sales.units"),
        "Units sold for each scenario.",
        vec!["measure"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "units",
            "dataType": "numeric",
            "nullable": false,
            "primaryKey": false,
            "semanticType": "unit_sales",
            "distinctCount": 25,
            "nullCount": 0,
            "totalCount": 100,
            "lowCardinalityEnum": false
        }),
    )
}

fn enum_entry() -> CatalogEntry {
    entry(
        "enum:channel.supermarket",
        CatalogKind::Enum,
        "Supermarket",
        Some("public.dim_channels.channel_name.Supermarket"),
        "Supermarket sales channel.",
        vec!["channel"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "dim_channels",
            "columnName": "channel_name",
            "columnId": "column:dim_channels.channel_name",
            "value": "Supermarket",
            "normalizedValue": "supermarket",
            "displayValue": "Supermarket",
            "frequency": 1,
            "frequencyPercentage": 25.0,
            "rank": 1,
            "synonyms": ["grocery", "supermarket channel"]
        }),
    )
}

fn relationship_entry() -> CatalogEntry {
    entry(
        "relationship:public.fact_sales.product_id->public.dim_products.product_id",
        CatalogKind::Relationship,
        "fact_sales_to_dim_products",
        None,
        "fact_sales.product_id references dim_products.product_id",
        vec!["relationship"],
        json!({
            "databaseId": "warehouse",
            "sourceTableId": "table:public.fact_sales",
            "targetTableId": "table:public.dim_products",
            "sourceSchema": "public",
            "sourceTable": "fact_sales",
            "sourceColumn": "product_id",
            "targetSchema": "public",
            "targetTable": "dim_products",
            "targetColumn": "product_id",
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0
        }),
    )
}

fn document_entry() -> CatalogEntry {
    entry(
        "document:slow-movers",
        CatalogKind::Document,
        "Slow mover playbook",
        Some("docs.slow_movers"),
        "Source document for slow moving product rules.",
        vec!["ops"],
        json!({
            "sourceDocumentId": "doc-slow",
            "contentAvailable": true,
            "contentSource": "kv",
            "extractionStatus": "processed",
            "extractedKnowledgeIds": ["knowledge:slow-movers"]
        }),
    )
}

fn search_request(query: &str, limit: usize) -> CatalogSearchRequest {
    CatalogSearchRequest {
        query: query.to_string(),
        kinds: Vec::new(),
        filters: CatalogSearchFilters::default(),
        limit,
        cursor: None,
    }
}

#[test]
fn exact_entry_id_search_matches_without_text_match() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("exact-id");
    index
        .rebuild(
            &scope,
            vec![project(
                table_entry("table:opaque_identifier", "orders", "Customer order data."),
                None,
            )],
        )
        .unwrap();

    let results =
        tokio_test::block_on(index.search(&scope, search_request("table:opaque_identifier", 10)))
            .unwrap();

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].entry_id, "table:opaque_identifier");
}

#[test]
fn matched_fields_use_tantivy_token_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("token-diagnostics");
    index
        .rebuild(
            &scope,
            vec![project(
                table_entry(
                    "abc",
                    "stock_keeping_units",
                    "xabcx token should not match the query.",
                ),
                None,
            )],
        )
        .unwrap();

    let results = tokio_test::block_on(index.search(&scope, search_request("abc", 10))).unwrap();

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].entry_id, "abc");
    assert!(
        !results.hits[0]
            .matched_fields
            .contains(&"description".to_string()),
        "diagnostics must not report substring-only text matches"
    );
}

#[test]
fn search_matches_synonyms_enum_values_document_body_and_facets() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("search");
    index
        .rebuild(
            &scope,
            vec![
                project(
                    table_entry("table:fact_sales", "fact_sales", "Scenario sales facts."),
                    None,
                ),
                project(enum_entry(), None),
                project(
                    document_entry(),
                    Some("Slow movers are products with low weekly unit velocity."),
                ),
            ],
        )
        .unwrap();

    let synonym_results =
        tokio_test::block_on(index.search(&scope, search_request("grocery", 10))).unwrap();
    assert_eq!(synonym_results.hits[0].entry_id, "enum:channel.supermarket");
    assert!(synonym_results.hits[0]
        .matched_fields
        .contains(&"synonyms".to_string()));
    assert!(synonym_results.hits[0]
        .match_signals
        .contains(&"synonym".to_string()));

    let enum_results =
        tokio_test::block_on(index.search(&scope, search_request("Supermarket", 10))).unwrap();
    assert_eq!(enum_results.hits[0].entry_id, "enum:channel.supermarket");
    assert!(enum_results.hits[0]
        .matched_fields
        .contains(&"enum_value".to_string()));

    let body_results =
        tokio_test::block_on(index.search(&scope, search_request("weekly velocity", 10))).unwrap();
    assert_eq!(body_results.hits[0].entry_id, "document:slow-movers");
    assert!(body_results.hits[0]
        .matched_fields
        .contains(&"document_body".to_string()));
    assert!(body_results.hits[0]
        .snippet
        .as_deref()
        .unwrap()
        .contains("weekly"));

    let facet_results =
        tokio_test::block_on(index.search(&scope, search_request("sales", 10))).unwrap();
    assert!(facet_results
        .facets
        .kinds
        .iter()
        .any(|facet| facet.value == "table" && facet.count == 1));
    assert!(facet_results
        .facets
        .schemas
        .iter()
        .any(|facet| facet.value == "public" && facet.count >= 1));
    assert!(facet_results
        .facets
        .tags
        .iter()
        .any(|facet| facet.value == "sales" && facet.count == 1));
}

#[test]
fn source_filters_do_not_match_generic_table_membership() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("source-filters");
    index
        .rebuild(
            &scope,
            vec![
                project(
                    table_entry("table:fact_sales", "fact_sales", "Sales source table."),
                    None,
                ),
                project(relationship_entry(), None),
            ],
        )
        .unwrap();

    let mut request = search_request("fact_sales", 10);
    request.filters.source_table = Some("fact_sales".to_string());

    let results = tokio_test::block_on(index.search(&scope, request)).unwrap();

    assert_eq!(results.hits.len(), 1);
    assert_eq!(
        results.hits[0].entry_id,
        "relationship:public.fact_sales.product_id->public.dim_products.product_id"
    );
}

#[test]
fn search_applies_kind_filters_exact_filters_and_cursor_offsets() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("filters");
    index
        .rebuild(
            &scope,
            vec![
                project(
                    table_entry("table:fact_sales", "fact_sales", "Sales units table."),
                    None,
                ),
                project(
                    table_entry("table:dim_products", "dim_products", "Products table."),
                    None,
                ),
                project(column_entry(), None),
            ],
        )
        .unwrap();

    let mut filtered = search_request("units", 10);
    filtered.kinds = vec![SemanticEntityKind::Column];
    filtered.filters = CatalogSearchFilters {
        schema: Some("public".to_string()),
        table: Some("fact_sales".to_string()),
        data_type: Some("numeric".to_string()),
        semantic_type: Some("unit_sales".to_string()),
        low_cardinality_enum: Some(false),
        tags: vec!["measure".to_string()],
        ..CatalogSearchFilters::default()
    };

    let filtered_results = tokio_test::block_on(index.search(&scope, filtered)).unwrap();
    assert_eq!(filtered_results.hits.len(), 1);
    assert_eq!(filtered_results.hits[0].entry_id, "column:fact_sales.units");

    let page_one = tokio_test::block_on(index.search(&scope, search_request("table", 1))).unwrap();
    assert_eq!(page_one.hits.len(), 1);
    assert!(page_one.has_more);

    let mut page_two_request = search_request("table", 1);
    page_two_request.cursor = page_one.next_cursor.clone();
    let page_two = tokio_test::block_on(index.search(&scope, page_two_request)).unwrap();
    assert_eq!(page_two.hits.len(), 1);
    assert_ne!(page_one.hits[0].entry_id, page_two.hits[0].entry_id);
}

#[test]
fn search_overfetch_window_cursor_resumes_after_last_consumed() {
    // A handler that over-fetches a window wider than the page it emits must be
    // able to resume strictly after the LAST CANDIDATE IT CONSUMED, not at the
    // window end. The backend surfaces that resume token per hit. Build a scope
    // with four entries matching one broad query, fetch the whole window, treat
    // the first two hits as "consumed", then resume from the second consumed
    // hit's `resume_cursor`. Page two must start at the third entry of page one
    // (no skip) and must not re-return either consumed id (no duplicate).
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("overfetch-resume");
    index
        .rebuild(
            &scope,
            vec![
                project(
                    table_entry("table:sales_a", "sales_a", "Sales facts."),
                    None,
                ),
                project(
                    table_entry("table:sales_b", "sales_b", "Sales facts."),
                    None,
                ),
                project(
                    table_entry("table:sales_c", "sales_c", "Sales facts."),
                    None,
                ),
                project(
                    table_entry("table:sales_d", "sales_d", "Sales facts."),
                    None,
                ),
            ],
        )
        .unwrap();

    // Over-fetch the full candidate window in one request.
    let page_one = tokio_test::block_on(index.search(&scope, search_request("sales", 10))).unwrap();
    assert_eq!(page_one.hits.len(), 4);
    assert!(
        page_one.hits.iter().all(|hit| hit.resume_cursor.is_some()),
        "every hit must carry a per-candidate resume cursor"
    );

    // Simulate consuming only the first two candidates of the over-fetched window.
    let consumed = &page_one.hits[..2];
    let resume_cursor = consumed
        .last()
        .unwrap()
        .resume_cursor
        .clone()
        .expect("consumed hit resume cursor");

    let mut page_two_request = search_request("sales", 10);
    page_two_request.cursor = Some(resume_cursor);
    let page_two = tokio_test::block_on(index.search(&scope, page_two_request)).unwrap();

    // No skip: page two resumes exactly at the third candidate of page one.
    assert_eq!(page_two.hits[0].entry_id, page_one.hits[2].entry_id);

    // No duplicate: neither consumed id reappears on page two.
    let consumed_ids: Vec<&str> = consumed.iter().map(|hit| hit.entry_id.as_str()).collect();
    assert!(page_two
        .hits
        .iter()
        .all(|hit| !consumed_ids.contains(&hit.entry_id.as_str())));
}

#[test]
fn equal_score_hits_break_ties_on_ascending_entry_id_across_rebuilds() {
    // Equal-score hits must break ties on ascending `entry_id` so that the
    // candidate ordering is stable across rebuilds, segment layouts, and
    // SQLite- vs Postgres-built indexes (entry_id is the catalog primary key).
    // A score-only collector would tie-break on `DocAddress`, i.e. on physical
    // insertion/segment order, which is not reproducible. Build four entries
    // crafted to score identically for one query but inserted in an order that
    // does not match `entry_id` order, then assert hits come back in strict
    // ascending `entry_id` order. Rebuild a second fresh index with a different
    // insertion order and assert the identical ordering.
    fn tied_entry(id: &str) -> CatalogDocumentProjection {
        // Identical name/content/tags across all four entries so the same query
        // token matches each field identically and the scores tie.
        project(
            table_entry(id, "shared_facts", "Shared catalog facts."),
            None,
        )
    }

    fn ordered_hit_ids(insertion_order: &[&str]) -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        let index = TantivyCatalogIndex::new(tmp.path());
        let scope = scope("tie-break");
        index
            .rebuild(
                &scope,
                insertion_order
                    .iter()
                    .map(|id| tied_entry(id))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let results =
            tokio_test::block_on(index.search(&scope, search_request("shared", 10))).unwrap();
        // The tie must be genuine: all four raw scores are equal.
        let raw_scores: Vec<f64> = results
            .hits
            .iter()
            .map(|hit| hit.raw_score.expect("raw score present"))
            .collect();
        assert_eq!(results.hits.len(), 4, "all four tied entries must match");
        assert!(
            raw_scores.windows(2).all(|pair| pair[0] == pair[1]),
            "expected genuinely tied raw scores, got {raw_scores:?}"
        );
        results
            .hits
            .iter()
            .map(|hit| hit.entry_id.clone())
            .collect()
    }

    let expected = vec![
        "table:a".to_string(),
        "table:b".to_string(),
        "table:m".to_string(),
        "table:z".to_string(),
    ];

    // Insertion order deliberately differs from entry_id order so DocAddress
    // order != entry_id order.
    let first = ordered_hit_ids(&["table:z", "table:a", "table:m", "table:b"]);
    assert_eq!(first, expected);

    // A second, freshly built index with a different physical layout must
    // produce the identical ascending order, proving the ordering is
    // independent of insertion/DocAddress order.
    let second = ordered_hit_ids(&["table:b", "table:m", "table:a", "table:z"]);
    assert_eq!(second, expected);
    assert_eq!(first, second);
}

#[test]
fn cursor_is_bound_to_original_query_and_filters() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("cursor-binding");
    index
        .rebuild(
            &scope,
            vec![
                project(
                    table_entry("table:fact_sales", "fact_sales", "Sales table."),
                    None,
                ),
                project(
                    table_entry("table:dim_products", "dim_products", "Products table."),
                    None,
                ),
            ],
        )
        .unwrap();

    let page_one = tokio_test::block_on(index.search(&scope, search_request("table", 1))).unwrap();
    let mut mismatched_request = search_request("products", 1);
    mismatched_request.cursor = page_one.next_cursor;

    let error = tokio_test::block_on(index.search(&scope, mismatched_request)).unwrap_err();

    assert!(matches!(error, CatalogError::InvalidQuery(message) if message.contains("cursor")));
}

#[test]
fn broad_queries_warn_when_facet_counts_are_truncated() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("facet-warning");
    let projections = (0..=1_000)
        .map(|i| {
            project(
                table_entry(
                    &format!("table:facet_{i}"),
                    &format!("facet_{i}"),
                    "Shared catalog table.",
                ),
                None,
            )
        })
        .collect::<Vec<_>>();
    index.rebuild(&scope, projections).unwrap();

    let results = tokio_test::block_on(index.search(&scope, search_request("catalog", 1))).unwrap();

    assert_eq!(results.candidate_count, 1_001);
    assert!(results
        .warnings
        .iter()
        .any(|warning| warning.contains("facet counts")));
}

#[test]
fn upsert_delete_and_health_track_index_state() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("health");

    let missing = tokio_test::block_on(index.health(&scope)).unwrap();
    assert!(matches!(missing, CatalogSearchHealth::Unavailable { .. }));

    index
        .rebuild(
            &scope,
            vec![project(
                table_entry("table:fact_sales", "fact_sales", "Old sales copy."),
                None,
            )],
        )
        .unwrap();

    let ready = tokio_test::block_on(index.health(&scope)).unwrap();
    assert_eq!(
        ready,
        CatalogSearchHealth::Ready {
            indexed_entries: 1,
            projection_version: PROJECTED_CATALOG_SCHEMA_VERSION
        }
    );

    index
        .mark_stale(&scope, "catalog write-through failed")
        .unwrap();
    let stale = tokio_test::block_on(index.health(&scope)).unwrap();
    assert!(matches!(
        stale,
        CatalogSearchHealth::Stale {
            indexed_entries: 1,
            reason,
            ..
        } if reason == "catalog write-through failed"
    ));

    index
        .upsert(
            &scope,
            project(
                table_entry("table:fact_sales", "fact_sales", "Fresh revenue copy."),
                None,
            ),
        )
        .unwrap();
    let still_stale_after_upsert = tokio_test::block_on(index.health(&scope)).unwrap();
    assert!(matches!(
        still_stale_after_upsert,
        CatalogSearchHealth::Stale { .. }
    ));

    let stale_search = tokio_test::block_on(index.search(&scope, search_request("Fresh", 10)))
        .expect_err("search must not serve direct writes while stale marker remains");
    assert!(
        matches!(stale_search, CatalogError::Unavailable(message) if message.contains("stale"))
    );

    index.delete(&scope, "table:fact_sales").unwrap();
    let still_stale_after_delete = tokio_test::block_on(index.health(&scope)).unwrap();
    assert!(matches!(
        still_stale_after_delete,
        CatalogSearchHealth::Stale { .. }
    ));

    index.rebuild(&scope, Vec::new()).unwrap();
    let empty_health = tokio_test::block_on(index.health(&scope)).unwrap();
    assert_eq!(
        empty_health,
        CatalogSearchHealth::Ready {
            indexed_entries: 0,
            projection_version: PROJECTED_CATALOG_SCHEMA_VERSION
        }
    );
}

#[test]
fn health_marks_old_projection_version_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("old-projection-version");
    let mut old_projection = project(
        table_entry("table:fact_sales", "fact_sales", "Sales facts."),
        None,
    );
    old_projection.projection_version = 1;
    index.rebuild(&scope, vec![old_projection]).unwrap();

    let health = tokio_test::block_on(index.health(&scope)).unwrap();

    assert!(matches!(
        health,
        CatalogSearchHealth::Stale {
            indexed_entries: 1,
            projection_version: 1,
            reason,
        } if reason.contains("projection_version")
    ));
}

#[test]
fn search_rejects_stale_index_even_if_reader_can_open() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("stale-search");
    index
        .rebuild(
            &scope,
            vec![project(
                table_entry("table:fact_sales", "fact_sales", "Sales facts."),
                None,
            )],
        )
        .unwrap();
    index.mark_stale(&scope, "catalog write").unwrap();

    let error = tokio_test::block_on(index.search(&scope, search_request("Sales", 10)))
        .expect_err("search must not serve a stale index");

    assert!(matches!(error, CatalogError::Unavailable(message) if message.contains("stale")));
}

#[test]
fn search_warns_and_skips_documents_missing_entry_id() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope = scope("missing-entry-id");
    let mut projection = project(
        table_entry("table:fact_sales", "fact_sales", "Broken projection."),
        None,
    );
    projection.entry_id.clear();
    index.rebuild(&scope, vec![projection]).unwrap();

    let results = tokio_test::block_on(index.search(&scope, search_request("Broken", 10))).unwrap();

    assert!(
        results.hits.is_empty(),
        "corrupt documents without entry_id must not surface as empty-id hits"
    );
    assert!(
        results
            .warnings
            .iter()
            .any(|warning| warning.contains("entry_id")),
        "missing entry_id should be diagnosable, got {:?}",
        results.warnings
    );
}
