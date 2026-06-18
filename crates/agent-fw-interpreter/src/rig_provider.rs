//! Stock Rig completion providers for heterogeneous agent registries.
//!
//! Applications should not need to invent a local enum just to mix
//! Anthropic with one or more OpenAI-compatible vendors.

use std::sync::Arc;

use agent_fw_agent::{
    DynRigAgentFactory, DynRigCompletionProvider, MissingDefaultProvider, RigCompletionProvider,
};
use agent_fw_core::ProviderSettingsMap;
#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;
use rig::client::{CompletionClient, ProviderClient};
use rig::providers::anthropic;
use rig::providers::openai::CompletionsClient as OpenAiCompletionsClient;
use rig_bedrock::client::Client as BedrockClient;

/// Anthropic completion provider with prompt-caching support.
#[derive(Clone)]
pub struct AnthropicCompletionProvider {
    client: anthropic::Client,
}

impl AnthropicCompletionProvider {
    pub fn from_client(client: anthropic::Client) -> Self {
        Self { client }
    }

    pub fn new(api_key: impl Into<String>) -> Result<Self, String> {
        anthropic::Client::builder()
            .api_key(api_key.into())
            .build()
            .map(Self::from_client)
            .map_err(|error| error.to_string())
    }

    pub fn into_dyn(self) -> DynRigCompletionProvider {
        Arc::new(self)
    }
}

impl RigCompletionProvider for AnthropicCompletionProvider {
    #[allow(deprecated)]
    fn completion_model(
        &self,
        model: &str,
        prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        if prompt_caching {
            CompletionModelHandle::new(Arc::new(
                self.client.completion_model(model).with_prompt_caching(),
            ))
        } else {
            CompletionModelHandle::new(Arc::new(self.client.completion_model(model)))
        }
    }

    fn supports_thinking(&self) -> bool {
        true
    }
}

/// OpenAI-compatible completion provider.
#[derive(Clone)]
pub struct OpenAiCompatibleCompletionProvider {
    client: OpenAiCompletionsClient,
    supports_thinking: bool,
}

impl OpenAiCompatibleCompletionProvider {
    pub fn from_client(client: OpenAiCompletionsClient) -> Self {
        Self {
            client,
            supports_thinking: false,
        }
    }

    pub fn new(api_key: impl Into<String>, base_url: &str) -> Result<Self, String> {
        OpenAiCompletionsClient::builder()
            .api_key(api_key.into())
            .base_url(base_url)
            .build()
            .map(Self::from_client)
            .map_err(|error| error.to_string())
    }

    pub fn openai(api_key: impl Into<String>) -> Result<Self, String> {
        Self::new(api_key, "https://api.openai.com/v1")
    }

    pub fn openrouter(api_key: impl Into<String>) -> Result<Self, String> {
        Self::new(api_key, "https://openrouter.ai/api/v1")
    }

    pub fn cerebras(api_key: impl Into<String>) -> Result<Self, String> {
        Self::new(api_key, "https://api.cerebras.ai/v1")
    }

    pub fn groq(api_key: impl Into<String>) -> Result<Self, String> {
        Self::new(api_key, "https://api.groq.com/openai/v1")
    }

    pub fn with_thinking_support(mut self, enabled: bool) -> Self {
        self.supports_thinking = enabled;
        self
    }

    pub fn into_dyn(self) -> DynRigCompletionProvider {
        Arc::new(self)
    }
}

pub fn stock_openai_compatible_base_url(provider_key: &str) -> Option<&'static str> {
    match provider_key {
        "openai" => Some("https://api.openai.com/v1"),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "cerebras" => Some("https://api.cerebras.ai/v1"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        _ => None,
    }
}

impl RigCompletionProvider for OpenAiCompatibleCompletionProvider {
    #[allow(deprecated)]
    fn completion_model(
        &self,
        model: &str,
        _prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        CompletionModelHandle::new(Arc::new(self.client.completion_model(model)))
    }

    fn supports_thinking(&self) -> bool {
        self.supports_thinking
    }
}

/// AWS Bedrock completion provider.
#[derive(Clone, Debug)]
pub struct BedrockCompletionProvider {
    client: BedrockClient,
}

impl BedrockCompletionProvider {
    pub fn from_client(client: BedrockClient) -> Self {
        Self { client }
    }

    pub fn from_env() -> Self {
        Self::from_client(BedrockClient::from_env())
    }

    pub fn with_profile_name(profile_name: &str) -> Self {
        Self::from_client(BedrockClient::with_profile_name(profile_name))
    }

    pub fn into_dyn(self) -> DynRigCompletionProvider {
        Arc::new(self)
    }
}

impl RigCompletionProvider for BedrockCompletionProvider {
    #[allow(deprecated)]
    fn completion_model(
        &self,
        model: &str,
        prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        if prompt_caching {
            CompletionModelHandle::new(Arc::new(
                self.client.completion_model(model).with_prompt_caching(),
            ))
        } else {
            CompletionModelHandle::new(Arc::new(self.client.completion_model(model)))
        }
    }

    fn supports_thinking(&self) -> bool {
        true
    }
}

/// Generic stock provider configuration for heterogeneous Rig registries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StockProviderSpec {
    Anthropic {
        api_key: String,
    },
    OpenAi {
        api_key: String,
    },
    OpenRouter {
        api_key: String,
    },
    Cerebras {
        api_key: String,
    },
    Groq {
        api_key: String,
    },
    BedrockFromEnv,
    OpenAiCompatible {
        key: String,
        api_key: String,
        base_url: String,
        supports_thinking: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StockProviderSettingsOptions {
    pub include_bedrock_from_env: bool,
}

impl StockProviderSpec {
    pub fn key(&self) -> &str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::OpenAi { .. } => "openai",
            Self::OpenRouter { .. } => "openrouter",
            Self::Cerebras { .. } => "cerebras",
            Self::Groq { .. } => "groq",
            Self::BedrockFromEnv => "bedrock",
            Self::OpenAiCompatible { key, .. } => key.as_str(),
        }
    }
}

#[derive(Debug)]
pub enum StockProviderFactoryError {
    ProviderBuild { provider: String, message: String },
    MissingDefault(MissingDefaultProvider),
}

impl std::fmt::Display for StockProviderFactoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderBuild { provider, message } => {
                write!(
                    f,
                    "provider '{}' could not be constructed: {}",
                    provider, message
                )
            }
            Self::MissingDefault(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for StockProviderFactoryError {}

impl From<MissingDefaultProvider> for StockProviderFactoryError {
    fn from(value: MissingDefaultProvider) -> Self {
        Self::MissingDefault(value)
    }
}

/// Build stock provider registry entries from a provider spec list.
pub fn stock_provider_entries(
    specs: impl IntoIterator<Item = StockProviderSpec>,
) -> Result<Vec<(String, DynRigCompletionProvider)>, StockProviderFactoryError> {
    let mut providers = Vec::new();
    for spec in specs {
        let key = spec.key().to_string();
        let provider = match spec {
            StockProviderSpec::Anthropic { api_key } => AnthropicCompletionProvider::new(api_key)
                .map(|p| p.into_dyn())
                .map_err(|message| StockProviderFactoryError::ProviderBuild {
                    provider: key.clone(),
                    message,
                })?,
            StockProviderSpec::OpenAi { api_key } => {
                OpenAiCompatibleCompletionProvider::openai(api_key)
                    .map(|p| p.into_dyn())
                    .map_err(|message| StockProviderFactoryError::ProviderBuild {
                        provider: key.clone(),
                        message,
                    })?
            }
            StockProviderSpec::OpenRouter { api_key } => {
                OpenAiCompatibleCompletionProvider::openrouter(api_key)
                    .map(|p| p.into_dyn())
                    .map_err(|message| StockProviderFactoryError::ProviderBuild {
                        provider: key.clone(),
                        message,
                    })?
            }
            StockProviderSpec::Cerebras { api_key } => {
                OpenAiCompatibleCompletionProvider::cerebras(api_key)
                    .map(|p| p.into_dyn())
                    .map_err(|message| StockProviderFactoryError::ProviderBuild {
                        provider: key.clone(),
                        message,
                    })?
            }
            StockProviderSpec::Groq { api_key } => {
                OpenAiCompatibleCompletionProvider::groq(api_key)
                    .map(|p| p.into_dyn())
                    .map_err(|message| StockProviderFactoryError::ProviderBuild {
                        provider: key.clone(),
                        message,
                    })?
            }
            StockProviderSpec::BedrockFromEnv => BedrockCompletionProvider::from_env().into_dyn(),
            StockProviderSpec::OpenAiCompatible {
                api_key,
                base_url,
                supports_thinking,
                ..
            } => OpenAiCompatibleCompletionProvider::new(api_key, &base_url)
                .map(|p| p.with_thinking_support(supports_thinking).into_dyn())
                .map_err(|message| StockProviderFactoryError::ProviderBuild {
                    provider: key.clone(),
                    message,
                })?,
        };
        providers.push((key, provider));
    }
    Ok(providers)
}

/// Convert canonical provider settings into a stock provider spec when supported.
///
/// The input contract is intentionally generic:
/// - `apiKey` for API-key based providers
/// - `baseUrl` for arbitrary OpenAI-compatible endpoints
/// - `supportsThinking` optional boolean string for custom endpoints
pub fn stock_provider_spec_from_settings(
    provider_key: &str,
    settings: &ProviderSettingsMap,
    options: &StockProviderSettingsOptions,
) -> Option<StockProviderSpec> {
    match provider_key {
        "anthropic" => settings
            .get("apiKey")
            .filter(|v| !v.is_empty())
            .map(|api_key| StockProviderSpec::Anthropic {
                api_key: api_key.clone(),
            }),
        "openai" => settings
            .get("apiKey")
            .filter(|v| !v.is_empty())
            .map(|api_key| StockProviderSpec::OpenAi {
                api_key: api_key.clone(),
            }),
        "openrouter" => settings
            .get("apiKey")
            .filter(|v| !v.is_empty())
            .map(|api_key| StockProviderSpec::OpenRouter {
                api_key: api_key.clone(),
            }),
        "cerebras" => settings
            .get("apiKey")
            .filter(|v| !v.is_empty())
            .map(|api_key| StockProviderSpec::Cerebras {
                api_key: api_key.clone(),
            }),
        "groq" => settings
            .get("apiKey")
            .filter(|v| !v.is_empty())
            .map(|api_key| StockProviderSpec::Groq {
                api_key: api_key.clone(),
            }),
        "bedrock" if options.include_bedrock_from_env => Some(StockProviderSpec::BedrockFromEnv),
        key => {
            let api_key = settings.get("apiKey").filter(|v| !v.is_empty())?;
            let base_url = settings.get("baseUrl").filter(|v| !v.is_empty())?;
            let supports_thinking = settings
                .get("supportsThinking")
                .and_then(|value| value.parse::<bool>().ok())
                .unwrap_or(false);
            Some(StockProviderSpec::OpenAiCompatible {
                key: key.to_string(),
                api_key: api_key.clone(),
                base_url: base_url.clone(),
                supports_thinking,
            })
        }
    }
}

pub fn stock_provider_specs_from_settings<I, K>(
    provider_settings: I,
    options: &StockProviderSettingsOptions,
) -> Vec<StockProviderSpec>
where
    I: IntoIterator<Item = (K, ProviderSettingsMap)>,
    K: Into<String>,
{
    provider_settings
        .into_iter()
        .filter_map(|(provider_key, settings)| {
            let provider_key = provider_key.into();
            stock_provider_spec_from_settings(&provider_key, &settings, options)
        })
        .collect()
}

/// Build a heterogeneous Rig factory from stock provider specs.
pub fn stock_provider_factory(
    default_key: impl Into<String>,
    specs: impl IntoIterator<Item = StockProviderSpec>,
) -> Result<DynRigAgentFactory, StockProviderFactoryError> {
    let default_key = default_key.into();
    let providers = stock_provider_entries(specs)?;
    Ok(DynRigAgentFactory::try_from_providers(
        default_key,
        providers,
    )?)
}

/// Build a heterogeneous Rig factory from canonical provider settings maps.
///
/// This is the low-ceremony path for applications that already represent
/// provider credentials/settings as a generic string map keyed by provider.
pub fn stock_provider_factory_from_settings<I, K>(
    default_key: impl Into<String>,
    provider_settings: I,
    options: StockProviderSettingsOptions,
) -> Result<DynRigAgentFactory, StockProviderFactoryError>
where
    I: IntoIterator<Item = (K, ProviderSettingsMap)>,
    K: Into<String>,
{
    let specs = stock_provider_specs_from_settings(provider_settings, &options);
    stock_provider_factory(default_key, specs)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn anthropic_provider_supports_thinking_and_dyn_conversion() {
        let provider = AnthropicCompletionProvider::new("test-key").expect("provider");
        assert!(provider.supports_thinking());
        let dyn_provider = provider.into_dyn();
        assert!(dyn_provider.supports_thinking());
    }

    #[test]
    fn openai_compatible_provider_defaults_to_no_thinking() {
        let provider = OpenAiCompatibleCompletionProvider::cerebras("test-key").expect("provider");
        assert!(!provider.supports_thinking());
        let provider = provider.with_thinking_support(true);
        assert!(provider.supports_thinking());
    }

    #[test]
    fn bedrock_provider_supports_thinking_and_dyn_conversion() {
        let provider = BedrockCompletionProvider::with_profile_name("test-profile");
        assert!(provider.supports_thinking());
        let dyn_provider = provider.into_dyn();
        assert!(dyn_provider.supports_thinking());
    }

    #[test]
    fn stock_provider_entries_builds_registered_keys() {
        let providers = stock_provider_entries([
            StockProviderSpec::Anthropic {
                api_key: "test-key".to_string(),
            },
            StockProviderSpec::Groq {
                api_key: "test-key".to_string(),
            },
        ])
        .expect("providers");
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].0, "anthropic");
        assert_eq!(providers[1].0, "groq");
    }

    #[test]
    fn stock_provider_factory_requires_default_key() {
        let err = match stock_provider_factory(
            "anthropic",
            [StockProviderSpec::Groq {
                api_key: "test-key".to_string(),
            }],
        ) {
            Ok(_) => panic!("expected missing default provider error"),
            Err(err) => err,
        };
        assert!(matches!(err, StockProviderFactoryError::MissingDefault(_)));
    }

    #[test]
    fn stock_provider_spec_from_settings_builds_known_provider() {
        let settings = BTreeMap::from([("apiKey".to_string(), "test-key".to_string())]);
        let spec = stock_provider_spec_from_settings(
            "anthropic",
            &settings,
            &StockProviderSettingsOptions::default(),
        )
        .unwrap();
        assert_eq!(spec.key(), "anthropic");
    }

    #[test]
    fn stock_provider_spec_from_settings_builds_custom_openai_compatible_provider() {
        let settings = BTreeMap::from([
            ("apiKey".to_string(), "test-key".to_string()),
            (
                "baseUrl".to_string(),
                "http://localhost:11434/v1".to_string(),
            ),
            ("supportsThinking".to_string(), "true".to_string()),
        ]);
        let spec = stock_provider_spec_from_settings(
            "ollama",
            &settings,
            &StockProviderSettingsOptions::default(),
        )
        .unwrap();
        match spec {
            StockProviderSpec::OpenAiCompatible {
                key,
                base_url,
                supports_thinking,
                ..
            } => {
                assert_eq!(key, "ollama");
                assert_eq!(base_url, "http://localhost:11434/v1");
                assert!(supports_thinking);
            }
            other => panic!("unexpected spec: {other:?}"),
        }
    }

    #[test]
    fn stock_provider_specs_from_settings_supports_bedrock_toggle() {
        let specs = stock_provider_specs_from_settings(
            [("bedrock".to_string(), BTreeMap::new())],
            &StockProviderSettingsOptions {
                include_bedrock_from_env: true,
            },
        );
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].key(), "bedrock");
    }
}
