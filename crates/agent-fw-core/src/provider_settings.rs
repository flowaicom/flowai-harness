use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub type ProviderSettingsMap = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigView {
    pub key: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
}

impl ProviderConfigView {
    pub fn from_summary(
        key: impl Into<String>,
        summary: crate::ProviderCatalogSummary,
        available: bool,
    ) -> Self {
        Self {
            key: key.into(),
            model: summary.default_model,
            display_name: summary.display_name,
            description: summary.description,
            available,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfoView {
    pub role: String,
    pub display_name: String,
    pub description: String,
}

impl AgentInfoView {
    pub fn new(
        role: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            role: role.into(),
            display_name: display_name.into(),
            description: description.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelSelectionView {
    pub coordinator: String,
    pub planner: String,
    pub executor: String,
}

impl AgentModelSelectionView {
    pub fn uniform(default_provider: impl Into<String>) -> Self {
        let default_provider = default_provider.into();
        Self {
            coordinator: default_provider.clone(),
            planner: default_provider.clone(),
            executor: default_provider,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigResponse {
    pub models: Vec<ProviderConfigView>,
    pub agents: Vec<AgentInfoView>,
    pub default_models: AgentModelSelectionView,
}

impl ModelConfigResponse {
    pub fn from_provider_catalog<S, I, F>(
        catalog: &crate::ModelCatalog,
        provider_keys: I,
        agents: Vec<AgentInfoView>,
        default_provider: impl Into<String>,
        is_available: F,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        F: Fn(&str) -> bool,
    {
        let models = provider_keys
            .into_iter()
            .map(|provider_key| {
                let provider_key = provider_key.as_ref();
                ProviderConfigView::from_summary(
                    provider_key,
                    catalog.summary_for_provider(provider_key),
                    is_available(provider_key),
                )
            })
            .collect();

        Self {
            models,
            agents,
            default_models: AgentModelSelectionView::uniform(default_provider),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListProviderModelsRequest {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub settings: ProviderSettingsMap,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyConnectionRequest {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub verifier: Option<String>,
    #[serde(default)]
    pub settings: ProviderSettingsMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingView {
    pub input_per_m_tok: f64,
    pub output_per_m_tok: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_per_m_tok: Option<f64>,
}

impl From<&crate::ModelPricing> for ModelPricingView {
    fn from(pricing: &crate::ModelPricing) -> Self {
        let cache_read = pricing.cache_read_per_million();
        Self {
            input_per_m_tok: pricing.input_per_million(),
            output_per_m_tok: pricing.output_per_million(),
            cache_read_per_m_tok: (cache_read > 0.0).then_some(cache_read),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilitiesView {
    pub streaming: bool,
    pub max_context_length: usize,
    pub max_output_tokens: usize,
}

impl From<&crate::model_catalog::ModelInfo> for ProviderModelCapabilitiesView {
    fn from(model: &crate::model_catalog::ModelInfo) -> Self {
        Self {
            streaming: model.capabilities.supports_streaming,
            max_context_length: model.context_window,
            max_output_tokens: model.max_output_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelView {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricingView>,
    pub pricing_source: crate::PricingSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ProviderModelCapabilitiesView>,
}

impl ProviderModelView {
    pub fn from_model_info(model: &crate::model_catalog::ModelInfo) -> Self {
        Self {
            id: model.preferred_id().to_string(),
            display_name: model.display_name.clone(),
            provider: model.provider.clone(),
            description: model.description.clone().or_else(|| {
                (!model.family.trim().is_empty()).then(|| format!("{} family", model.family))
            }),
            pricing: None,
            pricing_source: crate::PricingSource::None,
            capabilities: Some(ProviderModelCapabilitiesView::from(model)),
        }
    }

    pub fn without_capabilities(mut self) -> Self {
        self.capabilities = None;
        self
    }

    pub fn with_pricing(
        mut self,
        pricing: Option<&crate::ModelPricing>,
        pricing_source: crate::PricingSource,
    ) -> Self {
        self.pricing = pricing.map(ModelPricingView::from);
        self.pricing_source = pricing_source;
        self
    }
}

pub fn provider_model_views(
    catalog: &crate::ModelCatalog,
    provider: &str,
) -> Vec<ProviderModelView> {
    catalog
        .for_provider(provider)
        .into_iter()
        .map(ProviderModelView::from_model_info)
        .map(ProviderModelView::without_capabilities)
        .collect()
}

pub fn all_provider_model_views(catalog: &crate::ModelCatalog) -> Vec<ProviderModelView> {
    catalog
        .ordered_provider_keys()
        .into_iter()
        .flat_map(|provider| provider_model_views(catalog, provider))
        .collect()
}

pub fn find_provider_model_view(
    catalog: &crate::ModelCatalog,
    model_id: &str,
) -> Option<ProviderModelView> {
    let model = catalog.find(model_id)?;
    Some(ProviderModelView::from_model_info(model).without_capabilities())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProviderModelsResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub models: Vec<ProviderModelView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ListProviderModelsResponse {
    pub fn ok(provider: Option<String>, models: Vec<ProviderModelView>) -> Self {
        Self {
            success: true,
            provider,
            models,
            error: None,
        }
    }

    pub fn err(provider: Option<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            provider,
            models: Vec::new(),
            error: Some(message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        all_provider_model_views, find_provider_model_view, provider_model_views, AgentInfoView,
        AgentModelSelectionView, ListProviderModelsResponse, ModelConfigResponse, ModelPricingView,
        ProviderConfigView, ProviderModelCapabilitiesView, ProviderModelView,
    };
    use crate::{ModelCapabilities, ModelCatalog, ModelInfo, ModelPricing, PricingSource};

    #[test]
    fn model_pricing_view_from_pricing() {
        let pricing = ModelPricing::new("m", 3.0, 15.0, 0.30);
        let view = ModelPricingView::from(&pricing);
        assert_eq!(view.input_per_m_tok, 3.0);
        assert_eq!(view.output_per_m_tok, 15.0);
        assert_eq!(view.cache_read_per_m_tok, Some(0.30));
    }

    #[test]
    fn capabilities_view_from_model_info() {
        let model = ModelInfo {
            id: "m".to_string(),
            display_name: "Model".to_string(),
            provider: "anthropic".to_string(),
            description: None,
            context_window: 200_000,
            max_output_tokens: 8_000,
            capabilities: ModelCapabilities {
                supports_streaming: true,
                ..ModelCapabilities::default()
            },
            is_default: false,
            family: String::new(),
            aliases: Vec::new(),
        };
        let view = ProviderModelCapabilitiesView::from(&model);
        assert!(view.streaming);
        assert_eq!(view.max_context_length, 200_000);
        assert_eq!(view.max_output_tokens, 8_000);
    }

    #[test]
    fn list_provider_models_response_ok_sets_fields() {
        let response = ListProviderModelsResponse::ok(
            Some("anthropic".to_string()),
            vec![ProviderModelView {
                id: "claude-sonnet-4-5".to_string(),
                display_name: "Claude Sonnet 4.5".to_string(),
                provider: "anthropic".to_string(),
                description: None,
                pricing: None,
                pricing_source: PricingSource::None,
                capabilities: None,
            }],
        );
        assert!(response.success);
        assert_eq!(response.provider.as_deref(), Some("anthropic"));
        assert_eq!(response.models.len(), 1);
        assert!(response.error.is_none());
    }

    #[test]
    fn provider_model_view_from_model_info_uses_preferred_id_and_capabilities() {
        let model = ModelInfo {
            id: "anthropic/claude-sonnet-4-6-20250514".to_string(),
            display_name: "Claude Sonnet 4.6".to_string(),
            provider: "anthropic".to_string(),
            description: Some("Fast balanced model".to_string()),
            context_window: 200_000,
            max_output_tokens: 8_000,
            capabilities: ModelCapabilities::default(),
            is_default: false,
            family: "claude-sonnet".to_string(),
            aliases: vec!["claude-sonnet-4-6".to_string()],
        };

        let view = ProviderModelView::from_model_info(&model);
        assert_eq!(view.id, "claude-sonnet-4-6");
        assert!(view.capabilities.is_some());
        assert_eq!(view.pricing_source, PricingSource::None);
    }

    #[test]
    fn provider_model_views_project_catalog_models() {
        let models = provider_model_views(&ModelCatalog::builtin(), "anthropic");
        assert!(!models.is_empty());
        assert!(models.iter().all(|model| model.provider == "anthropic"));
        assert!(models.iter().all(|model| model.capabilities.is_none()));
    }

    #[test]
    fn all_provider_model_views_cover_catalog() {
        let models = all_provider_model_views(&ModelCatalog::builtin());
        assert!(!models.is_empty());
        assert!(models.iter().any(|model| model.provider == "anthropic"));
        assert!(models.iter().any(|model| model.provider == "openai"));
    }

    #[test]
    fn find_provider_model_view_uses_alias_matching() {
        let model = find_provider_model_view(&ModelCatalog::builtin(), "claude-opus-4-6");
        assert!(model.is_some());
        assert_eq!(model.unwrap().provider, "anthropic");
    }

    #[test]
    fn provider_config_view_from_summary_sets_fields() {
        let summary = ModelCatalog::builtin().summary_for_provider("anthropic");
        let view = ProviderConfigView::from_summary("anthropic", summary, true);
        assert_eq!(view.key, "anthropic");
        assert_eq!(view.model, "anthropic/claude-sonnet-4-6-20250514");
        assert!(view.available);
    }

    #[test]
    fn agent_model_selection_view_uniform_sets_all_roles() {
        let selection = AgentModelSelectionView::uniform("anthropic");
        assert_eq!(selection.coordinator, "anthropic");
        assert_eq!(selection.planner, "anthropic");
        assert_eq!(selection.executor, "anthropic");
    }

    #[test]
    fn model_config_response_from_provider_catalog_builds_summary() {
        let response = ModelConfigResponse::from_provider_catalog(
            &ModelCatalog::builtin(),
            ["anthropic", "openai"],
            vec![AgentInfoView::new(
                "coordinator",
                "Coordinator",
                "Orchestrates the pipeline",
            )],
            "anthropic",
            |provider| provider == "anthropic",
        );

        assert_eq!(response.models.len(), 2);
        assert_eq!(response.models[0].key, "anthropic");
        assert!(response.models[0].available);
        assert_eq!(response.models[1].key, "openai");
        assert!(!response.models[1].available);
        assert_eq!(response.default_models.coordinator, "anthropic");
        assert_eq!(response.agents.len(), 1);
    }
}
