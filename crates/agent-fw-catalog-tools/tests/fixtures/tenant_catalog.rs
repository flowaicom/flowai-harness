use agent_fw_catalog::{CatalogEntry, CatalogKind, CatalogRelation, CatalogScope, CatalogWriter};
use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_interpreter::{ScopedSqliteCatalog, SqliteCatalog};
use serde_json::json;

const DATABASE_ID: &str = "tenant-fixture";

pub fn scope_a() -> CatalogScope {
    CatalogScope::new(
        TenantId::new_unchecked("tenant-a"),
        WorkspaceId::new_unchecked("workspace-a"),
    )
}

pub fn scope_b() -> CatalogScope {
    CatalogScope::new(
        TenantId::new_unchecked("tenant-b"),
        WorkspaceId::new_unchecked("workspace-b"),
    )
}

pub fn entries_a() -> Vec<CatalogEntry> {
    // Scope is intentionally absent from CatalogEntry metadata/tags. These
    // colliding entries only become representable once catalog storage has
    // first-class tenant_id/workspace_id columns and scoped primary keys.
    scoped_entries("confirmed", "Scope A confirmed sales scope")
}

pub fn entries_b() -> Vec<CatalogEntry> {
    scoped_entries("private_b_only", "Scope B private_b_only sales scope")
}

#[allow(dead_code)]
pub fn entries() -> Vec<CatalogEntry> {
    let mut entries = entries_a();
    entries.extend(entries_b());
    entries
}

#[allow(dead_code)]
pub async fn sqlite_catalog_with_two_scopes() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory().unwrap();
    catalog
        .with_scope(scope_a())
        .save_in_transaction(entries_a())
        .await
        .unwrap();
    catalog
        .with_scope(scope_b())
        .save_in_transaction(entries_b())
        .await
        .unwrap();
    catalog.with_scope(scope_a())
}

#[allow(dead_code)]
pub async fn sqlite_catalog_with_two_scopes_b() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory().unwrap();
    catalog
        .with_scope(scope_a())
        .save_in_transaction(entries_a())
        .await
        .unwrap();
    catalog
        .with_scope(scope_b())
        .save_in_transaction(entries_b())
        .await
        .unwrap();
    catalog.with_scope(scope_b())
}

fn scoped_entries(status_value: &str, marker: &str) -> Vec<CatalogEntry> {
    vec![
        fact_sales_table(marker),
        order_status_column(marker),
        dim_products_table(marker),
        status_enum(status_value, marker),
    ]
}

fn fact_sales_table(marker: &str) -> CatalogEntry {
    CatalogEntry {
        id: table_id("fact_sales"),
        kind: CatalogKind::Table,
        name: "fact_sales".to_string(),
        qualified_name: Some("public.fact_sales".to_string()),
        content: format!("{marker}: sales fact table"),
        tags: tags(),
        links: vec![
            relation(column_id("fact_sales", "order_status"), "has_column"),
            relation(table_id("dim_products"), "references_table"),
        ],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": "public",
            "tableName": "fact_sales",
            "relationType": "table",
            "rowCount": null,
            "columnCount": 1,
            "preferredQuerySurface": false,
            "source": {},
            "marker": marker,
        }),
    }
}

fn order_status_column(marker: &str) -> CatalogEntry {
    CatalogEntry {
        id: column_id("fact_sales", "order_status"),
        kind: CatalogKind::Column,
        name: "order_status".to_string(),
        qualified_name: Some("public.fact_sales.order_status".to_string()),
        content: format!("{marker}: order status column"),
        tags: tags(),
        links: vec![relation(table_id("fact_sales"), "belongs_to")],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "order_status",
            "dataType": "text",
            "nullable": false,
            "primaryKey": false,
            "foreignKey": null,
            "isCategorical": true,
            "lowCardinalityEnum": true,
            "marker": marker,
        }),
    }
}

fn dim_products_table(marker: &str) -> CatalogEntry {
    CatalogEntry {
        id: table_id("dim_products"),
        kind: CatalogKind::Table,
        name: "dim_products".to_string(),
        qualified_name: Some("public.dim_products".to_string()),
        content: format!("{marker}: product dimension"),
        tags: tags(),
        links: vec![],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": "public",
            "tableName": "dim_products",
            "relationType": "table",
            "rowCount": null,
            "columnCount": 0,
            "preferredQuerySurface": false,
            "source": {},
            "marker": marker,
        }),
    }
}

fn status_enum(value: &str, marker: &str) -> CatalogEntry {
    CatalogEntry {
        id: enum_id("fact_sales", "order_status"),
        kind: CatalogKind::Enum,
        name: value.to_string(),
        qualified_name: Some(format!("public.fact_sales.order_status.{value}")),
        content: format!("{marker}: enum value {value}"),
        tags: tags(),
        links: vec![relation(
            column_id("fact_sales", "order_status"),
            "enum_value_of",
        )],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "order_status",
            "columnId": column_id("fact_sales", "order_status"),
            "value": value,
            "normalizedValue": value,
            "displayValue": value,
            "frequency": null,
            "frequencyPercentage": null,
            "rank": 1,
            "synonyms": [],
            "marker": marker,
        }),
    }
}

fn tags() -> Vec<String> {
    vec!["tenant_fixture".to_string()]
}

fn relation(target_id: String, kind: &str) -> CatalogRelation {
    CatalogRelation {
        target_id,
        kind: kind.to_string(),
        description: Some("tenant fixture relation".to_string()),
    }
}

fn table_id(table: &str) -> String {
    format!("table:public.{table}")
}

fn column_id(table: &str, column: &str) -> String {
    format!("column:public.{table}.{column}")
}

fn enum_id(table: &str, column: &str) -> String {
    format!("enum:public.{table}.{column}.status_value")
}
