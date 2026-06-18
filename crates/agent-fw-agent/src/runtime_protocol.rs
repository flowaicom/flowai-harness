use std::collections::{BTreeMap, HashMap};

use agent_fw_core::{ToolCompositionOverride, ToolDispatchOverrides, ToolRegistryOverride};
use serde::{Deserialize, Serialize};

use crate::model::ReasoningEffort;

/// Optional override for routing a logical agent role to a specific endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEndpointOverride {
    pub transport: String,
    pub settings: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_model: Option<String>,
}

/// Minimal chat request contract accepted by framework-native runtimes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatRequest {
    pub thread_id: String,
    pub messages: Vec<AgentChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_models: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_prompts: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_dispatch_overrides: Option<ToolDispatchOverrides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_composition_override: Option<ToolCompositionOverride>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_registry_override: Option<ToolRegistryOverride>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_endpoints: Option<HashMap<String, AgentEndpointOverride>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatMessage {
    pub role: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_chat_request_serializes_canonical_frontend_wire_shape() {
        let request = AgentChatRequest {
            thread_id: "sample-case-1".to_string(),
            messages: vec![AgentChatMessage {
                role: "user".to_string(),
                content: "Look up DEMO-1".to_string(),
            }],
            agent_models: None,
            agent_prompts: None,
            tool_dispatch_overrides: None,
            tool_composition_override: None,
            tool_registry_override: None,
            agent_endpoints: None,
            max_tokens: Some(4096),
            thinking_budget_tokens: Some(0),
            reasoning_effort: Some(ReasoningEffort::High),
            cache_control: Some(true),
            agent_id: None,
            role: Some("analyst".to_string()),
            session_id: None,
        };

        assert_eq!(
            serde_json::to_value(request).expect("serialize request"),
            json!({
                "threadId": "sample-case-1",
                "messages": [{"role": "user", "content": "Look up DEMO-1"}],
                "maxTokens": 4096,
                "thinkingBudgetTokens": 0,
                "reasoningEffort": "high",
                "cacheControl": true,
                "role": "analyst"
            })
        );
    }
}
