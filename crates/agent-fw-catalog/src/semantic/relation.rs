use serde::{Deserialize, Serialize};

/// Cardinality used by relationship metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cardinality {
    One,
    Many,
    Unknown,
}

/// Controlled relation vocabulary for warehouse and ontology-lite edges.
pub mod relation_kind {
    pub const HAS_COLUMN: &str = "has_column";
    pub const BELONGS_TO: &str = "belongs_to";
    pub const REFERENCES: &str = "references";
    pub const REFERENCED_BY: &str = "referenced_by";
    pub const REFERENCES_TABLE: &str = "references_table";
    pub const REFERENCED_BY_TABLE: &str = "referenced_by_table";
    pub const RELATIONSHIP_SOURCE_TABLE: &str = "relationship_source_table";
    pub const RELATIONSHIP_TARGET_TABLE: &str = "relationship_target_table";
    pub const ENUM_VALUE_OF: &str = "enum_value_of";
    pub const HAS_ENUM_VALUE: &str = "has_enum_value";
    pub const METRIC_USES: &str = "metric_uses";
    pub const KNOWLEDGE_APPLIES_TO: &str = "knowledge_applies_to";
    pub const DATA_QUALITY_FINDING_APPLIES_TO: &str = "data_quality_finding_applies_to";
    pub const EXTRACTED_FROM: &str = "extracted_from";
    pub const SYNONYM_OF: &str = "synonym_of";
    pub const EQUIVALENT_TO: &str = "equivalent_to";
    pub const SUB_CLASS_OF: &str = "sub_class_of";
    pub const APPLIES_TO: &str = "applies_to";
}
