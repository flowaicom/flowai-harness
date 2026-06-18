use agent_fw_catalog::{
    relation_kind, Cardinality, CatalogEntry, CatalogFacetValue, CatalogKind, CatalogProvenance,
    CatalogRef, CatalogScope, CatalogSearchCursor, CatalogSearchFacets, CatalogSearchFilters,
    CatalogSearchHealth, CatalogSearchHitRef, CatalogSearchRequest, ColumnMetadata,
    DocumentMetadata, EnumValueMetadata, KnowledgeMetadata, RelationshipMetadata, SemanticEntity,
    SemanticEntityKind,
};
use agent_fw_core::{TenantId, WorkspaceId};
use serde_json::json;

fn entry(kind: CatalogKind, metadata: serde_json::Value) -> CatalogEntry {
    CatalogEntry {
        id: format!("{kind}-id"),
        kind,
        name: kind.as_str().to_string(),
        qualified_name: Some(format!("public.{}", kind.as_str())),
        content: format!("{kind} content"),
        tags: Vec::new(),
        links: Vec::new(),
        metadata,
    }
}

#[test]
fn semantic_model_satisfies_reusable_laws() {
    agent_fw_test::semantic_laws::test_all();
}

#[test]
fn semantic_catalog_scope_preserves_tenant_identity() {
    let tenant_id = TenantId::new_unchecked("tenant-a");
    let scope = CatalogScope::new(tenant_id.clone(), WorkspaceId::new_unchecked("workspace-a"));

    assert_eq!(scope.tenant_context().resource_id(), &tenant_id);

    let other_scope = CatalogScope::new(
        TenantId::new_unchecked("tenant-b"),
        WorkspaceId::new_unchecked("workspace-a"),
    );
    assert_ne!(scope, other_scope);
}

#[test]
fn semantic_metadata_decodes_camel_case_contracts() {
    let column: ColumnMetadata = serde_json::from_value(json!({
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
    }))
    .unwrap();

    assert_eq!(column.column_name, "order_status");
    assert!(column.low_cardinality_enum);

    let enum_value: EnumValueMetadata = serde_json::from_value(json!({
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
    }))
    .unwrap();

    assert_eq!(enum_value.column_id, "col-order-status");
    assert_eq!(enum_value.synonyms, vec!["complete"]);

    let relationship: RelationshipMetadata = serde_json::from_value(json!({
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
    }))
    .unwrap();

    assert_eq!(relationship.source_cardinality, Cardinality::Many);
    assert_eq!(relationship.target_cardinality, Cardinality::One);
}

#[test]
fn semantic_document_and_knowledge_metadata_freeze_source_contract() {
    let document: DocumentMetadata = serde_json::from_value(json!({
        "sourceDocumentId": "doc-1",
        "contentAvailable": true,
        "contentSource": "kv",
        "extractionStatus": "processed",
        "extractedKnowledgeIds": ["knowledge-1"]
    }))
    .unwrap();

    assert_eq!(document.source_document_id, "doc-1");
    assert!(document.content_available);
    assert_eq!(document.content_source.as_deref(), Some("kv"));
    assert_eq!(document.extraction_status.as_deref(), Some("processed"));
    assert_eq!(document.extracted_knowledge_ids, vec!["knowledge-1"]);

    let knowledge: KnowledgeMetadata = serde_json::from_value(json!({
        "knowledgeType": "constraint",
        "scopeTables": ["public.fact_scenario"],
        "scopeColumns": ["public.fact_scenario.units"],
        "sqlExpression": "units >= 0",
        "synonyms": ["unit floor"],
        "sourceKnowledgeId": "k-1",
        "sourceDocumentId": "doc-1"
    }))
    .unwrap();

    assert_eq!(knowledge.source_knowledge_id.as_deref(), Some("k-1"));
    assert_eq!(knowledge.source_document_id.as_deref(), Some("doc-1"));

    let defaulted: KnowledgeMetadata = serde_json::from_value(json!({})).unwrap();
    assert_eq!(defaulted.source_knowledge_id, None);
    assert_eq!(defaulted.source_document_id, None);
}

#[test]
fn semantic_entry_conversion_decodes_document_metadata() {
    let document = entry(
        CatalogKind::Document,
        json!({
            "sourceDocumentId": "doc-1",
            "contentAvailable": true,
            "contentSource": "kv",
            "extractionStatus": "processed",
            "extractedKnowledgeIds": ["knowledge-1"]
        }),
    );

    let entity = SemanticEntity::try_from(document).unwrap();
    match entity {
        SemanticEntity::Document { metadata, .. } => {
            assert_eq!(metadata.source_document_id, "doc-1");
            assert!(metadata.content_available);
            assert_eq!(metadata.extracted_knowledge_ids, vec!["knowledge-1"]);
        }
        _ => panic!("expected document semantic entity"),
    }
}

#[test]
fn semantic_public_kind_names_hide_internal_storage_names() {
    assert_eq!(CatalogKind::Enum.public_name(), "enum_value");
    assert_eq!(SemanticEntityKind::EnumValue.public_name(), "enum_value");
    assert_eq!(CatalogKind::Enum.as_str(), "enum");
    assert!(!CatalogKind::Special.is_public_searchable());
    assert!(CatalogKind::Enum.is_public_searchable());
}

#[test]
fn semantic_relation_constants_and_refs_use_stable_contract_values() {
    assert_eq!(relation_kind::HAS_COLUMN, "has_column");
    assert_eq!(relation_kind::ENUM_VALUE_OF, "enum_value_of");
    assert_eq!(relation_kind::SUB_CLASS_OF, "sub_class_of");

    assert_eq!(
        CatalogRef::parse_table("public.fact_sales"),
        CatalogRef::QualifiedName {
            kind: Some(CatalogKind::Table),
            qualified_name: "public.fact_sales".to_string(),
        }
    );
    assert_eq!(
        CatalogRef::parse_column("order_status"),
        CatalogRef::Name {
            kind: CatalogKind::Column,
            name: "order_status".to_string(),
            schema: None,
        }
    );
}

#[test]
fn semantic_entry_conversion_decodes_kind_specific_metadata() {
    let table = entry(
        CatalogKind::Table,
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
                "enrichmentSource": "fixture",
                "modelId": "model-a",
                "schemaSnapshotAt": "2026-05-22T00:00:00Z",
                "targetFingerprint": "fingerprint-a"
            }
        }),
    );

    let entity = SemanticEntity::try_from(table).unwrap();
    assert_eq!(entity.kind(), SemanticEntityKind::Table);
    match entity {
        SemanticEntity::Table { metadata, .. } => {
            assert_eq!(metadata.table_name, "fact_sales");
            assert!(metadata.preferred_query_surface);
            assert_eq!(
                metadata.source,
                CatalogProvenance {
                    origin: None,
                    profiling_run_id: Some("profile-1".to_string()),
                    enrichment_source: Some("fixture".to_string()),
                    model_id: Some("model-a".to_string()),
                    fallback_reason: None,
                    schema_snapshot_at: Some("2026-05-22T00:00:00Z".to_string()),
                    target_fingerprint: Some("fingerprint-a".to_string()),
                }
            );
        }
        _ => panic!("expected table semantic entity"),
    }
}

#[test]
fn semantic_synonyms_are_typed_only_and_deduped() {
    let enum_entry = entry(
        CatalogKind::Enum,
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
            "synonyms": ["complete", "Complete", " paid order "]
        }),
    );
    let enum_entity = SemanticEntity::try_from(enum_entry).unwrap();
    assert_eq!(
        enum_entity.typed_synonyms(),
        vec!["complete".to_string(), "paid order".to_string()]
    );

    let raw_only = entry(
        CatalogKind::Table,
        json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "synonyms": ["must not leak"]
        }),
    );
    assert!(SemanticEntity::try_from(raw_only)
        .unwrap()
        .typed_synonyms()
        .is_empty());
}

#[test]
fn catalog_search_backend_contract_types_are_backend_agnostic() {
    let request = CatalogSearchRequest {
        query: "slow moving products".to_string(),
        kinds: vec![SemanticEntityKind::Knowledge, SemanticEntityKind::Metric],
        filters: CatalogSearchFilters {
            preferred_query_surface: Some(true),
            ..Default::default()
        },
        limit: 20,
        cursor: Some(CatalogSearchCursor::new("opaque-1")),
    };

    assert_eq!(request.query, "slow moving products");
    assert_eq!(request.cursor.as_ref().unwrap().as_str(), "opaque-1");
    let request_json = serde_json::to_value(&request).unwrap();
    assert_eq!(request_json["filters"]["preferred_query_surface"], true);
    assert!(request_json["filters"]["preferredQuerySurface"].is_null());

    let hit = CatalogSearchHitRef {
        entry_id: "knowledge:slow-moving-threshold".to_string(),
        score: 0.91,
        rank: 1,
        match_signals: vec!["synonym".to_string()],
        matched_fields: vec!["synonyms".to_string()],
        raw_score: Some(12.5),
        snippet: Some("slow-moving threshold".to_string()),
        resume_cursor: None,
    };
    assert_eq!(hit.entry_id, "knowledge:slow-moving-threshold");
    assert_eq!(hit.rank, 1);
    let hit_json = serde_json::to_value(&hit).unwrap();
    assert_eq!(hit_json["entry_id"], "knowledge:slow-moving-threshold");
    assert!(hit_json["entryId"].is_null());

    let mut facets = CatalogSearchFacets::default();
    facets.kinds.push(CatalogFacetValue {
        value: "knowledge".to_string(),
        count: 3,
    });
    facets.schemas.push(CatalogFacetValue {
        value: "public".to_string(),
        count: 2,
    });
    facets.tables.push(CatalogFacetValue {
        value: "public.fact_scenario".to_string(),
        count: 1,
    });
    facets.tags.push(CatalogFacetValue {
        value: "scenario_planning".to_string(),
        count: 1,
    });
    assert_eq!(facets.kinds[0].value, "knowledge");

    let health = CatalogSearchHealth::Ready {
        indexed_entries: 42,
        projection_version: 1,
    };
    assert!(health.is_ready());
}
