use std::convert::TryFrom;

use serde_json::json;

use super::{relation_kind, Cardinality, SemanticEntity, SemanticEntityKind};
use crate::{CatalogEntry, CatalogKind, CatalogRelation, RelationshipMetadata};

#[test]
fn semantic_entity_kind_mappings_cover_all_builtin_catalog_kinds() {
    let mappings = [
        (CatalogKind::Table, SemanticEntityKind::Table),
        (CatalogKind::Column, SemanticEntityKind::Column),
        (CatalogKind::Relationship, SemanticEntityKind::Relationship),
        (CatalogKind::Enum, SemanticEntityKind::EnumValue),
        (CatalogKind::Metric, SemanticEntityKind::Metric),
        (CatalogKind::Special, SemanticEntityKind::Special),
        (CatalogKind::Document, SemanticEntityKind::Document),
        (CatalogKind::Knowledge, SemanticEntityKind::Knowledge),
        (
            CatalogKind::DataQualityFinding,
            SemanticEntityKind::DataQualityFinding,
        ),
    ];

    for (catalog_kind, semantic_kind) in mappings {
        assert_eq!(SemanticEntityKind::from(catalog_kind), semantic_kind);
        assert_eq!(CatalogKind::from(semantic_kind), catalog_kind);
    }
}

#[test]
fn semantic_entities_decode_all_builtin_entity_metadata_without_legacy_names() {
    let entries = vec![
        table_entry(),
        column_entry(),
        enum_value_entry(),
        relationship_entry(),
        metric_entry(),
        document_entry(),
        knowledge_entry(),
        data_quality_entry(),
        special_entry(),
    ];

    for entry in entries {
        assert!(
            !entry.id.to_ascii_lowercase().contains("ukf"),
            "semantic catalog IDs must not use legacy naming: {}",
            entry.id
        );
        assert!(
            !entry
                .metadata
                .to_string()
                .to_ascii_lowercase()
                .contains("ukf"),
            "semantic metadata must not use legacy naming: {}",
            entry.id
        );

        let expected_kind = SemanticEntityKind::from(entry.kind);
        let entity = SemanticEntity::try_from(entry.clone()).unwrap();
        assert_eq!(entity.kind(), expected_kind);
        assert_eq!(CatalogKind::from(entity.kind()), entry.kind);
        assert_eq!(entity.entry().id, entry.id);
        assert_eq!(entity.into_entry().id, entry.id);
    }
}

#[test]
fn semantic_relationship_mapping_preserves_direction_and_cardinality() {
    let entry = relationship_entry();
    let entity = SemanticEntity::try_from(entry.clone()).unwrap();

    let SemanticEntity::Relationship { metadata, .. } = entity else {
        panic!("expected relationship entity");
    };

    assert_eq!(metadata.source_table, "fact_sales");
    assert_eq!(metadata.source_column, "product_id");
    assert_eq!(metadata.target_table, "dim_products");
    assert_eq!(metadata.target_column, "product_id");
    assert_eq!(metadata.source_cardinality, Cardinality::Many);
    assert_eq!(metadata.target_cardinality, Cardinality::One);

    let raw: RelationshipMetadata = serde_json::from_value(entry.metadata).unwrap();
    assert_eq!(raw.relationship_kind, "foreign_key");
}

fn table_entry() -> CatalogEntry {
    CatalogEntry {
        id: "table:public.fact_sales".to_string(),
        kind: CatalogKind::Table,
        name: "fact_sales".to_string(),
        qualified_name: Some("public.fact_sales".to_string()),
        content: "Sales facts.".to_string(),
        tags: vec!["[TYPE:table]".to_string()],
        links: vec![CatalogRelation {
            target_id: "column:public.fact_sales.product_id".to_string(),
            kind: relation_kind::HAS_COLUMN.to_string(),
            description: Some("product_id column".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "relationType": "base_table",
            "rowCount": 100,
            "columnCount": 1,
            "preferredQuerySurface": false,
            "source": {
                "enrichmentSource": "semantic_behavior_test"
            }
        }),
    }
}

fn column_entry() -> CatalogEntry {
    CatalogEntry {
        id: "column:public.fact_sales.product_id".to_string(),
        kind: CatalogKind::Column,
        name: "product_id".to_string(),
        qualified_name: Some("public.fact_sales.product_id".to_string()),
        content: "Product identifier.".to_string(),
        tags: vec!["[TYPE:column]".to_string()],
        links: vec![CatalogRelation {
            target_id: "table:public.fact_sales".to_string(),
            kind: relation_kind::BELONGS_TO.to_string(),
            description: Some("parent table".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "product_id",
            "dataType": "uuid",
            "nullable": false,
            "primaryKey": false,
            "foreignKey": {
                "referencedSchema": "public",
                "referencedTable": "dim_products",
                "referencedColumn": "product_id",
                "constraintName": "fk_sales_product"
            },
            "semanticType": "identifier",
            "distinctCount": 10,
            "nullCount": 0,
            "totalCount": 100,
            "lowCardinalityEnum": false
        }),
    }
}

fn enum_value_entry() -> CatalogEntry {
    CatalogEntry {
        id: "enum:public.fact_sales.order_status.confirmed".to_string(),
        kind: CatalogKind::Enum,
        name: "confirmed".to_string(),
        qualified_name: Some("public.fact_sales.order_status.confirmed".to_string()),
        content: "Confirmed order status.".to_string(),
        tags: vec!["[TYPE:enum]".to_string()],
        links: vec![CatalogRelation {
            target_id: "column:public.fact_sales.order_status".to_string(),
            kind: relation_kind::ENUM_VALUE_OF.to_string(),
            description: Some("enum value column".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "order_status",
            "columnId": "column:public.fact_sales.order_status",
            "value": "confirmed",
            "normalizedValue": "confirmed",
            "displayValue": "Confirmed",
            "frequency": 70,
            "frequencyPercentage": 70.0,
            "rank": 1,
            "synonyms": ["accepted", "booked"]
        }),
    }
}

fn relationship_entry() -> CatalogEntry {
    CatalogEntry {
        id: "relationship:public.fact_sales.product_id->public.dim_products.product_id".to_string(),
        kind: CatalogKind::Relationship,
        name: "fact_sales_to_dim_products".to_string(),
        qualified_name: None,
        content: "fact_sales.product_id references dim_products.product_id".to_string(),
        tags: vec!["[TYPE:relationship]".to_string()],
        links: vec![
            CatalogRelation {
                target_id: "table:public.fact_sales".to_string(),
                kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                description: Some("source table".to_string()),
            },
            CatalogRelation {
                target_id: "table:public.dim_products".to_string(),
                kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                description: Some("target table".to_string()),
            },
        ],
        metadata: json!({
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
    }
}

fn metric_entry() -> CatalogEntry {
    CatalogEntry {
        id: "metric:total_revenue".to_string(),
        kind: CatalogKind::Metric,
        name: "total_revenue".to_string(),
        qualified_name: None,
        content: "Total revenue calculation.".to_string(),
        tags: vec!["[TYPE:metric]".to_string()],
        links: vec![CatalogRelation {
            target_id: "table:public.fact_sales".to_string(),
            kind: relation_kind::METRIC_USES.to_string(),
            description: Some("metric source table".to_string()),
        }],
        metadata: json!({
            "formula": "SUM(net_amount)",
            "sourceTables": ["public.fact_sales"],
            "sourceColumns": ["public.fact_sales.net_amount"],
            "synonyms": ["sales", "income"]
        }),
    }
}

fn document_entry() -> CatalogEntry {
    CatalogEntry {
        id: "document:reporting-guide".to_string(),
        kind: CatalogKind::Document,
        name: "reporting-guide".to_string(),
        qualified_name: None,
        content: "Reporting guide.".to_string(),
        tags: vec!["[TYPE:document]".to_string()],
        links: vec![],
        metadata: json!({
            "sourceDocumentId": "reporting-guide",
            "contentAvailable": true,
            "contentSource": "kv",
            "extractionStatus": "Completed",
            "extractedKnowledgeIds": ["knowledge:merchant_reporting"]
        }),
    }
}

fn knowledge_entry() -> CatalogEntry {
    CatalogEntry {
        id: "knowledge:merchant_reporting".to_string(),
        kind: CatalogKind::Knowledge,
        name: "merchant_reporting".to_string(),
        qualified_name: None,
        content: "Merchant reporting guidance.".to_string(),
        tags: vec!["[TYPE:knowledge]".to_string()],
        links: vec![CatalogRelation {
            target_id: "table:public.v_sales_enriched".to_string(),
            kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
            description: Some("applies to reporting view".to_string()),
        }],
        metadata: json!({
            "knowledgeType": "BusinessRule",
            "scopeTables": ["public.v_sales_enriched"],
            "scopeColumns": ["public.v_sales_enriched.channel_name"],
            "sqlExpression": null,
            "synonyms": ["merchant", "retailer"],
            "sourceKnowledgeId": "knowledge-item-1",
            "sourceDocumentId": "reporting-guide"
        }),
    }
}

fn data_quality_entry() -> CatalogEntry {
    CatalogEntry {
        id: "dqf:public.fact_sales.net_amount.range".to_string(),
        kind: CatalogKind::DataQualityFinding,
        name: "net_amount_range".to_string(),
        qualified_name: Some("public.fact_sales.net_amount.range".to_string()),
        content: "Net amount should be non-negative.".to_string(),
        tags: vec!["[TYPE:data_quality_finding]".to_string()],
        links: vec![CatalogRelation {
            target_id: "column:public.fact_sales.net_amount".to_string(),
            kind: relation_kind::DATA_QUALITY_FINDING_APPLIES_TO.to_string(),
            description: Some("finding scope column".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "net_amount",
            "findingType": "range_anomaly",
            "scopeTables": ["public.fact_sales"],
            "scopeColumns": ["public.fact_sales.net_amount"],
            "typicalValueRange": "0..1000000",
            "validationRules": ["net_amount >= 0"]
        }),
    }
}

fn special_entry() -> CatalogEntry {
    CatalogEntry {
        id: "special:warehouse-note".to_string(),
        kind: CatalogKind::Special,
        name: "warehouse-note".to_string(),
        qualified_name: None,
        content: "Special catalog note.".to_string(),
        tags: vec!["[TYPE:special]".to_string()],
        links: vec![],
        metadata: json!({}),
    }
}
