//! Provider routing for per-agent chat interpreters.
//!
//! `RuntimeSpec.providers` is metadata: it tells the harness which providers
//! the spec expects to use. `RuntimeDeps.interpreter_providers` supplies the
//! effectful [`ChatInterpreter`](agent_fw_agent::ChatInterpreter) instances
//! under the same keys. Keeping this router here keeps provider names, id
//! families, and Flow AI routing policy out of `agent-fw-agent`.

use crate::ModelSpec;

/// Resolve `model` to a provider key.
///
/// `model.provider`, when present, wins — that is the explicit
/// disambiguation path the abstractions doc reserves for ambiguous ids
/// (decision A7). Otherwise, the id family routes by string prefix:
///
/// - `claude-*` → `"anthropic"`
/// - `anthropic.*` / `us.anthropic.*` (Bedrock ARNs) → `"bedrock"`
/// - `gpt-*`, `o1-*`, `o3-*` → `"openai-compatible"`
///
/// Unmapped families fall through to `"anthropic"`. Callers first validate
/// the key against [`RuntimeSpec::providers`](crate::RuntimeSpec::providers)
/// and then look it up in
/// [`RuntimeDeps::interpreter_providers`](crate::RuntimeDeps::interpreter_providers).
pub fn select_provider_key(model: &ModelSpec) -> String {
    if let Some(provider) = &model.provider {
        return provider.clone();
    }
    auto_route_by_id_family(&model.id).to_string()
}

fn auto_route_by_id_family(model_id: &str) -> &'static str {
    if model_id.starts_with("claude-") {
        "anthropic"
    } else if model_id.starts_with("anthropic.") || model_id.starts_with("us.anthropic.") {
        "bedrock"
    } else if model_id.starts_with("gpt-")
        || model_id.starts_with("o1-")
        || model_id.starts_with("o3-")
    {
        "openai-compatible"
    } else {
        "anthropic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_routes_claude_family_to_anthropic() {
        let model = ModelSpec::new("claude-sonnet-4-6");
        assert_eq!(select_provider_key(&model), "anthropic");
    }

    #[test]
    fn auto_routes_bedrock_arn_to_bedrock() {
        let model = ModelSpec::new("anthropic.claude-3-5-sonnet-20240620-v1:0");
        assert_eq!(select_provider_key(&model), "bedrock");
        let model = ModelSpec::new("us.anthropic.claude-3-7-sonnet-20250219-v1:0");
        assert_eq!(select_provider_key(&model), "bedrock");
    }

    #[test]
    fn auto_routes_openai_family_to_openai_compatible() {
        let model = ModelSpec::new("gpt-4o-mini");
        assert_eq!(select_provider_key(&model), "openai-compatible");
        let model = ModelSpec::new("o3-mini");
        assert_eq!(select_provider_key(&model), "openai-compatible");
    }

    #[test]
    fn explicit_provider_field_overrides_id_family() {
        let model = ModelSpec {
            id: "claude-sonnet-4-6".to_string(),
            provider: Some("bedrock".to_string()),
        };
        assert_eq!(select_provider_key(&model), "bedrock");
    }
}
