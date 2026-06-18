use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailsMode {
    None,
    Summary,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationMode {
    None,
    Refs,
    CompactEdges,
    FullEdges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsMode {
    None,
    RuntimeSummary,
    FullTrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationMode {
    DropWithWarning,
    SummarizeAndPage,
    FacetAndPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OutputPolicy {
    pub id: String,
    pub composer: String,
    pub detail_level: String,
    #[serde(default)]
    pub entity_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_description_chars: Option<usize>,
    pub details_mode: DetailsMode,
    pub relation_mode: RelationMode,
    pub diagnostics_mode: DiagnosticsMode,
    pub max_entities: usize,
    pub max_fields_per_entity: usize,
    pub truncation_mode: TruncationMode,
    #[serde(default)]
    pub model_affinity: Vec<String>,
    pub version: u32,
}

impl OutputPolicy {
    fn new(
        id: &str,
        composer: &str,
        detail_level: &str,
        details_mode: DetailsMode,
        relation_mode: RelationMode,
        diagnostics_mode: DiagnosticsMode,
        max_entities: usize,
        max_fields_per_entity: usize,
        truncation_mode: TruncationMode,
    ) -> Self {
        Self {
            id: id.to_string(),
            composer: composer.to_string(),
            detail_level: detail_level.to_string(),
            entity_fields: vec![
                "id".to_string(),
                "kind".to_string(),
                "name".to_string(),
                "qualified_name".to_string(),
                "description".to_string(),
                "details".to_string(),
                "match".to_string(),
                "warnings".to_string(),
            ],
            max_description_chars: None,
            details_mode,
            relation_mode,
            diagnostics_mode,
            max_entities,
            max_fields_per_entity,
            truncation_mode,
            model_affinity: vec!["generic_tool_calling".to_string()],
            version: 1,
        }
    }

    fn with_description_cap(mut self, max_chars: usize) -> Self {
        self.max_description_chars = Some(max_chars);
        self
    }
}

#[derive(Debug, Clone)]
pub struct OutputPolicyRegistry {
    policies: BTreeMap<String, OutputPolicy>,
}

impl Default for OutputPolicyRegistry {
    fn default() -> Self {
        let mut registry = Self {
            policies: BTreeMap::new(),
        };
        registry.register(
            OutputPolicy::new(
                "tool_use_compact",
                "tool_use",
                "compact",
                DetailsMode::Summary,
                RelationMode::None,
                DiagnosticsMode::RuntimeSummary,
                20,
                12,
                TruncationMode::SummarizeAndPage,
            )
            .with_description_cap(240),
        );
        registry.register(OutputPolicy::new(
            "entity_summary",
            "summary",
            "summary",
            DetailsMode::Summary,
            RelationMode::Refs,
            DiagnosticsMode::RuntimeSummary,
            50,
            50,
            TruncationMode::SummarizeAndPage,
        ));
        registry.register(OutputPolicy::new(
            "entity_verbose",
            "verbose",
            "verbose",
            DetailsMode::Full,
            RelationMode::Refs,
            DiagnosticsMode::RuntimeSummary,
            5,
            200,
            TruncationMode::DropWithWarning,
        ));
        registry.register(
            OutputPolicy::new(
                "schema_compact",
                "tool_use",
                "compact",
                DetailsMode::Summary,
                RelationMode::None,
                DiagnosticsMode::RuntimeSummary,
                10,
                200,
                TruncationMode::SummarizeAndPage,
            )
            .with_description_cap(240),
        );
        registry.register(OutputPolicy::new(
            "schema_verbose",
            "verbose",
            "verbose",
            DetailsMode::Full,
            RelationMode::Refs,
            DiagnosticsMode::RuntimeSummary,
            1,
            200,
            TruncationMode::DropWithWarning,
        ));
        registry
    }
}

impl OutputPolicyRegistry {
    pub fn register(&mut self, policy: OutputPolicy) {
        self.policies.insert(policy.id.clone(), policy);
    }

    pub fn get(&self, id: &str) -> Option<&OutputPolicy> {
        self.policies.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &OutputPolicy> {
        self.policies.values()
    }

    pub fn policy_for_tool(&self, tool_name: &str, requested_count: usize) -> &OutputPolicy {
        match tool_name {
            "search_catalog" => self.get("tool_use_compact"),
            "get_catalog_entities" if requested_count <= 5 => self.get("entity_verbose"),
            "get_catalog_entities" => self.get("entity_summary"),
            "list_schema_fields" if requested_count == 1 => self.get("schema_verbose"),
            "list_schema_fields" => self.get("schema_compact"),
            _ => self.get("entity_summary"),
        }
        .expect("default output policy registry is missing a built-in policy")
    }
}
