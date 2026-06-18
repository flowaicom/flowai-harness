use serde::{Deserialize, Serialize};

use super::relation::Cardinality;
use crate::{CatalogEntry, CatalogError};

/// Provenance for generated semantic metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub profiling_run_id: Option<String>,
    pub enrichment_source: Option<String>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub fallback_reason: Option<String>,
    pub schema_snapshot_at: Option<String>,
    pub target_fingerprint: Option<String>,
}

/// Stable provenance origin values for catalog facts.
pub mod provenance_origin {
    pub const PHYSICAL_SCHEMA: &str = "physical_schema";
    pub const LLM_ENRICHMENT: &str = "llm_enrichment";
    pub const CACHED_ENRICHMENT: &str = "cached_enrichment";
    pub const FALLBACK: &str = "fallback";
    pub const MANUAL: &str = "manual";
}

/// Typed metadata for `CatalogKind::Table`.
///
/// `database_id` identifies a target data source inside the workspace. It is
/// not an authorization boundary; tenant/workspace isolation belongs to
/// `CatalogScope` and catalog interpreter state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMetadata {
    pub database_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub relation_type: Option<String>,
    pub row_count: Option<i64>,
    pub column_count: Option<usize>,
    #[serde(default)]
    pub preferred_query_surface: bool,
    #[serde(default)]
    pub source: CatalogProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyMetadata {
    #[serde(alias = "targetSchema")]
    pub referenced_schema: String,
    #[serde(alias = "targetTable")]
    pub referenced_table: String,
    #[serde(alias = "targetColumn")]
    pub referenced_column: String,
    pub constraint_name: Option<String>,
}

/// Typed metadata for `CatalogKind::Column`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMetadata {
    pub database_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub column_name: String,
    pub data_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    pub foreign_key: Option<ForeignKeyMetadata>,
    pub semantic_type: Option<String>,
    pub distinct_count: Option<u64>,
    pub null_count: Option<u64>,
    pub total_count: Option<u64>,
    #[serde(default)]
    pub low_cardinality_enum: bool,
}

/// Typed metadata for a `CatalogKind::Enum` enum value entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueMetadata {
    pub database_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub column_name: String,
    pub column_id: String,
    pub value: String,
    pub normalized_value: String,
    pub display_value: Option<String>,
    pub frequency: Option<u64>,
    pub frequency_percentage: Option<f64>,
    pub rank: Option<u32>,
    #[serde(default)]
    pub synonyms: Vec<String>,
}

/// Typed metadata for `CatalogKind::Relationship`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipMetadata {
    pub database_id: String,
    pub source_table_id: String,
    pub target_table_id: String,
    pub source_schema: String,
    pub source_table: String,
    pub source_column: String,
    pub target_schema: String,
    pub target_table: String,
    pub target_column: String,
    pub source_cardinality: Cardinality,
    pub target_cardinality: Cardinality,
    pub relationship_kind: String,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub source: CatalogProvenance,
}

/// Typed metadata for `CatalogKind::Metric`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetricMetadata {
    pub formula: Option<String>,
    #[serde(default)]
    pub source_tables: Vec<String>,
    #[serde(default)]
    pub source_columns: Vec<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
}

/// Typed metadata for `CatalogKind::Knowledge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMetadata {
    pub knowledge_type: Option<String>,
    #[serde(default)]
    pub scope_tables: Vec<String>,
    #[serde(default)]
    pub scope_columns: Vec<String>,
    pub sql_expression: Option<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub source_knowledge_id: Option<String>,
    #[serde(default)]
    pub source_document_id: Option<String>,
}

/// Typed metadata for `CatalogKind::Document`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMetadata {
    pub source_document_id: String,
    #[serde(default)]
    pub content_available: bool,
    #[serde(default)]
    pub content_source: Option<String>,
    #[serde(default)]
    pub extraction_status: Option<String>,
    #[serde(default)]
    pub extracted_knowledge_ids: Vec<String>,
}

/// Typed metadata for generated `CatalogKind::DataQualityFinding` entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataQualityFindingMetadata {
    pub database_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub column_name: Option<String>,
    pub finding_type: Option<String>,
    #[serde(default)]
    pub scope_tables: Vec<String>,
    #[serde(default)]
    pub scope_columns: Vec<String>,
    #[serde(default)]
    pub source: CatalogProvenance,
    pub typical_value_range: Option<String>,
    #[serde(default)]
    pub validation_rules: Vec<String>,
}

pub fn decode_metadata<T: serde::de::DeserializeOwned>(
    entry: &CatalogEntry,
) -> Result<T, CatalogError> {
    serde_json::from_value(entry.metadata.clone()).map_err(|error| {
        CatalogError::InvalidQuery(format!(
            "invalid {} metadata for {}: {error}",
            entry.kind, entry.id
        ))
    })
}

pub(crate) fn dedupe_nonempty(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semantic_column_metadata_decodes_camel_case_contract() {
        let metadata: ColumnMetadata = serde_json::from_value(json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "order_status",
            "dataType": "text",
            "nullable": false,
            "primaryKey": true,
            "foreignKey": {
                "referencedSchema": "public",
                "referencedTable": "dim_status",
                "referencedColumn": "status_id",
                "constraintName": "fk_status"
            },
            "semanticType": "status",
            "distinctCount": 4,
            "nullCount": 0,
            "totalCount": 10,
            "lowCardinalityEnum": true
        }))
        .unwrap();

        assert!(metadata.primary_key);
        assert!(metadata.low_cardinality_enum);
        assert_eq!(metadata.foreign_key.unwrap().referenced_table, "dim_status");
    }

    #[test]
    fn semantic_enum_value_metadata_decodes_camel_case_contract() {
        let metadata: EnumValueMetadata = serde_json::from_value(json!({
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

        assert_eq!(metadata.column_id, "col-order-status");
        assert_eq!(metadata.synonyms, vec!["complete"]);
    }

    #[test]
    fn semantic_relationship_metadata_decodes_camel_case_contract() {
        let metadata: RelationshipMetadata = serde_json::from_value(json!({
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
            "confidence": 0.99,
            "source": {
                "origin": "physical_schema",
                "profilingRunId": "profile-1"
            }
        }))
        .unwrap();

        assert_eq!(metadata.source_cardinality, Cardinality::Many);
        assert_eq!(metadata.target_cardinality, Cardinality::One);
        assert_eq!(
            metadata.source.origin.as_deref(),
            Some(provenance_origin::PHYSICAL_SCHEMA)
        );
        assert_eq!(
            metadata.source.profiling_run_id.as_deref(),
            Some("profile-1")
        );
    }

    #[test]
    fn semantic_relationship_metadata_decodes_legacy_without_source() {
        let metadata: RelationshipMetadata = serde_json::from_value(json!({
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

        assert_eq!(metadata.source.origin, None);
        assert_eq!(metadata.source.profiling_run_id, None);
    }
}
