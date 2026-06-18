//! Generic trajectory-authoring tools for interactive test-case builders.
//!
//! These tools own the reusable builder-session workflow:
//! - thread previews
//! - trajectory composition
//! - trajectory mutation
//! - trace-segment import
//! - trajectory mode changes
//!
//! Applications still control:
//! - session key layout
//! - session TTL
//! - optional tool-name validation policy

use std::{sync::Arc, time::Duration};

use agent_fw_agent::{ComposedDispatcher, ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_eval::{
    TestCaseBuilderError, TestCaseBuilderSession, ToolCatalog, TrajectoryMode, TrajectorySource,
    TrajectoryStep, TrajectoryStepSource,
};
use agent_fw_tool::ToolEnvironment;
use agent_fw_workspace::{
    extract_thread_tool_segment, list_thread_summaries, ThreadSegmentError,
    WorkspaceToolEnvironmentExt,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::shared::{
    guarded, load_or_create_session, mutate_session, parse_input, summary_payload,
    BuilderToolkitConfig,
};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct HandlerError(pub(crate) String);

impl From<TestCaseBuilderError> for HandlerError {
    fn from(value: TestCaseBuilderError) -> Self {
        Self(value.to_string())
    }
}

fn collect_invalid_tool_names<'a>(
    catalog: Option<&dyn ToolCatalog>,
    tool_names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let Some(catalog) = catalog else {
        return Vec::new();
    };
    tool_names
        .into_iter()
        .filter(|name| !catalog.is_valid(name))
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListThreadsInput {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComposeTrajectoryToolEntry {
    name: String,
    source: Option<TrajectoryStepSource>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComposeTrajectoryInput {
    session_id: String,
    tools: Vec<ComposeTrajectoryToolEntry>,
    mode: Option<TrajectoryMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddTrajectoryStepInput {
    session_id: String,
    tool_name: String,
    position: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveTrajectoryStepInput {
    session_id: String,
    position: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderTrajectoryStepInput {
    session_id: String,
    from_position: usize,
    to_position: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeTraceSegmentInput {
    session_id: String,
    thread_id: String,
    from_index: usize,
    to_index: usize,
    insert_at: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdInput {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetTrajectoryModeInput {
    session_id: String,
    mode: TrajectoryMode,
}

pub struct ListThreadsHandler;

#[async_trait]
impl ToolHandler for ListThreadsHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "listThreads".into(),
            description: "List available chat threads with preview data (title, message count, tool call count, first user message). Call before mergeTraceSegment to discover thread IDs.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of threads to return (default 20, max 50)"
                    }
                },
                "required": []
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
        let input = match parse_input::<ListThreadsInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let workspace = match env.try_workspace() {
            Ok(workspace) => workspace,
            Err(err) => return ToolCallResult::error(tool_use_id, err.to_string()),
        };
        let limit = input.limit.unwrap_or(20).min(50);
        match list_thread_summaries(workspace.as_ref(), env.tenant().resource_id(), limit).await {
            Ok(result) => ToolCallResult::success(tool_use_id, json!(result)),
            Err(err) => ToolCallResult::error(tool_use_id, err.to_string()),
        }
    }
}

pub struct ComposeTrajectoryHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl ComposeTrajectoryHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for ComposeTrajectoryHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "composeTrajectory".into(),
            description: "Set or replace the entire composed trajectory. Validates tool names when a catalog is configured.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "tools": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "Tool name" },
                                "source": { "description": "Optional provenance metadata" }
                            },
                            "required": ["name"]
                        },
                        "description": "Ordered list of tool steps"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["anyOrder", "inOrder", "subset"],
                        "description": "Trajectory matching mode"
                    }
                },
                "required": ["sessionId", "tools"]
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
        let input = match parse_input::<ComposeTrajectoryInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };

        let invalid = collect_invalid_tool_names(
            self.config.catalog.as_deref(),
            input.tools.iter().map(|tool| tool.name.as_str()),
        );
        if !invalid.is_empty() {
            return ToolCallResult::error(
                tool_use_id,
                format!(
                    "Invalid tool names: {:?}. Use listEvalTools to see valid names.",
                    invalid
                ),
            );
        }

        let session_id = input.session_id;
        let mode = input.mode;
        let tools = input.tools;
        match mutate_session(&self.config, env, &session_id, |session| {
            let steps = tools
                .into_iter()
                .enumerate()
                .map(|(position, tool)| TrajectoryStep {
                    tool_name: tool.name,
                    source: tool.source.unwrap_or_else(TrajectoryStepSource::manual),
                    position,
                })
                .collect();
            session
                .replace_trajectory_steps(steps, vec![TrajectorySource::manual()])
                .map_err(HandlerError::from)?;
            if let Some(mode) = mode {
                session.set_mode(mode);
            }
            Ok::<(), HandlerError>(())
        })
        .await
        {
            Ok((session, _)) => ToolCallResult::success(
                tool_use_id,
                json!({ "trajectorySummary": summary_payload(&session) }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct AddTrajectoryStepHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl AddTrajectoryStepHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for AddTrajectoryStepHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "addTrajectoryStep".into(),
            description:
                "Insert a single step into the trajectory at the given position (or append).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "toolName": { "type": "string", "description": "Tool name" },
                    "position": { "type": "integer", "description": "Insert position (0-indexed). Omit to append." }
                },
                "required": ["sessionId", "toolName"]
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
        let input = match parse_input::<AddTrajectoryStepInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        if let Some(catalog) = self.config.catalog.as_deref() {
            if !catalog.is_valid(&input.tool_name) {
                return ToolCallResult::error(
                    tool_use_id,
                    format!(
                        "Invalid tool name: '{}'. Use listEvalTools to see valid names.",
                        input.tool_name
                    ),
                );
            }
        }

        let session_id = input.session_id;
        let tool_name = input.tool_name;
        let position = input.position;
        match mutate_session(&self.config, env, &session_id, |session| {
            let len = session.trajectory_steps.len();
            let inserted_at = position.unwrap_or(len);
            if inserted_at == len {
                session
                    .add_step(tool_name, TrajectoryStepSource::manual())
                    .map_err(HandlerError::from)?;
            } else {
                session
                    .insert_step(inserted_at, tool_name, TrajectoryStepSource::manual())
                    .map_err(HandlerError::from)?;
            }
            Ok::<usize, HandlerError>(inserted_at)
        })
        .await
        {
            Ok((session, inserted_at)) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "trajectorySummary": summary_payload(&session),
                    "insertedAt": inserted_at,
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct RemoveTrajectoryStepHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl RemoveTrajectoryStepHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for RemoveTrajectoryStepHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "removeTrajectoryStep".into(),
            description: "Remove a step from the trajectory by position.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "position": { "type": "integer", "description": "Position of the step to remove (0-indexed)" }
                },
                "required": ["sessionId", "position"]
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
        let input = match parse_input::<RemoveTrajectoryStepInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let session_id = input.session_id;
        let position = input.position;
        match mutate_session(&self.config, env, &session_id, |session| {
            let removed = session.remove_step(position).map_err(HandlerError::from)?;
            Ok::<String, HandlerError>(removed.tool_name)
        })
        .await
        {
            Ok((session, removed)) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "trajectorySummary": summary_payload(&session),
                    "removed": removed,
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct ReorderTrajectoryStepHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl ReorderTrajectoryStepHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for ReorderTrajectoryStepHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "reorderTrajectoryStep".into(),
            description: "Move a trajectory step from one position to another.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "fromPosition": { "type": "integer", "description": "Current position of the step" },
                    "toPosition": { "type": "integer", "description": "Target position for the step" }
                },
                "required": ["sessionId", "fromPosition", "toPosition"]
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
        let input = match parse_input::<ReorderTrajectoryStepInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let session_id = input.session_id;
        match mutate_session(&self.config, env, &session_id, |session| {
            session
                .move_step(input.from_position, input.to_position)
                .map_err(HandlerError::from)?;
            Ok::<(), HandlerError>(())
        })
        .await
        {
            Ok((session, _)) => ToolCallResult::success(
                tool_use_id,
                json!({ "trajectorySummary": summary_payload(&session) }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct MergeTraceSegmentHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl MergeTraceSegmentHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for MergeTraceSegmentHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "mergeTraceSegment".into(),
            description:
                "Import tool calls from an existing chat thread trace into the composed trajectory."
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "threadId": { "type": "string", "description": "Chat thread ID to extract from" },
                    "fromIndex": { "type": "integer", "description": "Start index in the extracted tool calls (inclusive)" },
                    "toIndex": { "type": "integer", "description": "End index in the extracted tool calls (exclusive)" },
                    "insertAt": { "type": "integer", "description": "Position to insert in trajectory (omit to append)" }
                },
                "required": ["sessionId", "threadId", "fromIndex", "toIndex"]
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
        let input = match parse_input::<MergeTraceSegmentInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let workspace = match env.try_workspace() {
            Ok(workspace) => workspace,
            Err(err) => return ToolCallResult::error(tool_use_id, err.to_string()),
        };
        let tool_calls = match extract_thread_tool_segment(
            workspace.as_ref(),
            env.tenant().resource_id(),
            &input.thread_id,
            input.from_index,
            input.to_index,
        )
        .await
        {
            Ok(tool_calls) => tool_calls,
            Err(ThreadSegmentError::Workspace(err)) => {
                return ToolCallResult::error(tool_use_id, err.to_string())
            }
            Err(err) => return ToolCallResult::error(tool_use_id, err.to_string()),
        };
        if let Err(result) = guarded(tool_use_id, env) {
            return result;
        }

        let merged_steps: Vec<TrajectoryStep> = tool_calls
            .iter()
            .map(|tool_call| {
                TrajectoryStep::from_thread(
                    tool_call.tool_name.clone(),
                    0,
                    input.thread_id.clone(),
                    tool_call.index,
                )
            })
            .collect();
        let warnings = collect_invalid_tool_names(
            self.config.catalog.as_deref(),
            merged_steps.iter().map(|step| step.tool_name.as_str()),
        )
        .into_iter()
        .map(|name| format!("'{}' not in planner catalog", name))
        .collect::<Vec<_>>();

        let session_id = input.session_id;
        let thread_id = input.thread_id;
        let from_index = input.from_index;
        let to_index = input.to_index;
        let insert_at = input.insert_at;
        let merged_count = merged_steps.len();
        match mutate_session(&self.config, env, &session_id, |session| {
            let insert_pos = insert_at.unwrap_or(session.trajectory_steps.len());
            session
                .insert_steps(insert_pos, merged_steps)
                .map_err(HandlerError::from)?;
            session.push_trajectory_source(TrajectorySource::ThreadSegment {
                thread_id: thread_id.clone(),
                from_index,
                to_index,
            });
            Ok::<(), HandlerError>(())
        })
        .await
        {
            Ok((session, _)) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "trajectorySummary": summary_payload(&session),
                    "mergedCount": merged_count,
                    "warnings": warnings,
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct GetComposedTrajectoryHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl GetComposedTrajectoryHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for GetComposedTrajectoryHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "getComposedTrajectory".into(),
            description: "View the current composed trajectory with provenance information.".into(),
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
            Ok(session) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "trajectory": session.trajectory_steps,
                    "sources": session.trajectory_sources,
                    "mode": session.trajectory_mode,
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

pub struct SetTrajectoryModeHandler {
    config: Arc<BuilderToolkitConfig>,
}

impl SetTrajectoryModeHandler {
    fn new(config: Arc<BuilderToolkitConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolHandler for SetTrajectoryModeHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "setTrajectoryMode".into(),
            description: "Change the trajectory matching mode (anyOrder/inOrder/subset) without modifying steps.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Builder session ID" },
                    "mode": {
                        "type": "string",
                        "enum": ["anyOrder", "inOrder", "subset"],
                        "description": "Trajectory matching mode"
                    }
                },
                "required": ["sessionId", "mode"]
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
        let input = match parse_input::<SetTrajectoryModeInput>(tool_use_id, input) {
            Ok(input) => input,
            Err(result) => return result,
        };
        let session_id = input.session_id;
        let mode = input.mode;
        match mutate_session(&self.config, env, &session_id, |session| {
            let previous_mode = session.trajectory_mode;
            session.set_mode(mode);
            Ok::<TrajectoryMode, HandlerError>(previous_mode)
        })
        .await
        {
            Ok((session, previous_mode)) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "trajectorySummary": summary_payload(&session),
                    "previousMode": previous_mode,
                }),
            ),
            Err(err) => ToolCallResult::error(tool_use_id, err),
        }
    }
}

/// Generic trajectory-authoring toolkit for interactive test-case builders.
pub struct TrajectoryAuthoringToolKit {
    config: Arc<BuilderToolkitConfig>,
}

impl Default for TrajectoryAuthoringToolKit {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryAuthoringToolKit {
    pub fn new() -> Self {
        Self {
            config: Arc::new(BuilderToolkitConfig::default()),
        }
    }

    pub fn with_tool_catalog(mut self, catalog: Arc<dyn ToolCatalog>) -> Self {
        Arc::make_mut(&mut self.config).catalog = Some(catalog);
        self
    }

    pub fn with_builder_sessions(
        mut self,
        sessions: agent_fw_eval::BuilderSessionStoreConfig,
    ) -> Self {
        Arc::make_mut(&mut self.config).sessions = sessions;
        self
    }

    pub fn with_session_ttl(mut self, ttl: Option<Duration>) -> Self {
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config.sessions.clone().with_ttl(ttl);
        self
    }

    pub fn with_session_key_fn<F>(mut self, session_key: F) -> Self
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config.sessions.clone().with_session_key_fn(session_key);
        self
    }

    pub fn with_session_factory<F>(mut self, session_factory: F) -> Self
    where
        F: Fn(&str) -> TestCaseBuilderSession + Send + Sync + 'static,
    {
        let config = Arc::make_mut(&mut self.config);
        config.sessions = config
            .sessions
            .clone()
            .with_session_factory(session_factory);
        self
    }

    pub fn handlers(&self) -> Vec<Arc<dyn ToolHandler>> {
        vec![
            Arc::new(ListThreadsHandler),
            Arc::new(ComposeTrajectoryHandler::new(Arc::clone(&self.config))),
            Arc::new(AddTrajectoryStepHandler::new(Arc::clone(&self.config))),
            Arc::new(RemoveTrajectoryStepHandler::new(Arc::clone(&self.config))),
            Arc::new(ReorderTrajectoryStepHandler::new(Arc::clone(&self.config))),
            Arc::new(MergeTraceSegmentHandler::new(Arc::clone(&self.config))),
            Arc::new(GetComposedTrajectoryHandler::new(Arc::clone(&self.config))),
            Arc::new(SetTrajectoryModeHandler::new(Arc::clone(&self.config))),
        ]
    }

    pub fn len(&self) -> usize {
        8
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
    use agent_fw_algebra::KVStore;
    use agent_fw_core::{id::TenantId, tenant::TenantContext};
    use agent_fw_eval::{load_builder_session, VecToolCatalog};
    use agent_fw_interpreter::DashMapKVStore;
    use agent_fw_workspace::{
        KVWorkspaceStore, Message, PersistedToolInteraction, Thread, WorkspaceStore,
        WorkspaceToolEnvironmentExt,
    };

    fn env() -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let workspace: Arc<dyn WorkspaceStore> = Arc::new(KVWorkspaceStore::new(Arc::clone(&kv)));
        ToolEnvironment::builder()
            .kv_arc(kv)
            .event_sink(NullEventSink)
            .sub_agents(NullSubAgentInvoker)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("test-tenant")))
            .build()
            .with_workspace(workspace)
    }

    async fn seed_thread(env: &ToolEnvironment) {
        env.workspace()
            .upsert_thread(
                env.tenant().resource_id(),
                &Thread {
                    id: "t1".into(),
                    title: Some("Pricing scenario".into()),
                    resource_id: String::new(),
                    source_id: None,
                    created_at: "2025-01-01T00:00:00Z".into(),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                },
            )
            .await
            .unwrap();
        env.workspace()
            .insert_message(
                env.tenant().resource_id(),
                &Message::new("user", "What pricing options exist?"),
                "t1",
            )
            .await
            .unwrap();
        env.workspace()
            .insert_message(
                env.tenant().resource_id(),
                &Message::with_tool_interactions(
                    "assistant",
                    "Searching...",
                    vec![
                        PersistedToolInteraction {
                            call_id: "inv1".into(),
                            tool_name: "query_data".into(),
                            arguments: json!({}),
                            result: json!({}),
                        },
                        PersistedToolInteraction {
                            call_id: "inv2".into(),
                            tool_name: "legacyFoo".into(),
                            arguments: json!({}),
                            result: json!({}),
                        },
                    ],
                ),
                "t1",
            )
            .await
            .unwrap();
    }

    #[test]
    fn toolkit_has_8_tools() {
        assert_eq!(TrajectoryAuthoringToolKit::new().len(), 8);
    }

    #[tokio::test]
    async fn toolkit_uses_explicit_builder_session_policy() {
        let env = env();
        let toolkit = TrajectoryAuthoringToolKit::new().with_builder_sessions(
            agent_fw_eval::BuilderSessionStoreConfig::default()
                .with_session_key_fn(|tenant, session_id| format!("{tenant}:builder:{session_id}"))
                .with_session_factory(|session_id| TestCaseBuilderSession::new(session_id, "")),
        );
        let handler = toolkit
            .handlers()
            .into_iter()
            .find(|handler| handler.definition().name == "composeTrajectory")
            .expect("composeTrajectory handler");

        let result = handler
            .handle(
                "call-1",
                json!({
                    "sessionId": "sess-2",
                    "tools": [{"name": "draft_plan"}],
                }),
                &env,
            )
            .await;
        assert!(!result.is_error);

        let stored = load_builder_session(
            env.kv().as_ref(),
            env.tenant().resource_id().as_str(),
            "test-tenant:builder:sess-2",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(stored.trajectory_steps.len(), 1);
        assert_eq!(stored.trajectory_steps[0].tool_name, "draft_plan");
    }

    #[tokio::test]
    async fn compose_handler_uses_custom_key_and_catalog_validation() {
        let env = env();
        let handler = ComposeTrajectoryHandler::new(Arc::new(BuilderToolkitConfig {
            catalog: Some(Arc::new(VecToolCatalog::new(vec!["draft_plan".into()]))),
            sessions: agent_fw_eval::BuilderSessionStoreConfig::default()
                .with_session_key_fn(|tenant, session_id| format!("{tenant}:builder:{session_id}"))
                .with_session_factory(|session_id| TestCaseBuilderSession::new(session_id, "")),
        }));

        let invalid = handler
            .handle(
                "call-1",
                json!({
                    "sessionId": "sess-1",
                    "tools": [{"name": "badTool"}],
                }),
                &env,
            )
            .await;
        assert!(invalid.is_error);

        let result = handler
            .handle(
                "call-2",
                json!({
                    "sessionId": "sess-1",
                    "tools": [{"name": "draft_plan"}],
                }),
                &env,
            )
            .await;
        assert!(!result.is_error);

        let stored = load_builder_session(
            env.kv().as_ref(),
            env.tenant().resource_id().as_str(),
            "test-tenant:builder:sess-1",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(stored.trajectory_steps.len(), 1);
        assert_eq!(stored.trajectory_steps[0].tool_name, "draft_plan");
    }

    #[tokio::test]
    async fn merge_handler_reads_workspace_segment_and_emits_warnings() {
        let env = env();
        seed_thread(&env).await;
        let handler = MergeTraceSegmentHandler::new(Arc::new(BuilderToolkitConfig {
            catalog: Some(Arc::new(VecToolCatalog::new(vec!["query_data".into()]))),
            sessions: agent_fw_eval::BuilderSessionStoreConfig::default()
                .with_session_key_fn(|_tenant, session_id: &str| session_id.to_string())
                .with_session_factory(|session_id| TestCaseBuilderSession::new(session_id, "")),
        }));

        let result = handler
            .handle(
                "call-1",
                json!({
                    "sessionId": "sess-1",
                    "threadId": "t1",
                    "fromIndex": 0,
                    "toIndex": 2,
                }),
                &env,
            )
            .await;

        assert!(!result.is_error);
        assert_eq!(result.content["mergedCount"], 2);
        assert_eq!(result.content["warnings"].as_array().unwrap().len(), 1);
        assert!(result.content["warnings"][0]
            .as_str()
            .unwrap()
            .contains("legacyFoo"));
    }
}
