use std::{collections::BTreeSet, sync::Arc};

use agent_fw_core::{ToolDispatchOverrides, ToolRegistryAddSource, ToolRegistryOverride};
use async_trait::async_trait;
use thiserror::Error;

use crate::{ToolCallResult, ToolDefinition, ToolDispatcher};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RequestScopedToolOverrideError {
    #[error("tool_registry add_tools mutations are not executable in request-scoped runtime handling yet")]
    UnsupportedToolRegistryAdd,
    #[error("tool_registry add_tools references unknown latent tool '{tool_id}'")]
    UnknownLatentTool { tool_id: String },
    #[error("tool_registry override has conflicting operations for: {tools}")]
    ConflictingToolRegistry { tools: String },
    #[error("tool_registry override produces duplicate visible tool name '{tool_name}'")]
    DuplicateVisibleToolName { tool_name: String },
}

impl RequestScopedToolOverrideError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedToolRegistryAdd => "UNSUPPORTED_TOOL_REGISTRY_ADD",
            Self::UnknownLatentTool { .. } => "UNKNOWN_LATENT_TOOL",
            Self::ConflictingToolRegistry { .. } => "INVALID_TOOL_REGISTRY_OVERRIDE",
            Self::DuplicateVisibleToolName { .. } => "INVALID_TOOL_REGISTRY_OVERRIDE",
        }
    }
}

struct RequestScopedToolDispatcher {
    base: Arc<dyn ToolDispatcher>,
    dispatch_overrides: ToolDispatchOverrides,
    registry_override: ToolRegistryOverride,
}

pub fn apply_request_scoped_tool_overrides(
    dispatcher: Arc<dyn ToolDispatcher>,
    dispatch_overrides: Option<ToolDispatchOverrides>,
    tool_registry_override: Option<ToolRegistryOverride>,
) -> Result<Arc<dyn ToolDispatcher>, RequestScopedToolOverrideError> {
    let dispatch_overrides = dispatch_overrides
        .map(|value| value.normalized())
        .unwrap_or_default();
    let registry_override = tool_registry_override
        .map(|value| value.normalized())
        .unwrap_or_default();
    if dispatch_overrides.is_empty() && registry_override.is_empty() {
        return Ok(dispatcher);
    }
    validate_tool_registry_runtime_override(&dispatcher, &registry_override)?;
    Ok(Arc::new(RequestScopedToolDispatcher {
        base: dispatcher,
        dispatch_overrides,
        registry_override,
    }))
}

fn validate_tool_registry_runtime_override(
    dispatcher: &Arc<dyn ToolDispatcher>,
    registry_override: &ToolRegistryOverride,
) -> Result<(), RequestScopedToolOverrideError> {
    if registry_override.is_empty() {
        return Ok(());
    }
    let conflicts = registry_override.conflicting_tools();
    if !conflicts.is_empty() {
        return Err(RequestScopedToolOverrideError::ConflictingToolRegistry {
            tools: conflicts.join(", "),
        });
    }

    let latent_definitions = dispatcher
        .latent_tool_definitions()
        .into_iter()
        .map(|definition| (definition.name.clone(), definition))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut visible_names = BTreeSet::new();
    for definition in dispatcher.tool_definitions() {
        let original_name = definition.name.clone();
        if !registry_allows_tool(registry_override, &original_name) {
            continue;
        }
        let visible_name = registry_visible_name(registry_override, &original_name)
            .unwrap_or_else(|| original_name.clone());
        if !visible_names.insert(visible_name.clone()) {
            return Err(RequestScopedToolOverrideError::DuplicateVisibleToolName {
                tool_name: visible_name,
            });
        }
    }
    for add_spec in &registry_override.add_tools {
        let Some(latent_tool_id) = add_spec
            .source
            .as_ref()
            .and_then(ToolRegistryAddSource::latent_tool_id)
        else {
            return Err(RequestScopedToolOverrideError::UnsupportedToolRegistryAdd);
        };
        if !latent_definitions.contains_key(latent_tool_id) {
            return Err(RequestScopedToolOverrideError::UnknownLatentTool {
                tool_id: latent_tool_id.to_string(),
            });
        }
        if !visible_names.insert(add_spec.tool_name.clone()) {
            return Err(RequestScopedToolOverrideError::DuplicateVisibleToolName {
                tool_name: add_spec.tool_name.clone(),
            });
        }
    }
    Ok(())
}

#[async_trait]
impl ToolDispatcher for RequestScopedToolDispatcher {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self
            .base
            .tool_definitions()
            .into_iter()
            .filter_map(|definition| {
                apply_tool_override_to_definition(
                    &self.dispatch_overrides,
                    &self.registry_override,
                    definition,
                )
            })
            .collect::<Vec<_>>();
        definitions.extend(self.base.latent_tool_definitions().into_iter().filter_map(
            |definition| {
                apply_added_tool_definition(
                    &self.dispatch_overrides,
                    &self.registry_override,
                    definition,
                )
            },
        ));
        definitions
    }

    fn latent_tool_definitions(&self) -> Vec<ToolDefinition> {
        let activated = self
            .registry_override
            .add_tools
            .iter()
            .filter_map(|tool| {
                tool.source
                    .as_ref()
                    .and_then(ToolRegistryAddSource::latent_tool_id)
            })
            .collect::<BTreeSet<_>>();
        self.base
            .latent_tool_definitions()
            .into_iter()
            .filter(|definition| !activated.contains(definition.name.as_str()))
            .collect()
    }

    async fn dispatch(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: serde_json::Value,
    ) -> ToolCallResult {
        if let Some(source_name) = registry_added_source_name(&self.registry_override, tool_name) {
            return self.base.dispatch(&source_name, tool_use_id, input).await;
        }
        if let Some(renamed_to) = registry_renamed_to(&self.registry_override, tool_name) {
            return ToolCallResult::error(
                tool_use_id,
                format!("tool '{tool_name}' is renamed to '{renamed_to}' for this request"),
            );
        }

        let resolved_tool_name = registry_original_name(&self.registry_override, tool_name)
            .unwrap_or_else(|| tool_name.to_string());
        if !registry_allows_tool(&self.registry_override, &resolved_tool_name) {
            return ToolCallResult::error(
                tool_use_id,
                format!("tool '{tool_name}' is disabled for this request"),
            );
        }
        if !self.dispatch_overrides.allows_tool(&resolved_tool_name) {
            return ToolCallResult::error(
                tool_use_id,
                format!("tool '{tool_name}' is disabled for this request"),
            );
        }
        self.base
            .dispatch(&resolved_tool_name, tool_use_id, input)
            .await
    }

    fn current_tool_call_id(&self) -> Option<String> {
        self.base.current_tool_call_id()
    }

    fn tool_call_id_cell(&self) -> Option<Arc<std::sync::Mutex<Option<String>>>> {
        self.base.tool_call_id_cell()
    }

    fn pending_card_cell(
        &self,
    ) -> Option<Arc<std::sync::Mutex<Option<agent_fw_tool::CommandCardPayload>>>> {
        self.base.pending_card_cell()
    }

    fn with_event_sink(
        self: Arc<Self>,
        sink: Arc<dyn agent_fw_algebra::EventSink>,
    ) -> Option<Arc<dyn ToolDispatcher>> {
        self.base.clone().with_event_sink(sink).map(|dispatcher| {
            Arc::new(RequestScopedToolDispatcher {
                base: dispatcher,
                dispatch_overrides: self.dispatch_overrides.clone(),
                registry_override: self.registry_override.clone(),
            }) as Arc<dyn ToolDispatcher>
        })
    }
}

fn apply_tool_override_to_definition(
    dispatch_overrides: &ToolDispatchOverrides,
    registry_override: &ToolRegistryOverride,
    mut definition: ToolDefinition,
) -> Option<ToolDefinition> {
    let original_name = definition.name.clone();
    if !registry_allows_tool(registry_override, &original_name) {
        return None;
    }
    if !dispatch_overrides.allows_tool(&original_name) {
        return None;
    }
    if let Some(description) = dispatch_overrides.description_overrides.get(&original_name) {
        definition.description = description.clone();
    }
    if let Some(visible_name) = registry_visible_name(registry_override, &original_name) {
        definition.name = visible_name;
    }
    Some(definition)
}

fn apply_added_tool_definition(
    dispatch_overrides: &ToolDispatchOverrides,
    registry_override: &ToolRegistryOverride,
    mut definition: ToolDefinition,
) -> Option<ToolDefinition> {
    let add_spec = registry_override.add_tools.iter().find(|tool| {
        tool.source
            .as_ref()
            .and_then(ToolRegistryAddSource::latent_tool_id)
            == Some(definition.name.as_str())
    })?;
    if let Some(description) = dispatch_overrides
        .description_overrides
        .get(&definition.name)
    {
        definition.description = description.clone();
    }
    if let Some(description) = &add_spec.description {
        definition.description = description.clone();
    }
    definition.name = add_spec.tool_name.clone();
    Some(definition)
}

fn registry_allows_tool(registry_override: &ToolRegistryOverride, tool_name: &str) -> bool {
    if registry_override
        .remove_tools
        .iter()
        .any(|candidate| candidate == tool_name)
    {
        return false;
    }
    if !registry_override.enable_tools.is_empty()
        && !registry_override
            .enable_tools
            .iter()
            .any(|candidate| candidate == tool_name)
    {
        return false;
    }
    !registry_override
        .disable_tools
        .iter()
        .any(|candidate| candidate == tool_name)
}

fn registry_visible_name(
    registry_override: &ToolRegistryOverride,
    original_name: &str,
) -> Option<String> {
    registry_override
        .rename_tools
        .iter()
        .find(|rename| rename.from_name == original_name)
        .map(|rename| rename.to_name.clone())
}

fn registry_original_name(
    registry_override: &ToolRegistryOverride,
    visible_name: &str,
) -> Option<String> {
    registry_override
        .rename_tools
        .iter()
        .find(|rename| rename.to_name == visible_name)
        .map(|rename| rename.from_name.clone())
}

fn registry_renamed_to<'a>(
    registry_override: &'a ToolRegistryOverride,
    original_name: &str,
) -> Option<&'a str> {
    registry_override
        .rename_tools
        .iter()
        .find(|rename| rename.from_name == original_name)
        .map(|rename| rename.to_name.as_str())
}

fn registry_added_source_name(
    registry_override: &ToolRegistryOverride,
    visible_name: &str,
) -> Option<String> {
    registry_override
        .add_tools
        .iter()
        .find(|tool| tool.tool_name == visible_name)
        .and_then(|tool| tool.source.as_ref())
        .and_then(ToolRegistryAddSource::latent_tool_id)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct StaticDispatcher;

    #[async_trait]
    impl ToolDispatcher for StaticDispatcher {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![
                ToolDefinition {
                    name: "summarizeRequest".to_string(),
                    description: "Summarize a request".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "draftChecklist".to_string(),
                    description: "Draft a checklist".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
            ]
        }

        fn latent_tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "latentChecklist".to_string(),
                description: "A hidden checklist tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            }]
        }

        async fn dispatch(
            &self,
            tool_name: &str,
            tool_use_id: &str,
            _input: serde_json::Value,
        ) -> ToolCallResult {
            ToolCallResult::success(
                tool_use_id,
                serde_json::json!({
                    "toolName": tool_name,
                }),
            )
        }
    }

    #[tokio::test]
    async fn request_scoped_dispatcher_applies_tool_registry_subset() {
        let dispatcher = apply_request_scoped_tool_overrides(
            Arc::new(StaticDispatcher),
            None,
            Some(ToolRegistryOverride {
                remove_tools: vec!["draftChecklist".to_string()],
                ..ToolRegistryOverride::default()
            }),
        )
        .expect("request-scoped dispatcher");

        let names = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["summarizeRequest".to_string()]);

        let result = dispatcher
            .dispatch("summarizeRequest", "tool-1", serde_json::json!({}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["toolName"], "summarizeRequest");
    }

    #[test]
    fn request_scoped_dispatcher_rejects_add_tool_mutations() {
        let error = apply_request_scoped_tool_overrides(
            Arc::new(StaticDispatcher),
            None,
            Some(ToolRegistryOverride {
                add_tools: vec![agent_fw_core::ToolRegistryAddSpec {
                    tool_name: "newTool".to_string(),
                    description: Some("new".to_string()),
                    source: None,
                }],
                ..ToolRegistryOverride::default()
            }),
        )
        .err()
        .expect("unsupported add_tools");

        assert_eq!(
            error,
            RequestScopedToolOverrideError::UnsupportedToolRegistryAdd
        );
        assert_eq!(error.code(), "UNSUPPORTED_TOOL_REGISTRY_ADD");
    }

    #[tokio::test]
    async fn request_scoped_dispatcher_supports_latent_add_tool_mutations() {
        let dispatcher = apply_request_scoped_tool_overrides(
            Arc::new(StaticDispatcher),
            None,
            Some(ToolRegistryOverride {
                add_tools: vec![agent_fw_core::ToolRegistryAddSpec {
                    tool_name: "checklistNow".to_string(),
                    description: Some("Use this hidden checklist tool when exposed.".to_string()),
                    source: Some(ToolRegistryAddSource::LatentRegistry {
                        tool_id: "latentChecklist".to_string(),
                    }),
                }],
                ..ToolRegistryOverride::default()
            }),
        )
        .expect("latent add_tools should be supported");

        let names = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "summarizeRequest".to_string(),
                "draftChecklist".to_string(),
                "checklistNow".to_string()
            ]
        );

        let result = dispatcher
            .dispatch("checklistNow", "tool-1", serde_json::json!({}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["toolName"], "latentChecklist");
    }
}
