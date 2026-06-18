//! Durable trace and provenance types for eval execution.
//!
//! The types in this module are pure data structures used to persist and
//! exchange tool-call traces captured during eval runs and test authoring.

use agent_fw_core::{EvalRunId, TestCaseId, ThreadId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Reference to a full persisted trace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TraceRef {
    pub trace_id: String,
}

impl TraceRef {
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
        }
    }
}

/// Reference to a contiguous step range inside a persisted trace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TraceSegmentRef {
    pub trace_id: String,
    pub from_step: u32,
    pub to_step: u32,
}

impl TraceSegmentRef {
    pub fn new(trace_id: impl Into<String>, from_step: u32, to_step: u32) -> Self {
        Self {
            trace_id: trace_id.into(),
            from_step,
            to_step,
        }
    }

    pub fn len(&self) -> u32 {
        self.to_step.saturating_sub(self.from_step)
    }

    pub fn is_empty(&self) -> bool {
        self.from_step >= self.to_step
    }
}

/// The stage of execution or authoring this trace belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStage {
    Selection,
    Confirmation,
    Holdout,
    Diagnostic,
    Builder,
    Authoring,
    Runtime,
}

/// Final status of the trace capture or the traced execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Partial,
}

/// The actor responsible for a trace step, when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceActor {
    User,
    Assistant,
    Tool,
    System,
    SubAgent,
}

/// Why a payload was omitted instead of stored inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceOmissionReason {
    NotCaptured,
    TooLarge,
    Sensitive,
    Unsupported,
}

/// Metadata for a redacted payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RedactedPayload {
    pub sha256: String,
    pub original_bytes: usize,
    pub policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Redaction-ready payload wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TracePayload {
    Inline { value: serde_json::Value },
    Redacted { redaction: RedactedPayload },
    Omitted { reason: TraceOmissionReason },
}

impl TracePayload {
    pub fn inline(value: serde_json::Value) -> Self {
        Self::Inline { value }
    }

    pub fn omitted(reason: TraceOmissionReason) -> Self {
        Self::Omitted { reason }
    }

    pub fn redact<T: Serialize>(
        value: &T,
        policy: impl Into<String>,
        summary: Option<String>,
    ) -> serde_json::Result<Self> {
        let bytes = serde_json::to_vec(value)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(Self::Redacted {
            redaction: RedactedPayload {
                sha256: hex::encode(hasher.finalize()),
                original_bytes: bytes.len(),
                policy: policy.into(),
                summary,
            },
        })
    }

    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline { .. })
    }
}

/// Correlation data for a persisted trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TraceScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_run_id: Option<EvalRunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_case_id: Option<TestCaseId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_index: Option<u32>,
}

impl TraceScope {
    pub fn for_eval_attempt(
        session_id: impl Into<String>,
        candidate_id: impl Into<String>,
        attempt_id: impl Into<String>,
        eval_run_id: EvalRunId,
        test_case_id: TestCaseId,
        sample_index: u32,
        thread_id: Option<ThreadId>,
    ) -> Self {
        Self {
            session_id: Some(session_id.into()),
            candidate_id: Some(candidate_id.into()),
            attempt_id: Some(attempt_id.into()),
            eval_run_id: Some(eval_run_id),
            test_case_id: Some(test_case_id),
            thread_id,
            sample_index: Some(sample_index),
        }
    }

    pub fn for_thread(thread_id: ThreadId) -> Self {
        Self {
            thread_id: Some(thread_id),
            ..Self::default()
        }
    }
}

/// Source lineage for a durable trace artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TraceProvenance {
    EvalSample {
        eval_run_id: EvalRunId,
        test_case_id: TestCaseId,
        sample_index: u32,
        stage: TraceStage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<ThreadId>,
    },
    WorkspaceThreadSegment {
        thread_id: ThreadId,
        from_index: u32,
        to_index: u32,
    },
    ManualBuilderEdit {
        builder_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    PromotedAuthoringDraft {
        draft_id: String,
        source_trace: TraceRef,
    },
    RecomposedSegment {
        source_segments: Vec<TraceSegmentRef>,
    },
}

/// A single tool-oriented step in a canonical trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TraceStep {
    pub ordinal: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TraceActor>,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub arguments: TracePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TracePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

impl TraceStep {
    pub fn tool_call(ordinal: u32, tool_name: impl Into<String>, arguments: TracePayload) -> Self {
        Self {
            ordinal,
            actor: Some(TraceActor::Assistant),
            tool_name: tool_name.into(),
            tool_call_id: None,
            arguments,
            result: None,
            started_at: None,
            completed_at: None,
            error: None,
            correlation_id: None,
        }
    }
}

/// Durable top-level trace artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TraceRecord {
    pub trace_id: String,
    pub workspace_id: WorkspaceId,
    pub stage: TraceStage,
    pub status: TraceStatus,
    pub scope: TraceScope,
    pub steps: Vec<TraceStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub provenance: TraceProvenance,
}

impl TraceRecord {
    pub fn trace_ref(&self) -> TraceRef {
        TraceRef::new(self.trace_id.clone())
    }

    pub fn segment_ref(&self, from_step: u32, to_step: u32) -> TraceSegmentRef {
        TraceSegmentRef::new(self.trace_id.clone(), from_step, to_step)
    }

    pub fn slice_steps(&self, segment: &TraceSegmentRef) -> Vec<TraceStep> {
        let len = self.steps.len();
        let from = usize::min(segment.from_step as usize, len);
        let to = usize::min(segment.to_step as usize, len).max(from);
        self.steps[from..to].to_vec()
    }
}
