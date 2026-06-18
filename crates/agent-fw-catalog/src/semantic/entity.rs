use serde::{Deserialize, Serialize};

use super::metadata::{
    dedupe_nonempty, ColumnMetadata, DataQualityFindingMetadata, DocumentMetadata,
    EnumValueMetadata, KnowledgeMetadata, MetricMetadata, RelationshipMetadata, TableMetadata,
};
use crate::{CatalogEntry, CatalogKind};

/// Built-in ontology-lite semantic entity kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEntityKind {
    Table,
    Column,
    Relationship,
    EnumValue,
    Metric,
    Special,
    Document,
    Knowledge,
    DataQualityFinding,
}

impl From<CatalogKind> for SemanticEntityKind {
    fn from(kind: CatalogKind) -> Self {
        match kind {
            CatalogKind::Table => Self::Table,
            CatalogKind::Column => Self::Column,
            CatalogKind::Relationship => Self::Relationship,
            CatalogKind::Enum => Self::EnumValue,
            CatalogKind::Metric => Self::Metric,
            CatalogKind::Special => Self::Special,
            CatalogKind::Document => Self::Document,
            CatalogKind::Knowledge => Self::Knowledge,
            CatalogKind::DataQualityFinding => Self::DataQualityFinding,
        }
    }
}

impl SemanticEntityKind {
    /// Agent-facing stable kind name.
    pub fn public_name(&self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Column => "column",
            Self::Relationship => "relationship",
            Self::EnumValue => "enum_value",
            Self::Metric => "metric",
            Self::Special => "special",
            Self::Document => "document",
            Self::Knowledge => "knowledge",
            Self::DataQualityFinding => "data_quality_finding",
        }
    }

    /// Whether this kind is currently discoverable through the public catalog
    /// search surface.
    pub fn is_public_searchable(&self) -> bool {
        !matches!(self, Self::Special)
    }
}

impl From<SemanticEntityKind> for CatalogKind {
    fn from(kind: SemanticEntityKind) -> Self {
        match kind {
            SemanticEntityKind::Table => Self::Table,
            SemanticEntityKind::Column => Self::Column,
            SemanticEntityKind::Relationship => Self::Relationship,
            SemanticEntityKind::EnumValue => Self::Enum,
            SemanticEntityKind::Metric => Self::Metric,
            SemanticEntityKind::Special => Self::Special,
            SemanticEntityKind::Document => Self::Document,
            SemanticEntityKind::Knowledge => Self::Knowledge,
            SemanticEntityKind::DataQualityFinding => Self::DataQualityFinding,
        }
    }
}

/// Typed semantic interpretation of a catalog entry.
#[derive(Debug, Clone)]
pub enum SemanticEntity {
    Table {
        entry: CatalogEntry,
        metadata: TableMetadata,
    },
    Column {
        entry: CatalogEntry,
        metadata: ColumnMetadata,
    },
    Relationship {
        entry: CatalogEntry,
        metadata: RelationshipMetadata,
    },
    EnumValue {
        entry: CatalogEntry,
        metadata: EnumValueMetadata,
    },
    Metric {
        entry: CatalogEntry,
        metadata: MetricMetadata,
    },
    Special {
        entry: CatalogEntry,
    },
    Document {
        entry: CatalogEntry,
        metadata: DocumentMetadata,
    },
    Knowledge {
        entry: CatalogEntry,
        metadata: KnowledgeMetadata,
    },
    DataQualityFinding {
        entry: CatalogEntry,
        metadata: DataQualityFindingMetadata,
    },
}

impl SemanticEntity {
    pub fn kind(&self) -> SemanticEntityKind {
        match self {
            Self::Table { .. } => SemanticEntityKind::Table,
            Self::Column { .. } => SemanticEntityKind::Column,
            Self::Relationship { .. } => SemanticEntityKind::Relationship,
            Self::EnumValue { .. } => SemanticEntityKind::EnumValue,
            Self::Metric { .. } => SemanticEntityKind::Metric,
            Self::Special { .. } => SemanticEntityKind::Special,
            Self::Document { .. } => SemanticEntityKind::Document,
            Self::Knowledge { .. } => SemanticEntityKind::Knowledge,
            Self::DataQualityFinding { .. } => SemanticEntityKind::DataQualityFinding,
        }
    }

    pub fn entry(&self) -> &CatalogEntry {
        match self {
            Self::Table { entry, .. }
            | Self::Column { entry, .. }
            | Self::Relationship { entry, .. }
            | Self::EnumValue { entry, .. }
            | Self::Metric { entry, .. }
            | Self::Special { entry }
            | Self::Document { entry, .. }
            | Self::Knowledge { entry, .. }
            | Self::DataQualityFinding { entry, .. } => entry,
        }
    }

    pub fn into_entry(self) -> CatalogEntry {
        match self {
            Self::Table { entry, .. }
            | Self::Column { entry, .. }
            | Self::Relationship { entry, .. }
            | Self::EnumValue { entry, .. }
            | Self::Metric { entry, .. }
            | Self::Special { entry }
            | Self::Document { entry, .. }
            | Self::Knowledge { entry, .. }
            | Self::DataQualityFinding { entry, .. } => entry,
        }
    }

    /// Synonyms from typed semantic metadata only.
    pub fn typed_synonyms(&self) -> Vec<String> {
        let synonyms = match self {
            Self::EnumValue { metadata, .. } => metadata.synonyms.clone(),
            Self::Metric { metadata, .. } => metadata.synonyms.clone(),
            Self::Knowledge { metadata, .. } => metadata.synonyms.clone(),
            _ => Vec::new(),
        };
        dedupe_nonempty(synonyms)
    }
}
