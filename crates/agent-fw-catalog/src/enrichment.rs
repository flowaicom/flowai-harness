//! Semantic enrichment — LLM-based table/column descriptions and knowledge extraction.
//!
//! The `SemanticEnricher` trait abstracts LLM calls for generating semantic
//! descriptions from physical schema + profiling data.
//!
//! # Laws
//!
//! L1 (Non-empty): `enrich_table(r).profile.description.len() > 0`
//! L2 (Cancellation): Cancelled token yields `Err(Cancelled)`
//! L3 (Idempotent structure): Same schema shape → structurally consistent output
//! L4 (Source fidelity): `CachedEnricher` returns `Cached`, direct returns `Fresh`

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

use crate::discovery::PhysicalTable;
use crate::knowledge::KnowledgeItem;
use crate::profiling::TableProfile;

/// Enrichment error.
#[derive(Debug, Clone, Error)]
pub enum EnrichmentError {
    #[error("LLM call failed: {0}")]
    LlmFailed(String),

    #[error("Failed to parse LLM response: {0}")]
    ParseFailed(String),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("LLM call timed out after {duration_ms}ms")]
    Timeout { duration_ms: u64 },

    #[error("Operation cancelled")]
    Cancelled,
}

impl EnrichmentError {
    /// Whether this error is transient and the operation should be retried.
    ///
    /// Retryable: `RateLimited`, `LlmFailed`, `ParseFailed` (LLM non-determinism).
    /// Non-retryable: `Cancelled` (intentional), `Timeout` (structurally likely to recur).
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::LlmFailed(_) | Self::ParseFailed(_) => true,
            Self::Cancelled | Self::Timeout { .. } => false,
        }
    }
}

/// Provenance of an enrichment result.
///
/// Forms a lattice: Fresh ⊔ Cached = Cached, * ⊔ Fallback = Fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnrichmentSource {
    Fresh,
    Cached,
    Fallback,
}

/// Result of enriching a table.
#[derive(Debug, Clone)]
pub struct EnrichmentResult {
    pub profile: SemanticTableProfile,
    pub source: EnrichmentSource,
    pub model_id: Option<String>,
    pub fallback_reason: Option<String>,
}

impl EnrichmentResult {
    pub fn fresh(profile: SemanticTableProfile) -> Self {
        Self {
            profile,
            source: EnrichmentSource::Fresh,
            model_id: None,
            fallback_reason: None,
        }
    }

    pub fn cached(profile: SemanticTableProfile) -> Self {
        Self {
            profile,
            source: EnrichmentSource::Cached,
            model_id: None,
            fallback_reason: None,
        }
    }

    pub fn fallback(profile: SemanticTableProfile) -> Self {
        Self {
            profile,
            source: EnrichmentSource::Fallback,
            model_id: None,
            fallback_reason: None,
        }
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_fallback_reason(mut self, reason: impl Into<String>) -> Self {
        self.fallback_reason = Some(reason.into());
        self
    }
}

/// Column name → description mapping (sorted for determinism).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ColumnDescriptions(BTreeMap<String, String>);

impl ColumnDescriptions {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.0.insert(key, value);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

/// Kind of inferred relationship between tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationshipKind {
    #[serde(rename = "one-to-many")]
    OneToMany,
    #[serde(rename = "many-to-many")]
    ManyToMany,
    #[serde(rename = "one-to-one")]
    OneToOne,
}

impl RelationshipKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OneToMany => "one-to-many",
            Self::ManyToMany => "many-to-many",
            Self::OneToOne => "one-to-one",
        }
    }
}

impl fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A pair of join columns (serializes as `[source, target]` tuple for backwards compat).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "(String, String)", into = "(String, String)")]
pub struct JoinPair {
    pub source_column: String,
    pub target_column: String,
}

impl From<(String, String)> for JoinPair {
    fn from((source, target): (String, String)) -> Self {
        Self {
            source_column: source,
            target_column: target,
        }
    }
}

impl From<JoinPair> for (String, String) {
    fn from(pair: JoinPair) -> Self {
        (pair.source_column, pair.target_column)
    }
}

/// An LLM-inferred relationship between tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferredRelationship {
    pub source_table: String,
    pub target_table: String,
    pub relationship_type: RelationshipKind,
    pub join_columns: Vec<JoinPair>,
    pub description: String,
}

/// A quality note about a column (validation rules, typical ranges).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityNote {
    pub column_name: String,
    pub notes: String,
    pub typical_value_range: Option<String>,
    pub validation_rules: Vec<String>,
}

/// LLM-generated semantic profile for a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTableProfile {
    pub description: String,
    pub short_description: String,
    pub column_descriptions: ColumnDescriptions,
    pub relationships: Vec<InferredRelationship>,
    pub quality_notes: Vec<QualityNote>,
}

/// Cached enrichment entry (for content-addressed caching).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedEnrichmentEntry {
    pub profile: SemanticTableProfile,
    pub source: EnrichmentSource,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub fallback_reason: Option<String>,
}

/// Request to enrich a table with semantic descriptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableEnrichmentRequest {
    pub table: PhysicalTable,
    pub sample_rows: Vec<serde_json::Value>,
    pub profile: TableProfile,
    pub database_context: Option<String>,
    /// Foreign key edges involving this table (inbound and outbound).
    ///
    /// Provides the LLM with cross-table context so it can generate
    /// more accurate descriptions (e.g., "this column references the
    /// primary key of the customers table").
    #[serde(default)]
    pub fk_edges: Vec<crate::discovery::ForeignKeyEdge>,
}

/// Request to extract knowledge from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeExtractionRequest {
    pub document_content: String,
    pub document_name: String,
    pub database_context: Option<String>,
    pub available_tables: Vec<String>,
    pub available_columns: Vec<String>,
}

/// Async LLM-based semantic enrichment.
#[async_trait]
pub trait SemanticEnricher: Send + Sync {
    /// Enrich a table with LLM-generated descriptions.
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError>;

    /// Extract knowledge items from a document.
    async fn extract_knowledge(
        &self,
        request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrichment_source_serde() {
        let json = serde_json::to_string(&EnrichmentSource::Fresh).unwrap();
        assert_eq!(json, "\"fresh\"");
        let parsed: EnrichmentSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, EnrichmentSource::Fresh);
    }

    #[test]
    fn column_descriptions_operations() {
        let mut cd = ColumnDescriptions::new();
        assert!(cd.is_empty());

        cd.insert("id".into(), "Primary key".into());
        cd.insert("name".into(), "User name".into());
        assert_eq!(cd.len(), 2);
        assert_eq!(cd.get("id"), Some(&"Primary key".to_string()));
    }

    #[test]
    fn join_pair_tuple_serde() {
        let pair = JoinPair {
            source_column: "user_id".into(),
            target_column: "id".into(),
        };
        let json = serde_json::to_string(&pair).unwrap();
        // Should serialize as ["user_id","id"]
        assert_eq!(json, "[\"user_id\",\"id\"]");

        let parsed: JoinPair = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source_column, "user_id");
        assert_eq!(parsed.target_column, "id");
    }

    #[test]
    fn semantic_table_profile_serde() {
        let profile = SemanticTableProfile {
            description: "Stores user accounts".into(),
            short_description: "User accounts".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };
        let json = serde_json::to_string(&profile).unwrap();
        let parsed: SemanticTableProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.description, "Stores user accounts");
    }

    #[test]
    fn enrichment_result_constructors() {
        let profile = SemanticTableProfile {
            description: "test".into(),
            short_description: "t".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };

        let fresh = EnrichmentResult::fresh(profile.clone());
        assert_eq!(fresh.source, EnrichmentSource::Fresh);

        let cached = EnrichmentResult::cached(profile.clone());
        assert_eq!(cached.source, EnrichmentSource::Cached);

        let fallback = EnrichmentResult::fallback(profile);
        assert_eq!(fallback.source, EnrichmentSource::Fallback);
    }
}
