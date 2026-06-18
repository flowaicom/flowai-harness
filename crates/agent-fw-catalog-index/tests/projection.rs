use std::convert::TryFrom;

use agent_fw_catalog::{
    CatalogEntry, CatalogKind, CatalogRelation, SemanticEntity, SemanticEntityKind,
};
use agent_fw_catalog_index::{CatalogDocumentProjection, PROJECTED_CATALOG_SCHEMA_VERSION};
use serde_json::json;

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

#[test]
fn projection_uses_public_kind_names_and_typed_synonyms_only() {
    let enum_entry = entry(
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
            "synonyms": ["grocery", "supermarket channel", "grocery"]
        }),
    );
    let enum_entity = SemanticEntity::try_from(enum_entry).unwrap();

    let projection = CatalogDocumentProjection::project(&enum_entity, None).unwrap();

    assert_eq!(projection.entry_id, "enum:channel.supermarket");
    assert_eq!(projection.kind, SemanticEntityKind::EnumValue);
    assert_eq!(projection.kind_name, "enum_value");
    assert_eq!(projection.synonyms, vec!["grocery", "supermarket channel"]);
    assert_eq!(projection.enum_value.as_deref(), Some("Supermarket"));
    assert_eq!(
        projection.enum_display_value.as_deref(),
        Some("Supermarket")
    );
    assert_eq!(
        projection.projection_version,
        PROJECTED_CATALOG_SCHEMA_VERSION
    );

    let table_entry = entry(
        "table:fact_sales",
        CatalogKind::Table,
        "fact_sales",
        Some("public.fact_sales"),
        "Sales fact table.",
        vec![],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true,
            "source": {
                "schemaSnapshotAt": "2026-05-30T12:00:00Z"
            },
            "synonyms": ["raw table alias"]
        }),
    );
    let table_entity = SemanticEntity::try_from(table_entry).unwrap();

    let table_projection = CatalogDocumentProjection::project(&table_entity, None).unwrap();

    assert!(table_projection.synonyms.is_empty());
    assert_eq!(
        table_projection.relation_type.as_deref(),
        Some("base_table")
    );
    assert_eq!(
        table_projection.updated_at.as_deref(),
        Some("2026-05-30T12:00:00Z")
    );
    assert!(!table_projection.synonyms_text().contains("raw table alias"));
}

#[test]
fn projection_maps_kind_specific_filter_fields() {
    let column_entry = entry(
        "column:fact_sales.units",
        CatalogKind::Column,
        "units",
        Some("public.fact_sales.units"),
        "Units sold.",
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
    );
    let column_entity = SemanticEntity::try_from(column_entry).unwrap();

    let projection = CatalogDocumentProjection::project(&column_entity, None).unwrap();

    assert_eq!(projection.database_id.as_deref(), Some("warehouse"));
    assert_eq!(projection.schema_name.as_deref(), Some("public"));
    assert_eq!(projection.table_name.as_deref(), Some("fact_sales"));
    assert_eq!(projection.column_name.as_deref(), Some("units"));
    assert_eq!(projection.data_type.as_deref(), Some("numeric"));
    assert_eq!(projection.semantic_type.as_deref(), Some("unit_sales"));
    assert_eq!(projection.nullable, Some(false));
    assert_eq!(projection.primary_key, Some(false));
    assert_eq!(projection.low_cardinality_enum, Some(false));
    assert!(projection.context_text().contains("fact_sales"));
}

#[test]
fn projection_maps_documented_kind_specific_search_fields() {
    let metric_entry = entry(
        "metric:revenue",
        CatalogKind::Metric,
        "total revenue",
        None,
        "Revenue metric.",
        vec!["finance"],
        json!({
            "formula": "SUM(net_amount)",
            "sourceTables": ["fact_sales"],
            "sourceColumns": ["net_amount"],
            "synonyms": ["sales value"]
        }),
    );
    let metric_entity = SemanticEntity::try_from(metric_entry).unwrap();
    let metric_projection = CatalogDocumentProjection::project(&metric_entity, None).unwrap();
    assert_eq!(
        metric_projection.formula_text.as_deref(),
        Some("SUM(net_amount)")
    );
    assert!(!metric_projection.context_text().contains("SUM(net_amount)"));

    let knowledge_entry = entry(
        "knowledge:slow-movers",
        CatalogKind::Knowledge,
        "slow mover rule",
        None,
        "Slow mover business rule.",
        vec!["ops"],
        json!({
            "knowledgeType": "sql_rule",
            "scopeTables": ["fact_sales"],
            "scopeColumns": ["units"],
            "sqlExpression": "units < 10",
            "synonyms": ["low velocity"]
        }),
    );
    let knowledge_entity = SemanticEntity::try_from(knowledge_entry).unwrap();
    let knowledge_projection = CatalogDocumentProjection::project(&knowledge_entity, None).unwrap();
    assert_eq!(
        knowledge_projection.sql_expression_text.as_deref(),
        Some("units < 10")
    );
    assert!(!knowledge_projection.context_text().contains("units < 10"));

    let finding_entry = entry(
        "dq:negative-net-amount",
        CatalogKind::DataQualityFinding,
        "negative net amount",
        Some("public.fact_sales.net_amount.negative"),
        "Net amount should be non-negative.",
        vec!["quality"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "net_amount",
            "findingType": "range",
            "typicalValueRange": ">= 0",
            "validationRules": ["net_amount >= 0", "net_amount IS NOT NULL"]
        }),
    );
    let finding_entity = SemanticEntity::try_from(finding_entry).unwrap();
    let finding_projection = CatalogDocumentProjection::project(&finding_entity, None).unwrap();
    assert_eq!(
        finding_projection.validation_rules,
        vec![
            "net_amount >= 0".to_string(),
            "net_amount IS NOT NULL".to_string()
        ]
    );
}

#[hegel::test]
fn table_projection_laws(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let table_name = format!("table_{suffix}");
    let table_entry = entry(
        &format!("table:{table_name}"),
        CatalogKind::Table,
        &table_name,
        Some(&format!("public.{table_name}")),
        "Table content.",
        vec!["warehouse", "warehouse"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table_name,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true,
            "synonyms": ["metadata synonym must not project"]
        }),
    );
    let entity = SemanticEntity::try_from(table_entry).unwrap();

    let projection =
        CatalogDocumentProjection::project(&entity, Some(" body ".to_string())).unwrap();

    assert_eq!(projection.kind, SemanticEntityKind::Table);
    assert_eq!(projection.kind_name, "table");
    assert!(projection.synonyms.is_empty());
    assert!(projection.source_table_filter_values.is_empty());
    assert!(projection.source_column_filter_values.is_empty());
    assert!(projection.target_table_filter_values.is_empty());
    assert!(projection.target_column_filter_values.is_empty());
    assert_filter_values_are_canonical(&projection.table_filter_values);
    assert_filter_values_are_canonical(&projection.context_parts);
}

#[hegel::test]
fn enum_projection_laws(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let value = format!("value_{suffix}");
    let enum_entry = entry(
        &format!("enum:{value}"),
        CatalogKind::Enum,
        &value,
        Some(&format!("public.dim_{suffix}.name.{value}")),
        "Enum content.",
        vec!["enum"],
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": format!("dim_{suffix}"),
            "columnName": "name",
            "columnId": format!("column:dim_{suffix}.name"),
            "value": value,
            "normalizedValue": format!("value_{suffix}"),
            "displayValue": format!("Value {suffix}"),
            "frequency": 1,
            "frequencyPercentage": 25.0,
            "rank": 1,
            "synonyms": ["typed synonym", "typed synonym"]
        }),
    );
    let entity = SemanticEntity::try_from(enum_entry).unwrap();

    let projection = CatalogDocumentProjection::project(&entity, None).unwrap();

    assert_eq!(projection.kind, SemanticEntityKind::EnumValue);
    assert_eq!(projection.low_cardinality_enum, Some(true));
    assert_eq!(projection.synonyms, vec!["typed synonym"]);
    assert!(projection.source_table_filter_values.is_empty());
    assert!(projection.target_table_filter_values.is_empty());
    assert_filter_values_are_canonical(&projection.table_filter_values);
    assert_filter_values_are_canonical(&projection.column_filter_values);
}

#[hegel::test]
fn relationship_projection_preserves_direction_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let source_table = format!("source_table_{suffix}");
    let target_table = format!("target_table_{suffix}");
    let source_column = format!("source_column_{suffix}");
    let target_column = format!("target_column_{suffix}");
    let relationship_entry = entry(
        &format!("relationship:{source_table}->{target_table}"),
        CatalogKind::Relationship,
        "source_to_target",
        None,
        "Relationship content.",
        vec!["relationship"],
        json!({
            "databaseId": "warehouse",
            "sourceTableId": format!("table:{source_table}"),
            "targetTableId": format!("table:{target_table}"),
            "sourceSchema": "public",
            "sourceTable": source_table,
            "sourceColumn": source_column,
            "targetSchema": "analytics",
            "targetTable": target_table,
            "targetColumn": target_column,
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0
        }),
    );
    let entity = SemanticEntity::try_from(relationship_entry).unwrap();

    let projection = CatalogDocumentProjection::project(&entity, None).unwrap();

    assert_eq!(projection.kind, SemanticEntityKind::Relationship);
    assert_eq!(projection.source_cardinality.as_deref(), Some("many"));
    assert_eq!(projection.target_cardinality.as_deref(), Some("one"));
    assert_eq!(projection.confidence.as_deref(), Some("1"));
    assert!(projection
        .source_table_filter_values
        .contains(&format!("source_table_{suffix}")));
    assert!(projection
        .source_table_filter_values
        .contains(&format!("public.source_table_{suffix}")));
    assert!(projection
        .target_table_filter_values
        .contains(&format!("target_table_{suffix}")));
    assert!(projection
        .target_table_filter_values
        .contains(&format!("analytics.target_table_{suffix}")));
    assert!(!projection
        .source_table_filter_values
        .contains(&format!("target_table_{suffix}")));
    assert!(!projection
        .target_table_filter_values
        .contains(&format!("source_table_{suffix}")));
    assert_filter_values_are_canonical(&projection.source_table_filter_values);
    assert_filter_values_are_canonical(&projection.source_column_filter_values);
    assert_filter_values_are_canonical(&projection.target_table_filter_values);
    assert_filter_values_are_canonical(&projection.target_column_filter_values);
}

#[hegel::test]
fn special_projection_is_rejected_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let special_entry = entry(
        &format!("special:{suffix}"),
        CatalogKind::Special,
        &format!("special_{suffix}"),
        None,
        "Special entry.",
        vec!["special"],
        json!({}),
    );
    let entity = SemanticEntity::try_from(special_entry).unwrap();

    let error = CatalogDocumentProjection::project(&entity, None).unwrap_err();

    assert!(error.to_string().contains("special"));
}

fn draw_suffix(tc: &hegel::TestCase) -> u16 {
    tc.draw(hegel::generators::integers::<u16>())
}

fn assert_filter_values_are_canonical(values: &[String]) {
    let mut seen = std::collections::HashSet::new();
    for value in values {
        assert_eq!(value, value.trim(), "filter values must be trimmed");
        assert!(!value.is_empty(), "filter values must not be empty");
        assert!(
            seen.insert(value.to_lowercase()),
            "filter values must be case-insensitively deduped: {values:?}"
        );
    }
}
