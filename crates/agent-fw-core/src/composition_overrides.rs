use serde::{Deserialize, Serialize};

/// Request-scoped tool composition guidance.
///
/// This is distinct from prompt overrides and tool allow/deny policy:
/// it carries structured sequencing guidance that interpreters can realize
/// as deterministic composition instructions without conflating it with
/// free-form prompt mutation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCompositionOverride {
    /// Preferred tool order for a request when the listed tools are relevant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_tool_sequence: Vec<String>,
    /// Additional composition guidance rendered by the interpreter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

impl ToolCompositionOverride {
    pub fn normalized(&self) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        let preferred_tool_sequence = self
            .preferred_tool_sequence
            .iter()
            .filter_map(|tool_name| {
                let trimmed = tool_name.trim();
                if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                    return None;
                }
                Some(trimmed.to_string())
            })
            .collect();
        let guidance = self
            .guidance
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        Self {
            preferred_tool_sequence,
            guidance,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.preferred_tool_sequence.is_empty() && self.guidance.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::ToolCompositionOverride;

    #[test]
    fn tool_composition_override_normalizes_and_dedupes() {
        let override_payload = ToolCompositionOverride {
            preferred_tool_sequence: vec![
                " query_data ".into(),
                "draft_plan".into(),
                "query_data".into(),
                "".into(),
            ],
            guidance: Some(" Prefer search before planning. ".into()),
        }
        .normalized();

        assert_eq!(
            override_payload.preferred_tool_sequence,
            vec!["query_data".to_string(), "draft_plan".to_string()]
        );
        assert_eq!(
            override_payload.guidance.as_deref(),
            Some("Prefer search before planning.")
        );
    }

    #[test]
    fn tool_composition_override_empty_when_no_sequence_or_guidance() {
        let override_payload = ToolCompositionOverride {
            preferred_tool_sequence: vec![" ".into()],
            guidance: Some(" ".into()),
        }
        .normalized();

        assert!(override_payload.is_empty());
    }
}
