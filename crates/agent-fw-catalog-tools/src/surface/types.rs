use agent_fw_catalog::{
    CatalogEntry, CatalogKind, CatalogSearchFilters, CatalogSearchHitRef, SemanticEntityKind,
};
use serde::{Deserialize, Serialize};

/// Agent-facing catalog entity kind names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEntityKind {
    Table,
    Column,
    Relationship,
    EnumValue,
    Metric,
    Knowledge,
    Document,
    DataQualityFinding,
    Special,
}

impl CatalogEntityKind {
    pub const PUBLIC_SEARCHABLE: [Self; 8] = [
        Self::Table,
        Self::Column,
        Self::Relationship,
        Self::EnumValue,
        Self::Metric,
        Self::Knowledge,
        Self::Document,
        Self::DataQualityFinding,
    ];

    pub const ALL_NAMES: [&'static str; 9] = [
        "table",
        "column",
        "relationship",
        "enum_value",
        "metric",
        "knowledge",
        "document",
        "data_quality_finding",
        "special",
    ];

    pub fn public_name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Column => "column",
            Self::Relationship => "relationship",
            Self::EnumValue => "enum_value",
            Self::Metric => "metric",
            Self::Knowledge => "knowledge",
            Self::Document => "document",
            Self::DataQualityFinding => "data_quality_finding",
            Self::Special => "special",
        }
    }

    pub fn is_public_searchable(self) -> bool {
        !matches!(self, Self::Special)
    }
}

impl From<CatalogKind> for CatalogEntityKind {
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

impl From<SemanticEntityKind> for CatalogEntityKind {
    fn from(kind: SemanticEntityKind) -> Self {
        match kind {
            SemanticEntityKind::Table => Self::Table,
            SemanticEntityKind::Column => Self::Column,
            SemanticEntityKind::Relationship => Self::Relationship,
            SemanticEntityKind::EnumValue => Self::EnumValue,
            SemanticEntityKind::Metric => Self::Metric,
            SemanticEntityKind::Special => Self::Special,
            SemanticEntityKind::Document => Self::Document,
            SemanticEntityKind::Knowledge => Self::Knowledge,
            SemanticEntityKind::DataQualityFinding => Self::DataQualityFinding,
        }
    }
}

impl From<CatalogEntityKind> for CatalogKind {
    fn from(kind: CatalogEntityKind) -> Self {
        match kind {
            CatalogEntityKind::Table => Self::Table,
            CatalogEntityKind::Column => Self::Column,
            CatalogEntityKind::Relationship => Self::Relationship,
            CatalogEntityKind::EnumValue => Self::Enum,
            CatalogEntityKind::Metric => Self::Metric,
            CatalogEntityKind::Special => Self::Special,
            CatalogEntityKind::Document => Self::Document,
            CatalogEntityKind::Knowledge => Self::Knowledge,
            CatalogEntityKind::DataQualityFinding => Self::DataQualityFinding,
        }
    }
}

impl From<CatalogEntityKind> for SemanticEntityKind {
    fn from(kind: CatalogEntityKind) -> Self {
        match kind {
            CatalogEntityKind::Table => Self::Table,
            CatalogEntityKind::Column => Self::Column,
            CatalogEntityKind::Relationship => Self::Relationship,
            CatalogEntityKind::EnumValue => Self::EnumValue,
            CatalogEntityKind::Metric => Self::Metric,
            CatalogEntityKind::Special => Self::Special,
            CatalogEntityKind::Document => Self::Document,
            CatalogEntityKind::Knowledge => Self::Knowledge,
            CatalogEntityKind::DataQualityFinding => Self::DataQualityFinding,
        }
    }
}

/// Agent-facing reference to a catalog entity.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CatalogEntityKind>,
}

impl CatalogRef {
    pub fn id(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            ..Self::default()
        }
    }

    pub fn canonical(entry: &CatalogEntry) -> Self {
        let kind = Some(CatalogEntityKind::from(entry.kind));
        if let Some(qualified_name) = entry
            .qualified_name
            .as_ref()
            .filter(|qualified_name| !qualified_name.trim().is_empty())
        {
            Self {
                qualified_name: Some(qualified_name.clone()),
                kind,
                ..Self::default()
            }
        } else {
            Self {
                id: Some(entry.id.clone()),
                kind,
                ..Self::default()
            }
        }
    }

    pub fn from_filter_hint(input: impl Into<String>, default_kind: CatalogEntityKind) -> Self {
        let input = input.into();
        let trimmed = input.trim();
        if is_direct_catalog_id(trimmed) {
            Self {
                id: Some(trimmed.to_string()),
                kind: Some(default_kind),
                ..Self::default()
            }
        } else if trimmed.contains('.') {
            Self {
                qualified_name: Some(trimmed.to_string()),
                kind: Some(default_kind),
                ..Self::default()
            }
        } else {
            Self {
                name: Some(trimmed.to_string()),
                kind: Some(default_kind),
                ..Self::default()
            }
        }
    }

    pub fn provided_reference_count(&self) -> usize {
        usize::from(
            self.id
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
        ) + usize::from(
            self.qualified_name
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
        ) + usize::from(
            self.name
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
        )
    }

    pub fn display_input(&self) -> String {
        if let Some(id) = &self.id {
            return id.clone();
        }
        if let Some(qualified_name) = &self.qualified_name {
            return qualified_name.clone();
        }
        if let Some(name) = &self.name {
            return name.clone();
        }
        "<empty-ref>".to_string()
    }
}

fn is_direct_catalog_id(input: &str) -> bool {
    (input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()))
        || input.starts_with("table:")
        || input.starts_with("column:")
        || input.starts_with("enum:")
        || input.starts_with("relationship:")
        || input.starts_with("metric:")
        || input.starts_with("knowledge:")
        || input.starts_with("document:")
        || input.starts_with("data_quality_finding:")
}

/// Reference used inside filters. Strings are canonicalized by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CatalogFilterRef {
    Ref(CatalogRef),
    String(String),
}

impl CatalogFilterRef {
    pub fn to_catalog_ref(&self, default_kind: CatalogEntityKind) -> CatalogRef {
        match self {
            Self::Ref(reference) => {
                let mut reference = reference.clone();
                if reference.kind.is_none() {
                    reference.kind = Some(default_kind);
                }
                reference
            }
            Self::String(input) => CatalogRef::from_filter_hint(input, default_kind),
        }
    }

    pub fn display_input(&self) -> serde_json::Value {
        match self {
            Self::Ref(reference) => serde_json::to_value(reference).unwrap_or_default(),
            Self::String(input) => serde_json::Value::String(input.clone()),
        }
    }
}

/// Agent-facing filter input.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_table: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_table: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_column: Option<CatalogFilterRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_query_surface: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_cardinality_enum: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolvedCatalogRef {
    pub id: String,
    pub kind: CatalogEntityKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
}

impl ResolvedCatalogRef {
    pub fn from_entry(entry: &CatalogEntry) -> Self {
        Self {
            id: entry.id.clone(),
            kind: CatalogEntityKind::from(entry.kind),
            name: entry.name.clone(),
            qualified_name: entry.qualified_name.clone(),
        }
    }

    pub fn catalog_ref(&self) -> CatalogRef {
        if let Some(qualified_name) = self
            .qualified_name
            .as_ref()
            .filter(|qualified_name| !qualified_name.trim().is_empty())
        {
            CatalogRef {
                qualified_name: Some(qualified_name.clone()),
                kind: Some(self.kind),
                ..CatalogRef::default()
            }
        } else {
            CatalogRef {
                id: Some(self.id.clone()),
                kind: Some(self.kind),
                ..CatalogRef::default()
            }
        }
    }

    pub fn canonical_filter_value(&self) -> String {
        self.qualified_name
            .clone()
            .unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogRefResolution {
    pub input: CatalogRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<ResolvedCatalogRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ambiguous: Vec<ResolvedCatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<String>,
}

impl CatalogRefResolution {
    pub fn resolved(input: CatalogRef, resolved: ResolvedCatalogRef) -> Self {
        Self {
            input,
            resolved: Some(resolved),
            ambiguous: Vec::new(),
            unresolved: None,
        }
    }

    pub fn ambiguous(input: CatalogRef, candidates: Vec<ResolvedCatalogRef>) -> Self {
        Self {
            input,
            resolved: None,
            ambiguous: candidates,
            unresolved: None,
        }
    }

    pub fn unresolved(input: CatalogRef, reason: impl Into<String>) -> Self {
        Self {
            input,
            resolved: None,
            ambiguous: Vec::new(),
            unresolved: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CanonicalizedFilterValue {
    pub field: String,
    pub input: serde_json::Value,
    pub canonical: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AmbiguousFilterValue {
    pub field: String,
    pub input: serde_json::Value,
    pub candidates: Vec<CatalogRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UnresolvedFilterValue {
    pub field: String,
    pub input: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FilterResolution {
    #[serde(default)]
    pub applied: CatalogFilters,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonicalized: Vec<CanonicalizedFilterValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ambiguous: Vec<AmbiguousFilterValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<UnresolvedFilterValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolvedCatalogFilters {
    pub backend_filters: CatalogSearchFilters,
    pub resolution: FilterResolution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MatchDiagnostics {
    pub score: f64,
    pub rank: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_fields: Vec<String>,
    pub score_interpretation: String,
}

impl From<&CatalogSearchHitRef> for MatchDiagnostics {
    fn from(hit: &CatalogSearchHitRef) -> Self {
        Self {
            score: hit.score,
            rank: hit.rank,
            match_signals: hit.match_signals.clone(),
            matched_fields: hit.matched_fields.clone(),
            score_interpretation: "response_local_relevance".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogEntity {
    pub id: String,
    pub kind: CatalogEntityKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub details: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<CatalogRef>,
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub match_diagnostics: Option<MatchDiagnostics>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogEntityAssembly {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<CatalogEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Pagination {
    pub limit: usize,
    pub returned: usize,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchCatalogFacets {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<FacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<FacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<FacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<FacetValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchCatalogDiagnostics {
    pub search_mode: String,
    pub backend: String,
    pub hydrated_count: usize,
    pub candidate_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_by_recheck: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_trips: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchCatalogInput {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<CatalogEntityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<CatalogFilters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchCatalogOutput {
    pub query: String,
    pub results: Vec<CatalogEntity>,
    pub facets: SearchCatalogFacets,
    pub suggested_filters: Vec<CatalogFilters>,
    pub filter_resolution: FilterResolution,
    pub pagination: Pagination,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub diagnostics: SearchCatalogDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetCatalogEntitiesInput {
    pub refs: Vec<CatalogRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetCatalogEntitiesOutput {
    pub entities: Vec<CatalogEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<CatalogRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListSchemaFieldsInput {
    pub tables: Vec<CatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<CatalogFilters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_per_table: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SchemaFieldsForTable {
    pub table: CatalogEntity,
    pub fields: Vec<CatalogEntity>,
    pub pagination: Pagination,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListSchemaFieldsOutput {
    pub tables: Vec<SchemaFieldsForTable>,
    pub filter_resolution: FilterResolution,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    Outgoing,
    Incoming,
    Both,
}

impl Default for RelationDirection {
    fn default() -> Self {
        Self::Both
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetCatalogRelationsInput {
    pub refs: Vec<CatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<RelationDirection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_kinds: Vec<CatalogEntityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_per_ref: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogGraphEdge {
    pub relation_kind: String,
    pub direction: RelationDirection,
    pub target: CatalogEntity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CatalogEntity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogRelationsForRef {
    pub source: CatalogEntity,
    pub relations: Vec<CatalogGraphEdge>,
    pub pagination: Pagination,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub type CatalogRelationResult = CatalogRelationsForRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetCatalogRelationsOutput {
    pub results: Vec<CatalogRelationsForRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathType {
    Join,
    Semantic,
    Any,
}

impl Default for PathType {
    fn default() -> Self {
        Self::Any
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetRelationPathsBetweenInput {
    #[serde(rename = "from")]
    pub from_ref: CatalogRef,
    pub to: Vec<CatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_type: Option<PathType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogRelationPathStep {
    pub entity: CatalogEntity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_relation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogRelationPath {
    #[serde(rename = "from")]
    pub from_entity: CatalogEntity,
    pub to: CatalogEntity,
    pub found: bool,
    pub path_type: PathType,
    pub steps: Vec<CatalogRelationPathStep>,
    pub length: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetRelationPathsBetweenOutput {
    pub paths: Vec<CatalogRelationPath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SampleTableDataInput {
    pub table: CatalogRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SampleTableDataOutput {
    pub table: CatalogEntity,
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecuteQueryInput {
    pub sql: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecuteQueryOutput {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
