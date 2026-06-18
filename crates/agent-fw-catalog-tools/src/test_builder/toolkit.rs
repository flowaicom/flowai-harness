//! Canonical test-case-builder toolkit.
//!
//! This composes the generic trajectory-authoring workflow with the remaining
//! generic builder tools:
//! - `listEvalTools`
//! - `setStructuredGroundTruth`
//! - `getGroundTruth`
//!
//! Applications should use this toolkit instead of mixing local wrappers with
//! `TrajectoryAuthoringToolKit`.

use std::{sync::Arc, time::Duration};

use agent_fw_agent::{ComposedDispatcher, ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_eval::{GroundTruth, TestCaseBuilderSession, ToolCatalog};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::shared::{
    guarded, load_or_create_session, mutate_session, parse_input, summary_payload,
    BuilderToolkitConfig,
};
use super::trajectory_toolkit::{HandlerError, TrajectoryAuthoringToolKit};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdInput {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetGroundTruthInput {
    session_id: String,
    ground_truth: GroundTruth,
}

/// List available evaluatable tools from the configured catalog.
pub struct ListEvalToolsHandler {
    catalog: Arc<dyn ToolCatalog>,
}

impl ListEvalToolsHandler {
    pub fn new(catalog: Arc<dyn ToolCatalog>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl ToolHandler for ListEvalToolsHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "listEvalTools".into(),
            description: "List all evaluatable tools that may appear in authored trajectories."
                .into(),
            input_schema: json!({ "type": "object", "properties": {}, "required": [] }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        _input: serde_json::Value,
        _env: &ToolEnvironment,
    ) -> ToolCallResult {
        let entries = self.catalog.entries();
        ToolCallResult::success(
            tool_use_id,
            json!({
                "tools": entries,
                "count": entries.len(),
            }),
        )
    }
}

/// Persist validated structured ground truth on the builder session.
pub struct SetStructuredGroundTruthHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl SetStructuredGroundTruthHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for SetStructuredGroundTruthHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "setStructuredGroundTruth".into(),
            description:
                "Set or replace structured ground truth for the builder session. Supports text, structured, flat, and multiGroup variants."
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "groundTruth": {
                        "type": "object",
                        "description": "Typed ground truth object with a 'kind' discriminator"
                    }
                },
                "required": ["sessionId", "groundTruth"]
            }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        if let Err(result) = guarded(tool_use_id, env) {
            return result;
        }
        let input = match parse_input::<SetGroundTruthInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let session_id = input.session_id;
        let ground_truth = input.ground_truth;

        match mutate_session(&self.config, env, &session_id, |session| {
            session
                .set_ground_truth(ground_truth.clone())
                .map_err(|reasons| HandlerError(reasons.join("; ")))?;
            Ok::<(), HandlerError>(())
        })
        .await
        {
            Ok((session, _)) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "kind": ground_truth.kind_name(),
                    "actionCount": ground_truth.action_count(),
                    "groupCount": ground_truth.group_count(),
                    "trajectorySummary": summary_payload(&session),
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

/// Read the current ground truth from the builder session.
pub struct GetGroundTruthHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl GetGroundTruthHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for GetGroundTruthHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "getGroundTruth".into(),
            description: "Read the current structured ground truth from the builder session."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" }
                },
                "required": ["sessionId"]
            }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        if let Err(result) = guarded(tool_use_id, env) {
            return result;
        }
        let input = match parse_input::<SessionIdInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };

        match load_or_create_session(&self.config, env, &input.session_id).await {
            Ok(session) => {
                let ground_truth = session.ground_truth().cloned();
                let summary = ground_truth
                    .as_ref()
                    .map_or_else(|| "No ground truth set".to_string(), GroundTruth::summary);
                ToolCallResult::success(
                    tool_use_id,
                    json!({
                        "groundTruth": ground_truth,
                        "summary": summary,
                    }),
                )
            }
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

/// Full builder toolkit: trajectory authoring + tool catalog + ground truth.
pub struct TestBuilderToolKit {
    config: Arc<BuilderToolkitConfig>,
    trajectory: TrajectoryAuthoringToolKit,
}

impl TestBuilderToolKit {
    pub fn new(catalog: Arc<dyn ToolCatalog>) -> Self {
        let mut config = BuilderToolkitConfig::default();
        config.catalog = Some(Arc::clone(&catalog));
        Self {
            config: Arc::new(config),
            trajectory: TrajectoryAuthoringToolKit::new().with_tool_catalog(catalog),
        }
    }

    pub fn with_builder_sessions(
        mut self,
        sessions: agent_fw_eval::BuilderSessionStoreConfig,
    ) -> Self {
        let config = Arc::make_mut(&mut self.config);
        config.sessions = sessions.clone();
        self.trajectory = self.trajectory.with_builder_sessions(sessions);
        self
    }

    pub fn with_session_ttl(mut self, ttl: Option<Duration>) -> Self {
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config.sessions.clone().with_ttl(ttl);
        self.trajectory = self.trajectory.with_session_ttl(ttl);
        self
    }

    pub fn with_session_key_fn<F>(mut self, session_key: F) -> Self
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        let session_key = Arc::new(session_key);
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config.sessions.clone().with_session_key_fn({
            let session_key = session_key.clone();
            move |tenant, session_id| session_key(tenant, session_id)
        });
        self.trajectory = self
            .trajectory
            .with_session_key_fn(move |tenant, session_id| session_key(tenant, session_id));
        self
    }

    pub fn with_session_factory<F>(mut self, session_factory: F) -> Self
    where
        F: Fn(&str) -> TestCaseBuilderSession + Send + Sync + 'static,
    {
        let session_factory = Arc::new(session_factory);
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config.sessions.clone().with_session_factory({
            let session_factory = session_factory.clone();
            move |session_id| session_factory(session_id)
        });
        self.trajectory = self
            .trajectory
            .with_session_factory(move |session_id| session_factory(session_id));
        self
    }

    pub fn handlers(&self) -> Vec<Arc<dyn ToolHandler>> {
        let mut handlers = self.trajectory.handlers();
        let catalog = self
            .config
            .catalog
            .clone()
            .expect("TestBuilderToolKit requires a tool catalog");
        handlers.push(Arc::new(ListEvalToolsHandler::new(catalog)));
        handlers.push(Arc::new(SetStructuredGroundTruthHandler::new(Arc::clone(
            &self.config,
        ))));
        handlers.push(Arc::new(GetGroundTruthHandler::new(Arc::clone(
            &self.config,
        ))));
        handlers
    }

    pub fn len(&self) -> usize {
        11
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn into_dispatcher(self, env: ToolEnvironment) -> ComposedDispatcher {
        ComposedDispatcher::new(env).with_handlers(self.handlers())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agent_fw_algebra::testing::{NullEventSink, NullSubAgentInvoker};
    use agent_fw_algebra::{KVStore, KVStoreExt};
    use agent_fw_core::{id::TenantId, tenant::TenantContext};
    use agent_fw_eval::{ToolCatalogEntry, VecToolCatalog};
    use agent_fw_interpreter::DashMapKVStore;
    use agent_fw_tool::ToolEnvironment;

    fn rich_catalog() -> Arc<dyn ToolCatalog> {
        Arc::new(VecToolCatalog::from_entries(vec![
            ToolCatalogEntry::named("draft_plan")
                .with_description("Create a plan")
                .with_category("planning"),
            ToolCatalogEntry::named("approve_plan")
                .with_description("Execute a plan")
                .with_category("execution"),
        ]))
    }

    #[test]
    fn toolkit_has_11_tools() {
        let kit = TestBuilderToolKit::new(rich_catalog());
        assert_eq!(kit.len(), 11);
    }

    #[test]
    fn handlers_include_ground_truth_and_list_tools() {
        let kit = TestBuilderToolKit::new(rich_catalog());
        let names: Vec<String> = kit
            .handlers()
            .iter()
            .map(|handler| handler.definition().name.clone())
            .collect();
        assert!(names.contains(&"listEvalTools".to_string()));
        assert!(names.contains(&"setStructuredGroundTruth".to_string()));
        assert!(names.contains(&"getGroundTruth".to_string()));
        assert!(names.contains(&"composeTrajectory".to_string()));
    }

    #[test]
    fn list_eval_tools_uses_catalog_entries() {
        let handler = ListEvalToolsHandler::new(rich_catalog());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let env = ToolEnvironment::builder()
            .kv_arc(kv)
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("test-tenant")))
            .build();
        let result = tokio_test::block_on(handler.handle("tool-1", json!({}), &env));
        let value = result.content;
        let tools = value.get("tools").and_then(|v| v.as_array()).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools[0].get("name").and_then(|v| v.as_str()),
            Some("draft_plan")
        );
        assert_eq!(
            tools[0].get("description").and_then(|v| v.as_str()),
            Some("Create a plan")
        );
    }

    #[test]
    fn get_ground_truth_description_mentions_structured_variant() {
        let handler =
            SetStructuredGroundTruthHandler::new(Arc::new(BuilderToolkitConfig::default()));
        let definition = handler.definition();
        assert!(definition.description.contains("structured"));
        assert!(definition.description.contains("multiGroup"));
    }

    #[tokio::test]
    async fn toolkit_uses_explicit_builder_session_policy() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let env = ToolEnvironment::builder()
            .kv_arc(Arc::clone(&kv))
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("test-tenant")))
            .build();
        let kit = TestBuilderToolKit::new(rich_catalog()).with_builder_sessions(
            agent_fw_eval::BuilderSessionStoreConfig::default()
                .with_session_key_fn(|tenant, session_id| format!("{tenant}:builder:{session_id}"))
                .with_session_factory(|session_id| TestCaseBuilderSession::new(session_id, "")),
        );
        let handler = kit
            .handlers()
            .into_iter()
            .find(|handler| handler.definition().name == "setStructuredGroundTruth")
            .expect("setStructuredGroundTruth handler");

        let result = handler
            .handle(
                "tool-1",
                json!({
                    "sessionId": "sess-9",
                    "groundTruth": {
                        "kind": "text",
                        "text": "answer"
                    }
                }),
                &env,
            )
            .await;
        assert!(!result.is_error);

        let stored = kv
            .get::<TestCaseBuilderSession>("test-tenant", "test-tenant:builder:sess-9")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.input(), "");
        assert!(stored.ground_truth().is_some());
    }
}
