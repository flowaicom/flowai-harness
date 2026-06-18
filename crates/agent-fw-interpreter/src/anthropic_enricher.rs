//! Anthropic-powered SemanticEnricher implementation.
//!
//! Calls the Anthropic API for table enrichment and knowledge extraction.
//! Uses non-streaming requests since full JSON responses are needed for
//! structured output parsing.

use agent_fw_catalog::enrichment::{
    ColumnDescriptions, EnrichmentError, EnrichmentResult, InferredRelationship, JoinPair,
    KnowledgeExtractionRequest, QualityNote, RelationshipKind, SemanticEnricher,
    SemanticTableProfile, TableEnrichmentRequest,
};
use agent_fw_catalog::knowledge::{KnowledgeItem, LlmKnowledgeItem};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Anthropic API-backed SemanticEnricher.
///
/// Uses Claude for:
/// - `enrich_table()`: Generates semantic profiles from schema + sample data
/// - `extract_knowledge()`: Extracts knowledge items from documents
pub struct AnthropicEnricher {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicEnricher {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-6".into(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Make a non-streaming call to the Messages API.
    async fn call_api(&self, prompt: &str) -> Result<String, EnrichmentError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}],
        });

        let start = std::time::Instant::now();
        tracing::info!(model = %self.model, "sending Anthropic request");
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| EnrichmentError::LlmFailed(e.to_string()))?;
        tracing::info!(
            model = %self.model,
            status = %response.status(),
            duration_ms = start.elapsed().as_millis() as u64,
            "Anthropic response received"
        );

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(EnrichmentError::LlmFailed(format!("{status}: {body_text}")));
        }

        let resp_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| EnrichmentError::LlmFailed(e.to_string()))?;

        let text = resp_json
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                EnrichmentError::ParseFailed(format!(
                    "Unexpected response schema: missing content[0].text in {}",
                    serde_json::to_string(&resp_json).unwrap_or_else(|_| "<unserializable>".into())
                ))
            })?;

        Ok(strip_code_fences(text))
    }
}

#[async_trait]
impl SemanticEnricher for AnthropicEnricher {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        let columns_desc = request
            .profile
            .columns
            .iter()
            .map(|c| {
                format!(
                    "- {} ({}): semantic_type={}, nulls={}/{}, distinct={}",
                    c.column_name,
                    c.data_type,
                    c.semantic_type,
                    c.null_count,
                    c.total_count,
                    c.distinct_count
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Analyze this database table and return a JSON semantic profile.\n\n\
            Table: {}.{}\n\n\
            Column profiling summary:\n{columns_desc}\n\n\
            {}\n\n\
            Return only JSON with this exact structure:\n\
            {{\n  \"description\": \"<1-2 sentence description>\",\n  \"shortDescription\": \"<brief label>\",\n  \
            \"columnDescriptions\": {{\"<col_name>\": \"<description>\", ...}},\n  \
            \"relationships\": [{{\"sourceTable\": \"<source table>\", \"targetTable\": \"<target table>\", \"relationshipType\": \"one-to-many\", \"joinColumns\": [], \"description\": \"<why these tables relate>\"}}],\n  \
            \"qualityNotes\": []\n}}\n\n\
            If no relationships or quality notes are clear, return empty arrays for those fields.",
            &request.table.schema_name,
            request.table.table_name,
            request.database_context.as_deref().unwrap_or(""),
        );

        let text = self.call_api(&prompt).await?;

        let profile = parse_semantic_profile(&text, &request.table.table_name)?;

        Ok(EnrichmentResult::fresh(profile).with_model_id(self.model.clone()))
    }

    async fn extract_knowledge(
        &self,
        request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        let prompt = format!(
            "Extract knowledge items from this document that are relevant to the database.\n\n\
            Document: {}\n\n{}\n\n\
            Available tables: {}\n\
            Available columns: {}\n\n\
            Return only a JSON array. Each item must use these exact camelCase fields:\n\
            {{\n  \"name\": \"<short stable label>\",\n  \"description\": \"<business meaning or rule>\",\n  \
            \"knowledgeType\": \"business_rule|predicate|terminology|constraint|temporal_rule|implicit_intent|data_quality|custom\",\n  \
            \"scopeTables\": [\"<table>\", ...],\n  \"scopeColumns\": [\"<table.column>\", ...],\n  \
            \"sqlExpression\": null,\n  \"synonyms\": []\n}}\n\n\
            Do not include id fields; the harness generates deterministic ids. Use [] for empty arrays and null for no SQL expression.",
            request.document_name,
            request.document_content,
            request.available_tables.join(", "),
            request.available_columns.join(", "),
        );

        let text = self.call_api(&prompt).await?;

        parse_knowledge_items(
            &text,
            &format!("{}\n{}", request.document_name, request.document_content),
        )
    }
}

/// Strip markdown code fences from LLM output.
fn strip_code_fences(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_semantic_profile(
    text: &str,
    default_source_table: &str,
) -> Result<SemanticTableProfile, EnrichmentError> {
    let raw: LlmSemanticTableProfile = serde_json::from_str(text)
        .map_err(|e| EnrichmentError::ParseFailed(format!("Failed to parse profile: {e}")))?;
    Ok(raw.into_profile(default_source_table))
}

fn parse_knowledge_items(text: &str, id_seed: &str) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
    let clean = strip_code_fences(text);
    let raw_items: Vec<LlmKnowledgeItem> = serde_json::from_str(&clean).map_err(|e| {
        EnrichmentError::ParseFailed(format!("Failed to parse knowledge items: {e}"))
    })?;
    let id_prefix = knowledge_id_prefix(id_seed);
    Ok(raw_items
        .into_iter()
        .enumerate()
        .map(|(index, item)| item.into_knowledge_item(&id_prefix, index))
        .collect())
}

fn knowledge_id_prefix(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let hex = hex::encode(digest);
    format!("knowledge-{}", &hex[..16])
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmSemanticTableProfile {
    description: String,
    short_description: String,
    column_descriptions: ColumnDescriptions,
    #[serde(default)]
    relationships: Vec<LlmRelationship>,
    #[serde(default)]
    quality_notes: Vec<Value>,
}

impl LlmSemanticTableProfile {
    fn into_profile(self, default_source_table: &str) -> SemanticTableProfile {
        let relationships = self
            .relationships
            .into_iter()
            .filter_map(|rel| rel.into_relationship(default_source_table))
            .collect();

        SemanticTableProfile {
            description: self.description,
            short_description: self.short_description,
            column_descriptions: self.column_descriptions,
            relationships,
            quality_notes: self
                .quality_notes
                .into_iter()
                .filter_map(parse_quality_note)
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmRelationship {
    source_table: Option<String>,
    target_table: Option<String>,
    relationship_type: Option<Value>,
    #[serde(default)]
    join_columns: Vec<Value>,
    #[serde(default)]
    description: String,
}

impl LlmRelationship {
    fn into_relationship(self, default_source_table: &str) -> Option<InferredRelationship> {
        Some(InferredRelationship {
            source_table: self
                .source_table
                .unwrap_or_else(|| default_source_table.to_string()),
            target_table: self.target_table?,
            relationship_type: parse_relationship_kind(self.relationship_type),
            join_columns: self
                .join_columns
                .into_iter()
                .filter_map(parse_join_pair)
                .collect(),
            description: self.description,
        })
    }
}

fn parse_relationship_kind(value: Option<Value>) -> RelationshipKind {
    let Some(kind) = value.and_then(|value| value.as_str().map(str::to_string)) else {
        return RelationshipKind::OneToMany;
    };
    match kind
        .trim()
        .to_ascii_lowercase()
        .replace(['_', ' '], "-")
        .as_str()
    {
        "many-to-many" => RelationshipKind::ManyToMany,
        "one-to-one" => RelationshipKind::OneToOne,
        "one-to-many" | "many-to-one" | "references" | "foreign-key" | "foreignkey" => {
            RelationshipKind::OneToMany
        }
        _ => RelationshipKind::OneToMany,
    }
}

fn parse_join_pair(value: Value) -> Option<JoinPair> {
    match value {
        Value::String(column) => same_name_join_pair(column),
        Value::Array(values) => {
            if values.len() != 2 {
                return None;
            }
            let source = values.first()?.as_str()?.to_string();
            let target = values.get(1)?.as_str()?.to_string();
            Some(JoinPair {
                source_column: source,
                target_column: target,
            })
        }
        Value::Object(object) => parse_join_pair_object(&object),
        _ => None,
    }
}

fn parse_join_pair_object(object: &Map<String, Value>) -> Option<JoinPair> {
    let source = string_field(
        object,
        &[
            "sourceColumn",
            "source_column",
            "source",
            "fromColumn",
            "from_column",
            "from",
            "leftColumn",
            "left_column",
            "left",
            "foreignKey",
            "foreign_key",
        ],
    );
    let target = string_field(
        object,
        &[
            "targetColumn",
            "target_column",
            "target",
            "toColumn",
            "to_column",
            "to",
            "rightColumn",
            "right_column",
            "right",
            "referencedColumn",
            "referenced_column",
            "referenceColumn",
            "reference_column",
            "primaryKey",
            "primary_key",
        ],
    );

    match (source, target) {
        (Some(source_column), Some(target_column)) => Some(JoinPair {
            source_column,
            target_column,
        }),
        (Some(column), None) | (None, Some(column)) => same_name_join_pair(column),
        _ => string_field(object, &["column", "columnName", "column_name", "name"])
            .and_then(same_name_join_pair),
    }
}

fn string_field(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn same_name_join_pair(column: String) -> Option<JoinPair> {
    let column = column.trim();
    if column.is_empty() {
        None
    } else {
        Some(JoinPair {
            source_column: column.to_string(),
            target_column: column.to_string(),
        })
    }
}

fn parse_quality_note(value: Value) -> Option<QualityNote> {
    match value {
        Value::String(notes) => table_quality_note(notes),
        Value::Object(object) => parse_quality_note_object(&object),
        _ => None,
    }
}

fn parse_quality_note_object(object: &Map<String, Value>) -> Option<QualityNote> {
    let column_name = string_field(object, &["columnName", "column_name", "column", "name"])
        .unwrap_or_else(|| "*".to_string());
    let notes = string_field(object, &["notes", "note", "description", "message", "text"])?;
    let typical_value_range = string_field(
        object,
        &[
            "typicalValueRange",
            "typical_value_range",
            "valueRange",
            "value_range",
            "range",
        ],
    );
    let validation_rules =
        string_list_field(object, &["validationRules", "validation_rules", "rules"]);

    Some(QualityNote {
        column_name,
        notes,
        typical_value_range,
        validation_rules,
    })
}

fn table_quality_note(notes: String) -> Option<QualityNote> {
    let notes = notes.trim();
    if notes.is_empty() {
        None
    } else {
        Some(QualityNote {
            column_name: "*".to_string(),
            notes: notes.to_string(),
            typical_value_range: None,
            validation_rules: vec![],
        })
    }
}

fn string_list_field(object: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| object.get(*key))
        .map(|value| match value {
            Value::String(rule) => trimmed_string(rule).into_iter().collect(),
            Value::Array(rules) => rules
                .iter()
                .filter_map(Value::as_str)
                .filter_map(trimmed_string)
                .collect(),
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

fn trimmed_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_code_fences_json() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn strip_code_fences_plain() {
        let input = "```\n[1, 2, 3]\n```";
        assert_eq!(strip_code_fences(input), "[1, 2, 3]");
    }

    #[test]
    fn strip_code_fences_no_fences() {
        let input = "{\"key\": \"value\"}";
        assert_eq!(strip_code_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn enricher_creation() {
        let enricher = AnthropicEnricher::new("test-key")
            .with_base_url("http://localhost:8080")
            .with_model("claude-haiku-4-5-20251001");
        assert_eq!(enricher.base_url, "http://localhost:8080");
        assert_eq!(enricher.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn parse_knowledge_items_generates_missing_ids_and_defaults_type() {
        let items = parse_knowledge_items(
            r#"[
                {
                    "name": "Slow moving products",
                    "description": "Products with low recent sell-through should be reviewed.",
                    "scopeTables": ["fact_sales", "dim_products"],
                    "scopeColumns": ["fact_sales.quantity"]
                }
            ]"#,
            "slow-moving-products.md\nProducts with low recent sell-through should be reviewed.",
        )
        .unwrap();

        assert_eq!(items.len(), 1);
        assert!(items[0].id.starts_with("knowledge-"));
        assert_eq!(items[0].knowledge_type.as_str(), "custom");
        assert_eq!(items[0].scope_tables, vec!["fact_sales", "dim_products"]);
    }

    #[test]
    fn parse_semantic_profile_defaults_missing_relationship_source_table() {
        let profile = parse_semantic_profile(
            r#"{
                "description": "Template dimension table.",
                "shortDescription": "Templates",
                "columnDescriptions": {
                    "template_id": "Template identifier"
                },
                "relationships": [{
                    "targetTable": "fact_sales",
                    "relationshipType": "one-to-many",
                    "joinColumns": [{"sourceColumn": "template_id", "targetColumn": "template_id"}],
                    "description": "Templates are referenced by facts."
                }],
                "qualityNotes": []
            }"#,
            "dim_templates",
        )
        .unwrap();

        assert_eq!(profile.relationships.len(), 1);
        assert_eq!(profile.relationships[0].source_table, "dim_templates");
        assert_eq!(profile.relationships[0].target_table, "fact_sales");
        assert_eq!(
            profile.relationships[0].join_columns[0].source_column,
            "template_id"
        );
    }

    #[test]
    fn parse_semantic_profile_accepts_common_join_column_shapes() {
        let profile = parse_semantic_profile(
            r#"{
                "description": "Brand dimension table.",
                "shortDescription": "Brands",
                "columnDescriptions": {
                    "brand_id": "Brand identifier"
                },
                "relationships": [
                    {
                        "sourceTable": "dim_brands",
                        "targetTable": "fact_sales",
                        "relationshipType": "one-to-many",
                        "joinColumns": [{"source": "brand_id", "target": "brand_id"}],
                        "description": "Brands are referenced by facts."
                    },
                    {
                        "sourceTable": "dim_brands",
                        "targetTable": "dim_products",
                        "relationshipType": "one-to-many",
                        "joinColumns": ["brand_id"],
                        "description": "Products belong to brands."
                    }
                ],
                "qualityNotes": []
            }"#,
            "dim_brands",
        )
        .unwrap();

        assert_eq!(profile.relationships.len(), 2);
        assert_eq!(
            profile.relationships[0].join_columns[0].source_column,
            "brand_id"
        );
        assert_eq!(
            profile.relationships[1].join_columns[0].target_column,
            "brand_id"
        );
    }

    #[test]
    fn parse_semantic_profile_tolerates_relationship_variants() {
        let profile = parse_semantic_profile(
            r#"{
                "description": "Brand dimension table.",
                "shortDescription": "Brands",
                "columnDescriptions": {
                    "brand_id": "Brand identifier"
                },
                "relationships": [{
                    "targetTable": "fact_sales",
                    "relationshipType": "many_to_one",
                    "joinColumns": [
                        ["brand_id", "brand_id"],
                        {"source_column": "brand_code", "referencedColumn": "brand_code"},
                        {"foreignKey": "brand_name"},
                        {"bad": true}
                    ],
                    "description": "Facts refer to brands."
                }],
                "qualityNotes": []
            }"#,
            "dim_brands",
        )
        .unwrap();

        assert_eq!(profile.relationships.len(), 1);
        assert_eq!(
            profile.relationships[0].relationship_type,
            RelationshipKind::OneToMany
        );
        assert_eq!(profile.relationships[0].join_columns.len(), 3);
        assert_eq!(
            profile.relationships[0].join_columns[1].target_column,
            "brand_code"
        );
        assert_eq!(
            profile.relationships[0].join_columns[2].source_column,
            "brand_name"
        );
    }

    #[test]
    fn parse_semantic_profile_accepts_string_quality_notes() {
        let profile = parse_semantic_profile(
            r#"{
                "description": "Brand dimension table.",
                "shortDescription": "Brands",
                "columnDescriptions": {
                    "brand_id": "Brand identifier"
                },
                "relationships": [],
                "qualityNotes": [
                    "Table shows 13 distinct brands with no null values across all columns"
                ]
            }"#,
            "dim_brands",
        )
        .unwrap();

        assert_eq!(profile.quality_notes.len(), 1);
        assert_eq!(profile.quality_notes[0].column_name, "*");
        assert_eq!(
            profile.quality_notes[0].notes,
            "Table shows 13 distinct brands with no null values across all columns"
        );
    }
}
