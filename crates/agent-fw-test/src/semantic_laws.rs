//! Ontology-lite semantic catalog law test harness.
//!
//! # Laws
//!
//! - L1 (Catalog kind roundtrip): `CatalogKind -> SemanticEntityKind -> CatalogKind`
//!   preserves the original catalog kind.
//! - L2 (Semantic kind roundtrip): `SemanticEntityKind -> CatalogKind -> SemanticEntityKind`
//!   preserves the original semantic kind.
//! - L3 (Valid metadata kind preservation): Converting a valid `CatalogEntry`
//!   into `SemanticEntity` preserves its catalog kind through the semantic kind.
//! - L4 (Entry roundtrip): Converting a valid `CatalogEntry` into `SemanticEntity`
//!   and back with `into_entry` preserves the storage envelope.
//! - L5 (Reference parser determinism): `CatalogRef` table and column parsing is
//!   deterministic and idempotent under surrounding whitespace trim.
//! - L6 (Metadata serde roundtrip): Representative semantic metadata values
//!   preserve explicit values and serde defaults.
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn semantic_model_satisfies_laws() {
//!     agent_fw_test::semantic_laws::test_all();
//! }
//! ```

use agent_fw_catalog::{
    Cardinality, CatalogEntry, CatalogKind, CatalogProvenance, CatalogRef, CatalogRelation,
    ColumnMetadata, DataQualityFindingMetadata, DocumentMetadata, EnumValueMetadata,
    ForeignKeyMetadata, KnowledgeMetadata, MetricMetadata, RelationshipMetadata, SemanticEntity,
    SemanticEntityKind, TableMetadata,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

/// Run all ontology-lite semantic catalog laws.
pub fn test_all() {
    law_catalog_kind_roundtrip();
    law_semantic_kind_roundtrip();
    law_valid_metadata_preserves_kind();
    law_entry_roundtrip_preserves_storage_envelope();
    law_catalog_refs_are_deterministic_and_trim_idempotent();
    law_metadata_serde_roundtrips_preserve_values_and_defaults();
}

/// L1: `CatalogKind -> SemanticEntityKind -> CatalogKind` is identity.
pub fn law_catalog_kind_roundtrip() {
    for kind in all_catalog_kinds() {
        let semantic_kind = SemanticEntityKind::from(kind);
        assert_eq!(
            CatalogKind::from(semantic_kind),
            kind,
            "L1: catalog kind {kind} must roundtrip through SemanticEntityKind",
        );
    }
}

/// L2: `SemanticEntityKind -> CatalogKind -> SemanticEntityKind` is identity.
pub fn law_semantic_kind_roundtrip() {
    for kind in all_semantic_kinds() {
        let catalog_kind = CatalogKind::from(kind);
        assert_eq!(
            SemanticEntityKind::from(catalog_kind),
            kind,
            "L2: semantic kind {kind:?} must roundtrip through CatalogKind",
        );
    }
}

/// L3: valid kind-specific metadata converts to the matching semantic kind.
pub fn law_valid_metadata_preserves_kind() {
    for entry in valid_entries() {
        let expected_kind = entry.kind;
        let entity = SemanticEntity::try_from(entry).unwrap_or_else(|error| {
            panic!("L3: valid metadata for {expected_kind} must convert: {error}")
        });
        assert_eq!(
            CatalogKind::from(entity.kind()),
            expected_kind,
            "L3: SemanticEntity kind must preserve CatalogEntry kind",
        );
    }
}

/// L4: valid semantic conversion preserves the original storage envelope.
pub fn law_entry_roundtrip_preserves_storage_envelope() {
    for entry in valid_entries() {
        let entity = SemanticEntity::try_from(entry.clone())
            .unwrap_or_else(|error| panic!("L4: valid entry must convert: {error}"));
        let roundtripped = entity.into_entry();
        assert_entry_eq(
            &roundtripped,
            &entry,
            "L4: SemanticEntity::into_entry must preserve the original entry",
        );
    }
}

/// L5: reference parsing is deterministic and trim-idempotent.
pub fn law_catalog_refs_are_deterministic_and_trim_idempotent() {
    let samples = [
        "fact_sales",
        "public.fact_sales",
        "warehouse.public.fact_sales",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ];

    for sample in samples {
        let padded = format!("  {sample}  ");
        assert_ref_law("L5 table", CatalogRef::parse_table, sample, &padded);
        assert_ref_law("L5 column", CatalogRef::parse_column, sample, &padded);
    }
}

/// L6: representative semantic metadata serde roundtrips preserve values/defaults.
pub fn law_metadata_serde_roundtrips_preserve_values_and_defaults() {
    assert_serde_roundtrip(
        "L6 table metadata",
        &TableMetadata {
            database_id: "warehouse".into(),
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            relation_type: Some("base_table".into()),
            row_count: Some(100),
            column_count: Some(3),
            preferred_query_surface: true,
            source: CatalogProvenance {
                origin: None,
                profiling_run_id: Some("profile-1".into()),
                enrichment_source: Some("fixture".into()),
                model_id: Some("model-a".into()),
                fallback_reason: None,
                schema_snapshot_at: Some("2026-05-22T00:00:00Z".into()),
                target_fingerprint: Some("fingerprint-a".into()),
            },
        },
    );
    assert_serde_roundtrip(
        "L6 column metadata",
        &ColumnMetadata {
            database_id: "warehouse".into(),
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            column_name: "order_status".into(),
            data_type: "text".into(),
            nullable: false,
            primary_key: true,
            foreign_key: Some(ForeignKeyMetadata {
                referenced_schema: "public".into(),
                referenced_table: "dim_status".into(),
                referenced_column: "status_id".into(),
                constraint_name: Some("fk_status".into()),
            }),
            semantic_type: Some("status".into()),
            distinct_count: Some(4),
            null_count: Some(0),
            total_count: Some(10),
            low_cardinality_enum: true,
        },
    );
    assert_serde_roundtrip(
        "L6 enum value metadata",
        &EnumValueMetadata {
            database_id: "warehouse".into(),
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            column_name: "order_status".into(),
            column_id: "col-order-status".into(),
            value: "paid".into(),
            normalized_value: "paid".into(),
            display_value: Some("Paid".into()),
            frequency: Some(7),
            frequency_percentage: Some(70.0),
            rank: Some(1),
            synonyms: vec!["complete".into()],
        },
    );
    assert_serde_roundtrip(
        "L6 relationship metadata",
        &RelationshipMetadata {
            database_id: "warehouse".into(),
            source_table_id: "table-fact-sales".into(),
            target_table_id: "table-dim-products".into(),
            source_schema: "public".into(),
            source_table: "fact_sales".into(),
            source_column: "product_id".into(),
            target_schema: "public".into(),
            target_table: "dim_products".into(),
            target_column: "product_id".into(),
            source_cardinality: Cardinality::Many,
            target_cardinality: Cardinality::One,
            relationship_kind: "foreign_key".into(),
            confidence: Some(0.99),
            source: CatalogProvenance {
                origin: Some("physical_schema".into()),
                profiling_run_id: Some("profile-1".into()),
                enrichment_source: None,
                model_id: None,
                fallback_reason: None,
                schema_snapshot_at: None,
                target_fingerprint: None,
            },
        },
    );
    assert_serde_roundtrip(
        "L6 metric metadata",
        &MetricMetadata {
            formula: Some("sum(amount)".into()),
            source_tables: vec!["table-fact-sales".into()],
            source_columns: vec!["col-amount".into()],
            synonyms: vec!["revenue".into()],
        },
    );
    assert_serde_roundtrip(
        "L6 knowledge metadata",
        &KnowledgeMetadata {
            knowledge_type: Some("business_rule".into()),
            scope_tables: vec!["table-fact-sales".into()],
            scope_columns: vec!["col-status".into()],
            sql_expression: Some("status = 'paid'".into()),
            synonyms: vec!["paid orders".into()],
            source_knowledge_id: Some("knowledge-paid-orders".into()),
            source_document_id: Some("document-modeling-guide".into()),
        },
    );
    assert_serde_roundtrip(
        "L6 document metadata",
        &DocumentMetadata {
            source_document_id: "document-modeling-guide".into(),
            content_available: true,
            content_source: Some("kv".into()),
            extraction_status: Some("processed".into()),
            extracted_knowledge_ids: vec!["knowledge-paid-orders".into()],
        },
    );
    assert_serde_roundtrip(
        "L6 data quality finding metadata",
        &DataQualityFindingMetadata {
            database_id: "warehouse".into(),
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            column_name: Some("amount".into()),
            finding_type: Some("range_anomaly".into()),
            scope_tables: vec!["public.fact_sales".into()],
            scope_columns: vec!["public.fact_sales.amount".into()],
            source: CatalogProvenance::default(),
            typical_value_range: Some("0..1000".into()),
            validation_rules: vec!["amount >= 0".into()],
        },
    );

    let default_table: TableMetadata = serde_json::from_value(json!({
        "databaseId": "warehouse",
        "schemaName": "public",
        "tableName": "fact_sales"
    }))
    .expect("L6: table defaults must deserialize");
    assert!(!default_table.preferred_query_surface);
    assert_eq!(default_table.source, CatalogProvenance::default());
    assert_serde_roundtrip("L6 defaulted table metadata", &default_table);

    let default_column: ColumnMetadata = serde_json::from_value(json!({
        "databaseId": "warehouse",
        "schemaName": "public",
        "tableName": "fact_sales",
        "columnName": "order_status",
        "dataType": "text",
        "nullable": false,
        "foreignKey": null
    }))
    .expect("L6: column defaults must deserialize");
    assert!(!default_column.primary_key);
    assert!(!default_column.low_cardinality_enum);
    assert_serde_roundtrip("L6 defaulted column metadata", &default_column);

    let default_metric: MetricMetadata =
        serde_json::from_value(json!({})).expect("L6: metric defaults must deserialize");
    assert!(default_metric.source_tables.is_empty());
    assert!(default_metric.source_columns.is_empty());
    assert!(default_metric.synonyms.is_empty());
    assert_serde_roundtrip("L6 defaulted metric metadata", &default_metric);

    let default_knowledge: KnowledgeMetadata =
        serde_json::from_value(json!({})).expect("L6: knowledge defaults must deserialize");
    assert!(default_knowledge.scope_tables.is_empty());
    assert!(default_knowledge.scope_columns.is_empty());
    assert!(default_knowledge.synonyms.is_empty());
    assert!(default_knowledge.source_knowledge_id.is_none());
    assert!(default_knowledge.source_document_id.is_none());
    assert_serde_roundtrip("L6 defaulted knowledge metadata", &default_knowledge);

    let default_document: DocumentMetadata = serde_json::from_value(json!({
        "sourceDocumentId": "document-modeling-guide"
    }))
    .expect("L6: document defaults must deserialize");
    assert!(!default_document.content_available);
    assert!(default_document.content_source.is_none());
    assert!(default_document.extraction_status.is_none());
    assert!(default_document.extracted_knowledge_ids.is_empty());
    assert_serde_roundtrip("L6 defaulted document metadata", &default_document);
}

fn all_catalog_kinds() -> [CatalogKind; 9] {
    [
        CatalogKind::Table,
        CatalogKind::Column,
        CatalogKind::Relationship,
        CatalogKind::Enum,
        CatalogKind::Metric,
        CatalogKind::Special,
        CatalogKind::Document,
        CatalogKind::Knowledge,
        CatalogKind::DataQualityFinding,
    ]
}

fn all_semantic_kinds() -> [SemanticEntityKind; 9] {
    [
        SemanticEntityKind::Table,
        SemanticEntityKind::Column,
        SemanticEntityKind::Relationship,
        SemanticEntityKind::EnumValue,
        SemanticEntityKind::Metric,
        SemanticEntityKind::Special,
        SemanticEntityKind::Document,
        SemanticEntityKind::Knowledge,
        SemanticEntityKind::DataQualityFinding,
    ]
}

fn valid_entries() -> Vec<CatalogEntry> {
    vec![
        entry(
            "table-fact-sales",
            CatalogKind::Table,
            "fact_sales",
            json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "fact_sales",
                "relationType": "base_table",
                "rowCount": 100,
                "columnCount": 3,
                "preferredQuerySurface": true,
                "source": {
                    "profilingRunId": "profile-1",
                    "enrichmentSource": "fixture"
                }
            }),
        ),
        entry(
            "col-order-status",
            CatalogKind::Column,
            "order_status",
            json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "fact_sales",
                "columnName": "order_status",
                "dataType": "text",
                "nullable": false,
                "primaryKey": false,
                "foreignKey": null,
                "semanticType": "status",
                "distinctCount": 4,
                "nullCount": 0,
                "totalCount": 10,
                "lowCardinalityEnum": true
            }),
        ),
        entry(
            "rel-sales-products",
            CatalogKind::Relationship,
            "fact_sales_product_id_to_dim_products_product_id",
            json!({
                "databaseId": "warehouse",
                "sourceTableId": "table-fact-sales",
                "targetTableId": "table-dim-products",
                "sourceSchema": "public",
                "sourceTable": "fact_sales",
                "sourceColumn": "product_id",
                "targetSchema": "public",
                "targetTable": "dim_products",
                "targetColumn": "product_id",
                "sourceCardinality": "many",
                "targetCardinality": "one",
                "relationshipKind": "foreign_key",
                "confidence": 0.99
            }),
        ),
        entry(
            "enum-order-status-paid",
            CatalogKind::Enum,
            "paid",
            json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "fact_sales",
                "columnName": "order_status",
                "columnId": "col-order-status",
                "value": "paid",
                "normalizedValue": "paid",
                "displayValue": "Paid",
                "frequency": 7,
                "frequencyPercentage": 70.0,
                "rank": 1,
                "synonyms": ["complete"]
            }),
        ),
        entry(
            "metric-revenue",
            CatalogKind::Metric,
            "revenue",
            json!({
                "formula": "sum(amount)",
                "sourceTables": ["table-fact-sales"],
                "sourceColumns": ["col-amount"],
                "synonyms": ["sales"]
            }),
        ),
        entry(
            "special-topology",
            CatalogKind::Special,
            "topology",
            json!({}),
        ),
        entry(
            "document-modeling-guide",
            CatalogKind::Document,
            "modeling guide",
            json!({
                "sourceDocumentId": "document-modeling-guide",
                "contentAvailable": true,
                "contentSource": "kv",
                "extractionStatus": "processed",
                "extractedKnowledgeIds": ["knowledge-paid-orders"]
            }),
        ),
        entry(
            "knowledge-paid-orders",
            CatalogKind::Knowledge,
            "paid orders",
            json!({
                "knowledgeType": "business_rule",
                "scopeTables": ["table-fact-sales"],
                "scopeColumns": ["col-order-status"],
                "sqlExpression": "order_status = 'paid'",
                "synonyms": ["complete orders"],
                "sourceKnowledgeId": "knowledge-paid-orders",
                "sourceDocumentId": "document-modeling-guide"
            }),
        ),
        entry(
            "dqf-amount-range",
            CatalogKind::DataQualityFinding,
            "amount range",
            json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "fact_sales",
                "columnName": "amount",
                "findingType": "range_anomaly",
                "scopeTables": ["public.fact_sales"],
                "scopeColumns": ["public.fact_sales.amount"],
                "typicalValueRange": "0..1000",
                "validationRules": ["amount >= 0"]
            }),
        ),
    ]
}

fn entry(id: &str, kind: CatalogKind, name: &str, metadata: serde_json::Value) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        kind,
        name: name.into(),
        qualified_name: Some(format!("public.{name}")),
        content: format!("{kind} contract fixture"),
        tags: vec!["semantic-law".into()],
        links: vec![CatalogRelation {
            target_id: "related-entry".into(),
            kind: "related_to".into(),
            description: Some("law fixture relation".into()),
        }],
        metadata,
    }
}

fn assert_entry_eq(actual: &CatalogEntry, expected: &CatalogEntry, message: &str) {
    assert_eq!(
        serde_json::to_value(actual).expect("actual entry must serialize"),
        serde_json::to_value(expected).expect("expected entry must serialize"),
        "{message}",
    );
}

fn assert_ref_law(label: &str, parse: fn(&str) -> CatalogRef, sample: &str, padded: &str) {
    let first = parse(sample);
    let second = parse(sample);
    assert_eq!(first, second, "{label}: parse must be deterministic");
    assert_eq!(
        parse(padded),
        parse(padded.trim()),
        "{label}: parse must be idempotent under trim",
    );
}

fn assert_serde_roundtrip<T>(label: &str, value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_value(value).unwrap_or_else(|error| {
        panic!("{label}: metadata value must serialize: {error}");
    });
    let decoded: T = serde_json::from_value(json).unwrap_or_else(|error| {
        panic!("{label}: metadata value must deserialize: {error}");
    });
    assert_eq!(
        &decoded, value,
        "{label}: serde roundtrip must preserve value"
    );
}
