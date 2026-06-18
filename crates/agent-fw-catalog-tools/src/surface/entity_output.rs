use agent_fw_catalog::{CatalogEntry, CatalogKind, SemanticEntity};
use serde_json::json;

use super::output_policy::{DetailsMode, OutputPolicy, RelationMode};
use super::types::{
    CatalogEntity, CatalogEntityAssembly, CatalogEntityKind, CatalogRef, MatchDiagnostics,
};

#[derive(Debug, Clone)]
pub struct CatalogEntityAssembler {
    policy: OutputPolicy,
}

impl CatalogEntityAssembler {
    pub fn new(policy: OutputPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &OutputPolicy {
        &self.policy
    }

    pub fn assemble(&self, entry: CatalogEntry) -> CatalogEntityAssembly {
        self.assemble_with_match(entry, None)
    }

    pub fn assemble_with_match(
        &self,
        entry: CatalogEntry,
        match_diagnostics: Option<MatchDiagnostics>,
    ) -> CatalogEntityAssembly {
        if entry.kind == CatalogKind::Special {
            return CatalogEntityAssembly {
                entity: None,
                warnings: vec![format!(
                    "catalog entry {} has kind special, which is reserved and not emitted by the public surface",
                    entry.id
                )],
            };
        }

        match SemanticEntity::try_from(entry.clone()) {
            Ok(entity) => CatalogEntityAssembly {
                entity: Some(self.entity_from_semantic(entity, match_diagnostics, Vec::new())),
                warnings: Vec::new(),
            },
            Err(error) => {
                let warning = error.to_string();
                CatalogEntityAssembly {
                    entity: Some(self.entity_from_invalid_entry(entry, match_diagnostics, warning)),
                    warnings: vec![error.to_string()],
                }
            }
        }
    }

    fn entity_from_semantic(
        &self,
        entity: SemanticEntity,
        match_diagnostics: Option<MatchDiagnostics>,
        mut warnings: Vec<String>,
    ) -> CatalogEntity {
        let entry = entity.entry().clone();
        let description = description_for_policy(&entry, &self.policy, &mut warnings);
        let mut details = match self.policy.details_mode {
            DetailsMode::None => json!({}),
            DetailsMode::Summary => summary_details_for_entity(&entity),
            DetailsMode::Full => full_details_for_entity(&entity),
        };
        enforce_detail_cap(&mut details, &self.policy, &mut warnings);
        let relations = match self.policy.relation_mode {
            RelationMode::None | RelationMode::CompactEdges | RelationMode::FullEdges => Vec::new(),
            RelationMode::Refs => entry
                .links
                .iter()
                .map(|link| CatalogRef::id(link.target_id.clone()))
                .collect(),
        };

        CatalogEntity {
            id: entry.id,
            kind: CatalogEntityKind::from(entity.kind()),
            name: entry.name,
            qualified_name: entry.qualified_name,
            description,
            tags: entry.tags,
            details,
            relations,
            match_diagnostics,
            warnings,
        }
    }

    fn entity_from_invalid_entry(
        &self,
        entry: CatalogEntry,
        match_diagnostics: Option<MatchDiagnostics>,
        warning: String,
    ) -> CatalogEntity {
        let mut warnings = vec![warning];
        let description = description_for_policy(&entry, &self.policy, &mut warnings);
        CatalogEntity {
            id: entry.id,
            kind: CatalogEntityKind::from(entry.kind),
            name: entry.name,
            qualified_name: entry.qualified_name,
            description,
            tags: entry.tags,
            details: json!({}),
            relations: Vec::new(),
            match_diagnostics,
            warnings,
        }
    }
}

fn description_for_policy(
    entry: &CatalogEntry,
    policy: &OutputPolicy,
    warnings: &mut Vec<String>,
) -> String {
    let Some(max_chars) = policy.max_description_chars else {
        return entry.content.clone();
    };
    let description = compact_description_source(entry, policy, warnings);
    let original_chars = description.chars().count();
    if original_chars <= max_chars {
        return description;
    }
    warnings.push(format!(
        "description for {} truncated from {original_chars} to {max_chars} chars by output policy {}",
        entry.id, policy.id
    ));
    truncate_chars(&description, max_chars)
}

fn compact_description_source(
    entry: &CatalogEntry,
    policy: &OutputPolicy,
    warnings: &mut Vec<String>,
) -> String {
    if entry.kind != CatalogKind::Table {
        return entry.content.clone();
    }

    let compact = strip_table_schema_sections(&entry.content);
    if compact != entry.content {
        warnings.push(format!(
            "description for {} omitted schema-heavy table sections by output policy {}",
            entry.id, policy.id
        ));
    }
    compact
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn strip_table_schema_sections(value: &str) -> String {
    let mut kept = Vec::new();
    for line in value.lines() {
        if is_schema_section_line(line) {
            break;
        }
        let clipped = clip_at_schema_marker(line);
        if clipped.len() != line.len() {
            let clipped = clipped.trim_end();
            if !clipped.is_empty() {
                kept.push(clipped.to_string());
            }
            break;
        }
        kept.push(line.to_string());
    }
    kept.join("\n").trim().to_string()
}

fn is_schema_section_line(line: &str) -> bool {
    let normalized = line
        .trim_start()
        .trim_start_matches(['-', '*'])
        .trim_start()
        .to_ascii_lowercase();
    schema_markers()
        .iter()
        .any(|marker| normalized.starts_with(marker))
}

fn clip_at_schema_marker(line: &str) -> &str {
    let lower = line.to_ascii_lowercase();
    let Some(position) = schema_markers()
        .iter()
        .filter_map(|marker| {
            lower
                .find(marker)
                .filter(|position| marker_boundary(line, *position))
        })
        .min()
    else {
        return line;
    };
    &line[..position]
}

fn marker_boundary(line: &str, position: usize) -> bool {
    position == 0
        || line[..position]
            .chars()
            .last()
            .map(|ch| ch.is_ascii_whitespace() || matches!(ch, '.' | ';' | ',' | ')' | '('))
            .unwrap_or(true)
}

fn schema_markers() -> &'static [&'static str] {
    &[
        "columns:",
        "column list:",
        "fields:",
        "row count:",
        "row_count:",
        "rows:",
    ]
}

fn enforce_detail_cap(
    details: &mut serde_json::Value,
    policy: &OutputPolicy,
    warnings: &mut Vec<String>,
) {
    let Some(object) = details.as_object_mut() else {
        return;
    };
    if object.len() <= policy.max_fields_per_entity {
        return;
    }
    let original = object.len();
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys.into_iter().skip(policy.max_fields_per_entity) {
        object.remove(&key);
    }
    warnings.push(format!(
        "details truncated from {original} to {} fields by output policy {}",
        policy.max_fields_per_entity, policy.id
    ));
}

fn summary_details_for_entity(entity: &SemanticEntity) -> serde_json::Value {
    match entity {
        SemanticEntity::Table { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "preferred_query_surface": metadata.preferred_query_surface,
        }),
        SemanticEntity::Column { metadata, .. } => json!({
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "data_type": &metadata.data_type,
            "semantic_type": &metadata.semantic_type,
            "nullable": metadata.nullable,
            "primary_key": metadata.primary_key,
            "low_cardinality_enum": metadata.low_cardinality_enum,
        }),
        SemanticEntity::Relationship { metadata, .. } => json!({
            "source_table": &metadata.source_table,
            "source_column": &metadata.source_column,
            "target_table": &metadata.target_table,
            "target_column": &metadata.target_column,
            "relationship_kind": &metadata.relationship_kind,
        }),
        SemanticEntity::EnumValue { metadata, .. } => json!({
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "value": &metadata.value,
            "display_value": &metadata.display_value,
            "rank": &metadata.rank,
        }),
        SemanticEntity::Metric { metadata, .. } => json!({
            "formula": &metadata.formula,
            "source_tables": &metadata.source_tables,
            "source_columns": &metadata.source_columns,
        }),
        SemanticEntity::Knowledge { metadata, .. } => json!({
            "knowledge_type": &metadata.knowledge_type,
            "scope_tables": &metadata.scope_tables,
            "scope_columns": &metadata.scope_columns,
            "source_document_id": &metadata.source_document_id,
        }),
        SemanticEntity::Document { metadata, .. } => json!({
            "source_document_id": &metadata.source_document_id,
            "content_available": metadata.content_available,
            "extraction_status": &metadata.extraction_status,
        }),
        SemanticEntity::DataQualityFinding { metadata, .. } => json!({
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "finding_type": &metadata.finding_type,
        }),
        SemanticEntity::Special { .. } => json!({}),
    }
}

fn full_details_for_entity(entity: &SemanticEntity) -> serde_json::Value {
    match entity {
        SemanticEntity::Table { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "relation_type": &metadata.relation_type,
            "row_count": &metadata.row_count,
            "column_count": &metadata.column_count,
            "preferred_query_surface": metadata.preferred_query_surface,
        }),
        SemanticEntity::Column { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "data_type": &metadata.data_type,
            "nullable": metadata.nullable,
            "primary_key": metadata.primary_key,
            "foreign_key": metadata.foreign_key.as_ref().map(|foreign_key| json!({
                "referenced_schema": &foreign_key.referenced_schema,
                "referenced_table": &foreign_key.referenced_table,
                "referenced_column": &foreign_key.referenced_column,
                "constraint_name": &foreign_key.constraint_name,
            })),
            "semantic_type": &metadata.semantic_type,
            "distinct_count": &metadata.distinct_count,
            "null_count": &metadata.null_count,
            "total_count": &metadata.total_count,
            "low_cardinality_enum": metadata.low_cardinality_enum,
        }),
        SemanticEntity::Relationship { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "source_table_id": &metadata.source_table_id,
            "target_table_id": &metadata.target_table_id,
            "source_schema": &metadata.source_schema,
            "source_table": &metadata.source_table,
            "source_column": &metadata.source_column,
            "target_schema": &metadata.target_schema,
            "target_table": &metadata.target_table,
            "target_column": &metadata.target_column,
            "source_cardinality": metadata.source_cardinality,
            "target_cardinality": metadata.target_cardinality,
            "relationship_kind": &metadata.relationship_kind,
            "confidence": &metadata.confidence,
            "source": &metadata.source,
        }),
        SemanticEntity::EnumValue { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "column_id": &metadata.column_id,
            "value": &metadata.value,
            "normalized_value": &metadata.normalized_value,
            "display_value": &metadata.display_value,
            "frequency": &metadata.frequency,
            "frequency_percentage": &metadata.frequency_percentage,
            "rank": &metadata.rank,
            "synonyms": &metadata.synonyms,
        }),
        SemanticEntity::Metric { metadata, .. } => json!({
            "formula": &metadata.formula,
            "source_tables": &metadata.source_tables,
            "source_columns": &metadata.source_columns,
            "synonyms": &metadata.synonyms,
        }),
        SemanticEntity::Knowledge { metadata, .. } => json!({
            "knowledge_type": &metadata.knowledge_type,
            "scope_tables": &metadata.scope_tables,
            "scope_columns": &metadata.scope_columns,
            "sql_expression": &metadata.sql_expression,
            "synonyms": &metadata.synonyms,
            "source_knowledge_id": &metadata.source_knowledge_id,
            "source_document_id": &metadata.source_document_id,
        }),
        SemanticEntity::Document { metadata, .. } => json!({
            "source_document_id": &metadata.source_document_id,
            "content_available": metadata.content_available,
            "content_source": &metadata.content_source,
            "extraction_status": &metadata.extraction_status,
            "extracted_knowledge_ids": &metadata.extracted_knowledge_ids,
        }),
        SemanticEntity::DataQualityFinding { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "finding_type": &metadata.finding_type,
            "scope_tables": &metadata.scope_tables,
            "scope_columns": &metadata.scope_columns,
            "typical_value_range": &metadata.typical_value_range,
            "validation_rules": &metadata.validation_rules,
        }),
        SemanticEntity::Special { .. } => json!({}),
    }
}
