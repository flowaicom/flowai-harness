use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Request-scoped tool surface overrides.
///
/// This is the structured carrier for:
/// - description mutations exposed to the model during tool registration
/// - allow/deny policy applied by the runtime dispatcher
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDispatchOverrides {
    /// Replacement descriptions keyed by tool name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub description_overrides: BTreeMap<String, String>,
    /// When non-empty, only these tools remain visible and dispatchable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_tools: Vec<String>,
    /// Tools hidden from the model and rejected at dispatch time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
}

impl ToolDispatchOverrides {
    pub fn normalized(&self) -> Self {
        let description_overrides = self
            .description_overrides
            .iter()
            .filter_map(|(tool_name, description)| {
                let tool_name = tool_name.trim();
                let description = description.trim();
                (!tool_name.is_empty() && !description.is_empty())
                    .then(|| (tool_name.to_string(), description.to_string()))
            })
            .collect();
        let enabled_tools = normalize_tool_name_list(&self.enabled_tools);
        let disabled_tools = normalize_tool_name_list(&self.disabled_tools);

        Self {
            description_overrides,
            enabled_tools,
            disabled_tools,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.description_overrides.is_empty()
            && self.enabled_tools.is_empty()
            && self.disabled_tools.is_empty()
    }

    pub fn overlapping_tools(&self) -> Vec<String> {
        let enabled = self.enabled_tools.iter().cloned().collect::<BTreeSet<_>>();
        let disabled = self.disabled_tools.iter().cloned().collect::<BTreeSet<_>>();
        enabled.intersection(&disabled).cloned().collect()
    }

    pub fn allows_tool(&self, tool_name: &str) -> bool {
        if self.disabled_tools.iter().any(|name| name == tool_name) {
            return false;
        }
        self.enabled_tools.is_empty() || self.enabled_tools.iter().any(|name| name == tool_name)
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
    fn tool_dispatch_overrides_normalize_and_filter() {
        let overrides = ToolDispatchOverrides {
            description_overrides: BTreeMap::from([
                (
                    " draft_plan ".into(),
                    " Create a plan only when needed. ".into(),
                ),
                ("".into(), "ignored".into()),
            ]),
            enabled_tools: vec![" draft_plan ".into(), "draft_plan".into()],
            disabled_tools: vec![" inspect ".into(), "inspect".into()],
        }
        .normalized();

        assert_eq!(
            overrides.description_overrides.get("draft_plan"),
            Some(&"Create a plan only when needed.".to_string())
        );
        assert_eq!(overrides.enabled_tools, vec!["draft_plan".to_string()]);
        assert_eq!(overrides.disabled_tools, vec!["inspect".to_string()]);
    }

    #[test]
    fn tool_dispatch_overrides_allow_and_block() {
        let overrides = ToolDispatchOverrides {
            description_overrides: BTreeMap::new(),
            enabled_tools: vec!["draft_plan".into()],
            disabled_tools: vec!["inspect".into()],
        };

        assert!(overrides.allows_tool("draft_plan"));
        assert!(!overrides.allows_tool("inspect"));
        assert!(!overrides.allows_tool("resolve"));
    }
}
