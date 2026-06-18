//! Shared fixture for the storage-agnostic semantic catalog behavior suites.
//!
//! Used by `semantic_catalog_behavior.rs` (SQLite, in-memory) and
//! `semantic_catalog_behavior_postgres.rs` (Postgres). Keeping the entries here
//! is the single source of truth that keeps the two backends from drifting.

#![allow(dead_code)]

use agent_fw_catalog::{relation_kind, CatalogEntry, CatalogKind, CatalogRelation};
use serde_json::json;

pub const DATABASE_ID: &str = "fixture_business";
pub const SCHEMA: &str = "public";

/// The behavior entries layered on top of `business_catalog::entries()`.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        enum_entry(
            "fact_sales",
            "order_status",
            "awaiting_payment",
            &["open invoice", "payment pending"],
            5,
        ),
        metric_entry(),
        knowledge_entry(),
        table_entry("audit_log"),
    ]
}

pub fn table_entry(table: &str) -> CatalogEntry {
    CatalogEntry {
        id: table_id(table),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(format!("{SCHEMA}.{table}")),
        content: "Operational audit records with no warehouse joins.".to_string(),
        tags: vec!["semantic_behavior_fixture".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": SCHEMA,
            "tableName": table,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 0,
            "preferredQuerySurface": false,
            "source": { "system": "semantic_behavior_fixture" },
        }),
    }
}

pub fn enum_entry(
    table: &str,
    column: &str,
    value: &str,
    synonyms: &[&str],
    rank: usize,
) -> CatalogEntry {
    CatalogEntry {
        id: enum_id(table, column, value),
        kind: CatalogKind::Enum,
        name: value.to_string(),
        qualified_name: Some(format!("{SCHEMA}.{table}.{column}.{value}")),
        content: format!("{value} value for {table}.{column}."),
        tags: vec!["semantic_behavior_fixture".to_string()],
        links: vec![relation(
            column_id(table, column),
            relation_kind::ENUM_VALUE_OF,
            "enum value column",
        )],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": SCHEMA,
            "tableName": table,
            "columnName": column,
            "columnId": column_id(table, column),
            "value": value,
            "normalizedValue": value,
            "displayValue": value.replace('_', " "),
            "rank": rank,
            "synonyms": synonyms,
        }),
    }
}

pub fn metric_entry() -> CatalogEntry {
    CatalogEntry {
        id: "metric:total_revenue".to_string(),
        kind: CatalogKind::Metric,
        name: "total_revenue".to_string(),
        qualified_name: None,
        content: "Total revenue calculation for confirmed transactions.".to_string(),
        tags: vec!["[TYPE:metric]".to_string(), "[DOMAIN:finance]".to_string()],
        links: vec![relation(
            table_id("fact_sales"),
            relation_kind::METRIC_USES,
            "metric source table",
        )],
        metadata: json!({
            "formula": "SUM(net_amount)",
            "sourceTables": ["public.fact_sales"],
            "sourceColumns": ["public.fact_sales.net_amount"],
            "synonyms": ["sales", "income", "net sales"],
        }),
    }
}

pub fn knowledge_entry() -> CatalogEntry {
    CatalogEntry {
        id: "knowledge:merchant_channel_reporting".to_string(),
        kind: CatalogKind::Knowledge,
        name: "merchant_channel_reporting".to_string(),
        qualified_name: None,
        content: "Channel reporting guidance for the preferred sales view.".to_string(),
        tags: vec!["[TYPE:knowledge]".to_string()],
        links: vec![relation(
            table_id("v_sales_enriched"),
            relation_kind::KNOWLEDGE_APPLIES_TO,
            "knowledge applies to preferred view",
        )],
        metadata: json!({
            "knowledgeType": "BusinessRule",
            "scopeTables": ["public.v_sales_enriched"],
            "scopeColumns": ["public.v_sales_enriched.channel_name"],
            "sqlExpression": null,
            "synonyms": ["merchant", "retailer", "seller"],
        }),
    }
}

pub fn relationship_entry(
    source_table: &str,
    target_table: &str,
    source_column: &str,
    target_column: &str,
) -> CatalogEntry {
    CatalogEntry {
        id: format!(
            "relationship:{}.{source_column}->{}.{target_column}",
            qualified_table(source_table),
            qualified_table(target_table)
        ),
        kind: CatalogKind::Relationship,
        name: format!("{source_table}_to_{target_table}"),
        qualified_name: None,
        content: format!(
            "{source_table}.{source_column} references {target_table}.{target_column}"
        ),
        tags: vec!["relationship".to_string(), "foreign_key".to_string()],
        links: vec![
            relation(
                table_id(source_table),
                relation_kind::RELATIONSHIP_SOURCE_TABLE,
                "relationship source table",
            ),
            relation(
                table_id(target_table),
                relation_kind::RELATIONSHIP_TARGET_TABLE,
                "relationship target table",
            ),
        ],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "sourceTableId": table_id(source_table),
            "targetTableId": table_id(target_table),
            "sourceSchema": SCHEMA,
            "sourceTable": source_table,
            "sourceColumn": source_column,
            "targetSchema": SCHEMA,
            "targetTable": target_table,
            "targetColumn": target_column,
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0,
        }),
    }
}

pub fn relation(target_id: String, kind: &str, description: &str) -> CatalogRelation {
    CatalogRelation {
        target_id,
        kind: kind.to_string(),
        description: Some(description.to_string()),
    }
}

pub fn table_id(table: &str) -> String {
    format!("table:{}", qualified_table(table))
}

pub fn column_id(table: &str, column: &str) -> String {
    format!("column:{}.{}", qualified_table(table), column)
}

pub fn enum_id(table: &str, column: &str, value: &str) -> String {
    format!("enum:{}.{}.{}", qualified_table(table), column, value)
}

pub fn qualified_table(table: &str) -> String {
    format!("{SCHEMA}.{table}")
}
