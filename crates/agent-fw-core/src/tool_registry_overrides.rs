use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolRegistryAddSource {
    LatentRegistry {
        #[serde(rename = "toolId", alias = "tool_id")]
        tool_id: String,
    },
}

impl ToolRegistryAddSource {
    fn normalized(&self) -> Option<Self> {
        match self {
            Self::LatentRegistry { tool_id } => {
                let tool_id = tool_id.trim();
                (!tool_id.is_empty()).then(|| Self::LatentRegistry {
                    tool_id: tool_id.to_string(),
                })
            }
        }
    }

    pub fn latent_tool_id(&self) -> Option<&str> {
        match self {
            Self::LatentRegistry { tool_id } => Some(tool_id.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRegistryAddSpec {
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolRegistryAddSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRegistryRenameSpec {
    pub from_name: String,
    pub to_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRegistryOverride {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_tools: Vec<ToolRegistryAddSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enable_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rename_tools: Vec<ToolRegistryRenameSpec>,
}

impl ToolRegistryOverride {
    pub fn normalized(&self) -> Self {
        let add_tools = self
            .add_tools
            .iter()
            .filter_map(|tool| {
                let tool_name = tool.tool_name.trim();
                if tool_name.is_empty() {
                    return None;
                }
                let description = tool
                    .description
                    .as_ref()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
                let source = tool
                    .source
                    .as_ref()
                    .and_then(ToolRegistryAddSource::normalized);
                Some(ToolRegistryAddSpec {
                    tool_name: tool_name.to_string(),
                    description,
                    source,
                })
            })
            .collect::<Vec<_>>();
        let remove_tools = normalize_tool_name_list(&self.remove_tools);
        let enable_tools = normalize_tool_name_list(&self.enable_tools);
        let disable_tools = normalize_tool_name_list(&self.disable_tools);
        let rename_tools = self
            .rename_tools
            .iter()
            .filter_map(|rename| {
                let from_name = rename.from_name.trim();
                let to_name = rename.to_name.trim();
                (!from_name.is_empty() && !to_name.is_empty() && from_name != to_name).then(|| {
                    ToolRegistryRenameSpec {
                        from_name: from_name.to_string(),
                        to_name: to_name.to_string(),
                    }
                })
            })
            .collect::<Vec<_>>();

        Self {
            add_tools,
            remove_tools,
            enable_tools,
            disable_tools,
            rename_tools,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.add_tools.is_empty()
            && self.remove_tools.is_empty()
            && self.enable_tools.is_empty()
            && self.disable_tools.is_empty()
            && self.rename_tools.is_empty()
    }

    pub fn conflicting_tools(&self) -> Vec<String> {
        let added = self
            .add_tools
            .iter()
            .map(|tool| tool.tool_name.clone())
            .collect::<BTreeSet<_>>();
        let removed = self.remove_tools.iter().cloned().collect::<BTreeSet<_>>();
        let enabled = self.enable_tools.iter().cloned().collect::<BTreeSet<_>>();
        let disabled = self.disable_tools.iter().cloned().collect::<BTreeSet<_>>();

        added
            .intersection(&removed)
            .chain(enabled.intersection(&disabled))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn non_executable_add_tools(&self) -> Vec<String> {
        self.add_tools
            .iter()
            .filter(|tool| tool.source.is_none())
            .map(|tool| tool.tool_name.clone())
            .collect()
    }
}

fn normalize_tool_name_list(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter_map(|name| {
            let trimmed = name.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_registry_override_normalizes_and_filters() {
        let normalized = ToolRegistryOverride {
            add_tools: vec![
                ToolRegistryAddSpec {
                    tool_name: " draft_plan ".into(),
                    description: Some(" Build a plan ".into()),
                    source: Some(ToolRegistryAddSource::LatentRegistry {
                        tool_id: " draft_plan ".into(),
                    }),
                },
                ToolRegistryAddSpec {
                    tool_name: "".into(),
                    description: Some("ignored".into()),
                    source: None,
                },
            ],
            remove_tools: vec![" inspect ".into(), "inspect".into()],
            enable_tools: vec![" draft_plan ".into(), "draft_plan".into()],
            disable_tools: vec![" resolve ".into(), "resolve".into()],
            rename_tools: vec![
                ToolRegistryRenameSpec {
                    from_name: " old ".into(),
                    to_name: " new ".into(),
                },
                ToolRegistryRenameSpec {
                    from_name: "same".into(),
                    to_name: "same".into(),
                },
            ],
        }
        .normalized();

        assert_eq!(normalized.add_tools.len(), 1);
        assert_eq!(normalized.add_tools[0].tool_name, "draft_plan");
        assert_eq!(
            normalized.add_tools[0].description.as_deref(),
            Some("Build a plan")
        );
        assert_eq!(
            normalized.add_tools[0]
                .source
                .as_ref()
                .and_then(ToolRegistryAddSource::latent_tool_id),
            Some("draft_plan")
        );
        assert_eq!(normalized.remove_tools, vec!["inspect".to_string()]);
        assert_eq!(normalized.enable_tools, vec!["draft_plan".to_string()]);
        assert_eq!(normalized.disable_tools, vec!["resolve".to_string()]);
        assert_eq!(normalized.rename_tools.len(), 1);
        assert_eq!(normalized.rename_tools[0].from_name, "old");
        assert_eq!(normalized.rename_tools[0].to_name, "new");
    }

    #[test]
    fn tool_registry_override_detects_conflicts() {
        let override_payload = ToolRegistryOverride {
            add_tools: vec![ToolRegistryAddSpec {
                tool_name: "draft_plan".into(),
                description: None,
                source: None,
            }],
            remove_tools: vec!["draft_plan".into()],
            enable_tools: vec!["query_data".into()],
            disable_tools: vec!["query_data".into()],
            rename_tools: Vec::new(),
        };

        assert_eq!(
            override_payload.conflicting_tools(),
            vec!["draft_plan".to_string(), "query_data".to_string()]
        );
    }

    #[test]
    fn tool_registry_override_reports_non_executable_adds() {
        let override_payload = ToolRegistryOverride {
            add_tools: vec![
                ToolRegistryAddSpec {
                    tool_name: "latentChecklist".into(),
                    description: None,
                    source: Some(ToolRegistryAddSource::LatentRegistry {
                        tool_id: "draftChecklist".into(),
                    }),
                },
                ToolRegistryAddSpec {
                    tool_name: "proseOnly".into(),
                    description: Some("no runtime source".into()),
                    source: None,
                },
            ],
            ..ToolRegistryOverride::default()
        };

        assert_eq!(
            override_payload.non_executable_add_tools(),
            vec!["proseOnly".to_string()]
        );
    }

    #[test]
    fn tool_registry_add_source_accepts_camel_case_tool_id() {
        let parsed: ToolRegistryAddSource = serde_json::from_value(serde_json::json!({
            "kind": "latent_registry",
            "toolId": "turboSummarize"
        }))
        .expect("camelCase source should deserialize");

        assert_eq!(parsed.latent_tool_id(), Some("turboSummarize"));

        let serialized = serde_json::to_value(&parsed).expect("serialize source");
        assert_eq!(
            serialized,
            serde_json::json!({
                "kind": "latent_registry",
                "toolId": "turboSummarize"
            })
        );
    }
}
