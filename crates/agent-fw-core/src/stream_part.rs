//! AI SDK Data Stream Protocol types
//!
//! This module defines the AI SDK Data Stream Protocol as an algebraic data type.
//! StreamPart is a closed sum type with an extensibility point via `Custom`.
//!
//! # Protocol Invariants
//!
//! 1. **Serialization totality**: `to_sse_bytes()` never panics. Every variant
//!    serializes to valid JSON matching the AI SDK spec.
//! 2. **Determinism**: Serialization is deterministic (same input → same output).
//! 3. **Ordering**: Within a stream, events follow this partial order:
//!    - `StepStart` precedes any `Text`/`Reasoning` in that step
//!    - `ToolInvocation(Call)` precedes `ToolProgress*` precedes `ToolInvocation(Result)`
//!    - `ToolAgent(Call)` precedes `ToolAgent(Result)`
//!    - For an approval-gated tool: `ToolInvocation(Call)` precedes
//!      `ApprovalRequired` precedes `ApprovalDecision` precedes
//!      `ToolInvocation(Result)` (pre-dispatch approval).
//!    - `Finish` is always the terminal event (at most one per stream)
//!    - `DataCostSummary` and `DataLatencySummary` appear after `Finish`
//! 4. **Error finality**: After `Error`, no further events are emitted.
//! 5. **Usage monotonicity**: `TokenUsage` in `Finish` is the cumulative total
//!    for the stream. It is never less than any intermediate usage snapshot.
//!
//! # Extensibility
//!
//! The `Custom` variant allows domain-specific events without modifying
//! the framework. Consumers define their own event_type strings and data shapes.

use super::approval::{ApprovalDecision, ApprovalRequest, PlanStatusChange};
use super::latency::LatencySummary;
use super::usage::TokenUsage;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};

/// The AI SDK Data Stream Protocol events.
///
/// This is a CLOSED sum type with a `Custom` extensibility point.
/// The compiler verifies exhaustive pattern matching on all variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum StreamPart {
    /// Incremental text token
    #[serde(rename = "text")]
    Text { text: String },

    /// Chain-of-thought reasoning (optional, model-dependent)
    #[serde(rename = "reasoning")]
    Reasoning { text: String },

    /// Step boundary marker
    #[serde(rename = "step-start")]
    StepStart,

    /// Tool invocation (call or result)
    #[serde(rename = "tool-invocation")]
    ToolInvocation(ToolInvocationData),

    /// Sub-agent invocation (call or result)
    #[serde(rename = "tool-agent")]
    ToolAgent(ToolAgentData),

    /// Sub-agent completion with usage metrics
    #[serde(rename = "data-tool-agent")]
    DataToolAgent { data: AgentUsage },

    /// File available for download
    #[serde(rename = "data-file-registered")]
    DataFileRegistered { data: FileRegistration },

    /// Aggregated cost summary (emitted at stream end)
    #[serde(rename = "data-cost-summary")]
    DataCostSummary { data: CostSummary },

    /// Latency metrics summary (emitted at stream end)
    #[serde(rename = "data-latency-summary")]
    DataLatencySummary { data: LatencySummary },

    /// Pre-computed UI payload for direct frontend rendering.
    /// Typed as `CommandCardPayload { dsl: String }` — matches the frontend's
    /// `CommandCardPayload` interface. Prevents lossy extraction.
    #[serde(rename = "data-flow-ui")]
    DataFlowUI { data: CommandCardPayload },

    /// Stream completion
    #[serde(rename = "finish")]
    Finish {
        #[serde(rename = "finishReason")]
        reason: FinishReason,
        usage: TokenUsage,
    },

    /// Incremental progress from a long-running tool.
    ///
    /// Emitted during tool execution (between `ToolInvocation(Call)` and
    /// `ToolInvocation(Result)`) to provide the frontend with phase-level
    /// progress updates.
    ///
    /// # Invariant: Monotonic phase progression
    ///
    /// Within a single tool execution, `phase_index` values form a
    /// non-decreasing sequence.
    #[serde(rename = "tool-progress")]
    ToolProgress(ToolProgressData),

    /// Pre-dispatch approval request (pre-dispatch approval).
    ///
    /// Emitted by `ApprovalLayer` (tool gate) or `GatedPlanExecutor`
    /// (plan gate) when a gated dispatch path pauses pending host
    /// decision. The host responds via `runtime.respond_to_approval(...)`.
    ///
    /// # Ordering
    ///
    /// For tools: follows `ToolInvocation(Call)`, precedes `ApprovalDecision`.
    /// For plans: precedes the executor's `tool_call` / progress events.
    #[serde(rename = "approval-required")]
    ApprovalRequired { data: ApprovalRequest },

    /// Host's decision on a pending approval (pre-dispatch approval).
    ///
    /// Emitted by the gate immediately after `store.resolve(...)` succeeds.
    /// For tools: precedes `ToolInvocation(Result)`. For plans: precedes
    /// the `PlanStatusChange { to: "approved" }` event.
    #[serde(rename = "approval-decision")]
    ApprovalDecision { data: ApprovalDecision },

    /// Plan lifecycle transition, including the `pending_approval`
    /// display alias that has no corresponding `PlanStatus` variant.
    ///
    /// Emitted by `GatedPlanExecutor` to signal the host that a plan
    /// moved (e.g., `draft → pending_approval → approved → executing`).
    #[serde(rename = "plan-status-change")]
    PlanStatusChange { data: PlanStatusChange },

    /// Error (non-recoverable)
    #[serde(rename = "error")]
    Error { error: ErrorInfo },

    /// Domain-specific custom event.
    ///
    /// Extensibility point for events not covered by the core protocol.
    /// The `event_type` field is used as the SSE event type identifier.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // A domain layer defines a custom UI event:
    /// StreamPart::Custom {
    ///     event_type: "data-flow-ui".to_string(),
    ///     data: serde_json::json!({ "dsl": "{...}" }),
    /// }
    /// ```
    #[serde(rename = "custom")]
    Custom {
        /// Event type identifier (e.g., "data-flow-ui")
        event_type: String,
        /// Arbitrary JSON payload
        data: serde_json::Value,
    },
}

/// Progress update from a long-running tool.
///
/// Carries a phase label, a monotonically non-decreasing phase index,
/// and an optional structured milestone (e.g., `{"matched": 142}`).
/// The frontend uses `total_phases` to render a determinate progress indicator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgressData {
    /// Tool name (e.g., "draft_plan"). Correlates with the active ToolInvocation.
    pub tool_name: String,
    /// Tool call ID from rig-core. Enables precise correlation when multiple
    /// calls to the same tool are in-flight concurrently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Human-readable phase label (e.g., "Resolving entities").
    pub label: String,
    /// 0-based phase index (monotonically non-decreasing within a tool execution).
    pub phase_index: u8,
    /// Total expected phases (enables determinate progress: "2 of 4").
    pub total_phases: u8,
    /// Optional structured data for this phase (e.g., product count after resolution).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<serde_json::Value>,
}

/// Tool invocation data (used for both call and result states).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocationData {
    #[serde(rename = "toolInvocationId")]
    pub id: String,
    #[serde(rename = "toolName")]
    pub name: String,
    pub args: serde_json::Value,
    #[serde(flatten)]
    pub state: ToolInvocationState,
}

/// Tool invocation state - either call (no result) or result (with result).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ToolInvocationState {
    Call,
    Result { result: serde_json::Value },
}

/// Sub-agent invocation data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAgentData {
    #[serde(rename = "agentName")]
    pub agent_name: String,
    #[serde(rename = "toolInvocationId")]
    pub invocation_id: String,
    #[serde(flatten)]
    pub state: ToolAgentState,
}

/// Sub-agent state - either call or result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ToolAgentState {
    Call,
    Result,
}

impl StreamPart {
    /// Serialize to SSE wire format.
    ///
    /// # Laws
    /// 1. Never panics (total function)
    /// 2. Output is valid UTF-8
    /// 3. Output matches pattern: `data: {json}\n\n`
    ///
    /// # Safety Invariant
    ///
    /// Serialization is infallible because all `StreamPart` variants contain
    /// only `String`, `bool`, `u64`, `serde_json::Value`, and fixed enums —
    /// none of which produce serialization errors. This invariant is upheld by
    /// the type definition: no `f64` fields (which could be NaN), no maps with
    /// non-string keys, no custom `Serialize` impls that can fail.
    pub fn to_sse_bytes(&self) -> Bytes {
        let json = serde_json::to_string(self).unwrap_or_else(|e| {
            // Fallback: emit an error event instead of panicking.
            // This preserves L1 (totality) even if the invariant is violated.
            format!(
                "{{\"type\":\"error\",\"error\":{{\"message\":\"serialization failed: {e}\"}}}}"
            )
        });
        Bytes::from(format!("data: {json}\n\n"))
    }

    /// Create a text delta event.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create a reasoning delta event.
    pub fn reasoning(text: impl Into<String>) -> Self {
        Self::Reasoning { text: text.into() }
    }

    /// Create a tool call event.
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        Self::ToolInvocation(ToolInvocationData {
            id: id.into(),
            name: name.into(),
            args,
            state: ToolInvocationState::Call,
        })
    }

    /// Create a tool result event.
    pub fn tool_result(
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
        result: serde_json::Value,
    ) -> Self {
        Self::ToolInvocation(ToolInvocationData {
            id: id.into(),
            name: name.into(),
            args,
            state: ToolInvocationState::Result { result },
        })
    }

    /// Create a sub-agent call event.
    pub fn sub_agent_call(agent_name: impl Into<String>, invocation_id: impl Into<String>) -> Self {
        Self::ToolAgent(ToolAgentData {
            agent_name: agent_name.into(),
            invocation_id: invocation_id.into(),
            state: ToolAgentState::Call,
        })
    }

    /// Create a sub-agent result event.
    pub fn sub_agent_result(
        agent_name: impl Into<String>,
        invocation_id: impl Into<String>,
    ) -> Self {
        Self::ToolAgent(ToolAgentData {
            agent_name: agent_name.into(),
            invocation_id: invocation_id.into(),
            state: ToolAgentState::Result,
        })
    }

    /// Create a finish event.
    pub fn finish(reason: FinishReason, usage: TokenUsage) -> Self {
        Self::Finish { reason, usage }
    }

    /// Create a tool progress event.
    pub fn tool_progress(
        tool_name: impl Into<String>,
        tool_call_id: Option<String>,
        label: impl Into<String>,
        phase_index: u8,
        total_phases: u8,
        milestone: Option<serde_json::Value>,
    ) -> Self {
        Self::ToolProgress(ToolProgressData {
            tool_name: tool_name.into(),
            tool_call_id,
            label: label.into(),
            phase_index,
            total_phases,
            milestone,
        })
    }

    /// Create an error event.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            error: ErrorInfo::new(message),
        }
    }

    /// Create a cost summary event.
    pub fn cost_summary(summary: CostSummary) -> Self {
        Self::DataCostSummary { data: summary }
    }

    /// Create a latency summary event.
    pub fn latency_summary(summary: LatencySummary) -> Self {
        Self::DataLatencySummary { data: summary }
    }

    /// Create a data-tool-agent event with per-agent usage.
    pub fn data_tool_agent(usage: AgentUsage) -> Self {
        Self::DataToolAgent { data: usage }
    }

    /// Create a pre-computed UI event for direct frontend rendering.
    pub fn data_flow_ui(dsl: impl Into<String>) -> Self {
        Self::DataFlowUI {
            data: CommandCardPayload { dsl: dsl.into() },
        }
    }

    /// Create a custom domain-specific event.
    pub fn custom(event_type: impl Into<String>, data: serde_json::Value) -> Self {
        Self::Custom {
            event_type: event_type.into(),
            data,
        }
    }

    /// Create an `approval-required` event for a pending request (pre-dispatch approval).
    pub fn approval_required(request: ApprovalRequest) -> Self {
        Self::ApprovalRequired { data: request }
    }

    /// Create an `approval-decision` event for a recorded decision (pre-dispatch approval).
    pub fn approval_decision(decision: ApprovalDecision) -> Self {
        Self::ApprovalDecision { data: decision }
    }

    /// Create a `plan-status-change` event (pre-dispatch approval).
    pub fn plan_status_change(
        plan_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self::PlanStatusChange {
            data: PlanStatusChange {
                plan_id: plan_id.into(),
                from: from.into(),
                to: to.into(),
            },
        }
    }

    /// Check if this is a tool call (not result).
    pub fn is_tool_call(&self) -> bool {
        matches!(
            self,
            Self::ToolInvocation(ToolInvocationData {
                state: ToolInvocationState::Call,
                ..
            })
        )
    }

    /// Check if this is a tool result.
    pub fn is_tool_result(&self) -> bool {
        matches!(
            self,
            Self::ToolInvocation(ToolInvocationData {
                state: ToolInvocationState::Result { .. },
                ..
            })
        )
    }

    /// Get the tool invocation ID if this is a tool event.
    pub fn tool_invocation_id(&self) -> Option<&str> {
        match self {
            Self::ToolInvocation(data) => Some(&data.id),
            Self::ToolAgent(data) => Some(&data.invocation_id),
            _ => None,
        }
    }
}

/// Construct a `data-flow-ui` event from a CommandCard DSL string.
pub fn command_card_ui(dsl: impl Into<String>) -> StreamPart {
    StreamPart::data_flow_ui(dsl)
}

/// Extract the DSL string from a typed `CommandCardPayload`.
pub fn extract_dsl(data: &CommandCardPayload) -> &str {
    &data.dsl
}

/// Reason for stream completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

/// Usage metrics for a sub-agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentUsage {
    #[serde(rename = "agentName")]
    pub agent_name: String,
    pub model: String,
    pub usage: TokenUsage,
}

/// Cost summary aggregating usage across agents.
///
/// Only stores `agents`. Totals are computed during serialization
/// by folding over `agents[*].usage` with the TokenUsage monoid.
#[derive(Debug, Clone, PartialEq)]
pub struct CostSummary {
    pub agents: Vec<AgentUsage>,
}

impl CostSummary {
    /// Create from a list of agent usages.
    pub fn new(agents: Vec<AgentUsage>) -> Self {
        Self { agents }
    }

    /// Compute aggregate usage via monoid fold.
    pub fn total_usage(&self) -> TokenUsage {
        self.agents
            .iter()
            .map(|a| &a.usage)
            .fold(TokenUsage::ZERO, |acc, u| acc.combine(u))
    }
}

/// Custom serialization includes computed totals.
impl Serialize for CostSummary {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let total = self.total_usage();
        let mut state = s.serialize_struct("CostSummary", 6)?;
        state.serialize_field("agents", &self.agents)?;
        state.serialize_field("totalPromptTokens", &total.prompt_tokens)?;
        state.serialize_field("totalCompletionTokens", &total.completion_tokens)?;
        state.serialize_field("totalCacheReadInputTokens", &total.cache_read_input_tokens)?;
        state.serialize_field(
            "totalCacheCreationInputTokens",
            &total.cache_creation_input_tokens,
        )?;
        state.serialize_field("totalTokens", &total.total())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CostSummary {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Helper {
            agents: Vec<AgentUsage>,
        }
        let helper = Helper::deserialize(d)?;
        Ok(CostSummary {
            agents: helper.agents,
        })
    }
}

/// File registration event data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRegistration {
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub filename: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
    pub timestamp: DateTime<Utc>,
}

/// Error information for error events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ErrorInfo {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

// ============================================================================
// Message Parts — persistent representation of assistant messages
// ============================================================================

/// State of a tool/agent invocation in persisted message parts.
///
/// Replaces stringly-typed `"call"` / `"result"` — makes illegal states
/// unrepresentable. Serde serializes to `"call"` / `"result"` for wire compat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessagePartState {
    Call,
    Result,
}

/// Typed payload for `DataFlowUI` — replaces `serde_json::Value`.
///
/// The frontend expects `{ dsl: string }`. By typing it here, we prevent
/// lossy extraction (e.g., silently coercing a JSON object to `""`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandCardPayload {
    pub dsl: String,
}

/// A part of a persisted assistant message.
///
/// Mirrors the frontend's `MessagePart` union type. Both Python and Rust
/// backends serialize to this shape; the frontend's `parseBackendMessage()`
/// reads it back.
///
/// # Relationship to StreamPart
///
/// `StreamPart` is the wire-format for SSE (streaming, ephemeral).
/// `MessagePart` is the persistence-format for DB storage (durable).
///
/// ```text
///   StreamPart   ─── streaming ───>  Frontend  <── loading ───  MessagePart
///     (SSE)                                                       (DB)
/// ```
///
/// The `MessagePartAccumulator` bridges the two: it consumes StreamParts
/// during streaming and produces a `Vec<MessagePart>` for persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum MessagePart {
    /// Plain text content
    #[serde(rename = "text")]
    Text { text: String },

    /// Chain-of-thought reasoning
    #[serde(rename = "reasoning")]
    Reasoning { text: String },

    /// Tool invocation (call or result)
    #[serde(rename = "tool-invocation")]
    ToolInvocation {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: serde_json::Value,
        state: MessagePartState,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
    },

    /// Sub-agent invocation
    #[serde(rename = "tool-agent")]
    ToolAgent {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "agentName")]
        agent_name: String,
        state: MessagePartState,
    },

    /// Approval/command card DSL
    #[serde(rename = "flow-ui")]
    FlowUI { dsl: String },

    /// Downloadable file
    #[serde(rename = "file")]
    File {
        #[serde(rename = "fileId")]
        file_id: String,
        filename: String,
    },
}

/// Accumulates `StreamPart` events into persistable `MessagePart` entries.
///
/// # Usage
///
/// ```ignore
/// let mut acc = MessagePartAccumulator::new();
/// for part in stream_parts {
///     acc.push(&part);
/// }
/// let (parts, plain_text) = acc.finish();
/// // parts: Vec<MessagePart> for `parts` field
/// // plain_text: String for backward-compatible `content` field
/// ```
///
/// # Laws
///
/// 1. **Text coalescence**: Consecutive `Text` events produce at most one
///    `MessagePart::Text` entry (text is buffered, not duplicated).
/// 2. **Reasoning coalescence**: Consecutive `Reasoning` events produce at most
///    one `MessagePart::Reasoning` entry (reasoning is buffered, not duplicated).
/// 3. **Tool boundary**: A tool-invocation flushes pending text and reasoning,
///    ensuring parts don't straddle tool calls.
/// 4. **Idempotent finish**: Calling `finish()` multiple times is safe.
#[derive(Debug, Default)]
pub struct MessagePartAccumulator {
    parts: Vec<MessagePart>,
    pending_text: String,
    pending_reasoning: String,
    plain_text: String,
}

impl MessagePartAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a StreamPart, updating internal state.
    pub fn push(&mut self, part: &StreamPart) {
        match part {
            StreamPart::Text { text } => {
                self.flush_reasoning();
                self.pending_text.push_str(text);
                self.plain_text.push_str(text);
            }
            StreamPart::Reasoning { text } => {
                self.flush_text();
                self.pending_reasoning.push_str(text);
            }
            StreamPart::ToolInvocation(data) => {
                self.flush_text();
                self.flush_reasoning();
                let (state, result) = match &data.state {
                    ToolInvocationState::Call => (MessagePartState::Call, None),
                    ToolInvocationState::Result { result } => {
                        (MessagePartState::Result, Some(result.clone()))
                    }
                };
                self.parts.push(MessagePart::ToolInvocation {
                    tool_call_id: data.id.clone(),
                    tool_name: data.name.clone(),
                    args: data.args.clone(),
                    state,
                    result,
                });
            }
            StreamPart::ToolAgent(data) => {
                let state = match data.state {
                    ToolAgentState::Call => MessagePartState::Call,
                    ToolAgentState::Result => MessagePartState::Result,
                };
                self.parts.push(MessagePart::ToolAgent {
                    tool_call_id: data.invocation_id.clone(),
                    agent_name: data.agent_name.clone(),
                    state,
                });
            }
            StreamPart::DataFlowUI { data } => {
                self.flush_text();
                self.flush_reasoning();
                self.parts.push(MessagePart::FlowUI {
                    dsl: data.dsl.clone(),
                });
            }
            StreamPart::DataFileRegistered { data } => {
                self.parts.push(MessagePart::File {
                    file_id: data.file_id.clone(),
                    filename: data.filename.clone(),
                });
            }
            // StepStart, ToolProgress, DataCostSummary, DataLatencySummary,
            // Finish, Error, Custom — not persisted as message parts
            _ => {}
        }
    }

    /// Finish accumulation and return (parts, plain_text).
    ///
    /// `parts` is for the `parts` field in the persisted message.
    /// `plain_text` is for the backward-compatible `content` field.
    pub fn finish(mut self) -> (Vec<MessagePart>, String) {
        self.flush_text();
        self.flush_reasoning();
        (self.parts, self.plain_text)
    }

    /// Flush pending text buffer into a Text part.
    fn flush_text(&mut self) {
        if !self.pending_text.is_empty() {
            self.parts.push(MessagePart::Text {
                text: std::mem::take(&mut self.pending_text),
            });
        }
    }

    /// Flush pending reasoning buffer into a Reasoning part.
    fn flush_reasoning(&mut self) {
        if !self.pending_reasoning.is_empty() {
            self.parts.push(MessagePart::Reasoning {
                text: std::mem::take(&mut self.pending_reasoning),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_event_serializes() {
        let event = StreamPart::text("Hello, world!");
        let bytes = event.to_sse_bytes();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("data: "));
        assert!(s.ends_with("\n\n"));
        assert!(s.contains("\"text\":\"Hello, world!\""));
    }

    #[test]
    fn tool_call_serializes_with_call_state() {
        let event = StreamPart::tool_call("call-123", "my_tool", serde_json::json!({"x": 1}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"state\":\"call\""));
        assert!(json.contains("\"toolInvocationId\":\"call-123\""));
        assert!(json.contains("\"type\":\"tool-invocation\""));
    }

    #[test]
    fn tool_result_serializes_with_result_state() {
        let event = StreamPart::tool_result(
            "call-123",
            "my_tool",
            serde_json::json!({"x": 1}),
            serde_json::json!("result value"),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"state\":\"result\""));
        assert!(json.contains("\"result\":\"result value\""));
    }

    #[test]
    fn finish_event_includes_usage() {
        let usage = TokenUsage::simple(100, 50);
        let event = StreamPart::finish(FinishReason::Stop, usage);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"promptTokens\":100"));
        assert!(json.contains("\"completionTokens\":50"));
        assert!(json.contains("\"totalTokens\":150"));
    }

    #[test]
    fn cost_summary_computes_totals() {
        let summary = CostSummary::new(vec![
            AgentUsage {
                agent_name: "planner".to_string(),
                model: "gpt-4".to_string(),
                usage: TokenUsage::simple(100, 50),
            },
            AgentUsage {
                agent_name: "executor".to_string(),
                model: "gpt-4".to_string(),
                usage: TokenUsage::simple(200, 100),
            },
        ]);

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"totalPromptTokens\":300"));
        assert!(json.contains("\"totalCompletionTokens\":150"));
        assert!(json.contains("\"totalTokens\":450"));
    }

    #[test]
    fn is_tool_call_detection() {
        let call = StreamPart::tool_call("id", "name", serde_json::json!({}));
        let result =
            StreamPart::tool_result("id", "name", serde_json::json!({}), serde_json::json!({}));
        let text = StreamPart::text("hello");

        assert!(call.is_tool_call());
        assert!(!call.is_tool_result());

        assert!(!result.is_tool_call());
        assert!(result.is_tool_result());

        assert!(!text.is_tool_call());
        assert!(!text.is_tool_result());
    }

    #[test]
    fn tool_invocation_id_extraction() {
        let call = StreamPart::tool_call("call-123", "name", serde_json::json!({}));
        assert_eq!(call.tool_invocation_id(), Some("call-123"));

        let agent = StreamPart::sub_agent_call("agent", "inv-456");
        assert_eq!(agent.tool_invocation_id(), Some("inv-456"));

        let text = StreamPart::text("hello");
        assert_eq!(text.tool_invocation_id(), None);
    }

    #[test]
    fn error_info_with_code() {
        let error = ErrorInfo::new("Something went wrong").with_code("ERR_001");
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"code\":\"ERR_001\""));
    }

    #[test]
    fn error_info_without_code_omits_field() {
        let error = ErrorInfo::new("Something went wrong");
        let json = serde_json::to_string(&error).unwrap();
        assert!(!json.contains("code"));
    }

    #[test]
    fn sub_agent_call_serializes() {
        let event = StreamPart::sub_agent_call("planner", "inv-001");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool-agent\""));
        assert!(json.contains("\"agentName\":\"planner\""));
        assert!(json.contains("\"state\":\"call\""));
    }

    #[test]
    fn sub_agent_result_serializes() {
        let event = StreamPart::sub_agent_result("planner", "inv-001");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool-agent\""));
        assert!(json.contains("\"state\":\"result\""));
    }

    #[test]
    fn data_tool_agent_serializes() {
        let event = StreamPart::data_tool_agent(AgentUsage {
            agent_name: "planner".to_string(),
            model: "claude-opus-4-6".to_string(),
            usage: TokenUsage::simple(100, 50),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"data-tool-agent\""));
        assert!(json.contains("\"agentName\":\"planner\""));
        assert!(json.contains("\"promptTokens\":100"));
    }

    #[test]
    fn tool_progress_serializes() {
        let event = StreamPart::tool_progress("draft_plan", None, "Resolving entities", 1, 4, None);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool-progress\""));
        assert!(json.contains("\"toolName\":\"draft_plan\""));
        assert!(json.contains("\"label\":\"Resolving entities\""));
        assert!(json.contains("\"phaseIndex\":1"));
        assert!(json.contains("\"totalPhases\":4"));
        assert!(!json.contains("milestone"));
        assert!(!json.contains("toolCallId"));
    }

    #[test]
    fn tool_progress_with_milestone_serializes() {
        let milestone = serde_json::json!({"matched": 142, "brands": 5});
        let event =
            StreamPart::tool_progress("draft_plan", None, "Storing plan", 2, 4, Some(milestone));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"milestone\""));
        assert!(json.contains("\"matched\":142"));
    }

    #[test]
    fn tool_progress_roundtrips() {
        let milestone = serde_json::json!({"matched": 42});
        let event = StreamPart::tool_progress(
            "draft_plan",
            Some("call-42".to_string()),
            "Resolving entities",
            1,
            4,
            Some(milestone),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"toolCallId\":\"call-42\""));
        let deserialized: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn approval_required_serializes() {
        use crate::approval::ApprovalKind;
        let event = StreamPart::approval_required(ApprovalRequest {
            id: crate::ApprovalId::new_unchecked("apr-1"),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: serde_json::json!({"x": 1}),
            glimpse: None,
            resource_id: crate::TenantId::new_unchecked("acme"),
            thread_id: crate::ThreadId::new_unchecked("th-1"),
            correlation_id: Some("tool_use_42".into()),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"approval-required\""));
        assert!(json.contains("\"target\":\"create_scenario\""));
        let parsed: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn approval_decision_event_serializes() {
        let event = StreamPart::approval_decision(ApprovalDecision::approve(
            crate::ApprovalId::new_unchecked("apr-1"),
        ));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"approval-decision\""));
        assert!(json.contains("\"outcome\":\"approve\""));
        let parsed: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn plan_status_change_event_serializes() {
        let event = StreamPart::plan_status_change("plan-1", "draft", "pending_approval");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"plan-status-change\""));
        assert!(json.contains("\"to\":\"pending_approval\""));
        let parsed: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn data_flow_ui_serializes() {
        let event = StreamPart::data_flow_ui("{\"components\":[]}");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"data-flow-ui\""));
        assert!(json.contains("\"dsl\""));
    }

    #[test]
    fn data_flow_ui_roundtrips() {
        let event = StreamPart::data_flow_ui("test-dsl");
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn custom_event_serializes() {
        let event = StreamPart::custom("my-domain-event", serde_json::json!({"key": "value"}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"custom\""));
        assert!(
            json.contains("my-domain-event"),
            "Expected 'my-domain-event' in: {json}"
        );
        assert!(json.contains("\"key\""));
    }

    #[test]
    fn custom_event_roundtrips() {
        let event = StreamPart::custom(
            "my-custom-event",
            serde_json::json!({"key": "value", "count": 42}),
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }

    // ── MessagePartAccumulator tests ────────────────────────────

    #[test]
    fn accumulator_text_coalescence() {
        // Law 1: Consecutive text events produce one text part
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::text("Hello, "));
        acc.push(&StreamPart::text("world!"));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 1);
        assert_eq!(plain, "Hello, world!");
        assert!(matches!(&parts[0], MessagePart::Text { text } if text == "Hello, world!"));
    }

    #[test]
    fn accumulator_tool_boundary_flushes_text() {
        // Law 2: Tool invocations flush pending text
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::text("Before tool. "));
        acc.push(&StreamPart::tool_call(
            "call-1",
            "myTool",
            serde_json::json!({}),
        ));
        acc.push(&StreamPart::text("After tool."));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 3); // text, tool, text
        assert!(matches!(&parts[0], MessagePart::Text { text } if text == "Before tool. "));
        assert!(
            matches!(&parts[1], MessagePart::ToolInvocation { state, .. } if *state == MessagePartState::Call)
        );
        assert!(matches!(&parts[2], MessagePart::Text { text } if text == "After tool."));
        assert_eq!(plain, "Before tool. After tool.");
    }

    #[test]
    fn accumulator_tool_result() {
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::tool_result(
            "call-1",
            "search",
            serde_json::json!({"q": "nike"}),
            serde_json::json!({"count": 42}),
        ));
        let (parts, _) = acc.finish();

        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::ToolInvocation {
                tool_call_id,
                tool_name,
                state,
                result,
                ..
            } => {
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(tool_name, "search");
                assert_eq!(*state, MessagePartState::Result);
                assert!(result.is_some());
            }
            _ => panic!("expected ToolInvocation"),
        }
    }

    #[test]
    fn accumulator_reasoning() {
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::reasoning("Thinking about this..."));
        acc.push(&StreamPart::text("Answer."));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 2);
        // Reasoning is flushed when text arrives (different event type boundary)
        assert!(
            matches!(&parts[0], MessagePart::Reasoning { text } if text == "Thinking about this...")
        );
        assert!(matches!(&parts[1], MessagePart::Text { text } if text == "Answer."));
        assert_eq!(plain, "Answer.");
    }

    #[test]
    fn accumulator_sub_agent() {
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::sub_agent_call("planner", "inv-1"));
        acc.push(&StreamPart::sub_agent_result("planner", "inv-1"));
        let (parts, _) = acc.finish();

        assert_eq!(parts.len(), 2);
        assert!(
            matches!(&parts[0], MessagePart::ToolAgent { state, .. } if *state == MessagePartState::Call)
        );
        assert!(
            matches!(&parts[1], MessagePart::ToolAgent { state, .. } if *state == MessagePartState::Result)
        );
    }

    #[test]
    fn accumulator_flow_ui() {
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::text("Here's the plan: "));
        acc.push(&StreamPart::data_flow_ui("{\"type\":\"approval\"}"));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(&parts[1], MessagePart::FlowUI { dsl } if !dsl.is_empty()));
        assert_eq!(plain, "Here's the plan: ");
    }

    #[test]
    fn accumulator_ignores_non_persistent_events() {
        // StepStart, ToolProgress, Finish, Error, etc. are not persisted
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::StepStart);
        acc.push(&StreamPart::text("Hello"));
        acc.push(&StreamPart::tool_progress(
            "build", None, "Phase 1", 0, 3, None,
        ));
        acc.push(&StreamPart::finish(
            FinishReason::Stop,
            TokenUsage::simple(100, 50),
        ));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 1);
        assert_eq!(plain, "Hello");
    }

    #[test]
    fn accumulator_empty_produces_empty() {
        // Law 3: Idempotent finish
        let acc = MessagePartAccumulator::new();
        let (parts, plain) = acc.finish();
        assert!(parts.is_empty());
        assert!(plain.is_empty());
    }

    #[test]
    fn message_part_serializes_correctly() {
        // Verify MessagePart serialization matches frontend expectations
        let part = MessagePart::ToolInvocation {
            tool_call_id: "call-1".to_string(),
            tool_name: "search".to_string(),
            args: serde_json::json!({"q": "test"}),
            state: MessagePartState::Call,
            result: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"tool-invocation\""));
        assert!(json.contains("\"toolCallId\":\"call-1\""));
        assert!(json.contains("\"toolName\":\"search\""));
        assert!(json.contains("\"state\":\"call\""));
        assert!(!json.contains("result"));
    }

    #[test]
    fn message_part_roundtrips() {
        let parts = vec![
            MessagePart::Text {
                text: "Hello".to_string(),
            },
            MessagePart::Reasoning {
                text: "Thinking".to_string(),
            },
            MessagePart::ToolInvocation {
                tool_call_id: "c1".to_string(),
                tool_name: "search".to_string(),
                args: serde_json::json!({}),
                state: MessagePartState::Result,
                result: Some(serde_json::json!(42)),
            },
            MessagePart::ToolAgent {
                tool_call_id: "c2".to_string(),
                agent_name: "planner".to_string(),
                state: MessagePartState::Call,
            },
            MessagePart::FlowUI {
                dsl: "{}".to_string(),
            },
        ];
        let json = serde_json::to_string(&parts).unwrap();
        let deserialized: Vec<MessagePart> = serde_json::from_str(&json).unwrap();
        assert_eq!(parts, deserialized);
    }

    // ── New tests: reasoning coalescence, MessagePartState, typed FlowUI ──

    #[test]
    fn accumulator_reasoning_coalescence() {
        // Law 2: Consecutive reasoning events produce one reasoning part
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::reasoning("Step 1. "));
        acc.push(&StreamPart::reasoning("Step 2. "));
        acc.push(&StreamPart::reasoning("Step 3."));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 1);
        assert!(
            matches!(&parts[0], MessagePart::Reasoning { text } if text == "Step 1. Step 2. Step 3.")
        );
        assert!(plain.is_empty()); // reasoning doesn't contribute to plain text
    }

    #[test]
    fn accumulator_reasoning_flushed_at_text_boundary() {
        // Reasoning then text → two separate parts
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::reasoning("Thinking..."));
        acc.push(&StreamPart::text("Answer."));
        let (parts, _) = acc.finish();

        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Reasoning { text } if text == "Thinking..."));
        assert!(matches!(&parts[1], MessagePart::Text { text } if text == "Answer."));
    }

    #[test]
    fn accumulator_reasoning_flushed_at_tool_boundary() {
        // Reasoning then tool → reasoning flushed before tool
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::reasoning("Let me search..."));
        acc.push(&StreamPart::tool_call(
            "c1",
            "search",
            serde_json::json!({}),
        ));
        let (parts, _) = acc.finish();

        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Reasoning { .. }));
        assert!(matches!(&parts[1], MessagePart::ToolInvocation { .. }));
    }

    #[test]
    fn accumulator_interleaved_text_reasoning() {
        // text → reasoning → text produces 3 separate coalesced parts
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::text("A"));
        acc.push(&StreamPart::text("B"));
        acc.push(&StreamPart::reasoning("R1"));
        acc.push(&StreamPart::reasoning("R2"));
        acc.push(&StreamPart::text("C"));
        let (parts, plain) = acc.finish();

        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], MessagePart::Text { text } if text == "AB"));
        assert!(matches!(&parts[1], MessagePart::Reasoning { text } if text == "R1R2"));
        assert!(matches!(&parts[2], MessagePart::Text { text } if text == "C"));
        assert_eq!(plain, "ABC");
    }

    #[test]
    fn message_part_state_roundtrips() {
        // MessagePartState serde: call → "call", result → "result"
        let call_json = serde_json::to_string(&MessagePartState::Call).unwrap();
        assert_eq!(call_json, "\"call\"");
        let result_json = serde_json::to_string(&MessagePartState::Result).unwrap();
        assert_eq!(result_json, "\"result\"");

        let call: MessagePartState = serde_json::from_str("\"call\"").unwrap();
        assert_eq!(call, MessagePartState::Call);
        let result: MessagePartState = serde_json::from_str("\"result\"").unwrap();
        assert_eq!(result, MessagePartState::Result);
    }

    #[test]
    fn command_card_payload_roundtrips() {
        let payload = CommandCardPayload {
            dsl: "{\"type\":\"approval\"}".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"dsl\""));
        let deserialized: CommandCardPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, deserialized);
    }

    #[test]
    fn accumulator_flow_ui_typed() {
        // Typed FlowUI extraction instead of stringly-typed
        let mut acc = MessagePartAccumulator::new();
        acc.push(&StreamPart::data_flow_ui("{\"components\":[]}"));
        let (parts, _) = acc.finish();

        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::FlowUI { dsl } => assert_eq!(dsl, "{\"components\":[]}"),
            _ => panic!("expected FlowUI"),
        }
    }

    // =========================================================================
    // Property-Based Tests (Hegel) — StreamPart
    // =========================================================================

    use crate::latency::LatencySummary;
    use hegel::generators;

    fn draw_stream_part(tc: &hegel::TestCase) -> StreamPart {
        let variant = tc.draw(generators::integers::<u8>().max_value(13));
        match variant {
            0 => StreamPart::text(&tc.draw(generators::text().max_size(50))),
            1 => StreamPart::reasoning(&tc.draw(generators::text().max_size(50))),
            2 => StreamPart::StepStart,
            3 => StreamPart::tool_call(
                &tc.draw(generators::text().min_size(1).max_size(10)),
                &tc.draw(generators::text().min_size(1).max_size(10)),
                serde_json::json!({}),
            ),
            4 => StreamPart::tool_result(
                &tc.draw(generators::text().min_size(1).max_size(10)),
                &tc.draw(generators::text().min_size(1).max_size(10)),
                serde_json::json!({}),
                serde_json::json!("ok"),
            ),
            5 => StreamPart::sub_agent_call(
                &tc.draw(generators::text().min_size(1).max_size(10)),
                &tc.draw(generators::text().min_size(1).max_size(10)),
            ),
            6 => StreamPart::sub_agent_result(
                &tc.draw(generators::text().min_size(1).max_size(10)),
                &tc.draw(generators::text().min_size(1).max_size(10)),
            ),
            7 => StreamPart::finish(
                FinishReason::Stop,
                TokenUsage::simple(
                    tc.draw(generators::integers::<u64>()),
                    tc.draw(generators::integers::<u64>()),
                ),
            ),
            8 => StreamPart::error(&tc.draw(generators::text().max_size(50))),
            9 => StreamPart::cost_summary(CostSummary::new(vec![])),
            10 => StreamPart::latency_summary(LatencySummary::zero()),
            11 => StreamPart::data_flow_ui(&tc.draw(generators::text().max_size(50))),
            12 => StreamPart::DataFileRegistered {
                data: FileRegistration {
                    file_id: tc.draw(generators::text().min_size(1).max_size(10)),
                    filename: tc.draw(generators::text().min_size(1).max_size(10)),
                    thread_id: tc.draw(generators::text().min_size(1).max_size(10)),
                    timestamp: chrono::Utc::now(),
                },
            },
            _ => StreamPart::custom(
                &tc.draw(generators::text().min_size(1).max_size(10)),
                serde_json::json!({"key": "value"}),
            ),
        }
    }

    #[hegel::test]
    fn to_sse_bytes_never_panics(tc: hegel::TestCase) {
        let part = draw_stream_part(&tc);
        let bytes = part.to_sse_bytes();
        // Must produce valid SSE format: "data: {...}\n\n"
        let s = std::str::from_utf8(&bytes).expect("SSE must be valid UTF-8");
        assert!(s.starts_with("data: "), "SSE must start with 'data: '");
        assert!(s.ends_with("\n\n"), "SSE must end with double newline");
    }

    #[hegel::test]
    fn stream_part_serde_roundtrip(tc: hegel::TestCase) {
        let original = draw_stream_part(&tc);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: StreamPart = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[hegel::test]
    fn stream_part_serialization_is_json_object(tc: hegel::TestCase) {
        let part = draw_stream_part(&tc);
        let json = serde_json::to_string(&part).unwrap();
        // Every StreamPart should serialize to a JSON object with a "type" field
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            value.is_object(),
            "StreamPart must serialize to a JSON object"
        );
        assert!(
            value.get("type").is_some(),
            "StreamPart JSON must have a 'type' field"
        );
    }

    // --- MessagePart serde roundtrip ---

    fn draw_message_part(tc: &hegel::TestCase) -> MessagePart {
        let variant = tc.draw(generators::integers::<u8>().max_value(5));
        match variant {
            0 => MessagePart::Text {
                text: tc.draw(generators::text().max_size(50)),
            },
            1 => MessagePart::Reasoning {
                text: tc.draw(generators::text().max_size(50)),
            },
            2 => MessagePart::ToolInvocation {
                tool_call_id: tc.draw(generators::text().min_size(1).max_size(10)),
                tool_name: tc.draw(generators::text().min_size(1).max_size(10)),
                args: serde_json::json!({}),
                state: if tc.draw(generators::booleans()) {
                    MessagePartState::Call
                } else {
                    MessagePartState::Result
                },
                result: if tc.draw(generators::booleans()) {
                    Some(serde_json::json!("ok"))
                } else {
                    None
                },
            },
            3 => MessagePart::ToolAgent {
                tool_call_id: tc.draw(generators::text().min_size(1).max_size(10)),
                agent_name: tc.draw(generators::text().min_size(1).max_size(10)),
                state: if tc.draw(generators::booleans()) {
                    MessagePartState::Call
                } else {
                    MessagePartState::Result
                },
            },
            4 => MessagePart::FlowUI {
                dsl: tc.draw(generators::text().max_size(50)),
            },
            _ => MessagePart::File {
                file_id: tc.draw(generators::text().min_size(1).max_size(10)),
                filename: tc.draw(generators::text().min_size(1).max_size(10)),
            },
        }
    }

    #[hegel::test]
    fn message_part_serde_roundtrip(tc: hegel::TestCase) {
        let original = draw_message_part(&tc);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    // --- Accumulator coalescence laws ---

    #[hegel::test]
    fn accumulator_text_coalescence_prop(tc: hegel::TestCase) {
        // Consecutive text events should coalesce into one Text part
        let n = tc.draw(generators::integers::<usize>().min_value(1).max_value(10));
        let mut acc = MessagePartAccumulator::new();
        let mut expected = String::new();
        for _ in 0..n {
            let chunk = tc.draw(generators::text().max_size(20));
            acc.push(&StreamPart::text(&chunk));
            expected.push_str(&chunk);
        }
        let (parts, plain) = acc.finish();
        // All text should be in one part (coalescence) plus the plain text
        assert_eq!(plain, expected);
        if !expected.is_empty() {
            assert!(
                parts
                    .iter()
                    .filter(|p| matches!(p, MessagePart::Text { .. }))
                    .count()
                    <= 1,
                "consecutive text events should coalesce into at most 1 text part"
            );
        }
    }

    #[hegel::test]
    fn accumulator_reasoning_coalescence_prop(tc: hegel::TestCase) {
        // Consecutive reasoning events should coalesce into one Reasoning part
        let n = tc.draw(generators::integers::<usize>().min_value(1).max_value(10));
        let mut acc = MessagePartAccumulator::new();
        let mut expected = String::new();
        for _ in 0..n {
            let chunk = tc.draw(generators::text().max_size(20));
            acc.push(&StreamPart::reasoning(&chunk));
            expected.push_str(&chunk);
        }
        let (parts, _) = acc.finish();
        if !expected.is_empty() {
            assert!(
                parts
                    .iter()
                    .filter(|p| matches!(p, MessagePart::Reasoning { .. }))
                    .count()
                    <= 1,
                "consecutive reasoning events should coalesce into at most 1 reasoning part"
            );
        }
    }
}
