use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::runtime_contract::ExecutionModeKindV1;

pub const AGENT_MANIFEST_SCHEMA_V1: &str = "agent-manifest/v1";
pub const AGENT_EVAL_RESULT_SCHEMA_V1: &str = "agent-eval-result/v1";
pub const AGENT_TRACE_ARTIFACT_SCHEMA_V1: &str = "agent-trace-artifact/v1";
pub const DEFAULT_EVAL_RESULT_ARTIFACT_PATH: &str = ".agent-fw/runtime/eval-result.json";
pub const DEFAULT_TRACE_ARTIFACT_PATH: &str = ".agent-fw/runtime/trace-artifact.json";
pub const DEFAULT_TRACE_ARTIFACTS_PATH: &str = ".agent-fw/runtime/trace-artifacts.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentManifestV1 {
    pub schema_version: String,
    pub agent_id: String,
    pub name: String,
    pub language: String,
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEvalResultV1 {
    pub schema_version: String,
    pub agent_id: String,
    pub execution_mode: ExecutionModeKindV1,
    pub eval_run_id: String,
    pub status: EvalStatusV1,
    #[serde(default)]
    pub normalized_score: Option<f64>,
    pub started_at: String,
    pub finished_at: String,
    #[serde(default)]
    pub case_results: Vec<EvalCaseResultV1>,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRefV1>,
    #[serde(default)]
    pub trace_refs: Vec<String>,
}

impl AgentEvalResultV1 {
    #[must_use]
    pub fn is_normalized(&self) -> bool {
        self.schema_version == AGENT_EVAL_RESULT_SCHEMA_V1
            && self
                .normalized_score
                .map(|score| (0.0..=1.0).contains(&score))
                .unwrap_or(true)
            && self.case_results.iter().all(|case| {
                case.normalized_score
                    .map(|score| (0.0..=1.0).contains(&score))
                    .unwrap_or(true)
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalCaseResultV1 {
    pub case_id: String,
    pub label: String,
    pub status: EvalStatusV1,
    #[serde(default)]
    pub normalized_score: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRefV1>,
    #[serde(default)]
    pub trace_refs: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatusV1 {
    Passed,
    Failed,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTraceArtifactV1 {
    pub schema_version: String,
    pub agent_id: String,
    pub trace_id: String,
    pub execution_mode: ExecutionModeKindV1,
    pub started_at: String,
    pub finished_at: String,
    pub run_context: TraceRunContextV1,
    #[serde(default)]
    pub steps: Vec<TraceStepV1>,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRefV1>,
}

impl AgentTraceArtifactV1 {
    #[must_use]
    pub fn is_normalized(&self) -> bool {
        self.schema_version == AGENT_TRACE_ARTIFACT_SCHEMA_V1
            && self
                .steps
                .windows(2)
                .all(|pair| pair[0].step_id <= pair[1].step_id)
    }
}

#[derive(Debug, Error)]
pub enum RunArtifactIoError {
    #[error("failed to access artifact path: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to encode or decode artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn write_eval_result_artifact(
    root: impl AsRef<Path>,
    artifact: &AgentEvalResultV1,
) -> Result<PathBuf, RunArtifactIoError> {
    write_json_artifact(
        root.as_ref().join(DEFAULT_EVAL_RESULT_ARTIFACT_PATH),
        artifact,
    )
}

pub fn load_eval_result_artifact(
    root: impl AsRef<Path>,
) -> Result<Option<AgentEvalResultV1>, RunArtifactIoError> {
    load_json_artifact(root.as_ref().join(DEFAULT_EVAL_RESULT_ARTIFACT_PATH))
}

pub fn write_trace_artifact(
    root: impl AsRef<Path>,
    artifact: &AgentTraceArtifactV1,
) -> Result<PathBuf, RunArtifactIoError> {
    write_json_artifact(root.as_ref().join(DEFAULT_TRACE_ARTIFACT_PATH), artifact)
}

pub fn load_trace_artifact(
    root: impl AsRef<Path>,
) -> Result<Option<AgentTraceArtifactV1>, RunArtifactIoError> {
    load_json_artifact(root.as_ref().join(DEFAULT_TRACE_ARTIFACT_PATH))
}

pub fn write_trace_artifacts(
    root: impl AsRef<Path>,
    artifacts: &[AgentTraceArtifactV1],
) -> Result<PathBuf, RunArtifactIoError> {
    write_json_artifact(root.as_ref().join(DEFAULT_TRACE_ARTIFACTS_PATH), artifacts)
}

pub fn load_trace_artifacts(
    root: impl AsRef<Path>,
) -> Result<Option<Vec<AgentTraceArtifactV1>>, RunArtifactIoError> {
    load_json_artifact(root.as_ref().join(DEFAULT_TRACE_ARTIFACTS_PATH))
}

fn write_json_artifact<T: Serialize + ?Sized>(
    path: PathBuf,
    value: &T,
) -> Result<PathBuf, RunArtifactIoError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(value)?)?;
    Ok(path)
}

fn load_json_artifact<T: for<'de> Deserialize<'de>>(
    path: PathBuf,
) -> Result<Option<T>, RunArtifactIoError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRunContextV1 {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub candidate_id: Option<String>,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub eval_run_id: Option<String>,
    #[serde(default)]
    pub case_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceStepV1 {
    pub step_id: String,
    #[serde(default)]
    pub parent_step_id: Option<String>,
    pub kind: TraceStepKindV1,
    pub status: TraceStepStatusV1,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub input_ref: Option<ArtifactRefV1>,
    #[serde(default)]
    pub output_ref: Option<ArtifactRefV1>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStepKindV1 {
    Message,
    ModelTurn,
    ToolCall,
    ToolResult,
    Observation,
    SystemEvent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStepStatusV1 {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRefV1 {
    pub artifact_id: String,
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub media_type: Option<String>,
}
