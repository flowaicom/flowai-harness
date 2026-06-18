//! Mock SemanticEnricher — returns fixed enrichment results for testing.

use async_trait::async_trait;

use agent_fw_catalog::{
    ColumnDescriptions, EnrichmentError, EnrichmentResult, EnrichmentSource,
    KnowledgeExtractionRequest, KnowledgeItem, SemanticEnricher, SemanticTableProfile,
    TableEnrichmentRequest,
};

/// Mock enricher that returns generated descriptions based on schema.
pub struct MockEnricher {
    source: EnrichmentSource,
}

impl MockEnricher {
    pub fn new() -> Self {
        Self {
            source: EnrichmentSource::Fresh,
        }
    }

    /// Create a mock enricher that reports cached results.
    pub fn cached() -> Self {
        Self {
            source: EnrichmentSource::Cached,
        }
    }

    /// Create a mock enricher that reports fallback results.
    pub fn fallback() -> Self {
        Self {
            source: EnrichmentSource::Fallback,
        }
    }
}

impl Default for MockEnricher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SemanticEnricher for MockEnricher {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        let table_name = &request.table.table_name;
        let mut col_descs = ColumnDescriptions::new();
        for col in &request.table.columns {
            col_descs.insert(
                col.column_name.clone(),
                format!("{} column of {} table", col.column_name, table_name),
            );
        }

        let profile = SemanticTableProfile {
            description: format!("The {table_name} table stores {table_name}-related data."),
            short_description: format!("{table_name} data"),
            column_descriptions: col_descs,
            relationships: vec![],
            quality_notes: vec![],
        };

        let fallback_reason = if self.source == EnrichmentSource::Fallback {
            Some("enricher reported fallback".to_string())
        } else {
            None
        };

        Ok(EnrichmentResult {
            profile,
            source: self.source,
            model_id: None,
            fallback_reason,
        })
    }

    async fn extract_knowledge(
        &self,
        request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        // Return a single knowledge item derived from the document
        Ok(vec![KnowledgeItem {
            id: format!("mock-k-0"),
            name: format!("Knowledge from {}", request.document_name),
            description: format!(
                "Extracted from document: {}",
                &request.document_content[..request.document_content.len().min(100)]
            ),
            knowledge_type: agent_fw_catalog::KnowledgeType::Custom,
            scope_tables: request.available_tables,
            scope_columns: vec![],
            sql_expression: None,
            synonyms: vec![],
            source_document_id: None,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{ColumnInfo, PhysicalTable, TableProfile};

    fn sample_request() -> TableEnrichmentRequest {
        TableEnrichmentRequest {
            table: PhysicalTable {
                schema_name: "public".into(),
                table_name: "orders".into(),
                columns: vec![
                    ColumnInfo {
                        column_name: "id".into(),
                        data_type: "integer".into(),
                        is_nullable: false,
                        column_default: None,
                        ordinal_position: 1,
                        is_primary_key: true,
                        foreign_key: None,
                    },
                    ColumnInfo {
                        column_name: "amount".into(),
                        data_type: "numeric".into(),
                        is_nullable: false,
                        column_default: None,
                        ordinal_position: 2,
                        is_primary_key: false,
                        foreign_key: None,
                    },
                ],
                constraints: vec![],
                indexes: vec![],
                row_count: 1000,
            },
            sample_rows: vec![],
            profile: TableProfile {
                table_name: "orders".into(),
                columns: vec![],
            },
            database_context: None,
            fk_edges: vec![],
        }
    }

    #[tokio::test]
    async fn enrich_returns_non_empty() {
        let enricher = MockEnricher::new();
        let result = enricher.enrich_table(sample_request()).await.unwrap();

        assert!(!result.profile.description.is_empty());
        assert_eq!(result.source, EnrichmentSource::Fresh);
        assert_eq!(result.profile.column_descriptions.len(), 2);
    }

    #[tokio::test]
    async fn cached_enricher_reports_cached() {
        let enricher = MockEnricher::cached();
        let result = enricher.enrich_table(sample_request()).await.unwrap();
        assert_eq!(result.source, EnrichmentSource::Cached);
    }

    #[tokio::test]
    async fn extract_knowledge_returns_item() {
        let enricher = MockEnricher::new();
        let items = enricher
            .extract_knowledge(KnowledgeExtractionRequest {
                document_content: "Revenue is calculated as SUM(amount)".into(),
                document_name: "metrics.md".into(),
                database_context: None,
                available_tables: vec!["orders".into()],
                available_columns: vec!["amount".into()],
            })
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert!(items[0].name.contains("metrics.md"));
    }
}
