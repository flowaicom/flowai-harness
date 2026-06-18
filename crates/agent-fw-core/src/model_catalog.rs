//! Model configuration catalog — pure model/provider metadata.
//!
//! This module is framework-wide, not Studio-specific. It defines the reusable
//! provider/model catalog surface consumed by Studio, embedded runtimes, and
//! external applications that need a static or project-extended model registry.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// How a provider expects credentials to be supplied.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderCredentialMode {
    ApiKey,
    CloudCredentials,
    None,
}

impl Default for ProviderCredentialMode {
    fn default() -> Self {
        Self::ApiKey
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegion {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub model_count: Option<usize>,
}

impl ProviderRegion {
    pub fn new(
        key: &str,
        display_name: &str,
        description: &str,
        model_count: Option<usize>,
    ) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            model_count,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderSettingKind {
    Secret,
    Text,
    Select,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingOption {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
}

impl ProviderSettingOption {
    pub fn new(key: &str, display_name: &str, description: &str) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSetting {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub kind: ProviderSettingKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
    #[serde(default)]
    pub options: Vec<ProviderSettingOption>,
}

impl ProviderSetting {
    pub fn secret(key: &str, display_name: &str, description: &str) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            kind: ProviderSettingKind::Secret,
            required: false,
            default_value: None,
            env_var: None,
            options: Vec::new(),
        }
    }

    pub fn text(
        key: &str,
        display_name: &str,
        description: &str,
        default_value: Option<&str>,
    ) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            kind: ProviderSettingKind::Text,
            required: false,
            default_value: default_value.map(str::to_string),
            env_var: None,
            options: Vec::new(),
        }
    }

    pub fn select(
        key: &str,
        display_name: &str,
        description: &str,
        default_value: Option<&str>,
        options: Vec<ProviderSettingOption>,
    ) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            kind: ProviderSettingKind::Select,
            required: false,
            default_value: default_value.map(str::to_string),
            env_var: None,
            options,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn with_env_var(mut self, env_var: &str) -> Self {
        self.env_var = Some(env_var.to_string());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointTransportInfo {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub settings: Vec<ProviderSetting>,
}

impl EndpointTransportInfo {
    pub fn new(key: &str, display_name: &str, description: &str) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            settings: Vec::new(),
        }
    }

    pub fn with_settings(mut self, settings: Vec<ProviderSetting>) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_setting(mut self, setting: ProviderSetting) -> Self {
        self.settings.push(setting);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub credential_mode: ProviderCredentialMode,
    #[serde(default)]
    pub default_region: Option<String>,
    #[serde(default)]
    pub regions: Vec<ProviderRegion>,
    #[serde(default)]
    pub settings: Vec<ProviderSetting>,
    #[serde(default)]
    pub region_env_var: Option<String>,
    #[serde(default)]
    pub endpoint_transport: Option<String>,
}

impl ProviderInfo {
    pub fn new(
        key: &str,
        display_name: &str,
        description: &str,
        credential_mode: ProviderCredentialMode,
    ) -> Self {
        Self {
            key: key.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            credential_mode,
            default_region: None,
            regions: Vec::new(),
            settings: Vec::new(),
            region_env_var: None,
            endpoint_transport: None,
        }
    }

    pub fn api_key(key: &str, display_name: &str, description: &str) -> Self {
        Self::new(
            key,
            display_name,
            description,
            ProviderCredentialMode::ApiKey,
        )
    }

    pub fn cloud_credentials(key: &str, display_name: &str, description: &str) -> Self {
        Self::new(
            key,
            display_name,
            description,
            ProviderCredentialMode::CloudCredentials,
        )
    }

    pub fn without_credentials(key: &str, display_name: &str, description: &str) -> Self {
        Self::new(key, display_name, description, ProviderCredentialMode::None)
    }

    fn fallback(key: &str) -> Self {
        Self {
            key: key.to_string(),
            display_name: humanize_provider_key(key),
            description: format!("Available {} models", humanize_provider_key(key)),
            credential_mode: ProviderCredentialMode::ApiKey,
            default_region: None,
            regions: Vec::new(),
            settings: Vec::new(),
            region_env_var: None,
            endpoint_transport: None,
        }
    }

    pub fn with_regions(
        mut self,
        default_region: Option<&str>,
        regions: Vec<ProviderRegion>,
    ) -> Self {
        self.default_region = default_region.map(str::to_string);
        self.regions = regions;
        self
    }

    pub fn with_region(mut self, default_region: Option<&str>, region: ProviderRegion) -> Self {
        self.default_region = default_region.map(str::to_string);
        self.regions.push(region);
        self
    }

    pub fn with_settings(mut self, settings: Vec<ProviderSetting>) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_setting(mut self, setting: ProviderSetting) -> Self {
        self.settings.push(setting);
        self
    }

    pub fn with_region_env_var(mut self, env_var: &str) -> Self {
        self.region_env_var = Some(env_var.to_string());
        self
    }

    pub fn with_endpoint_transport(mut self, transport: &str) -> Self {
        self.endpoint_transport = Some(transport.to_string());
        self
    }

    pub fn resolved_settings(&self) -> Vec<ProviderSetting> {
        let mut settings = match self.credential_mode {
            ProviderCredentialMode::ApiKey => vec![ProviderSetting::secret(
                "apiKey",
                "API Key",
                &format!("Credentials for {}", self.display_name),
            )
            .with_env_var(&default_provider_api_key_env(&self.key))],
            ProviderCredentialMode::CloudCredentials | ProviderCredentialMode::None => Vec::new(),
        };

        if !self.regions.is_empty() {
            let mut region_setting = ProviderSetting::select(
                "region",
                "Region",
                &format!("Select the {} region", self.display_name),
                self.default_region.as_deref(),
                self.regions
                    .iter()
                    .map(|region| {
                        ProviderSettingOption::new(
                            &region.key,
                            &region.display_name,
                            &region.description,
                        )
                    })
                    .collect(),
            );
            if let Some(env_var) = &self.region_env_var {
                region_setting = region_setting.with_env_var(env_var);
            }
            settings.push(region_setting);
        }

        for explicit in &self.settings {
            if let Some(existing) = settings
                .iter_mut()
                .find(|setting| setting.key == explicit.key)
            {
                *existing = explicit.clone();
            } else {
                settings.push(explicit.clone());
            }
        }

        settings
    }
}

pub fn default_provider_api_key_env(provider_key: &str) -> String {
    let normalized = provider_key
        .chars()
        .map(|ch| match ch {
            'a'..='z' => ch.to_ascii_uppercase(),
            'A'..='Z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect::<String>();
    format!("{normalized}_API_KEY")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogConfig {
    #[serde(default = "default_include_builtin")]
    pub include_builtin: bool,
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
    #[serde(default)]
    pub endpoint_transports: Vec<EndpointTransportInfo>,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalogSummary {
    pub key: String,
    pub display_name: String,
    pub default_model: String,
    pub description: String,
}

/// Source of model pricing metadata in provider-model listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PricingSource {
    Gateway,
    #[serde(rename = "aws-pricing")]
    AwsPricing,
    Direct,
    Static,
    None,
}

fn default_include_builtin() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_streaming: bool,
    pub supports_system_prompt: bool,
    pub supports_json_mode: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            supports_system_prompt: true,
            supports_json_mode: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub context_window: usize,
    #[serde(default)]
    pub max_output_tokens: usize,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub family: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

impl ModelInfo {
    pub fn new(id: &str, provider: &str, display_name: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            provider: provider.to_string(),
            description: None,
            context_window: 0,
            max_output_tokens: 0,
            capabilities: ModelCapabilities::default(),
            is_default: false,
            family: String::new(),
            aliases: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_alias(mut self, alias: &str) -> Self {
        if !self.aliases.iter().any(|existing| existing == alias) {
            self.aliases.push(alias.to_string());
        }
        self
    }

    pub fn with_aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for alias in aliases {
            let alias = alias.into();
            if !self.aliases.iter().any(|existing| existing == &alias) {
                self.aliases.push(alias);
            }
        }
        self
    }

    pub fn with_context_window(mut self, context_window: usize) -> Self {
        self.context_window = context_window;
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: usize) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_family(mut self, family: &str) -> Self {
        self.family = family.to_string();
        self
    }

    pub fn as_default(mut self) -> Self {
        self.is_default = true;
        self
    }

    pub fn matches_id(&self, candidate: &str) -> bool {
        self.id == candidate || self.aliases.iter().any(|alias| alias == candidate)
    }

    pub fn preferred_id(&self) -> &str {
        self.aliases
            .first()
            .map(String::as_str)
            .unwrap_or(self.id.as_str())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
    #[serde(default)]
    pub endpoint_transports: Vec<EndpointTransportInfo>,
    pub models: Vec<ModelInfo>,
}

impl ModelCatalog {
    pub fn builtin() -> Self {
        Self::builtin_ref().clone()
    }

    pub fn builtin_ref() -> &'static Self {
        static BUILTIN: OnceLock<ModelCatalog> = OnceLock::new();
        BUILTIN.get_or_init(|| Self {
            providers: Self::builtin_provider_infos(),
            endpoint_transports: Self::builtin_endpoint_transports(),
            models: vec![
                ModelInfo::new("claude-opus-4-6-20250514", "anthropic", "Claude Opus 4.6")
                    .with_description("Most intelligent, 1M context (beta), 128K output")
                    .with_alias("claude-opus-4-6")
                    .with_context_window(200_000)
                    .with_max_output_tokens(32_000)
                    .with_capabilities(ModelCapabilities {
                        supports_tools: true,
                        supports_vision: true,
                        supports_streaming: true,
                        supports_system_prompt: true,
                        supports_json_mode: false,
                    })
                    .with_family("claude"),
                ModelInfo::new(
                    "claude-sonnet-4-6-20250514",
                    "anthropic",
                    "Claude Sonnet 4.6",
                )
                .with_description("Fast flagship Claude with strong coding and reasoning")
                .with_context_window(200_000)
                .with_max_output_tokens(16_000)
                .with_capabilities(ModelCapabilities {
                    supports_tools: true,
                    supports_vision: true,
                    supports_streaming: true,
                    supports_system_prompt: true,
                    supports_json_mode: false,
                })
                .as_default()
                .with_family("claude"),
                ModelInfo::new(
                    "claude-sonnet-4-5-20250929",
                    "anthropic",
                    "Claude Sonnet 4.5",
                )
                .with_description("Best balance of speed and intelligence")
                .with_context_window(200_000)
                .with_max_output_tokens(8_192)
                .with_capabilities(ModelCapabilities {
                    supports_tools: true,
                    supports_vision: true,
                    supports_streaming: true,
                    supports_system_prompt: true,
                    supports_json_mode: false,
                })
                .with_family("claude"),
                ModelInfo::new("claude-haiku-4-5-20251001", "anthropic", "Claude Haiku 4.5")
                    .with_description("Fastest and most compact Claude")
                    .with_context_window(200_000)
                    .with_max_output_tokens(8_192)
                    .with_capabilities(ModelCapabilities {
                        supports_tools: true,
                        supports_vision: true,
                        supports_streaming: true,
                        supports_system_prompt: true,
                        supports_json_mode: false,
                    })
                    .with_family("claude"),
                ModelInfo::new("gpt-4o", "openai", "GPT-4o")
                    .with_description("General-purpose multimodal GPT-4o")
                    .with_context_window(128_000)
                    .with_max_output_tokens(16_384)
                    .with_capabilities(ModelCapabilities {
                        supports_tools: true,
                        supports_vision: true,
                        supports_streaming: true,
                        supports_system_prompt: true,
                        supports_json_mode: true,
                    })
                    .as_default()
                    .with_family("gpt"),
                ModelInfo::new("gpt-4o-mini", "openai", "GPT-4o Mini")
                    .with_description("Fast and low-cost GPT-4o variant")
                    .with_context_window(128_000)
                    .with_max_output_tokens(16_384)
                    .with_capabilities(ModelCapabilities {
                        supports_tools: true,
                        supports_vision: true,
                        supports_streaming: true,
                        supports_system_prompt: true,
                        supports_json_mode: true,
                    })
                    .with_family("gpt"),
                ModelInfo::new("gpt-5.3-codex", "openai", "GPT-5.3 Codex")
                    .with_description("Agentic coding model")
                    .with_family("gpt"),
                ModelInfo::new("gpt-5.2-2025-12-11", "openai", "GPT-5.2")
                    .with_description("Flagship GPT model")
                    .with_family("gpt"),
                ModelInfo::new("gpt-5-mini", "openai", "GPT-5 Mini")
                    .with_description("Fast and affordable GPT-5 tier")
                    .with_family("gpt"),
                ModelInfo::new("gpt-5-nano", "openai", "GPT-5 Nano")
                    .with_description("Ultra-lightweight GPT-5 tier")
                    .with_family("gpt"),
                ModelInfo::new("zai-glm-4.7", "cerebras", "ZAI-GLM 4.7")
                    .with_description("Ultra-fast inference via Cerebras hardware")
                    .as_default()
                    .with_family("glm"),
                ModelInfo::new("llama-3.3-70b", "cerebras", "Llama 3.3 70B")
                    .with_description("Llama served via Cerebras")
                    .with_family("llama"),
                ModelInfo::new(
                    "anthropic.claude-opus-4-6-v1",
                    "bedrock",
                    "Claude Opus 4.6 (Bedrock)",
                )
                .with_description("Most intelligent Claude via AWS Bedrock")
                .with_family("claude"),
                ModelInfo::new(
                    "anthropic.claude-sonnet-4-5-20250929-v1:0",
                    "bedrock",
                    "Claude Sonnet 4.5 (Bedrock)",
                )
                .with_description("Claude Sonnet 4.5 via AWS Bedrock")
                .as_default()
                .with_family("claude"),
                ModelInfo::new(
                    "anthropic.claude-haiku-4-5-20251001-v1:0",
                    "bedrock",
                    "Claude Haiku 4.5 (Bedrock)",
                )
                .with_description("Fast Claude via AWS Bedrock")
                .with_family("claude"),
                ModelInfo::new("amazon.nova-pro-v1:0", "bedrock", "Amazon Nova Pro")
                    .with_description("Amazon foundation model")
                    .with_family("nova"),
                ModelInfo::new("amazon.nova-lite-v1:0", "bedrock", "Amazon Nova Lite")
                    .with_description("Compact Amazon foundation model")
                    .with_family("nova"),
                ModelInfo::new("amazon.nova-micro-v1:0", "bedrock", "Amazon Nova Micro")
                    .with_description("Smallest Amazon foundation model")
                    .with_family("nova"),
                ModelInfo::new("gpt-oss-120b", "groq", "GPT-OSS 120B")
                    .with_description("Ultra-fast 120B model on Groq")
                    .as_default()
                    .with_family("gpt-oss"),
                ModelInfo::new("moonshotai/kimi-k2-instruct-0905", "groq", "Kimi K2")
                    .with_description("Ultra-fast Groq-hosted Kimi")
                    .with_family("kimi"),
                ModelInfo::new("llama-3.3-70b-versatile", "groq", "Llama 3.3 70B")
                    .with_description("Groq-hosted Llama 70B")
                    .with_family("llama"),
                ModelInfo::new("MiniMaxAI/MiniMax-M2", "deepinfra", "MiniMax M2")
                    .with_description("Serverless inference via DeepInfra")
                    .as_default()
                    .with_family("minimax"),
                ModelInfo::new("gemini-3-pro-preview", "google", "Gemini 3 Pro")
                    .with_description("1M context multimodal Gemini")
                    .as_default()
                    .with_family("gemini"),
                ModelInfo::new("gemini-2.5-flash", "google", "Gemini 2.5 Flash")
                    .with_description("Fast Gemini multimodal model")
                    .with_family("gemini"),
                ModelInfo::new("gpt-5.3-codex", "azure", "GPT-5.3 Codex (Azure)")
                    .with_description("Agentic coding via Azure OpenAI")
                    .as_default()
                    .with_family("gpt"),
                ModelInfo::new("gpt-5.2-2025-12-11", "azure", "GPT-5.2 (Azure)")
                    .with_description("GPT-5.2 via Azure OpenAI")
                    .with_family("gpt"),
            ],
        })
    }

    pub fn get(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn find(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.matches_id(id))
    }

    pub fn with_provider_info(mut self, provider: ProviderInfo) -> Self {
        self.providers
            .retain(|existing| existing.key != provider.key);
        self.providers.push(provider);
        self.providers.sort_by(|a, b| a.key.cmp(&b.key));
        self
    }

    pub fn with_model_info(mut self, model: ModelInfo) -> Self {
        self.models
            .retain(|existing| !(existing.provider == model.provider && existing.id == model.id));
        self.models.push(model);
        self
    }

    pub fn with_provider(
        mut self,
        provider: ProviderInfo,
        models: impl IntoIterator<Item = ModelInfo>,
    ) -> Self {
        self = self.with_provider_info(provider);
        for model in models {
            self = self.with_model_info(model);
        }
        self
    }

    pub fn with_endpoint_transport_info(mut self, transport: EndpointTransportInfo) -> Self {
        self.endpoint_transports
            .retain(|existing| existing.key != transport.key);
        self.endpoint_transports.push(transport);
        self.endpoint_transports.sort_by(|a, b| a.key.cmp(&b.key));
        self
    }

    pub fn merge(mut self, other: ModelCatalog) -> Self {
        for provider in other.providers {
            self.providers
                .retain(|existing| existing.key != provider.key);
            self.providers.push(provider);
        }
        self.providers.sort_by(|a, b| a.key.cmp(&b.key));

        for transport in other.endpoint_transports {
            self.endpoint_transports
                .retain(|existing| existing.key != transport.key);
            self.endpoint_transports.push(transport);
        }
        self.endpoint_transports.sort_by(|a, b| a.key.cmp(&b.key));

        for model in other.models {
            self.models.retain(|existing| {
                !(existing.provider == model.provider && existing.id == model.id)
            });
            self.models.push(model);
        }

        self.models.sort_by(|a, b| {
            a.provider
                .cmp(&b.provider)
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.id.cmp(&b.id))
        });
        self
    }

    pub fn from_config(config: ModelCatalogConfig) -> Self {
        let mut catalog = if config.include_builtin {
            Self::builtin()
        } else {
            Self::default()
        };
        for provider in config.providers {
            catalog = catalog.with_provider_info(provider);
        }
        for transport in config.endpoint_transports {
            catalog = catalog.with_endpoint_transport_info(transport);
        }
        for model in config.models {
            catalog = catalog.with_model_info(model);
        }
        catalog
    }

    pub fn for_provider(&self, provider: &str) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    pub fn default_for_provider(&self, provider: &str) -> Option<&ModelInfo> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.is_default)
            .or_else(|| self.models.iter().find(|m| m.provider == provider))
    }

    pub fn providers(&self) -> Vec<&str> {
        let mut providers: Vec<&str> = self
            .providers
            .iter()
            .map(|provider| provider.key.as_str())
            .chain(self.models.iter().map(|m| m.provider.as_str()))
            .collect();
        providers.sort();
        providers.dedup();
        providers
    }

    pub fn ordered_provider_keys(&self) -> Vec<&str> {
        let mut keys = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        for provider in &self.providers {
            if seen.insert(provider.key.as_str()) {
                keys.push(provider.key.as_str());
            }
        }

        let mut model_only_keys = self
            .models
            .iter()
            .map(|model| model.provider.as_str())
            .filter(|provider| !seen.contains(provider))
            .collect::<Vec<_>>();
        model_only_keys.sort_unstable();
        model_only_keys.dedup();
        keys.extend(model_only_keys);

        keys
    }

    pub fn has_provider(&self, provider: &str) -> bool {
        self.providers.iter().any(|info| info.key == provider)
            || self.models.iter().any(|model| model.provider == provider)
    }

    pub fn builtin_provider_infos() -> Vec<ProviderInfo> {
        vec![
            ProviderInfo::api_key("anthropic", "Anthropic", "Available Anthropic models"),
            ProviderInfo::api_key("openai", "OpenAI", "Available OpenAI models")
                .with_endpoint_transport("openai-compatible"),
            ProviderInfo::api_key("cerebras", "Cerebras", "Available Cerebras models")
                .with_endpoint_transport("openai-compatible"),
            ProviderInfo::cloud_credentials(
                "bedrock",
                "AWS Bedrock",
                "Available AWS Bedrock models",
            )
            .with_regions(
                Some("us-east-1"),
                vec![
                    ProviderRegion::new("us-west-2", "US West (Oregon)", "", Some(100)),
                    ProviderRegion::new("us-east-1", "US East (N. Virginia)", "", Some(94)),
                    ProviderRegion::new("us-east-2", "US East (Ohio)", "", Some(64)),
                    ProviderRegion::new("ap-south-1", "Asia Pacific (Mumbai)", "", Some(49)),
                    ProviderRegion::new("ap-northeast-1", "Asia Pacific (Tokyo)", "", Some(46)),
                    ProviderRegion::new("eu-west-1", "Europe (Ireland)", "", Some(39)),
                    ProviderRegion::new("eu-west-2", "Europe (London)", "", Some(38)),
                    ProviderRegion::new("sa-east-1", "South America (São Paulo)", "", Some(32)),
                    ProviderRegion::new("eu-central-1", "Europe (Frankfurt)", "", Some(24)),
                    ProviderRegion::new("eu-west-3", "Europe (Paris)", "", Some(22)),
                    ProviderRegion::new("ap-southeast-2", "Asia Pacific (Sydney)", "", Some(21)),
                    ProviderRegion::new("ap-northeast-2", "Asia Pacific (Seoul)", "", Some(17)),
                    ProviderRegion::new("ap-southeast-1", "Asia Pacific (Singapore)", "", Some(16)),
                    ProviderRegion::new("ca-central-1", "Canada (Central)", "", Some(15)),
                ],
            )
            .with_region_env_var("AWS_REGION"),
            ProviderInfo::api_key("groq", "Groq", "Available Groq models")
                .with_endpoint_transport("openai-compatible"),
            ProviderInfo::api_key("deepinfra", "DeepInfra", "Available DeepInfra models")
                .with_endpoint_transport("openai-compatible"),
            ProviderInfo::api_key("google", "Google AI", "Available Google AI models"),
            ProviderInfo::cloud_credentials(
                "azure",
                "Azure OpenAI",
                "Available Azure OpenAI models",
            )
            .with_endpoint_transport("openai-compatible"),
        ]
    }

    pub fn builtin_endpoint_transports() -> Vec<EndpointTransportInfo> {
        vec![EndpointTransportInfo::new(
            "openai-compatible",
            "OpenAI-Compatible",
            "HTTP APIs exposing an OpenAI-compatible models surface",
        )
        .with_setting(
            ProviderSetting::text(
                "baseUrl",
                "Base URL",
                "Root endpoint URL, for example http://localhost:5001/v1",
                None,
            )
            .required(),
        )
        .with_setting(ProviderSetting::secret(
            "apiKey",
            "API Key",
            "Optional API key sent as a Bearer token",
        ))]
    }

    pub fn provider_info(&self, provider: &str) -> ProviderInfo {
        self.provider_infos()
            .into_iter()
            .find(|info| info.key == provider)
            .unwrap_or_else(|| ProviderInfo::fallback(provider))
    }

    pub fn summary_for_provider(&self, provider: &str) -> ProviderCatalogSummary {
        let provider_info = self.provider_info(provider);
        let display_name = provider_info.display_name.clone();
        let (default_model, description) = match self.default_for_provider(provider) {
            Some(model) => (
                format!("{provider}/{}", model.preferred_id()),
                format!("{} ({display_name})", model.display_name),
            ),
            None => (provider.to_string(), provider_info.description.clone()),
        };

        ProviderCatalogSummary {
            key: provider.to_string(),
            display_name,
            default_model,
            description,
        }
    }

    pub fn provider_infos(&self) -> Vec<ProviderInfo> {
        let mut infos = self.providers.clone();
        let keys = self
            .ordered_provider_keys()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut by_key = std::collections::BTreeMap::new();
        for info in infos.drain(..) {
            by_key.insert(info.key.clone(), info);
        }
        for provider in keys {
            by_key
                .entry(provider.clone())
                .or_insert_with(|| ProviderInfo::fallback(&provider));
        }
        by_key.into_values().collect()
    }

    pub fn endpoint_transport_info(&self, key: &str) -> Option<&EndpointTransportInfo> {
        self.endpoint_transports
            .iter()
            .find(|transport| transport.key == key)
    }

    pub fn endpoint_transport_infos(&self) -> Vec<&EndpointTransportInfo> {
        let mut transports = self.endpoint_transports.iter().collect::<Vec<_>>();
        transports.sort_by(|a, b| a.key.cmp(&b.key));
        transports
    }

    pub fn families(&self) -> Vec<&str> {
        let mut families: Vec<&str> = self.models.iter().map(|m| m.family.as_str()).collect();
        families.sort();
        families.dedup();
        families
    }
}

fn humanize_provider_key(key: &str) -> String {
    key.split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let mut output = String::new();
            if let Some(first) = chars.next() {
                output.push(first.to_ascii_uppercase());
            }
            output.extend(chars);
            output
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_has_models() {
        let catalog = ModelCatalog::builtin();
        assert!(!catalog.models.is_empty());
    }

    #[test]
    fn get_by_id_works() {
        let catalog = ModelCatalog::builtin();
        let model = catalog.get("claude-sonnet-4-6-20250514");
        assert!(model.is_some());
        assert_eq!(model.unwrap().display_name, "Claude Sonnet 4.6");
    }

    #[test]
    fn find_matches_aliases() {
        let catalog = ModelCatalog::builtin();
        let model = catalog.find("claude-opus-4-6").expect("alias lookup");
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.id, "claude-opus-4-6-20250514");
        assert_eq!(model.preferred_id(), "claude-opus-4-6");
    }

    #[test]
    fn provider_infos_include_metadata_only_providers() {
        let catalog = ModelCatalog::builtin();
        let providers = catalog.provider_infos();
        assert!(providers.iter().any(|provider| provider.key == "bedrock"));
        assert!(providers.iter().any(|provider| provider.key == "groq"));
    }

    #[test]
    fn builtin_ref_is_stable() {
        let a = ModelCatalog::builtin_ref() as *const ModelCatalog;
        let b = ModelCatalog::builtin_ref() as *const ModelCatalog;
        assert_eq!(a, b);
    }

    #[test]
    fn ordered_provider_keys_preserve_declared_provider_order() {
        let keys = ModelCatalog::builtin_ref().ordered_provider_keys();
        assert_eq!(
            keys,
            vec![
                "anthropic",
                "openai",
                "cerebras",
                "bedrock",
                "groq",
                "deepinfra",
                "google",
                "azure",
            ]
        );
    }

    #[test]
    fn has_provider_matches_known_and_unknown_keys() {
        let catalog = ModelCatalog::builtin_ref();
        assert!(catalog.has_provider("anthropic"));
        assert!(catalog.has_provider("bedrock"));
        assert!(!catalog.has_provider("does-not-exist"));
    }

    #[test]
    fn bedrock_has_bundled_models_and_default() {
        let catalog = ModelCatalog::builtin();
        let models = catalog.for_provider("bedrock");
        assert!(!models.is_empty());
        assert_eq!(
            catalog
                .default_for_provider("bedrock")
                .map(|model| model.id.as_str()),
            Some("anthropic.claude-sonnet-4-5-20250929-v1:0")
        );
    }

    #[test]
    fn from_config_can_replace_builtin_catalog() {
        let catalog = ModelCatalog::from_config(ModelCatalogConfig {
            include_builtin: false,
            providers: vec![ProviderInfo::without_credentials(
                "custom",
                "Custom",
                "Custom provider",
            )],
            endpoint_transports: vec![EndpointTransportInfo::new(
                "custom-http",
                "Custom HTTP",
                "Custom endpoint transport",
            )
            .with_setting(
                ProviderSetting::text("baseUrl", "Base URL", "Transport root URL", None).required(),
            )],
            models: vec![ModelInfo::new("custom-model", "custom", "Custom Model")
                .with_context_window(16_000)
                .with_max_output_tokens(2_000)
                .with_family("custom")
                .as_default()],
        });

        assert_eq!(catalog.providers(), vec!["custom"]);
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(
            catalog
                .endpoint_transport_info("custom-http")
                .map(|transport| transport.display_name.as_str()),
            Some("Custom HTTP")
        );
        assert_eq!(
            catalog
                .default_for_provider("custom")
                .map(|m| m.id.as_str()),
            Some("custom-model")
        );
    }

    #[test]
    fn summary_for_provider_uses_default_model_when_present() {
        let summary = ModelCatalog::builtin().summary_for_provider("anthropic");
        assert_eq!(summary.key, "anthropic");
        assert_eq!(summary.display_name, "Anthropic");
        assert!(summary.default_model.starts_with("anthropic/"));
        assert!(summary.description.contains("Anthropic"));
    }

    #[test]
    fn summary_for_provider_falls_back_when_no_models_exist() {
        let catalog = ModelCatalog::from_config(ModelCatalogConfig {
            include_builtin: false,
            providers: vec![ProviderInfo::without_credentials(
                "custom",
                "Custom",
                "Available Custom models",
            )],
            endpoint_transports: vec![],
            models: vec![],
        });

        let summary = catalog.summary_for_provider("custom");
        assert_eq!(summary.key, "custom");
        assert_eq!(summary.display_name, "Custom");
        assert_eq!(summary.default_model, "custom");
        assert_eq!(summary.description, "Available Custom models");
    }
}
