//! Model identification, routing, and agent configuration.

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::HashMap;

/// Domain-neutral label attached to an agent registration.
///
/// The framework treats this as opaque metadata for logging, metrics, and
/// consumer-side pattern matching. Harnesses that need semantic roles own
/// their role protocol and may map those roles to labels at the framework
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentLabel(String);

impl AgentLabel {
    /// Create an opaque agent label.
    pub fn new(label: impl Into<String>) -> Self {
        Self(label.into())
    }

    /// Borrow the label text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AgentLabel {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AgentLabel {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Model identifier newtype.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId(String);

impl ModelId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ModelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for ModelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Anthropic reasoning-effort hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

/// Pure runtime model settings shared by frontend, Studio, and interpreters.
///
/// This is a description value. Interpreters decide how to map it onto a
/// provider protocol. The defaults intentionally match the framework's
/// current Claude 4.6 policy: high effort, adaptive thinking (`0` manual
/// budget), prompt caching enabled, and a high response-token ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    /// Hard response cap. Must be greater than zero.
    #[serde(rename = "maxTokens")]
    pub max_tokens: u32,
    /// Manual thinking-token budget. `0` means adaptive/no manual cap.
    #[serde(rename = "thinkingBudgetTokens", alias = "thinkingBudget")]
    pub thinking_budget: u32,
    /// Anthropic effort hint.
    #[serde(rename = "reasoningEffort")]
    pub reasoning_effort: ReasoningEffort,
    /// Whether provider prompt caching should be enabled where supported.
    #[serde(rename = "cacheControl")]
    pub cache_control: bool,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            max_tokens: 16_384,
            thinking_budget: 0,
            reasoning_effort: ReasoningEffort::High,
            cache_control: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelSettingsError {
    #[error("max_tokens must be greater than zero")]
    InvalidMaxTokens,
}

impl ModelSettings {
    /// Build validated model settings.
    pub fn new(
        max_tokens: u32,
        thinking_budget: u32,
        reasoning_effort: ReasoningEffort,
        cache_control: bool,
    ) -> Result<Self, ModelSettingsError> {
        if max_tokens == 0 {
            return Err(ModelSettingsError::InvalidMaxTokens);
        }
        Ok(Self {
            max_tokens,
            thinking_budget,
            reasoning_effort,
            cache_control,
        })
    }

    /// Return a new value with optional overrides applied.
    pub fn with_overrides(
        self,
        max_tokens: Option<u32>,
        thinking_budget: Option<u32>,
        reasoning_effort: Option<ReasoningEffort>,
        cache_control: Option<bool>,
    ) -> Result<Self, ModelSettingsError> {
        Self::new(
            max_tokens.unwrap_or(self.max_tokens),
            thinking_budget.unwrap_or(self.thinking_budget),
            reasoning_effort.unwrap_or(self.reasoning_effort),
            cache_control.unwrap_or(self.cache_control),
        )
    }
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

impl Default for ReasoningEffort {
    fn default() -> Self {
        Self::High
    }
}

pub fn anthropic_model_supports_effort(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-opus-4-5")
}

pub fn anthropic_model_supports_adaptive_thinking(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.contains("claude-opus-4-6") || normalized.contains("claude-sonnet-4-6")
}

pub fn anthropic_model_supports_max_effort(model: &str) -> bool {
    model
        .trim()
        .to_ascii_lowercase()
        .contains("claude-opus-4-6")
}

fn normalize_anthropic_effort(model: &str, effort: ReasoningEffort) -> ReasoningEffort {
    if matches!(effort, ReasoningEffort::Max) && !anthropic_model_supports_max_effort(model) {
        ReasoningEffort::High
    } else {
        effort
    }
}

pub fn anthropic_reasoning_params(
    model: &str,
    supports_thinking: bool,
    thinking_budget: u32,
    reasoning_effort: Option<ReasoningEffort>,
) -> Option<JsonValue> {
    let mut params = JsonMap::new();

    if let Some(effort) = reasoning_effort.filter(|_| anthropic_model_supports_effort(model)) {
        params.insert(
            "output_config".to_string(),
            serde_json::json!({
                "effort": normalize_anthropic_effort(model, effort).as_str(),
            }),
        );
    }

    if supports_thinking && anthropic_model_supports_adaptive_thinking(model) {
        if thinking_budget > 0 {
            params.insert(
                "thinking".to_string(),
                serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": thinking_budget,
                }),
            );
        } else {
            params.insert(
                "thinking".to_string(),
                serde_json::json!({
                    "type": "adaptive",
                }),
            );
        }
    } else if supports_thinking && thinking_budget > 0 {
        params.insert(
            "thinking".to_string(),
            serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking_budget,
            }),
        );
    }

    if params.is_empty() {
        None
    } else {
        Some(JsonValue::Object(params))
    }
}

/// Agent configuration (immutable after creation).
///
/// Describes the model, system prompt, and behavior of an agent
/// without specifying its tools (those come from `ToolSuite`).
#[derive(Debug, Clone)]
pub struct AgentBlueprint {
    /// Model to use for this agent.
    pub model: ModelId,
    /// System prompt.
    pub system_prompt: String,
    /// Whether to enable prompt caching.
    pub prompt_caching: bool,
    /// Budget for extended thinking (token count). `None` = thinking disabled.
    pub thinking_budget: Option<u32>,
    /// Anthropic reasoning effort hint.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Maximum tokens for the response. `None` = model default (typically 8192).
    pub max_tokens: Option<u32>,
}

impl AgentBlueprint {
    pub fn new(model: impl Into<ModelId>, system_prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: system_prompt.into(),
            prompt_caching: false,
            thinking_budget: None,
            reasoning_effort: None,
            max_tokens: None,
        }
    }

    /// Enable/disable prompt caching.
    pub fn prompt_caching(mut self, enabled: bool) -> Self {
        self.prompt_caching = enabled;
        self
    }

    /// Explicitly disable prompt caching.
    pub fn without_caching(mut self) -> Self {
        self.prompt_caching = false;
        self
    }

    /// Set extended thinking budget (token count).
    pub fn thinking_budget(mut self, budget: u32) -> Self {
        self.thinking_budget = Some(budget);
        self
    }

    /// Set Anthropic reasoning effort.
    pub fn reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Set maximum response tokens.
    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Apply a validated model settings description to this blueprint.
    pub fn model_settings(mut self, settings: ModelSettings) -> Self {
        self.prompt_caching = settings.cache_control;
        self.thinking_budget = Some(settings.thinking_budget);
        self.reasoning_effort = Some(settings.reasoning_effort);
        self.max_tokens = Some(settings.max_tokens);
        self
    }

    // Keep old names as aliases for backward compatibility.
    #[doc(hidden)]
    pub fn with_prompt_caching(self, enabled: bool) -> Self {
        self.prompt_caching(enabled)
    }

    #[doc(hidden)]
    pub fn with_thinking_budget(self, budget: u32) -> Self {
        self.thinking_budget(budget)
    }

    #[doc(hidden)]
    pub fn with_reasoning_effort(self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort(effort)
    }

    #[doc(hidden)]
    pub fn with_max_tokens(self, max_tokens: u32) -> Self {
        self.max_tokens(max_tokens)
    }

    #[doc(hidden)]
    pub fn with_model_settings(self, settings: ModelSettings) -> Self {
        self.model_settings(settings)
    }
}

/// Multi-provider model router.
///
/// Routes model specifications like `"provider/model"` to the correct
/// provider client. Supports O(1) lookup by provider key.
///
/// # Example
///
/// ```ignore
/// let router = ModelRouter::new()
///     .with_provider("anthropic", anthropic_client)
///     .with_provider("cerebras", cerebras_client)
///     .with_default("anthropic");
///
/// let resolved = router.resolve("cerebras/llama-4"); // → Cerebras + "llama-4"
/// ```
pub struct ModelRouter<P> {
    providers: HashMap<String, P>,
    default_key: String,
}

/// Result of resolving a model spec.
pub struct ResolvedModel<'a, P> {
    pub provider: &'a P,
    pub model_name: String,
}

impl<P> ModelRouter<P> {
    /// Create a new router with a default provider.
    pub fn new(default_key: impl Into<String>, default_provider: P) -> Self {
        let default_key = default_key.into();
        let mut providers = HashMap::new();
        providers.insert(default_key.clone(), default_provider);
        Self {
            providers,
            default_key,
        }
    }

    /// Add a provider.
    pub fn with_provider(mut self, key: impl Into<String>, provider: P) -> Self {
        self.providers.insert(key.into(), provider);
        self
    }

    /// Resolve a model specification.
    ///
    /// Format: `"provider/model"` or just `"model"` (uses default provider).
    /// Returns `None` if the provider is not registered.
    pub fn resolve(&self, spec: &str) -> Option<ResolvedModel<'_, P>> {
        if let Some((provider_key, model_name)) = spec.split_once('/') {
            self.providers.get(provider_key).map(|p| ResolvedModel {
                provider: p,
                model_name: model_name.to_string(),
            })
        } else {
            self.providers
                .get(&self.default_key)
                .map(|p| ResolvedModel {
                    provider: p,
                    model_name: spec.to_string(),
                })
        }
    }

    /// Resolve a model specification, falling back to the default provider when
    /// the provider prefix is unknown.
    ///
    /// Unknown prefixes are stripped before fallback so `"unknown/model-x"`
    /// resolves to `(default, "model-x")`, not `(default, "unknown/model-x")`.
    ///
    /// This is the right policy for UIs that let users switch providers at
    /// runtime: passing the full unrecognized spec through to any provider is
    /// always worse than using the default provider with the intended model name.
    ///
    /// **Invariant**: `ModelRouter::new` always registers the default provider,
    /// so this returns `Some` for any router constructed through the public API.
    pub fn resolve_or_default(&self, spec: &str) -> Option<ResolvedModel<'_, P>> {
        if let Some((provider_key, model_name)) = spec.split_once('/') {
            if let Some(provider) = self.providers.get(provider_key) {
                return Some(ResolvedModel {
                    provider,
                    model_name: model_name.to_string(),
                });
            }

            return self.default_provider().map(|provider| ResolvedModel {
                provider,
                model_name: model_name.to_string(),
            });
        }

        self.default_provider().map(|provider| ResolvedModel {
            provider,
            model_name: spec.to_string(),
        })
    }

    /// Get a provider by key.
    pub fn provider(&self, key: &str) -> Option<&P> {
        self.providers.get(key)
    }

    /// Get the default provider.
    ///
    /// **Invariant**: `new()` always inserts the default provider, and no public
    /// method removes keys, so this always returns `Some`. Returns `Option` for
    /// totality — callers that trust the invariant can `.unwrap()` in tests.
    pub fn default_provider(&self) -> Option<&P> {
        self.providers.get(&self.default_key)
    }

    /// Get the default provider key.
    pub fn default_key(&self) -> &str {
        &self.default_key
    }

    /// Check if a provider is registered.
    pub fn has_provider(&self, key: &str) -> bool {
        self.providers.contains_key(key)
    }

    /// List available provider keys.
    pub fn available_providers(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_display() {
        let id = ModelId::new("claude-opus-4-6");
        assert_eq!(id.to_string(), "claude-opus-4-6");
    }

    #[test]
    fn model_id_from_str() {
        let id: ModelId = "claude-sonnet-4".into();
        assert_eq!(id.as_str(), "claude-sonnet-4");
    }

    #[test]
    fn model_id_from_string() {
        let id: ModelId = String::from("claude-haiku-4").into();
        assert_eq!(id.as_str(), "claude-haiku-4");
    }

    #[test]
    fn agent_blueprint_creation() {
        // new() accepts impl Into<ModelId> — no ModelId::new() needed
        let bp = AgentBlueprint::new("model", "You are helpful");
        assert_eq!(bp.model.as_str(), "model");
        assert_eq!(bp.system_prompt, "You are helpful");
        assert!(!bp.prompt_caching);
    }

    #[test]
    fn agent_blueprint_with_caching() {
        let bp = AgentBlueprint::new("model", "prompt").prompt_caching(true);
        assert!(bp.prompt_caching);
    }

    #[test]
    fn agent_blueprint_without_caching() {
        let bp = AgentBlueprint::new("model", "prompt")
            .prompt_caching(true)
            .without_caching();
        assert!(!bp.prompt_caching);
    }

    #[test]
    fn agent_blueprint_backward_compat() {
        // Old with_ prefix still works
        let bp = AgentBlueprint::new(ModelId::new("model"), "prompt")
            .with_prompt_caching(true)
            .with_thinking_budget(8192)
            .with_reasoning_effort(ReasoningEffort::Medium)
            .with_max_tokens(4096);
        assert!(bp.prompt_caching);
        assert_eq!(bp.thinking_budget, Some(8192));
        assert_eq!(bp.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(bp.max_tokens, Some(4096));
    }

    #[test]
    fn model_settings_default_matches_framework_policy() {
        let settings = ModelSettings::default();
        assert_eq!(settings.max_tokens, 16_384);
        assert_eq!(settings.thinking_budget, 0);
        assert_eq!(settings.reasoning_effort, ReasoningEffort::High);
        assert!(settings.cache_control);
    }

    #[test]
    fn model_settings_rejects_truncating_zero_max_tokens() {
        assert_eq!(
            ModelSettings::new(0, 0, ReasoningEffort::High, true).unwrap_err(),
            ModelSettingsError::InvalidMaxTokens
        );
    }

    #[test]
    fn model_settings_overrides_are_immutable_and_precise() {
        let base = ModelSettings::default();
        let updated = base
            .with_overrides(
                Some(4096),
                Some(1024),
                Some(ReasoningEffort::Max),
                Some(false),
            )
            .expect("model settings override");

        assert_eq!(base, ModelSettings::default());
        assert_eq!(
            updated,
            ModelSettings {
                max_tokens: 4096,
                thinking_budget: 1024,
                reasoning_effort: ReasoningEffort::Max,
                cache_control: false,
            }
        );
    }

    #[test]
    fn model_settings_deserializes_frontend_wire_shape() {
        let settings: ModelSettings = serde_json::from_value(serde_json::json!({
            "maxTokens": 8192,
            "thinkingBudgetTokens": 0,
            "reasoningEffort": "max",
            "cacheControl": true,
        }))
        .expect("frontend model settings");

        assert_eq!(
            settings,
            ModelSettings {
                max_tokens: 8192,
                thinking_budget: 0,
                reasoning_effort: ReasoningEffort::Max,
                cache_control: true,
            }
        );
    }

    #[test]
    fn agent_blueprint_accepts_model_settings_description() {
        let settings =
            ModelSettings::new(8192, 0, ReasoningEffort::High, true).expect("valid model settings");
        let bp = AgentBlueprint::new("claude-opus-4-6", "prompt").model_settings(settings);

        assert_eq!(bp.max_tokens, Some(8192));
        assert_eq!(bp.thinking_budget, Some(0));
        assert_eq!(bp.reasoning_effort, Some(ReasoningEffort::High));
        assert!(bp.prompt_caching);
    }

    #[test]
    fn anthropic_reasoning_params_use_adaptive_for_4_6_with_zero_budget() {
        let params =
            anthropic_reasoning_params("claude-opus-4-6", true, 0, Some(ReasoningEffort::High))
                .unwrap();

        assert_eq!(params["thinking"]["type"], "adaptive");
        assert_eq!(params["output_config"]["effort"], "high");
    }

    #[test]
    fn anthropic_reasoning_params_low_effort_zero_budget_keeps_adaptive_thinking() {
        let params =
            anthropic_reasoning_params("claude-sonnet-4-6", true, 0, Some(ReasoningEffort::Low))
                .unwrap();

        assert_eq!(params["thinking"]["type"], "adaptive");
        assert_eq!(params["output_config"]["effort"], "low");
    }

    #[test]
    fn anthropic_reasoning_params_use_manual_budget_when_limited() {
        let params = anthropic_reasoning_params(
            "claude-opus-4-6",
            true,
            4096,
            Some(ReasoningEffort::Medium),
        )
        .unwrap();

        assert_eq!(params["thinking"]["type"], "enabled");
        assert_eq!(params["thinking"]["budget_tokens"], 4096);
        assert_eq!(params["output_config"]["effort"], "medium");
    }

    #[test]
    fn model_router_resolves_default() {
        let router = ModelRouter::new("anthropic", "client-a");
        let resolved = router.resolve("claude-opus-4-6").unwrap();
        assert_eq!(resolved.provider, &"client-a");
        assert_eq!(resolved.model_name, "claude-opus-4-6");
    }

    #[test]
    fn model_router_resolves_with_provider() {
        let router =
            ModelRouter::new("anthropic", "client-a").with_provider("cerebras", "client-c");

        let resolved = router.resolve("cerebras/llama-4").unwrap();
        assert_eq!(resolved.provider, &"client-c");
        assert_eq!(resolved.model_name, "llama-4");
    }

    #[test]
    fn model_router_unknown_provider() {
        let router = ModelRouter::new("anthropic", "client-a");
        assert!(router.resolve("unknown/model").is_none());
    }

    #[test]
    fn model_router_resolve_or_default_strips_unknown_prefix() {
        let router = ModelRouter::new("anthropic", "client-a");
        let resolved = router.resolve_or_default("unknown/model").unwrap();
        assert_eq!(resolved.provider, &"client-a");
        assert_eq!(resolved.model_name, "model");
    }

    #[test]
    fn model_router_introspection_helpers() {
        let router =
            ModelRouter::new("anthropic", "client-a").with_provider("cerebras", "client-c");
        assert_eq!(router.default_key(), "anthropic");
        assert!(router.has_provider("anthropic"));
        assert!(router.has_provider("cerebras"));
        assert!(!router.has_provider("bedrock"));

        let mut providers = router.available_providers();
        providers.sort_unstable();
        assert_eq!(providers, vec!["anthropic", "cerebras"]);
    }

    // ─── AgentLabel ────────────────────────────────────────────────

    #[test]
    fn agent_label_is_transparent_string_metadata() {
        let label = AgentLabel::new("planner");

        assert_eq!(label.as_str(), "planner");
        assert_eq!(label.to_string(), "planner");
        assert_eq!(serde_json::to_string(&label).unwrap(), "\"planner\"");

        let parsed: AgentLabel = serde_json::from_str("\"planner\"").unwrap();
        assert_eq!(parsed, label);
    }
}
