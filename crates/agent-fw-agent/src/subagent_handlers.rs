//! Sub-agent delegation tool handlers.
//!
//! The framework already ships every piece needed to delegate from one
//! agent to another at the *algebra* level:
//!
//! - [`SubAgentInvoker`](agent_fw_algebra::SubAgentInvoker) — the trait an
//!   agent network exposes for sibling invocation.
//! - [`AgentOrchestrator`](crate::AgentOrchestrator) — the canonical
//!   `SubAgentInvoker` impl.
//! - [`ToolEnvironment::sub_agents()`](agent_fw_tool::ToolEnvironment::sub_agents)
//!   — the accessor the env exposes to handlers.
//!
//! What was missing was a generic [`ToolHandler`] that bridges the LLM
//! tool-use loop to that algebra. Without it, the LLM in a coordinator
//! agent has no way to invoke a planner or executor sub-agent — even
//! though everything else (the orchestrator, the sub-agent trait, the
//! env accessor) is already in place. This module fills that gap so any
//! consumer (the Flow AI Harness runtime, vertical adapters, third-party agent
//! systems) gets the same delegation primitive.
//!
//! [`CallAgentHandler`] is the single canonical delegation tool. Consumers
//! that want a different schema (e.g. a typed `delegate(role, message)`
//! shape) wrap or replace this handler in their own dispatcher.

use agent_fw_algebra::sub_agent::SubAgentRequest;
use agent_fw_algebra::SubAgentError;
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;

use crate::{ToolCallResult, ToolDefinition, ToolHandler};

/// Generic `call_agent(agent, prompt)` handler bridging the LLM tool-use
/// loop to [`agent_fw_algebra::SubAgentInvoker`].
///
/// This is the framework's canonical delegation tool. It reads the
/// `SubAgentInvoker` from the [`ToolEnvironment`] — installed by the
/// caller via `ToolEnvironment::builder().sub_agents_arc(...)` — and
/// forwards a freshly-built [`SubAgentRequest`]. No state lives on the
/// handler; one `Arc<CallAgentHandler>` can be cloned into any dispatcher
/// and works against whatever sub-agent invoker the env carries.
///
/// Consumers register this handler in agents that should be able to
/// delegate (typically a coordinator). The orchestrator emits its own
/// `sub_agent_call` / `sub_agent_result` framing around the invocation,
/// so this handler's own success payload only needs the final response
/// text and routing metadata.
///
/// # Laws
///
/// - **L1 (Transparency)**: the handler never decorates the response —
///   the LLM sees exactly what the sub-agent produced as `response`.
/// - **L2 (No silent fallbacks)**: an unknown agent name surfaces as a
///   tool error with the list of available agents, not a quiet no-op.
#[derive(Debug, Clone, Default)]
pub struct CallAgentHandler;

#[async_trait]
impl ToolHandler for CallAgentHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "call_agent".into(),
            description: "Invoke a named sub-agent with a prompt. The sub-agent runs in its \
                own thread scope and streams its own events; this tool returns the sub-agent's \
                final response text. Use to delegate planning, execution, or specialist work \
                to a registered sub-agent."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "Name of the sub-agent to invoke (must be registered in the orchestrator)."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Task or instruction for the sub-agent."
                    }
                },
                "required": ["agent", "prompt"]
            }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let agent = match input.get("agent").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolCallResult::error(tool_use_id, "Missing 'agent' name string"),
        };
        let prompt = match input.get("prompt").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolCallResult::error(tool_use_id, "Missing 'prompt' string"),
        };

        let invoker = env.sub_agents();
        if !invoker.has_agent(&agent) {
            let available = invoker.available_agents();
            return ToolCallResult::error(
                tool_use_id,
                format!("Unknown sub-agent '{agent}'. Available agents: {available:?}"),
            );
        }

        match invoker
            .invoke(SubAgentRequest::new(agent.clone(), prompt).with_invocation_id(tool_use_id))
            .await
        {
            Ok(result) => ToolCallResult::success(
                tool_use_id,
                serde_json::json!({
                    "agent": result.agent_name,
                    "response": result.response,
                    "model": result.model,
                }),
            ),
            Err(SubAgentError::NotFound(a)) => {
                ToolCallResult::error(tool_use_id, format!("Sub-agent not found: {a}"))
            }
            Err(SubAgentError::Cancelled) => {
                ToolCallResult::error(tool_use_id, "Sub-agent invocation cancelled")
            }
            Err(SubAgentError::AgentFailed(msg)) => {
                ToolCallResult::error(tool_use_id, format!("Sub-agent failed: {msg}"))
            }
            Err(SubAgentError::Internal(msg)) => {
                ToolCallResult::error(tool_use_id, format!("Internal sub-agent error: {msg}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::sub_agent::SubAgentResult;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_algebra::{CancellationToken, EventSink, KVStore, SubAgentInvoker};
    use agent_fw_core::tenant::TenantContext;
    use agent_fw_core::usage::TokenUsage;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    /// A `SubAgentInvoker` for testing — records the most recent request
    /// and replies with a canned response. Allows asserting that
    /// `CallAgentHandler` forwards correctly without spinning up a full
    /// orchestrator.
    struct StubInvoker {
        agents: Vec<String>,
        seen: Arc<StdMutex<Option<SubAgentRequest>>>,
    }

    impl StubInvoker {
        fn new(agents: Vec<&str>) -> Self {
            Self {
                agents: agents.into_iter().map(String::from).collect(),
                seen: Arc::new(StdMutex::new(None)),
            }
        }
    }

    #[async_trait]
    impl SubAgentInvoker for StubInvoker {
        async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
            *self.seen.lock().unwrap() = Some(request.clone());
            Ok(SubAgentResult::new(
                request.agent_name.clone(),
                request.resolved_invocation_id(),
                format!("{} done: {}", request.agent_name, request.prompt),
                TokenUsage::ZERO,
                "test-model",
            ))
        }
        fn has_agent(&self, name: &str) -> bool {
            self.agents.iter().any(|a| a == name)
        }
        fn available_agents(&self) -> Vec<String> {
            self.agents.clone()
        }
    }

    fn env_with_invoker(invoker: Arc<dyn SubAgentInvoker>) -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test"));
        ToolEnvironment::new(kv, sink, invoker, tenant, cancel)
    }

    fn bare_env() -> ToolEnvironment {
        env_with_invoker(Arc::new(NullSubAgentInvoker))
    }

    #[tokio::test]
    async fn call_agent_definition_advertises_required_fields() {
        let def = CallAgentHandler.definition();
        assert_eq!(def.name, "call_agent");
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "agent"));
        assert!(required.iter().any(|v| v == "prompt"));
    }

    #[tokio::test]
    async fn call_agent_forwards_to_env_sub_agents_and_returns_response() {
        let stub = Arc::new(StubInvoker::new(vec!["planner", "executor"]));
        let env = env_with_invoker(stub.clone() as Arc<dyn SubAgentInvoker>);

        let result = CallAgentHandler
            .handle(
                "tu-1",
                serde_json::json!({"agent": "planner", "prompt": "build a plan"}),
                &env,
            )
            .await;

        assert!(!result.is_error, "call_agent errored: {result:?}");
        assert_eq!(result.content["agent"], "planner");
        assert_eq!(result.content["response"], "planner done: build a plan");
        let seen = stub.seen.lock().unwrap().clone().expect("invoker called");
        assert_eq!(seen.agent_name, "planner");
        assert_eq!(seen.prompt, "build a plan");
        assert_eq!(seen.invocation_id.as_deref(), Some("tu-1"));
    }

    #[tokio::test]
    async fn call_agent_rejects_unknown_agent_with_available_list() {
        let stub = Arc::new(StubInvoker::new(vec!["planner"]));
        let env = env_with_invoker(stub as Arc<dyn SubAgentInvoker>);
        let result = CallAgentHandler
            .handle(
                "tu-2",
                serde_json::json!({"agent": "executor", "prompt": "go"}),
                &env,
            )
            .await;
        assert!(result.is_error);
        let msg = result.content["error"].as_str().unwrap();
        assert!(msg.contains("Unknown sub-agent 'executor'"));
        assert!(msg.contains("planner"));
    }

    #[tokio::test]
    async fn call_agent_validates_input_shape() {
        let stub = Arc::new(StubInvoker::new(vec!["planner"]));
        let env = env_with_invoker(stub as Arc<dyn SubAgentInvoker>);

        let missing_agent = CallAgentHandler
            .handle("tu-3", serde_json::json!({"prompt": "x"}), &env)
            .await;
        assert!(missing_agent.is_error);

        let missing_prompt = CallAgentHandler
            .handle("tu-4", serde_json::json!({"agent": "planner"}), &env)
            .await;
        assert!(missing_prompt.is_error);

        let empty_agent = CallAgentHandler
            .handle(
                "tu-5",
                serde_json::json!({"agent": "", "prompt": "x"}),
                &env,
            )
            .await;
        assert!(empty_agent.is_error);
    }

    #[tokio::test]
    async fn call_agent_against_null_invoker_reports_unknown_agent() {
        // L2: no silent fallback — even with no agents registered, the
        // user gets a clear error rather than a quiet no-op.
        let env = bare_env();
        let result = CallAgentHandler
            .handle(
                "tu-6",
                serde_json::json!({"agent": "anything", "prompt": "x"}),
                &env,
            )
            .await;
        assert!(result.is_error);
    }
}
