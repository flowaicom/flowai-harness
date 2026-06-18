use std::convert::TryFrom;

use agent_fw_catalog::{
    CatalogEntry, CatalogKind, CatalogRelation, CatalogScope, CatalogSearchBackend,
    CatalogSearchFilters, CatalogSearchRequest, SemanticEntity,
};
use agent_fw_catalog_index::{CatalogDocumentProjection, CatalogIndexPaths, TantivyCatalogIndex};
use agent_fw_core::{TenantId, WorkspaceId};
use serde_json::json;

fn scope(tenant: &str, workspace: &str) -> CatalogScope {
    CatalogScope::new(
        TenantId::new_unchecked(tenant),
        WorkspaceId::new_unchecked(workspace),
    )
}

fn table_entry(id: &str, table: &str, content: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(format!("public.{table}")),
        content: content.to_string(),
        tags: vec!["sales".to_string()],
        links: Vec::<CatalogRelation>::new(),
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    }
}

fn project(entry: CatalogEntry) -> CatalogDocumentProjection {
    let entity = SemanticEntity::try_from(entry).unwrap();
    CatalogDocumentProjection::project(&entity, None).unwrap()
}

fn request(query: &str) -> CatalogSearchRequest {
    CatalogSearchRequest {
        query: query.to_string(),
        kinds: Vec::new(),
        filters: CatalogSearchFilters::default(),
        limit: 10,
        cursor: None,
    }
}

#[test]
fn index_paths_are_hashed_per_tenant_and_workspace_under_catalog_root() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = CatalogIndexPaths::new(tmp.path());
    let first = paths.scope_path(&scope("tenant-a", "workspace-a"));
    let second = paths.scope_path(&scope("tenant-a", "workspace-b"));

    assert_ne!(first, second);
    assert!(first.starts_with(tmp.path().join(".agent-fw/indexes/catalog")));
    assert_eq!(
        first.components().count(),
        tmp.path()
            .join(".agent-fw/indexes/catalog/aa/bb")
            .components()
            .count()
    );
    assert!(!first.to_string_lossy().contains("tenant-a"));
    assert!(!first.to_string_lossy().contains("workspace-a"));
}

#[test]
fn search_isolated_by_scope_even_with_same_entry_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyCatalogIndex::new(tmp.path());
    let scope_a = scope("tenant-a", "workspace-a");
    let scope_b = scope("tenant-b", "workspace-a");

    index
        .rebuild(
            &scope_a,
            vec![project(table_entry(
                "table:shared",
                "fact_sales",
                "Alpha scoped sales facts.",
            ))],
        )
        .unwrap();
    index
        .rebuild(
            &scope_b,
            vec![project(table_entry(
                "table:shared",
                "fact_sales",
                "Beta scoped sales facts.",
            ))],
        )
        .unwrap();

    let alpha = tokio_test::block_on(index.search(&scope_a, request("Alpha"))).unwrap();
    assert_eq!(alpha.hits.len(), 1);
    assert_eq!(alpha.hits[0].entry_id, "table:shared");

    let cross_scope_alpha = tokio_test::block_on(index.search(&scope_b, request("Alpha"))).unwrap();
    assert!(cross_scope_alpha.hits.is_empty());

    let beta = tokio_test::block_on(index.search(&scope_b, request("Beta"))).unwrap();
    assert_eq!(beta.hits.len(), 1);
    assert_eq!(beta.hits[0].entry_id, "table:shared");
}
