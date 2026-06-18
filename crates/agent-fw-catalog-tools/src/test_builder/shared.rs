use std::sync::Arc;

use agent_fw_agent::ToolCallResult;
use agent_fw_eval::{BuilderSessionStoreConfig, TestCaseBuilderSession};
use agent_fw_tool::ToolEnvironment;

#[derive(Clone)]
pub(crate) struct BuilderToolkitConfig {
    pub catalog: Option<Arc<dyn agent_fw_eval::ToolCatalog>>,
    pub sessions: BuilderSessionStoreConfig,
}

impl Default for BuilderToolkitConfig {
    fn default() -> Self {
        Self {
            catalog: None,
            sessions: BuilderSessionStoreConfig::default(),
        }
    }
}

pub(crate) async fn load_or_create_session(
    config: &BuilderToolkitConfig,
    env: &ToolEnvironment,
    session_id: &str,
) -> Result<TestCaseBuilderSession, String> {
    let tenant = env.tenant().resource_id().as_str();
    config
        .sessions
        .load_or_create(env.kv().as_ref(), tenant, session_id)
        .await
        .map_err(|e| format!("failed to load session '{session_id}': {e}"))
}

pub(crate) async fn mutate_session<F, T, E>(
    config: &BuilderToolkitConfig,
    env: &ToolEnvironment,
    session_id: &str,
    f: F,
) -> Result<(TestCaseBuilderSession, T), String>
where
    F: FnOnce(&mut TestCaseBuilderSession) -> Result<T, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let tenant = env.tenant().resource_id().as_str();
    config
        .sessions
        .mutate(env.kv().as_ref(), tenant, session_id, f)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn summary_payload(session: &TestCaseBuilderSession) -> serde_json::Value {
    let summary = session.summary();
    serde_json::json!({
        "sessionId": summary.session_id,
        "stepCount": summary.step_count,
        "tools": summary.tool_names,
        "mode": summary.trajectory_mode,
        "hasGroundTruth": summary.has_ground_truth,
        "tagCount": summary.tag_count,
    })
}

pub(crate) fn parse_input<T: serde::de::DeserializeOwned>(
    tool_use_id: &str,
    input: serde_json::Value,
) -> Result<T, ToolCallResult> {
    serde_json::from_value(input)
        .map_err(|e| ToolCallResult::error(tool_use_id, format!("Invalid input: {e}")))
}

pub(crate) fn guarded(tool_use_id: &str, env: &ToolEnvironment) -> Result<(), ToolCallResult> {
    env.ensure_active()
        .map_err(|e| ToolCallResult::error(tool_use_id, e.to_string()))
}
