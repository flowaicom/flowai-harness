//! Column profiling types — statistical profiles of table columns.
//!
//! These types are produced by the profiling service and consumed by the
//! semantic enricher for LLM-based enrichment.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Semantic type classification for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SemanticType {
    Numeric,
    Categorical,
    Text,
    Temporal,
    Identifier,
    Json,
    Array,
    Binary,
    Geographic,
    Monetary,
    Unknown,
}

impl std::fmt::Display for SemanticType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numeric => write!(f, "numeric"),
            Self::Categorical => write!(f, "categorical"),
            Self::Text => write!(f, "text"),
            Self::Temporal => write!(f, "temporal"),
            Self::Identifier => write!(f, "identifier"),
            Self::Json => write!(f, "json"),
            Self::Array => write!(f, "array"),
            Self::Binary => write!(f, "binary"),
            Self::Geographic => write!(f, "geographic"),
            Self::Monetary => write!(f, "monetary"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Type-specific column statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TypeSpecificStats {
    #[serde(rename_all = "camelCase")]
    Numeric {
        min: Option<f64>,
        max: Option<f64>,
        mean: Option<f64>,
        p25: Option<f64>,
        p50: Option<f64>,
        p75: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Categorical {
        top_values: Vec<CategoryValue>,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        max_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_length: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        avg_length: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detected_pattern: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Temporal {
        min_time: Option<String>,
        max_time: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Json {
        type_distribution: HashMap<String, i64>,
        top_keys: Vec<String>,
    },
    Unprofilable,
}

/// A category value with its frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryValue {
    pub value: String,
    pub count: i64,
    pub percentage: f64,
}

/// Statistical profile for a single column.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProfile {
    pub column_name: String,
    pub data_type: String,
    pub null_count: i64,
    pub distinct_count: i64,
    pub total_count: i64,
    pub semantic_type: SemanticType,
    pub stats: TypeSpecificStats,
}

/// Statistical profile for a table (all columns).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableProfile {
    pub table_name: String,
    pub columns: Vec<ColumnProfile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_specific_stats_tagged_serde() {
        let numeric = TypeSpecificStats::Numeric {
            min: Some(0.0),
            max: Some(100.0),
            mean: Some(50.0),
            p25: Some(25.0),
            p50: Some(50.0),
            p75: Some(75.0),
        };
        let json = serde_json::to_string(&numeric).unwrap();
        assert!(json.contains("\"type\":\"numeric\""));

        let parsed: TypeSpecificStats = serde_json::from_str(&json).unwrap();
        match parsed {
            TypeSpecificStats::Numeric { min, max, .. } => {
                assert_eq!(min, Some(0.0));
                assert_eq!(max, Some(100.0));
            }
            _ => panic!("Expected Numeric"),
        }
    }

    #[test]
    fn categorical_stats_serde() {
        let cat = TypeSpecificStats::Categorical {
            top_values: vec![
                CategoryValue {
                    value: "Electronics".into(),
                    count: 500,
                    percentage: 0.35,
                },
                CategoryValue {
                    value: "Clothing".into(),
                    count: 300,
                    percentage: 0.21,
                },
            ],
        };
        let json = serde_json::to_string(&cat).unwrap();
        let parsed: TypeSpecificStats = serde_json::from_str(&json).unwrap();
        match parsed {
            TypeSpecificStats::Categorical { top_values } => {
                assert_eq!(top_values.len(), 2);
            }
            _ => panic!("Expected Categorical"),
        }
    }

    #[test]
    fn table_profile_serde() {
        let profile = TableProfile {
            table_name: "orders".into(),
            columns: vec![ColumnProfile {
                column_name: "amount".into(),
                data_type: "numeric".into(),
                null_count: 0,
                distinct_count: 950,
                total_count: 1000,
                semantic_type: SemanticType::Monetary,
                stats: TypeSpecificStats::Numeric {
                    min: Some(1.0),
                    max: Some(9999.99),
                    mean: Some(150.0),
                    p25: Some(30.0),
                    p50: Some(80.0),
                    p75: Some(200.0),
                },
            }],
        };
        let json = serde_json::to_string(&profile).unwrap();
        let parsed: TableProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.columns[0].semantic_type, SemanticType::Monetary);
    }

    #[test]
    fn text_stats_omit_absent_optional_fields() {
        let stats = TypeSpecificStats::Text {
            max_length: Some(64),
            min_length: None,
            avg_length: None,
            detected_pattern: None,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"maxLength\":64"));
        assert!(!json.contains("\"minLength\""));
        assert!(!json.contains("\"avgLength\""));
        assert!(!json.contains("\"detectedPattern\""));
    }

    #[test]
    fn semantic_type_display_matches_serde() {
        // Display output must match serde rename_all = "lowercase"
        assert_eq!(SemanticType::Numeric.to_string(), "numeric");
        assert_eq!(SemanticType::Categorical.to_string(), "categorical");
        assert_eq!(SemanticType::Text.to_string(), "text");
        assert_eq!(SemanticType::Temporal.to_string(), "temporal");
        assert_eq!(SemanticType::Identifier.to_string(), "identifier");
        assert_eq!(SemanticType::Json.to_string(), "json");
        assert_eq!(SemanticType::Array.to_string(), "array");
        assert_eq!(SemanticType::Binary.to_string(), "binary");
        assert_eq!(SemanticType::Geographic.to_string(), "geographic");
        assert_eq!(SemanticType::Monetary.to_string(), "monetary");
        assert_eq!(SemanticType::Unknown.to_string(), "unknown");
    }
}
