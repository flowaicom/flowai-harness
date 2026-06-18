use agent_fw_catalog::{Cardinality, CatalogError, SemanticEntity, SemanticEntityKind};

use crate::error::CatalogIndexError;

// Bumped to 3 when the low-severity projection cleanup added the remaining
// documented diagnostic and kind-specific fields. Older indexes do not have
// those fields in their Tantivy schema and must be rebuilt before serving.
pub const PROJECTED_CATALOG_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDocumentProjection {
    pub entry_id: String,
    pub kind: SemanticEntityKind,
    pub kind_name: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub description: String,
    pub tags: Vec<String>,
    pub database_id: Option<String>,
    pub schema_name: Option<String>,
    pub table_name: Option<String>,
    pub column_name: Option<String>,
    pub data_type: Option<String>,
    pub semantic_type: Option<String>,
    pub knowledge_type: Option<String>,
    pub relation_kind: Option<String>,
    pub source_table: Option<String>,
    pub source_column: Option<String>,
    pub target_table: Option<String>,
    pub target_column: Option<String>,
    pub preferred_query_surface: Option<bool>,
    pub low_cardinality_enum: Option<bool>,
    pub enum_value: Option<String>,
    pub enum_normalized_value: Option<String>,
    pub enum_display_value: Option<String>,
    pub synonyms: Vec<String>,
    pub updated_at: Option<String>,
    pub relation_type: Option<String>,
    pub nullable: Option<bool>,
    pub primary_key: Option<bool>,
    pub foreign_key_text: Option<String>,
    pub source_cardinality: Option<String>,
    pub target_cardinality: Option<String>,
    pub confidence: Option<String>,
    pub formula_text: Option<String>,
    pub sql_expression_text: Option<String>,
    pub validation_rules: Vec<String>,
    pub table_filter_values: Vec<String>,
    pub column_filter_values: Vec<String>,
    pub source_table_filter_values: Vec<String>,
    pub source_column_filter_values: Vec<String>,
    pub target_table_filter_values: Vec<String>,
    pub target_column_filter_values: Vec<String>,
    pub context_parts: Vec<String>,
    pub document_body: Option<String>,
    pub projection_version: u32,
}

impl CatalogDocumentProjection {
    pub fn project(
        entity: &SemanticEntity,
        document_body: Option<String>,
    ) -> Result<Self, CatalogIndexError> {
        if !entity.kind().is_public_searchable() {
            return Err(CatalogIndexError::UnsupportedKind(
                entity.kind().public_name().to_string(),
            ));
        }

        let entry = entity.entry();
        let mut projection = Self {
            entry_id: entry.id.clone(),
            kind: entity.kind(),
            kind_name: entity.kind().public_name().to_string(),
            name: entry.name.clone(),
            qualified_name: entry.qualified_name.clone(),
            description: entry.content.clone(),
            tags: entry.tags.clone(),
            database_id: None,
            schema_name: None,
            table_name: None,
            column_name: None,
            data_type: None,
            semantic_type: None,
            knowledge_type: None,
            relation_kind: None,
            source_table: None,
            source_column: None,
            target_table: None,
            target_column: None,
            preferred_query_surface: None,
            low_cardinality_enum: None,
            enum_value: None,
            enum_normalized_value: None,
            enum_display_value: None,
            synonyms: entity.typed_synonyms(),
            updated_at: entity_updated_at(entity),
            relation_type: None,
            nullable: None,
            primary_key: None,
            foreign_key_text: None,
            source_cardinality: None,
            target_cardinality: None,
            confidence: None,
            formula_text: None,
            sql_expression_text: None,
            validation_rules: Vec::new(),
            table_filter_values: Vec::new(),
            column_filter_values: Vec::new(),
            source_table_filter_values: Vec::new(),
            source_column_filter_values: Vec::new(),
            target_table_filter_values: Vec::new(),
            target_column_filter_values: Vec::new(),
            context_parts: Vec::new(),
            document_body: clean_optional(document_body),
            projection_version: PROJECTED_CATALOG_SCHEMA_VERSION,
        };

        match entity {
            SemanticEntity::Table { metadata, .. } => {
                projection.database_id = Some(metadata.database_id.clone());
                projection.schema_name = Some(metadata.schema_name.clone());
                projection.table_name = Some(metadata.table_name.clone());
                projection.relation_type = metadata.relation_type.clone();
                projection.preferred_query_surface = Some(metadata.preferred_query_surface);
                projection
                    .table_filter_values
                    .push(metadata.table_name.clone());
                projection.push_qualified_table(&metadata.schema_name, &metadata.table_name);
                projection.push_context(metadata.relation_type.clone());
                projection.push_context(Some(metadata.schema_name.clone()));
                projection.push_context(Some(metadata.table_name.clone()));
            }
            SemanticEntity::Column { metadata, .. } => {
                projection.database_id = Some(metadata.database_id.clone());
                projection.schema_name = Some(metadata.schema_name.clone());
                projection.table_name = Some(metadata.table_name.clone());
                projection.column_name = Some(metadata.column_name.clone());
                projection.data_type = Some(metadata.data_type.clone());
                projection.semantic_type = metadata.semantic_type.clone();
                projection.nullable = Some(metadata.nullable);
                projection.primary_key = Some(metadata.primary_key);
                projection.low_cardinality_enum = Some(metadata.low_cardinality_enum);
                projection
                    .table_filter_values
                    .push(metadata.table_name.clone());
                projection.push_qualified_table(&metadata.schema_name, &metadata.table_name);
                projection
                    .column_filter_values
                    .push(metadata.column_name.clone());
                projection.push_qualified_column(
                    &metadata.schema_name,
                    &metadata.table_name,
                    &metadata.column_name,
                );
                projection.push_context(Some(metadata.table_name.clone()));
                projection.push_context(Some(metadata.column_name.clone()));
                projection.push_context(Some(metadata.data_type.clone()));
                projection.push_context(metadata.semantic_type.clone());
                if let Some(foreign_key) = &metadata.foreign_key {
                    projection.foreign_key_text = Some(
                        [
                            foreign_key.referenced_schema.as_str(),
                            foreign_key.referenced_table.as_str(),
                            foreign_key.referenced_column.as_str(),
                            foreign_key.constraint_name.as_deref().unwrap_or_default(),
                        ]
                        .join(" "),
                    );
                    projection.push_context(Some(foreign_key.referenced_table.clone()));
                    projection.push_context(Some(foreign_key.referenced_column.clone()));
                }
            }
            SemanticEntity::Relationship { metadata, .. } => {
                projection.database_id = Some(metadata.database_id.clone());
                projection.schema_name = Some(metadata.source_schema.clone());
                projection.table_name = Some(metadata.source_table.clone());
                projection.column_name = Some(metadata.source_column.clone());
                projection.relation_kind = Some(metadata.relationship_kind.clone());
                projection.source_table = Some(metadata.source_table.clone());
                projection.source_column = Some(metadata.source_column.clone());
                projection.target_table = Some(metadata.target_table.clone());
                projection.target_column = Some(metadata.target_column.clone());
                projection.source_cardinality = Some(cardinality_name(metadata.source_cardinality));
                projection.target_cardinality = Some(cardinality_name(metadata.target_cardinality));
                projection.confidence =
                    metadata.confidence.map(|confidence| confidence.to_string());
                projection.table_filter_values.extend([
                    metadata.source_table.clone(),
                    metadata.target_table.clone(),
                    metadata.source_table_id.clone(),
                    metadata.target_table_id.clone(),
                ]);
                projection.source_table_filter_values.extend([
                    metadata.source_table.clone(),
                    metadata.source_table_id.clone(),
                ]);
                projection.target_table_filter_values.extend([
                    metadata.target_table.clone(),
                    metadata.target_table_id.clone(),
                ]);
                projection.push_qualified_table(&metadata.source_schema, &metadata.source_table);
                projection.push_qualified_table(&metadata.target_schema, &metadata.target_table);
                projection
                    .push_source_qualified_table(&metadata.source_schema, &metadata.source_table);
                projection
                    .push_target_qualified_table(&metadata.target_schema, &metadata.target_table);
                projection.column_filter_values.extend([
                    metadata.source_column.clone(),
                    metadata.target_column.clone(),
                ]);
                projection
                    .source_column_filter_values
                    .push(metadata.source_column.clone());
                projection
                    .target_column_filter_values
                    .push(metadata.target_column.clone());
                projection.push_qualified_column(
                    &metadata.source_schema,
                    &metadata.source_table,
                    &metadata.source_column,
                );
                projection.push_qualified_column(
                    &metadata.target_schema,
                    &metadata.target_table,
                    &metadata.target_column,
                );
                projection.push_source_qualified_column(
                    &metadata.source_schema,
                    &metadata.source_table,
                    &metadata.source_column,
                );
                projection.push_target_qualified_column(
                    &metadata.target_schema,
                    &metadata.target_table,
                    &metadata.target_column,
                );
                projection.push_context(Some(metadata.relationship_kind.clone()));
                projection.push_context(Some(metadata.source_table.clone()));
                projection.push_context(Some(metadata.target_table.clone()));
                projection.push_context(Some(metadata.source_column.clone()));
                projection.push_context(Some(metadata.target_column.clone()));
            }
            SemanticEntity::EnumValue { metadata, .. } => {
                projection.database_id = Some(metadata.database_id.clone());
                projection.schema_name = Some(metadata.schema_name.clone());
                projection.table_name = Some(metadata.table_name.clone());
                projection.column_name = Some(metadata.column_name.clone());
                projection.low_cardinality_enum = Some(true);
                projection.enum_value = Some(metadata.value.clone());
                projection.enum_normalized_value = Some(metadata.normalized_value.clone());
                projection.enum_display_value = metadata.display_value.clone();
                projection
                    .table_filter_values
                    .push(metadata.table_name.clone());
                projection.push_qualified_table(&metadata.schema_name, &metadata.table_name);
                projection
                    .column_filter_values
                    .push(metadata.column_name.clone());
                projection
                    .column_filter_values
                    .push(metadata.column_id.clone());
                projection.push_qualified_column(
                    &metadata.schema_name,
                    &metadata.table_name,
                    &metadata.column_name,
                );
                projection.push_context(Some(metadata.table_name.clone()));
                projection.push_context(Some(metadata.column_name.clone()));
                projection.push_context(Some(metadata.value.clone()));
                projection.push_context(metadata.display_value.clone());
            }
            SemanticEntity::Metric { metadata, .. } => {
                projection.formula_text = metadata.formula.clone();
                projection
                    .table_filter_values
                    .extend(metadata.source_tables.clone());
                projection
                    .source_table_filter_values
                    .extend(metadata.source_tables.clone());
                projection
                    .column_filter_values
                    .extend(metadata.source_columns.clone());
                projection
                    .source_column_filter_values
                    .extend(metadata.source_columns.clone());
                projection
                    .context_parts
                    .extend(metadata.source_tables.clone());
                projection
                    .context_parts
                    .extend(metadata.source_columns.clone());
            }
            SemanticEntity::Knowledge { metadata, .. } => {
                projection.knowledge_type = metadata.knowledge_type.clone();
                projection.sql_expression_text = metadata.sql_expression.clone();
                projection
                    .table_filter_values
                    .extend(metadata.scope_tables.clone());
                projection
                    .source_table_filter_values
                    .extend(metadata.scope_tables.clone());
                projection
                    .column_filter_values
                    .extend(metadata.scope_columns.clone());
                projection
                    .source_column_filter_values
                    .extend(metadata.scope_columns.clone());
                projection.push_context(metadata.knowledge_type.clone());
                projection.push_context(metadata.source_document_id.clone());
                projection
                    .context_parts
                    .extend(metadata.scope_tables.clone());
                projection
                    .context_parts
                    .extend(metadata.scope_columns.clone());
            }
            SemanticEntity::Document { metadata, .. } => {
                projection.push_context(Some(metadata.source_document_id.clone()));
                projection.push_context(metadata.content_source.clone());
                projection.push_context(metadata.extraction_status.clone());
                projection
                    .context_parts
                    .extend(metadata.extracted_knowledge_ids.clone());
            }
            SemanticEntity::DataQualityFinding { metadata, .. } => {
                projection.database_id = Some(metadata.database_id.clone());
                projection.schema_name = Some(metadata.schema_name.clone());
                projection.table_name = Some(metadata.table_name.clone());
                projection.column_name = metadata.column_name.clone();
                projection
                    .table_filter_values
                    .push(metadata.table_name.clone());
                projection
                    .source_table_filter_values
                    .push(metadata.table_name.clone());
                projection
                    .source_table_filter_values
                    .extend(metadata.scope_tables.clone());
                projection.push_qualified_table(&metadata.schema_name, &metadata.table_name);
                projection.push_source_qualified_table(&metadata.schema_name, &metadata.table_name);
                if let Some(column_name) = &metadata.column_name {
                    projection.column_filter_values.push(column_name.clone());
                    projection
                        .source_column_filter_values
                        .push(column_name.clone());
                    projection.push_qualified_column(
                        &metadata.schema_name,
                        &metadata.table_name,
                        column_name,
                    );
                    projection.push_source_qualified_column(
                        &metadata.schema_name,
                        &metadata.table_name,
                        column_name,
                    );
                }
                projection
                    .context_parts
                    .extend(metadata.scope_tables.clone());
                projection
                    .context_parts
                    .extend(metadata.scope_columns.clone());
                projection
                    .source_column_filter_values
                    .extend(metadata.scope_columns.clone());
                projection.push_context(metadata.finding_type.clone());
                projection.push_context(metadata.typical_value_range.clone());
                projection.validation_rules = metadata.validation_rules.clone();
                projection
                    .context_parts
                    .extend(metadata.validation_rules.clone());
            }
            SemanticEntity::Special { .. } => {
                return Err(CatalogIndexError::UnsupportedKind("special".to_string()));
            }
        }

        projection.table_filter_values = dedupe_nonempty(projection.table_filter_values);
        projection.column_filter_values = dedupe_nonempty(projection.column_filter_values);
        projection.source_table_filter_values =
            dedupe_nonempty(projection.source_table_filter_values);
        projection.source_column_filter_values =
            dedupe_nonempty(projection.source_column_filter_values);
        projection.target_table_filter_values =
            dedupe_nonempty(projection.target_table_filter_values);
        projection.target_column_filter_values =
            dedupe_nonempty(projection.target_column_filter_values);
        projection.validation_rules = dedupe_nonempty(projection.validation_rules);
        projection.context_parts = dedupe_nonempty(projection.context_parts);

        Ok(projection)
    }

    pub fn synonyms_text(&self) -> String {
        self.synonyms.join(" ")
    }

    pub fn context_text(&self) -> String {
        self.context_parts.join(" ")
    }

    pub fn validation_rules_text(&self) -> String {
        self.validation_rules.join(" ")
    }

    fn push_context(&mut self, value: Option<String>) {
        if let Some(value) = value {
            if !value.trim().is_empty() {
                self.context_parts.push(value);
            }
        }
    }

    fn push_qualified_table(&mut self, schema_name: &str, table_name: &str) {
        self.table_filter_values
            .push(format!("{schema_name}.{table_name}"));
    }

    fn push_qualified_column(&mut self, schema_name: &str, table_name: &str, column_name: &str) {
        self.column_filter_values
            .push(format!("{schema_name}.{table_name}.{column_name}"));
    }

    fn push_source_qualified_table(&mut self, schema_name: &str, table_name: &str) {
        self.source_table_filter_values
            .push(format!("{schema_name}.{table_name}"));
    }

    fn push_source_qualified_column(
        &mut self,
        schema_name: &str,
        table_name: &str,
        column_name: &str,
    ) {
        self.source_column_filter_values
            .push(format!("{schema_name}.{table_name}.{column_name}"));
    }

    fn push_target_qualified_table(&mut self, schema_name: &str, table_name: &str) {
        self.target_table_filter_values
            .push(format!("{schema_name}.{table_name}"));
    }

    fn push_target_qualified_column(
        &mut self,
        schema_name: &str,
        table_name: &str,
        column_name: &str,
    ) {
        self.target_column_filter_values
            .push(format!("{schema_name}.{table_name}.{column_name}"));
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn dedupe_nonempty(values: Vec<String>) -> Vec<String> {
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

fn cardinality_name(cardinality: Cardinality) -> String {
    match cardinality {
        Cardinality::One => "one",
        Cardinality::Many => "many",
        Cardinality::Unknown => "unknown",
    }
    .to_string()
}

fn entity_updated_at(entity: &SemanticEntity) -> Option<String> {
    match entity {
        SemanticEntity::Table { metadata, .. } => {
            clean_optional(metadata.source.schema_snapshot_at.clone())
        }
        SemanticEntity::Relationship { metadata, .. } => {
            clean_optional(metadata.source.schema_snapshot_at.clone())
        }
        SemanticEntity::DataQualityFinding { metadata, .. } => {
            clean_optional(metadata.source.schema_snapshot_at.clone())
        }
        _ => None,
    }
}

impl From<CatalogError> for CatalogIndexError {
    fn from(error: CatalogError) -> Self {
        CatalogIndexError::InvalidQuery(error.to_string())
    }
}
