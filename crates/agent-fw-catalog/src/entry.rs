//! CatalogEntry — the core unit of the data catalog.
//!
//! Everything in the catalog (tables, columns, relationships, metrics, knowledge)
//! is represented as a `CatalogEntry` with a `CatalogKind` discriminator.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Error type for catalog operations.
#[derive(Debug, Clone, Error)]
pub enum CatalogError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Catalog unavailable: {0}")]
    Unavailable(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),
}

/// The kind of item in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKind {
    Table,
    Column,
    Relationship,
    Enum,
    Metric,
    Special,
    Document,
    Knowledge,
    #[serde(rename = "data_quality_finding")]
    DataQualityFinding,
}

impl CatalogKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Column => "column",
            Self::Relationship => "relationship",
            Self::Enum => "enum",
            Self::Metric => "metric",
            Self::Special => "special",
            Self::Document => "document",
            Self::Knowledge => "knowledge",
            Self::DataQualityFinding => "data_quality_finding",
        }
    }

    /// Agent-facing stable kind name.
    ///
    /// `CatalogKind::Enum` remains the storage enum variant, but the public
    /// catalog tool contract calls those entities `enum_value`.
    pub fn public_name(&self) -> &'static str {
        match self {
            Self::Enum => "enum_value",
            _ => self.as_str(),
        }
    }

    /// Whether this kind is currently discoverable through the public catalog
    /// search surface.
    pub fn is_public_searchable(&self) -> bool {
        !matches!(self, Self::Special)
    }
}

impl fmt::Display for CatalogKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CatalogKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "column" => Ok(Self::Column),
            "relationship" => Ok(Self::Relationship),
            "enum" => Ok(Self::Enum),
            "metric" => Ok(Self::Metric),
            "special" => Ok(Self::Special),
            "document" => Ok(Self::Document),
            "knowledge" => Ok(Self::Knowledge),
            "data_quality_finding" | "dataqualityfinding" => Ok(Self::DataQualityFinding),
            _ => Err(format!("Unknown CatalogKind: {s}")),
        }
    }
}

/// Backwards-compatible alias.
pub type CatalogEntryKind = CatalogKind;

/// A single item in the data catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    #[serde(rename = "itemType")]
    pub kind: CatalogKind,
    pub name: String,
    pub qualified_name: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    #[serde(rename = "related")]
    pub links: Vec<CatalogRelation>,
    pub metadata: serde_json::Value,
}

/// A directional relationship between catalog entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRelation {
    pub target_id: String,
    #[serde(rename = "relationType")]
    pub kind: String,
    pub description: Option<String>,
}

/// A join path between two tables via catalog entries.
///
/// `steps` includes the `from` table as `steps[0]` and `length == steps.len()`
/// (legacy shape, unchanged for backward compatibility). The parallel `hops`
/// vector carries typed join metadata for each edge traversed: invariant
/// `hops.len() == steps.len().saturating_sub(1)`, and `hops[i]` describes the
/// edge from `steps[i]` to `steps[i + 1]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinPath {
    pub steps: Vec<CatalogEntry>,
    pub length: usize,
    /// Typed metadata for each hop. `hops[i]` describes the edge
    /// `steps[i] -> steps[i + 1]`. Defaulted for backward-compatible
    /// deserialization of legacy `JoinPath` values that predate this field.
    #[serde(default)]
    pub hops: Vec<JoinHop>,
}

/// Typed metadata describing a single hop (edge) of a [`JoinPath`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinHop {
    /// Relation kind for this hop (e.g. `references_table`).
    pub relation_kind: String,
    /// Source/target join columns and join type, when a relationship vertex was found.
    pub from_column: Option<String>,
    pub to_column: Option<String>,
    pub join_type: Option<String>,
    /// Human-readable description (mirrors the materialized link description).
    pub description: Option<String>,
    /// The relationship vertex id that justified the hop, when known.
    pub relationship_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_kind_roundtrip() {
        for kind in [
            CatalogKind::Table,
            CatalogKind::Column,
            CatalogKind::Relationship,
            CatalogKind::Enum,
            CatalogKind::Metric,
            CatalogKind::Special,
            CatalogKind::Document,
            CatalogKind::Knowledge,
            CatalogKind::DataQualityFinding,
        ] {
            let s = kind.as_str();
            let parsed: CatalogKind = s.parse().unwrap();
            assert_eq!(kind, parsed);

            let json = serde_json::to_string(&kind).unwrap();
            let deser: CatalogKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, deser);
        }
    }

    #[test]
    fn catalog_entry_serde() {
        let entry = CatalogEntry {
            id: "tbl-001".into(),
            kind: CatalogKind::Table,
            name: "users".into(),
            qualified_name: Some("public.users".into()),
            content: "User accounts table".into(),
            tags: vec!["core".into()],
            links: vec![CatalogRelation {
                target_id: "col-001".into(),
                kind: "has_column".into(),
                description: None,
            }],
            metadata: serde_json::json!({"row_count": 1000}),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"itemType\":\"table\""));
        assert!(json.contains("\"related\""));

        let parsed: CatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "tbl-001");
        assert_eq!(parsed.kind, CatalogKind::Table);
    }
}
