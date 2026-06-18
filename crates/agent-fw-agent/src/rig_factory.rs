//! Ergonomic Rig agent construction on top of [`ModelRouter`].
//!
//! Applications should not have to re-implement the same provider/model shell:
//! - resolve `"provider/model"` specs
//! - create a type-erased Rig completion model
//! - apply prompt caching when supported
//! - enable extended thinking only for providers that support it
//! - start an `AgentBuilder` from an [`AgentBlueprint`]
//!
//! Domain apps should keep tool composition local, but this generic shell
//! belongs in the framework.

use std::sync::Arc;

use crate::{anthropic_reasoning_params, AgentBlueprint, ModelRouter, ResolvedModel};
use rig::agent::{Agent, AgentBuilder, WithBuilderTools};
#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;

/// Type-erased Rig builder used by multi-provider applications.
#[allow(deprecated)]
pub type RigBuilder = AgentBuilder<CompletionModelHandle<'static>>;

/// Type-erased Rig tool builder used after tools are attached.
#[allow(deprecated)]
pub type RigToolBuilder = AgentBuilder<CompletionModelHandle<'static>, (), WithBuilderTools>;

/// Type-erased Rig agent returned by [`RigAgentFactory`].
#[allow(deprecated)]
pub type RigAgent = Agent<CompletionModelHandle<'static>>;

/// Trait-object completion provider for heterogeneous provider registries.
pub type DynRigCompletionProvider = Arc<dyn RigCompletionProvider>;

/// Type-erased Rig agent factory that can mix heterogeneous provider clients.
pub type DynRigAgentFactory = RigAgentFactory<DynRigCompletionProvider>;

/// Provider capability contract required to build Rig agents from a
/// framework [`ModelRouter`].
pub trait RigCompletionProvider: Send + Sync {
    /// Create a type-erased Rig completion model for the given model name.
    ///
    /// Implementations may ignore `prompt_caching` when the underlying
    /// provider/runtime does not support it.
    #[allow(deprecated)]
    fn completion_model(&self, model: &str, prompt_caching: bool)
        -> CompletionModelHandle<'static>;

    /// Whether this provider supports extended thinking.
    fn supports_thinking(&self) -> bool {
        false
    }
}

impl<T> RigCompletionProvider for Arc<T>
where
    T: RigCompletionProvider + ?Sized,
{
    #[allow(deprecated)]
    fn completion_model(
        &self,
        model: &str,
        prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        self.as_ref().completion_model(model, prompt_caching)
    }

    fn supports_thinking(&self) -> bool {
        self.as_ref().supports_thinking()
    }
}

impl<T> RigCompletionProvider for Box<T>
where
    T: RigCompletionProvider + ?Sized,
{
    #[allow(deprecated)]
    fn completion_model(
        &self,
        model: &str,
        prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        self.as_ref().completion_model(model, prompt_caching)
    }

    fn supports_thinking(&self) -> bool {
        self.as_ref().supports_thinking()
    }
}

/// Framework-owned Rig agent factory built on top of [`ModelRouter`].
///
/// This owns the generic provider/model shell while leaving tool selection
/// and prompt policy with the application.
pub struct RigAgentFactory<P> {
    router: ModelRouter<P>,
}

/// Error returned when constructing a [`RigAgentFactory`] from a provider list
/// that does not contain the declared default provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("default provider '{default_key}' is not registered")]
pub struct MissingDefaultProvider {
    default_key: String,
}

impl MissingDefaultProvider {
    pub fn default_key(&self) -> &str {
        &self.default_key
    }
}

impl<P> RigAgentFactory<P> {
    /// Create a new factory with a default provider.
    pub fn new(default_key: impl Into<String>, default_provider: P) -> Self {
        Self {
            router: ModelRouter::new(default_key, default_provider),
        }
    }

    /// Add a provider.
    pub fn with_provider(mut self, key: impl Into<String>, provider: P) -> Self {
        self.router = self.router.with_provider(key, provider);
        self
    }

    /// Build a factory from a provider list and an explicit default key.
    pub fn try_from_providers(
        default_key: impl Into<String>,
        providers: impl IntoIterator<Item = (String, P)>,
    ) -> Result<Self, MissingDefaultProvider> {
        let default_key = default_key.into();
        let mut default_provider = None;
        let mut remaining = Vec::new();

        for (key, provider) in providers {
            if key == default_key {
                default_provider = Some(provider);
            } else {
                remaining.push((key, provider));
            }
        }

        let mut factory = Self::new(
            default_key.clone(),
            default_provider.ok_or_else(|| MissingDefaultProvider {
                default_key: default_key.clone(),
            })?,
        );
        for (key, provider) in remaining {
            factory = factory.with_provider(key, provider);
        }
        Ok(factory)
    }

    /// Resolve a model specification using the underlying router.
    pub fn resolve(&self, spec: &str) -> Option<ResolvedModel<'_, P>> {
        self.router.resolve(spec)
    }

    /// Resolve a model specification, falling back to the default provider
    /// when the prefix is unknown.
    pub fn resolve_or_default(&self, spec: &str) -> Option<ResolvedModel<'_, P>> {
        self.router.resolve_or_default(spec)
    }

    /// Get a provider by key.
    pub fn provider(&self, key: &str) -> Option<&P> {
        self.router.provider(key)
    }

    /// Get the default provider.
    pub fn default_provider(&self) -> Option<&P> {
        self.router.default_provider()
    }

    /// Get the default provider key.
    pub fn default_key(&self) -> &str {
        self.router.default_key()
    }

    /// Check if a provider is registered.
    pub fn has_provider(&self, key: &str) -> bool {
        self.router.has_provider(key)
    }

    /// List available provider keys.
    pub fn available_providers(&self) -> Vec<&str> {
        self.router.available_providers()
    }

    /// Access the underlying router.
    pub fn router(&self) -> &ModelRouter<P> {
        &self.router
    }
}

impl RigAgentFactory<DynRigCompletionProvider> {
    /// Create a heterogeneous provider factory with one default provider.
    pub fn new_dyn(
        default_key: impl Into<String>,
        default_provider: impl RigCompletionProvider + 'static,
    ) -> Self {
        Self::new(default_key, Arc::new(default_provider))
    }

    /// Add one heterogeneous provider without forcing the caller to wrap it.
    pub fn with_dyn_provider(
        self,
        key: impl Into<String>,
        provider: impl RigCompletionProvider + 'static,
    ) -> Self {
        self.with_provider(key, Arc::new(provider))
    }
}

impl<P: RigCompletionProvider> RigAgentFactory<P> {
    /// Resolve a model spec into a type-erased Rig completion model.
    #[allow(deprecated)]
    pub fn resolve_completion_model(
        &self,
        model_spec: &str,
        prompt_caching: bool,
    ) -> CompletionModelHandle<'static> {
        let resolved = self
            .resolve_or_default(model_spec)
            .expect("default provider must exist");
        resolved
            .provider
            .completion_model(&resolved.model_name, prompt_caching)
    }

    /// Resolve the effective extended-thinking budget for the given model spec.
    ///
    /// Providers that do not support thinking always return `0`.
    pub fn thinking_budget_for(&self, model_spec: &str, base_budget: u32) -> u32 {
        if base_budget == 0 {
            return 0;
        }

        let resolved = self
            .resolve_or_default(model_spec)
            .expect("default provider must exist");

        if resolved.provider.supports_thinking() {
            base_budget
        } else {
            0
        }
    }

    /// Start a Rig [`AgentBuilder`] from the given blueprint.
    #[allow(deprecated)]
    pub fn agent_builder(&self, blueprint: &AgentBlueprint) -> RigBuilder {
        let completion_model =
            self.resolve_completion_model(blueprint.model.as_str(), blueprint.prompt_caching);
        let mut builder = AgentBuilder::new(completion_model).preamble(&blueprint.system_prompt);
        if let Some(max_tokens) = blueprint.max_tokens {
            builder = builder.max_tokens(max_tokens as u64);
        }
        builder
    }

    /// Start a Rig [`AgentBuilder`] and apply extended thinking only when the
    /// resolved provider supports it.
    #[allow(deprecated)]
    pub fn agent_builder_with_thinking(
        &self,
        blueprint: &AgentBlueprint,
        base_budget: u32,
    ) -> RigBuilder {
        let mut builder = self.agent_builder(blueprint);
        let thinking_budget = self.thinking_budget_for(blueprint.model.as_str(), base_budget);
        if let Some(params) = anthropic_reasoning_params(
            blueprint.model.as_str(),
            self.resolve_or_default(blueprint.model.as_str())
                .expect("default provider must exist")
                .provider
                .supports_thinking(),
            blueprint.thinking_budget.unwrap_or(thinking_budget),
            blueprint.reasoning_effort,
        ) {
            builder = builder.additional_params(params);
        }
        builder
    }

    /// Build a Rig agent from a blueprint by applying an application-supplied
    /// tool/configuration recipe.
    ///
    /// Applications keep tool selection local, while the framework owns the
    /// generic builder shell and the final `default_max_turns(...).build()`
    /// ceremony.
    #[allow(deprecated)]
    pub fn build_agent<F>(
        &self,
        blueprint: &AgentBlueprint,
        max_turns: usize,
        configure: F,
    ) -> RigAgent
    where
        F: FnOnce(RigBuilder) -> RigToolBuilder,
    {
        configure(self.agent_builder(blueprint))
            .default_max_turns(max_turns)
            .build()
    }

    /// Build a Rig agent like [`Self::build_agent`] but only enables extended
    /// thinking when the resolved provider supports it.
    #[allow(deprecated)]
    pub fn build_agent_with_thinking<F>(
        &self,
        blueprint: &AgentBlueprint,
        base_budget: u32,
        max_turns: usize,
        configure: F,
    ) -> RigAgent
    where
        F: FnOnce(RigBuilder) -> RigToolBuilder,
    {
        configure(self.agent_builder_with_thinking(blueprint, base_budget))
            .default_max_turns(max_turns)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(deprecated)]
    use rig::client::completion::CompletionModelHandle;
    use rig::client::CompletionClient;
    use rig::providers::anthropic;
    use std::sync::Arc;

    #[derive(Clone)]
    struct TestProvider {
        client: anthropic::Client,
        supports_thinking: bool,
    }

    impl TestProvider {
        fn new(supports_thinking: bool) -> Self {
            let client = anthropic::Client::builder()
                .api_key("test-key")
                .build()
                .expect("anthropic client");
            Self {
                client,
                supports_thinking,
            }
        }
    }

    impl RigCompletionProvider for TestProvider {
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
            self.supports_thinking
        }
    }

    fn test_factory() -> RigAgentFactory<TestProvider> {
        RigAgentFactory::new("anthropic", TestProvider::new(true))
    }

    #[test]
    fn dyn_factory_accepts_heterogeneous_provider_objects() {
        let factory = RigAgentFactory::new_dyn("anthropic", TestProvider::new(true))
            .with_dyn_provider("backup", TestProvider::new(false));

        assert!(factory.has_provider("anthropic"));
        assert!(factory.has_provider("backup"));
        assert_eq!(factory.thinking_budget_for("backup/model", 4_000), 0);
    }

    #[test]
    fn try_from_providers_builds_registry_from_list() {
        let factory = RigAgentFactory::try_from_providers(
            "anthropic",
            vec![
                ("anthropic".to_string(), TestProvider::new(true)),
                ("backup".to_string(), TestProvider::new(false)),
            ],
        )
        .expect("factory");

        assert_eq!(factory.default_key(), "anthropic");
        assert!(factory.has_provider("anthropic"));
        assert!(factory.has_provider("backup"));
    }

    #[test]
    fn try_from_providers_requires_default_provider() {
        let error = match RigAgentFactory::try_from_providers(
            "anthropic",
            vec![("backup".to_string(), TestProvider::new(false))],
        ) {
            Ok(_) => panic!("missing default should fail"),
            Err(error) => error,
        };

        assert_eq!(error.default_key(), "anthropic");
    }

    #[test]
    fn thinking_budget_uses_provider_capability() {
        let factory = test_factory();
        assert_eq!(factory.thinking_budget_for("claude-sonnet-4", 8_000), 8_000);

        let factory = RigAgentFactory::new("openai", TestProvider::new(false));
        assert_eq!(factory.thinking_budget_for("gpt-4.1", 8_000), 0);
    }

    #[test]
    fn thinking_budget_unknown_prefix_falls_back_to_default_provider() {
        let factory = test_factory();
        assert_eq!(
            factory.thinking_budget_for("cerebras/zai-glm-4.7", 4_000),
            4_000
        );
    }

    #[test]
    fn resolve_completion_model_accepts_bare_and_prefixed_specs() {
        let factory = RigAgentFactory::new("anthropic", TestProvider::new(true))
            .with_provider("backup", TestProvider::new(false));

        let _ = factory.resolve_completion_model("claude-sonnet-4", true);
        let _ = factory.resolve_completion_model("backup/claude-haiku-4", false);
    }

    #[test]
    fn agent_builder_accepts_framework_blueprint() {
        let factory = test_factory();
        let blueprint = AgentBlueprint::new("claude-sonnet-4", "You are helpful.");

        let _ = factory.agent_builder(&blueprint);
        let _ = factory.agent_builder_with_thinking(&blueprint, 8_000);
    }

    #[tokio::test]
    async fn build_agent_finishes_common_builder_path() {
        let factory = test_factory();
        let blueprint = AgentBlueprint::new("claude-sonnet-4", "You are helpful.");

        let _ = factory.build_agent(&blueprint, 4, |builder| builder.tool(crate::CalculatorTool));
        let _ = factory.build_agent_with_thinking(&blueprint, 8_000, 4, |builder| {
            builder.tool(crate::GetCurrentTimeTool)
        });
    }
}
