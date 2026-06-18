//! Knowledge items, documents, and metrics.
//!
//! These types represent domain knowledge extracted from documents,
//! business rules, and defined metrics.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Extraction status for documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Pending,
    Processing,
    Processed,
    Failed,
}

/// A document that may contain extractable knowledge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentItem {
    pub id: String,
    pub name: String,
    pub content: String,
    pub target_database_id: Option<String>,
    pub extraction_status: ExtractionStatus,
    pub extracted_knowledge_ids: Vec<String>,
    pub created_at: String,
}

/// Type of knowledge extracted from documents or defined by users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    BusinessRule,
    Predicate,
    Terminology,
    Constraint,
    TemporalRule,
    ImplicitIntent,
    DataQuality,
    Custom,
}

impl KnowledgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BusinessRule => "business_rule",
            Self::Predicate => "predicate",
            Self::Terminology => "terminology",
            Self::Constraint => "constraint",
            Self::TemporalRule => "temporal_rule",
            Self::ImplicitIntent => "implicit_intent",
            Self::DataQuality => "data_quality",
            Self::Custom => "custom",
        }
    }

    /// Parse from a label string (case-insensitive, underscore-tolerant).
    pub fn from_label(label: Option<&str>) -> Self {
        match label.map(|s| s.to_lowercase().replace('-', "_")).as_deref() {
            Some("business_rule") => Self::BusinessRule,
            Some("predicate") => Self::Predicate,
            Some("terminology") => Self::Terminology,
            Some("constraint") => Self::Constraint,
            Some("temporal_rule") => Self::TemporalRule,
            Some("implicit_intent") => Self::ImplicitIntent,
            Some("data_quality") => Self::DataQuality,
            _ => Self::Custom,
        }
    }
}

impl fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A piece of domain knowledge with scope and synonyms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub knowledge_type: KnowledgeType,
    pub scope_tables: Vec<String>,
    pub scope_columns: Vec<String>,
    pub sql_expression: Option<String>,
    pub synonyms: Vec<String>,
    pub source_document_id: Option<String>,
}

/// Raw LLM output for knowledge extraction (nullable fields).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmKnowledgeItem {
    pub name: String,
    pub description: String,
    pub knowledge_type: Option<String>,
    pub scope_tables: Option<Vec<String>>,
    pub scope_columns: Option<Vec<String>>,
    pub sql_expression: Option<String>,
    pub synonyms: Option<Vec<String>>,
}

impl LlmKnowledgeItem {
    /// Convert to a proper KnowledgeItem with generated id.
    pub fn into_knowledge_item(self, id_prefix: &str, index: usize) -> KnowledgeItem {
        KnowledgeItem {
            id: format!("{id_prefix}-{index}"),
            name: self.name,
            description: self.description,
            knowledge_type: KnowledgeType::from_label(self.knowledge_type.as_deref()),
            scope_tables: self.scope_tables.unwrap_or_default(),
            scope_columns: self.scope_columns.unwrap_or_default(),
            sql_expression: self.sql_expression,
            synonyms: self.synonyms.unwrap_or_default(),
            source_document_id: None,
        }
    }
}

/// Aggregation type for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationType {
    Sum,
    Avg,
    Count,
    Min,
    Max,
    CountDistinct,
    #[serde(other)]
    Other,
}

impl AggregationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Count => "count",
            Self::Min => "min",
            Self::Max => "max",
            Self::CountDistinct => "count_distinct",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for AggregationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Output format for a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Numeric,
    Percentage,
    Currency,
    Text,
    #[serde(other)]
    Other,
}

impl OutputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Numeric => "numeric",
            Self::Percentage => "percentage",
            Self::Currency => "currency",
            Self::Text => "text",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A defined metric (KPI / measure) with formula and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricItem {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub formula: String,
    pub formula_description: String,
    pub source_tables: Vec<String>,
    pub source_columns: Vec<String>,
    pub aggregation_type: AggregationType,
    pub time_grain: Option<String>,
    pub output_type: OutputFormat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_type_from_label() {
        assert_eq!(
            KnowledgeType::from_label(Some("business_rule")),
            KnowledgeType::BusinessRule
        );
        assert_eq!(
            KnowledgeType::from_label(Some("Business-Rule")),
            KnowledgeType::BusinessRule
        );
        assert_eq!(
            KnowledgeType::from_label(Some("unknown")),
            KnowledgeType::Custom
        );
        assert_eq!(KnowledgeType::from_label(None), KnowledgeType::Custom);
    }

    #[test]
    fn llm_knowledge_item_conversion() {
        let raw = LlmKnowledgeItem {
            name: "Active customers".into(),
            description: "Customers with orders in last 90 days".into(),
            knowledge_type: Some("predicate".into()),
            scope_tables: Some(vec!["customers".into()]),
            scope_columns: None,
            sql_expression: Some("last_order_date > NOW() - INTERVAL '90 days'".into()),
            synonyms: Some(vec!["recent customers".into()]),
        };

        let item = raw.into_knowledge_item("doc-1", 0);
        assert_eq!(item.id, "doc-1-0");
        assert_eq!(item.knowledge_type, KnowledgeType::Predicate);
        assert_eq!(item.scope_columns, Vec::<String>::new());
    }

    #[test]
    fn metric_item_serde() {
        let metric = MetricItem {
            id: "m-001".into(),
            name: "revenue".into(),
            display_name: "Total Revenue".into(),
            formula: "SUM(amount)".into(),
            formula_description: "Sum of all order amounts".into(),
            source_tables: vec!["orders".into()],
            source_columns: vec!["amount".into()],
            aggregation_type: AggregationType::Sum,
            time_grain: Some("monthly".into()),
            output_type: OutputFormat::Currency,
        };

        let json = serde_json::to_string(&metric).unwrap();
        let parsed: MetricItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.aggregation_type, AggregationType::Sum);
        assert_eq!(parsed.output_type, OutputFormat::Currency);
    }

    #[test]
    fn aggregation_type_other_fallback() {
        let json = "\"unknown_agg\"";
        let parsed: AggregationType = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, AggregationType::Other);
    }
}
