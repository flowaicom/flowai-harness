//! Protocol-enforcing stream construction with algebraic guarantees.
//!
//! This module provides safe construction of AI SDK protocol streams.
//! The key insight: **make illegal states unrepresentable**.
//!
//! # Problem
//!
//! The AI SDK protocol has invariants that raw `Vec<StreamPart>` cannot enforce:
//! 1. Every `ToolResult` must reference an existing `ToolCall`
//! 2. Streams cannot end with unresolved pending calls
//! 3. Causality: results must follow their corresponding calls
//!
//! Violating these invariants causes frontend bugs that are hard to trace.
//!
//! # Solution: Type-Driven Design
//!
//! ```text
//! StreamBuilder ─[&mut self emit_*]─► &mut StreamBuilder
//!       │
//!       └─[self.finish()]─► Result<ValidatedStream, Error>
//!                                  │
//!                                  └─► EventStream (for composition)
//! ```
//!
//! - `StreamBuilder`: Mutable builder tracking pending calls. All mutation
//!   methods take `&mut self`, enabling natural loop ingestion. The builder
//!   remains usable after errors (error paths never corrupt state).
//! - `ValidatedStream`: Proof that all invariants are satisfied (private ctor).
//! - `EventStream`: Monoid newtype over `Vec<StreamPart>`.
//! - `finish()` consumes self — after finishing, the builder is gone.
//!
//! # Laws
//!
//! ## StreamBuilder Laws (Protocol Enforcement)
//!
//! - **L1. Call Tracking**: `emit_call(call)` ⟹ `call.id ∈ pending_calls`
//! - **L2. Result Matching**: `emit_result(result)` succeeds IFF `result.call_id ∈ pending_calls`
//! - **L3. Completion**: `finish()` succeeds IFF `pending_calls = ∅`
//! - **L4. Causality**: Valid streams have results only for emitted calls
//! - **L5. Error Safety**: Failed mutations leave the builder unchanged
//!
//! ## EventStream Laws (Monoid)
//!
//! - **M1. Identity**: `EMPTY.concat(s) == s == s.concat(EMPTY)`
//! - **M2. Associativity**: `(a.concat(b)).concat(c) == a.concat(b.concat(c))`

use std::collections::HashSet;

use crate::stream_part::{
    ErrorInfo, FinishReason, StreamPart, ToolAgentData, ToolAgentState, ToolInvocationData,
    ToolInvocationState,
};
use crate::usage::TokenUsage;
use thiserror::Error;

// =============================================================================
// Error Types
// =============================================================================

/// Errors from stream construction.
///
/// These represent protocol violations that would cause frontend bugs.
/// By catching them at construction time, we prevent runtime failures.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum StreamBuilderError {
    /// A tool result was emitted without a corresponding call.
    #[error("Orphan result: tool result '{call_id}' has no matching call")]
    OrphanResult { call_id: String },

    /// A sub-agent result was emitted without a corresponding call.
    #[error("Orphan sub-agent result: invocation '{invocation_id}' has no matching call")]
    OrphanSubAgentResult { invocation_id: String },

    /// Attempted to finish with unresolved tool calls.
    #[error("Unresolved calls remain: {call_ids:?}")]
    UnresolvedCalls { call_ids: Vec<String> },

    /// Attempted to finish with unresolved sub-agent calls.
    #[error("Unresolved sub-agent calls remain: {invocation_ids:?}")]
    UnresolvedSubAgentCalls { invocation_ids: Vec<String> },

    /// Duplicate tool call ID.
    #[error("Duplicate call ID: '{call_id}' already pending")]
    DuplicateCallId { call_id: String },

    /// Duplicate sub-agent invocation ID.
    #[error("Duplicate sub-agent invocation ID: '{invocation_id}' already pending")]
    DuplicateSubAgentId { invocation_id: String },

    /// Invalid raw event type passed to `emit_raw`.
    #[error("Invalid raw event: {reason}. Use the dedicated method instead.")]
    InvalidRawEvent { reason: String },
}

// =============================================================================
// Input Types for Building
// =============================================================================

/// A tool call to be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }

    fn into_stream_part(self) -> StreamPart {
        StreamPart::ToolInvocation(ToolInvocationData {
            id: self.id,
            name: self.name,
            args: self.args,
            state: ToolInvocationState::Call,
        })
    }
}

/// A tool result to be emitted. The `call_id` must reference an existing pending call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub args: serde_json::Value,
    pub value: serde_json::Value,
}

impl ToolResult {
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
        value: serde_json::Value,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            args,
            value,
        }
    }

    fn into_stream_part(self) -> StreamPart {
        StreamPart::ToolInvocation(ToolInvocationData {
            id: self.call_id,
            name: self.name,
            args: self.args,
            state: ToolInvocationState::Result { result: self.value },
        })
    }
}

/// A sub-agent call to be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAgentCall {
    pub agent_name: String,
    pub invocation_id: String,
}

impl SubAgentCall {
    pub fn new(agent_name: impl Into<String>, invocation_id: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            invocation_id: invocation_id.into(),
        }
    }

    fn into_stream_part(self) -> StreamPart {
        StreamPart::ToolAgent(ToolAgentData {
            agent_name: self.agent_name,
            invocation_id: self.invocation_id,
            state: ToolAgentState::Call,
        })
    }
}

/// A sub-agent result to be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAgentResult {
    pub agent_name: String,
    pub invocation_id: String,
}

impl SubAgentResult {
    pub fn new(agent_name: impl Into<String>, invocation_id: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            invocation_id: invocation_id.into(),
        }
    }

    fn into_stream_part(self) -> StreamPart {
        StreamPart::ToolAgent(ToolAgentData {
            agent_name: self.agent_name,
            invocation_id: self.invocation_id,
            state: ToolAgentState::Result,
        })
    }
}

// =============================================================================
// Termination
// =============================================================================

/// How a stream terminates.
///
/// Every valid stream must end with exactly one termination event.
#[derive(Debug, Clone, PartialEq)]
pub enum Termination {
    Finish {
        reason: FinishReason,
        usage: TokenUsage,
    },
    Error {
        message: String,
        code: Option<String>,
    },
}

impl Termination {
    pub fn finish(reason: FinishReason, usage: TokenUsage) -> Self {
        Self::Finish { reason, usage }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            code: None,
        }
    }

    pub fn error_with_code(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            code: Some(code.into()),
        }
    }

    fn into_stream_part(self) -> StreamPart {
        match self {
            Self::Finish { reason, usage } => StreamPart::Finish { reason, usage },
            Self::Error { message, code } => StreamPart::Error {
                error: ErrorInfo { message, code },
            },
        }
    }
}

// =============================================================================
// StreamBuilder — Protocol-Enforcing State Machine
// =============================================================================

/// A builder for constructing protocol-compliant streams.
///
/// Tracks pending tool and sub-agent calls, enforcing that all are resolved
/// before the stream can be finalized into a [`ValidatedStream`].
///
/// All mutation methods take `&mut self` — the builder remains usable after
/// errors (error paths never corrupt state, L5). `finish()` consumes `self`
/// to produce the proof term [`ValidatedStream`].
///
/// # Laws
///
/// 1. `emit_call(call)` ⟹ `call.id ∈ pending_tool_calls`
/// 2. `emit_result(result)` succeeds IFF `result.call_id ∈ pending_tool_calls`
/// 3. `finish()` succeeds IFF `pending_tool_calls = ∅ ∧ pending_sub_agent_calls = ∅`
/// 5. On error, builder state is unchanged (safe to inspect or retry)
#[derive(Debug, Clone)]
pub struct StreamBuilder {
    events: Vec<StreamPart>,
    pending_tool_calls: HashSet<String>,
    pending_sub_agent_calls: HashSet<String>,
}

impl Default for StreamBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamBuilder {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            pending_tool_calls: HashSet::new(),
            pending_sub_agent_calls: HashSet::new(),
        }
    }

    // =========================================================================
    // Simple Event Emission (Always Succeeds)
    // =========================================================================

    pub fn emit_text(&mut self, text: impl Into<String>) -> &mut Self {
        self.events.push(StreamPart::text(text));
        self
    }

    pub fn emit_step_start(&mut self) -> &mut Self {
        self.events.push(StreamPart::StepStart);
        self
    }

    pub fn emit_reasoning(&mut self, text: impl Into<String>) -> &mut Self {
        self.events
            .push(StreamPart::Reasoning { text: text.into() });
        self
    }

    /// Emit a raw StreamPart event.
    ///
    /// Returns `InvalidRawEvent` if the event is a ToolInvocation, ToolAgent,
    /// Finish, or Error. Use the dedicated methods for those.
    pub fn emit_raw(&mut self, part: StreamPart) -> Result<&mut Self, StreamBuilderError> {
        match &part {
            StreamPart::ToolInvocation(_) => {
                return Err(StreamBuilderError::InvalidRawEvent {
                    reason: "Use emit_call/emit_result for tool invocations".to_string(),
                });
            }
            StreamPart::ToolAgent(_) => {
                return Err(StreamBuilderError::InvalidRawEvent {
                    reason: "Use emit_sub_agent_call/emit_sub_agent_result for sub-agent events"
                        .to_string(),
                });
            }
            StreamPart::Finish { .. } | StreamPart::Error { .. } => {
                return Err(StreamBuilderError::InvalidRawEvent {
                    reason: "Use finish() to terminate the stream".to_string(),
                });
            }
            _ => {}
        }
        self.events.push(part);
        Ok(self)
    }

    // =========================================================================
    // Tool Call/Result (Tracked State)
    // =========================================================================

    /// Emit a tool call event.
    ///
    /// # Law: Call Tracking
    /// After this call: `call.id ∈ pending_tool_calls`
    ///
    /// # Law: Error Safety
    /// On `DuplicateCallId`, builder state is unchanged.
    pub fn emit_call(&mut self, call: ToolCall) -> Result<&mut Self, StreamBuilderError> {
        if self.pending_tool_calls.contains(&call.id) {
            return Err(StreamBuilderError::DuplicateCallId { call_id: call.id });
        }
        self.pending_tool_calls.insert(call.id.clone());
        self.events.push(call.into_stream_part());
        Ok(self)
    }

    /// Emit a tool result event.
    ///
    /// # Law: Result Matching
    /// Succeeds IFF `result.call_id ∈ pending_tool_calls`
    ///
    /// # Law: Error Safety
    /// On `OrphanResult`, builder state is unchanged.
    pub fn emit_result(&mut self, result: ToolResult) -> Result<&mut Self, StreamBuilderError> {
        if !self.pending_tool_calls.remove(&result.call_id) {
            return Err(StreamBuilderError::OrphanResult {
                call_id: result.call_id,
            });
        }
        self.events.push(result.into_stream_part());
        Ok(self)
    }

    // =========================================================================
    // Sub-Agent Call/Result (Tracked State)
    // =========================================================================

    pub fn emit_sub_agent_call(
        &mut self,
        call: SubAgentCall,
    ) -> Result<&mut Self, StreamBuilderError> {
        if self.pending_sub_agent_calls.contains(&call.invocation_id) {
            return Err(StreamBuilderError::DuplicateSubAgentId {
                invocation_id: call.invocation_id,
            });
        }
        self.pending_sub_agent_calls
            .insert(call.invocation_id.clone());
        self.events.push(call.into_stream_part());
        Ok(self)
    }

    pub fn emit_sub_agent_result(
        &mut self,
        result: SubAgentResult,
    ) -> Result<&mut Self, StreamBuilderError> {
        if !self.pending_sub_agent_calls.remove(&result.invocation_id) {
            return Err(StreamBuilderError::OrphanSubAgentResult {
                invocation_id: result.invocation_id,
            });
        }
        self.events.push(result.into_stream_part());
        Ok(self)
    }

    // =========================================================================
    // Ingestion (Classify raw StreamPart into builder operations)
    // =========================================================================

    /// Ingest a raw `StreamPart` by classifying it into the correct builder operation.
    ///
    /// Designed for loop ingestion:
    /// ```ignore
    /// let mut builder = StreamBuilder::new();
    /// for part in incoming_parts {
    ///     builder.ingest(part)?;
    /// }
    /// let stream = builder.finish(termination)?;
    /// ```
    pub fn ingest(&mut self, part: StreamPart) -> Result<&mut Self, StreamBuilderError> {
        match part {
            StreamPart::ToolInvocation(ToolInvocationData {
                ref id,
                ref name,
                ref args,
                state: ToolInvocationState::Call,
            }) => self.emit_call(ToolCall::new(id, name, args.clone())),

            StreamPart::ToolInvocation(ToolInvocationData {
                ref id,
                ref name,
                ref args,
                state: ToolInvocationState::Result { ref result },
            }) => self.emit_result(ToolResult::new(id, name, args.clone(), result.clone())),

            StreamPart::ToolAgent(ToolAgentData {
                ref agent_name,
                ref invocation_id,
                state: ToolAgentState::Call,
            }) => self.emit_sub_agent_call(SubAgentCall::new(agent_name, invocation_id)),

            StreamPart::ToolAgent(ToolAgentData {
                ref agent_name,
                ref invocation_id,
                state: ToolAgentState::Result,
            }) => self.emit_sub_agent_result(SubAgentResult::new(agent_name, invocation_id)),

            StreamPart::Finish { .. } | StreamPart::Error { .. } => {
                Err(StreamBuilderError::InvalidRawEvent {
                    reason: "Termination events must use finish()".to_string(),
                })
            }
            other => self.emit_raw(other),
        }
    }

    // =========================================================================
    // Termination (consumes self — produces proof term)
    // =========================================================================

    /// Finish the stream with a termination event.
    ///
    /// Consumes the builder and produces a [`ValidatedStream`] — the proof term
    /// that all protocol invariants are satisfied.
    ///
    /// # Law: Completion
    /// Succeeds IFF `pending_tool_calls = ∅ ∧ pending_sub_agent_calls = ∅`
    pub fn finish(self, termination: Termination) -> Result<ValidatedStream, StreamBuilderError> {
        if !self.pending_tool_calls.is_empty() {
            let mut call_ids: Vec<_> = self.pending_tool_calls.into_iter().collect();
            call_ids.sort();
            return Err(StreamBuilderError::UnresolvedCalls { call_ids });
        }
        if !self.pending_sub_agent_calls.is_empty() {
            let mut invocation_ids: Vec<_> = self.pending_sub_agent_calls.into_iter().collect();
            invocation_ids.sort();
            return Err(StreamBuilderError::UnresolvedSubAgentCalls { invocation_ids });
        }
        Ok(ValidatedStream {
            events: self.events,
            termination,
        })
    }

    // =========================================================================
    // Inspection
    // =========================================================================

    pub fn pending_tool_calls(&self) -> &HashSet<String> {
        &self.pending_tool_calls
    }

    pub fn pending_sub_agent_calls(&self) -> &HashSet<String> {
        &self.pending_sub_agent_calls
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn has_pending_calls(&self) -> bool {
        !self.pending_tool_calls.is_empty() || !self.pending_sub_agent_calls.is_empty()
    }
}

// =============================================================================
// ValidatedStream — Proof of Protocol Compliance
// =============================================================================

/// A validated stream where all protocol invariants are guaranteed.
///
/// Can only be constructed via [`StreamBuilder::finish()`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedStream {
    events: Vec<StreamPart>,
    termination: Termination,
}

impl ValidatedStream {
    pub fn events(&self) -> &[StreamPart] {
        &self.events
    }

    pub fn termination(&self) -> &Termination {
        &self.termination
    }

    /// Number of events (excluding the termination event).
    ///
    /// Satisfies the Rust convention: `is_empty() ≡ len() == 0`.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the stream contains no events (excluding termination).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Total number of parts including the termination event.
    ///
    /// Always `>= 1` (every ValidatedStream has exactly one termination).
    pub fn total_parts(&self) -> usize {
        self.events.len() + 1
    }

    pub fn into_event_stream(self) -> EventStream {
        let mut parts = self.events;
        parts.push(self.termination.into_stream_part());
        EventStream(parts)
    }

    pub fn iter(&self) -> impl Iterator<Item = &StreamPart> + '_ {
        self.events.iter()
    }

    pub fn usage(&self) -> Option<&TokenUsage> {
        match &self.termination {
            Termination::Finish { usage, .. } => Some(usage),
            Termination::Error { .. } => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.termination, Termination::Error { .. })
    }
}

// =============================================================================
// EventStream — Monoid for Composing Agent Outputs
// =============================================================================

/// A stream of events that supports monoidal composition.
///
/// # Laws (Monoid)
///
/// - **Identity**: `EMPTY.concat(s) == s == s.concat(EMPTY)`
/// - **Associativity**: `(a.concat(b)).concat(c) == a.concat(b.concat(c))`
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventStream(Vec<StreamPart>);

impl EventStream {
    pub const EMPTY: Self = Self(Vec::new());

    pub fn from_parts(parts: Vec<StreamPart>) -> Self {
        Self(parts)
    }

    pub fn singleton(part: StreamPart) -> Self {
        Self(vec![part])
    }

    #[must_use]
    pub fn concat(self, other: Self) -> Self {
        let mut parts = self.0;
        parts.extend(other.0);
        Self(parts)
    }

    pub fn concat_all(streams: impl IntoIterator<Item = Self>) -> Self {
        streams
            .into_iter()
            .fold(Self::EMPTY, |acc, s| acc.concat(s))
    }

    pub fn parts(&self) -> &[StreamPart] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_parts(self) -> Vec<StreamPart> {
        self.0
    }

    pub fn iter(&self) -> impl Iterator<Item = &StreamPart> {
        self.0.iter()
    }
}

impl IntoIterator for EventStream {
    type Item = StreamPart;
    type IntoIter = std::vec::IntoIter<StreamPart>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a EventStream {
    type Item = &'a StreamPart;
    type IntoIter = std::slice::Iter<'a, StreamPart>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl FromIterator<StreamPart> for EventStream {
    fn from_iter<I: IntoIterator<Item = StreamPart>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Extend<StreamPart> for EventStream {
    fn extend<I: IntoIterator<Item = StreamPart>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}

// =============================================================================
// Free Functions
// =============================================================================

pub fn empty_stream() -> EventStream {
    EventStream::EMPTY
}

pub fn concat_streams(a: EventStream, b: EventStream) -> EventStream {
    a.concat(b)
}

pub fn concat_all_streams(streams: impl IntoIterator<Item = EventStream>) -> EventStream {
    EventStream::concat_all(streams)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_builder_is_empty() {
        let builder = StreamBuilder::new();
        assert_eq!(builder.event_count(), 0);
        assert!(builder.pending_tool_calls().is_empty());
        assert!(builder.pending_sub_agent_calls().is_empty());
        assert!(!builder.has_pending_calls());
    }

    #[test]
    fn emit_text_adds_event() {
        let mut builder = StreamBuilder::new();
        builder.emit_text("Hello");
        assert_eq!(builder.event_count(), 1);
    }

    #[test]
    fn emit_call_tracks_pending() {
        let call = ToolCall::new("call-1", "my_tool", json!({}));
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        assert!(builder.pending_tool_calls().contains("call-1"));
        assert!(builder.has_pending_calls());
    }

    #[test]
    fn emit_result_removes_pending() {
        let call = ToolCall::new("call-1", "my_tool", json!({}));
        let result = ToolResult::new("call-1", "my_tool", json!({}), json!("done"));
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        builder.emit_result(result).unwrap();
        assert!(!builder.pending_tool_calls().contains("call-1"));
        assert!(!builder.has_pending_calls());
    }

    #[test]
    fn orphan_result_fails() {
        let result = ToolResult::new("call-999", "my_tool", json!({}), json!("done"));
        let mut builder = StreamBuilder::new();
        let err = builder.emit_result(result).unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::OrphanResult { call_id } if call_id == "call-999"
        ));
    }

    #[test]
    fn duplicate_call_fails() {
        let call1 = ToolCall::new("call-1", "my_tool", json!({}));
        let call2 = ToolCall::new("call-1", "my_tool", json!({}));
        let mut builder = StreamBuilder::new();
        builder.emit_call(call1).unwrap();
        let err = builder.emit_call(call2).unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::DuplicateCallId { call_id } if call_id == "call-1"
        ));
    }

    #[test]
    fn finish_with_pending_calls_fails() {
        let call = ToolCall::new("call-1", "my_tool", json!({}));
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        let err = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::UnresolvedCalls { call_ids } if call_ids == vec!["call-1"]
        ));
    }

    #[test]
    fn finish_with_resolved_calls_succeeds() {
        let call = ToolCall::new("call-1", "my_tool", json!({}));
        let result = ToolResult::new("call-1", "my_tool", json!({}), json!("done"));
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        builder.emit_result(result).unwrap();
        let stream = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        assert_eq!(stream.events().len(), 2);
    }

    #[test]
    fn finish_empty_builder_succeeds() {
        let stream = StreamBuilder::new()
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        assert!(stream.is_empty());
    }

    #[test]
    fn sub_agent_call_tracking() {
        let call = SubAgentCall::new("planner", "inv-1");
        let mut builder = StreamBuilder::new();
        builder.emit_sub_agent_call(call).unwrap();
        assert!(builder.pending_sub_agent_calls().contains("inv-1"));
    }

    #[test]
    fn sub_agent_result_removes_pending() {
        let call = SubAgentCall::new("planner", "inv-1");
        let result = SubAgentResult::new("planner", "inv-1");
        let mut builder = StreamBuilder::new();
        builder.emit_sub_agent_call(call).unwrap();
        builder.emit_sub_agent_result(result).unwrap();
        assert!(!builder.pending_sub_agent_calls().contains("inv-1"));
    }

    #[test]
    fn orphan_sub_agent_result_fails() {
        let result = SubAgentResult::new("planner", "inv-999");
        let mut builder = StreamBuilder::new();
        let err = builder.emit_sub_agent_result(result).unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::OrphanSubAgentResult { invocation_id } if invocation_id == "inv-999"
        ));
    }

    #[test]
    fn finish_with_pending_sub_agents_fails() {
        let call = SubAgentCall::new("planner", "inv-1");
        let mut builder = StreamBuilder::new();
        builder.emit_sub_agent_call(call).unwrap();
        let err = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::UnresolvedSubAgentCalls { invocation_ids } if invocation_ids == vec!["inv-1"]
        ));
    }

    #[test]
    fn validated_stream_usage() {
        let usage = TokenUsage::simple(100, 50);
        let mut builder = StreamBuilder::new();
        builder.emit_text("Hello");
        let stream = builder
            .finish(Termination::finish(FinishReason::Stop, usage.clone()))
            .unwrap();
        assert_eq!(stream.usage(), Some(&usage));
    }

    #[test]
    fn validated_stream_error_has_no_usage() {
        let stream = StreamBuilder::new()
            .finish(Termination::error("Something went wrong"))
            .unwrap();
        assert_eq!(stream.usage(), None);
        assert!(stream.is_error());
    }

    #[test]
    fn validated_stream_to_event_stream() {
        let mut builder = StreamBuilder::new();
        builder.emit_text("Hello");
        let stream = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        let events = stream.into_event_stream();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn empty_stream_is_identity() {
        let stream = EventStream::singleton(StreamPart::text("hello"));
        assert_eq!(EventStream::EMPTY.concat(stream.clone()), stream.clone());
        assert_eq!(stream.clone().concat(EventStream::EMPTY), stream);
    }

    #[test]
    fn concat_combines_streams() {
        let a = EventStream::singleton(StreamPart::text("a"));
        let b = EventStream::singleton(StreamPart::text("b"));
        let combined = a.concat(b);
        assert_eq!(combined.len(), 2);
    }

    #[test]
    fn concat_all_folds_streams() {
        let streams = vec![
            EventStream::singleton(StreamPart::text("a")),
            EventStream::singleton(StreamPart::text("b")),
            EventStream::singleton(StreamPart::text("c")),
        ];
        let combined = EventStream::concat_all(streams);
        assert_eq!(combined.len(), 3);
    }

    #[test]
    fn concat_all_empty_is_identity() {
        let combined = EventStream::concat_all(std::iter::empty());
        assert_eq!(combined, EventStream::EMPTY);
    }

    // =========================================================================
    // Error Safety (L5) — builder survives errors
    // =========================================================================

    #[test]
    fn builder_survives_orphan_result() {
        let mut builder = StreamBuilder::new();
        builder.emit_text("before");
        let _err = builder
            .emit_result(ToolResult::new("nope", "t", json!({}), json!(null)))
            .unwrap_err();
        // Builder is still usable, state unchanged
        assert_eq!(builder.event_count(), 1);
        builder.emit_text("after");
        assert_eq!(builder.event_count(), 2);
        let stream = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        assert_eq!(stream.events().len(), 2);
    }

    #[test]
    fn builder_survives_duplicate_call() {
        let mut builder = StreamBuilder::new();
        builder
            .emit_call(ToolCall::new("c1", "t", json!({})))
            .unwrap();
        let _err = builder
            .emit_call(ToolCall::new("c1", "t", json!({})))
            .unwrap_err();
        // Original call still pending
        assert!(builder.pending_tool_calls().contains("c1"));
        builder
            .emit_result(ToolResult::new("c1", "t", json!({}), json!(null)))
            .unwrap();
        let _stream = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
    }

    // =========================================================================
    // Ingest Tests
    // =========================================================================

    #[test]
    fn ingest_text_event() {
        let mut builder = StreamBuilder::new();
        builder.ingest(StreamPart::text("hello")).unwrap();
        assert_eq!(builder.event_count(), 1);
        assert!(!builder.has_pending_calls());
    }

    #[test]
    fn ingest_tool_call_result_pair() {
        let call_part = StreamPart::tool_call("call-1", "search", json!({"q": "milk"}));
        let result_part =
            StreamPart::tool_result("call-1", "search", json!({"q": "milk"}), json!(["milk-1"]));
        let mut builder = StreamBuilder::new();
        builder.ingest(call_part).unwrap();
        builder.ingest(result_part).unwrap();
        assert_eq!(builder.event_count(), 2);
        assert!(!builder.has_pending_calls());
        let validated = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        assert_eq!(validated.events().len(), 2);
    }

    #[test]
    fn ingest_orphan_result_fails() {
        let result_part = StreamPart::tool_result("call-999", "search", json!({}), json!("done"));
        let mut builder = StreamBuilder::new();
        let err = builder.ingest(result_part).unwrap_err();
        assert!(matches!(
            err,
            StreamBuilderError::OrphanResult { call_id } if call_id == "call-999"
        ));
    }

    #[test]
    fn ingest_finish_rejected() {
        let finish_part = StreamPart::Finish {
            reason: FinishReason::Stop,
            usage: TokenUsage::ZERO,
        };
        let mut builder = StreamBuilder::new();
        let err = builder.ingest(finish_part).unwrap_err();
        assert!(matches!(err, StreamBuilderError::InvalidRawEvent { .. }));
    }

    #[test]
    fn ingest_sub_agent_call_result_pair() {
        let call_part = StreamPart::sub_agent_call("planner", "inv-1");
        let result_part = StreamPart::sub_agent_result("planner", "inv-1");
        let mut builder = StreamBuilder::new();
        builder.ingest(call_part).unwrap();
        builder.ingest(result_part).unwrap();
        assert_eq!(builder.event_count(), 2);
        assert!(!builder.has_pending_calls());
    }

    #[test]
    fn ingest_loop_pattern() {
        // Demonstrates the natural loop ingestion the &mut self API enables
        let parts = vec![
            StreamPart::text("Analyzing..."),
            StreamPart::tool_call("c1", "query", json!({"sql": "SELECT 1"})),
            StreamPart::tool_result("c1", "query", json!({"sql": "SELECT 1"}), json!([1])),
            StreamPart::text("Done."),
        ];

        let mut builder = StreamBuilder::new();
        for part in parts {
            builder.ingest(part).unwrap();
        }
        let stream = builder
            .finish(Termination::finish(FinishReason::Stop, TokenUsage::ZERO))
            .unwrap();
        assert_eq!(stream.events().len(), 4);
    }

    #[test]
    fn full_conversation_stream() {
        let mut builder = StreamBuilder::new();
        builder.emit_step_start();
        builder.emit_text("Let me search for that...");
        builder
            .emit_call(ToolCall::new(
                "call-1",
                "search_entities",
                json!({"query": "sample"}),
            ))
            .unwrap();
        builder
            .emit_result(ToolResult::new(
                "call-1",
                "search_entities",
                json!({"query": "sample"}),
                json!({"entities": ["entity-1", "entity-2"]}),
            ))
            .unwrap();
        builder.emit_text("Found 2 entities matching your search.");
        builder
            .emit_call(ToolCall::new(
                "call-2",
                "create_entity_set",
                json!({"ids": ["entity-1", "entity-2"]}),
            ))
            .unwrap();
        builder
            .emit_result(ToolResult::new(
                "call-2",
                "create_entity_set",
                json!({"ids": ["entity-1", "entity-2"]}),
                json!({"entitySetId": "set-123", "count": 2}),
            ))
            .unwrap();
        builder.emit_text("I've created an entity set with 2 items.");
        let stream = builder
            .finish(Termination::finish(
                FinishReason::Stop,
                TokenUsage::simple(500, 200),
            ))
            .unwrap();

        assert_eq!(stream.events().len(), 8);
        assert_eq!(stream.usage(), Some(&TokenUsage::simple(500, 200)));
        let events = stream.into_event_stream();
        assert_eq!(events.len(), 9);
    }

    // =========================================================================
    // Property Tests — StreamBuilder Laws (Hegel)
    // =========================================================================

    use hegel::generators;

    fn draw_tool_call(tc: &hegel::TestCase) -> ToolCall {
        let id = format!(
            "call-{}",
            tc.draw(generators::text().min_size(1).max_size(10))
        );
        let name = format!(
            "tool_{}",
            tc.draw(generators::text().min_size(1).max_size(10))
        );
        ToolCall::new(id, name, json!({}))
    }

    #[hegel::test]
    fn law_emit_call_tracks_pending(tc: hegel::TestCase) {
        let call = draw_tool_call(&tc);
        let id = call.id.clone();
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        assert!(builder.pending_tool_calls().contains(&id));
    }

    #[hegel::test]
    fn law_emit_result_requires_pending(tc: hegel::TestCase) {
        let call = draw_tool_call(&tc);
        let result = ToolResult::new(
            call.id.clone(),
            call.name.clone(),
            call.args.clone(),
            json!("result"),
        );
        // Emitting result without a matching call should fail
        let mut b1 = StreamBuilder::new();
        assert!(b1.emit_result(result.clone()).is_err());

        // Emitting result after a matching call should succeed
        let mut b2 = StreamBuilder::new();
        b2.emit_call(call).unwrap();
        assert!(b2.emit_result(result).is_ok());
    }

    #[hegel::test]
    fn law_finish_requires_empty_pending(tc: hegel::TestCase) {
        let call = draw_tool_call(&tc);
        let termination = Termination::finish(FinishReason::Stop, TokenUsage::ZERO);

        // Empty builder finishes successfully
        assert!(StreamBuilder::new().finish(termination.clone()).is_ok());

        // Builder with pending call cannot finish
        let mut with_pending = StreamBuilder::new();
        with_pending.emit_call(call).unwrap();
        assert!(with_pending.finish(termination).is_err());
    }

    #[hegel::test]
    fn law_complete_cycle_allows_finish(tc: hegel::TestCase) {
        let call = draw_tool_call(&tc);
        let result = ToolResult::new(
            call.id.clone(),
            call.name.clone(),
            call.args.clone(),
            json!("done"),
        );
        let termination = Termination::finish(FinishReason::Stop, TokenUsage::ZERO);
        let mut builder = StreamBuilder::new();
        builder.emit_call(call).unwrap();
        builder.emit_result(result).unwrap();
        assert!(builder.finish(termination).is_ok());
    }

    // =========================================================================
    // Property Tests — EventStream Monoid Laws (Hegel)
    // =========================================================================

    fn draw_event_stream(tc: &hegel::TestCase) -> EventStream {
        let n = tc.draw(generators::integers::<usize>().max_value(5));
        let parts: Vec<StreamPart> = (0..n)
            .map(|_| StreamPart::text(tc.draw(generators::text().max_size(20))))
            .collect();
        EventStream::from_parts(parts)
    }

    #[hegel::test]
    fn law_monoid_left_identity(tc: hegel::TestCase) {
        let s = draw_event_stream(&tc);
        assert_eq!(EventStream::EMPTY.concat(s.clone()), s);
    }

    #[hegel::test]
    fn law_monoid_right_identity(tc: hegel::TestCase) {
        let s = draw_event_stream(&tc);
        assert_eq!(s.clone().concat(EventStream::EMPTY), s);
    }

    #[hegel::test]
    fn law_monoid_associativity(tc: hegel::TestCase) {
        let a = draw_event_stream(&tc);
        let b = draw_event_stream(&tc);
        let c = draw_event_stream(&tc);
        let left = a.clone().concat(b.clone()).concat(c.clone());
        let right = a.concat(b.concat(c));
        assert_eq!(left, right);
    }

    #[hegel::test]
    fn law_concat_all_equivalence(tc: hegel::TestCase) {
        let a = draw_event_stream(&tc);
        let b = draw_event_stream(&tc);
        let c = draw_event_stream(&tc);
        let via_fold = EventStream::concat_all([a.clone(), b.clone(), c.clone()]);
        let via_manual = a.concat(b).concat(c);
        assert_eq!(via_fold, via_manual);
    }
}
