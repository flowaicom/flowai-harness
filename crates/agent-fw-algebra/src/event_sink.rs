//! EventSink algebra for push-based event emission.
//!
//! This trait abstracts over how stream parts are emitted to consumers.
//! It converts the push-based callback model into our domain.
//!
//! # Design
//!
//! EventSink is **synchronous** (non-blocking) by design. This is critical because:
//! - LLM streaming callbacks must not block
//! - Backpressure is handled via bounded channels (drop or buffer)
//! - The trait is naturally object-safe (no async methods)
//!
//! # Laws
//!
//! Implementations must satisfy these laws:
//!
//! - **L1. Totality**: `emit` never panics
//! - **L2. Order Preservation**: Events emitted in sequence arrive in that sequence.
//!   `emit(a); emit(b)` implies consumer sees `a` before `b`.
//! - **L3. Non-Blocking**: `emit` returns immediately (buffered)
//! - **L4. Closure Semantics**: After `close()`, `emit` returns `false`
//! - **L5. Idempotent Close**: Multiple calls to `close()` have no additional effect.

use agent_fw_core::approval::{ApprovalDecision, ApprovalRequest};
use agent_fw_core::stream_part::{ToolAgentState, ToolInvocationState};
use agent_fw_core::StreamPart;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Push-based event emission (synchronous, non-blocking).
///
/// This trait is **object-safe** and can be used as `Arc<dyn EventSink>`.
/// It's designed for LLM streaming hooks where blocking is not acceptable.
pub trait EventSink: Send + Sync {
    /// Emit a stream part to consumers.
    ///
    /// Returns `true` if the part was accepted, `false` if the sink is closed
    /// or the event was dropped (e.g., buffer full).
    ///
    /// This method is **non-blocking** - it returns immediately.
    fn emit(&self, part: StreamPart) -> bool;

    /// Close the sink, signaling no more events will be emitted.
    ///
    /// After close, subsequent `emit` calls return `false`.
    /// Calling close multiple times is idempotent.
    fn close(&self);

    /// Check if the sink is still open (accepting events).
    fn is_open(&self) -> bool;
}

/// Extension trait for EventSink with convenience methods.
///
/// These methods are NOT object-safe due to generic parameters,
/// but provide ergonomic APIs when you have a concrete type.
pub trait EventSinkExt: EventSink {
    /// Emit a text event.
    fn emit_text(&self, text: impl Into<String>) -> bool {
        self.emit(StreamPart::text(text))
    }

    /// Emit an error event.
    fn emit_error(&self, message: impl Into<String>) -> bool {
        self.emit(StreamPart::error(message))
    }

    /// Emit a tool call event.
    fn emit_tool_call(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
    ) -> bool {
        self.emit(StreamPart::tool_call(id, name, args))
    }

    /// Emit a tool result event.
    fn emit_tool_result(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
        result: serde_json::Value,
    ) -> bool {
        self.emit(StreamPart::tool_result(id, name, args, result))
    }

    /// Emit a DataFlowUI event (approval card DSL).
    fn emit_data_flow_ui(&self, dsl: impl Into<String>) -> bool {
        self.emit(StreamPart::data_flow_ui(dsl))
    }

    /// Emit a tool progress event.
    fn emit_tool_progress(
        &self,
        tool_name: impl Into<String>,
        tool_call_id: Option<String>,
        label: impl Into<String>,
        phase_index: u8,
        total_phases: u8,
        milestone: Option<serde_json::Value>,
    ) -> bool {
        self.emit(StreamPart::tool_progress(
            tool_name,
            tool_call_id,
            label,
            phase_index,
            total_phases,
            milestone,
        ))
    }

    /// Emit an `approval-required` event (pre-dispatch approval).
    fn emit_approval_required(&self, request: ApprovalRequest) -> bool {
        self.emit(StreamPart::approval_required(request))
    }

    /// Emit an `approval-decision` event (pre-dispatch approval).
    fn emit_approval_decision(&self, decision: ApprovalDecision) -> bool {
        self.emit(StreamPart::approval_decision(decision))
    }

    /// Emit a `plan-status-change` event (pre-dispatch approval).
    fn emit_plan_status_change(
        &self,
        plan_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> bool {
        self.emit(StreamPart::plan_status_change(plan_id, from, to))
    }
}

// Blanket implementation
impl<T: EventSink + ?Sized> EventSinkExt for T {}

// =============================================================================
// ValidatingEventSink
// =============================================================================

/// An EventSink wrapper that validates ToolCall→ToolResult causality in real-time.
///
/// Wraps an inner sink, forwarding all events immediately while tracking:
/// - Pending tool calls (emit call adds, emit result removes)
/// - Pending sub-agent calls (same pattern)
///
/// On close(), logs a warning if any calls remain unresolved.
/// In debug builds, asserts on orphan results (result without preceding call).
pub struct ValidatingEventSink {
    inner: Arc<dyn EventSink>,
    pending_tool_calls: Mutex<HashSet<String>>,
    pending_sub_agent_calls: Mutex<HashSet<String>>,
}

impl ValidatingEventSink {
    /// Wrap an existing EventSink with causality validation.
    pub fn wrap(inner: Arc<dyn EventSink>) -> Self {
        Self {
            inner,
            pending_tool_calls: Mutex::new(HashSet::new()),
            pending_sub_agent_calls: Mutex::new(HashSet::new()),
        }
    }
}

/// Lock a mutex, recovering from poison with a warning.
///
/// A poisoned mutex means another thread panicked while holding the lock.
/// We recover to uphold L1 (emit never panics), but log the incident so
/// it's observable in diagnostics.
fn recover_mutex<'a, T>(m: &'a Mutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| {
        tracing::warn!(mutex = name, "mutex poisoned, recovering");
        e.into_inner()
    })
}

impl EventSink for ValidatingEventSink {
    fn emit(&self, part: StreamPart) -> bool {
        // Track causality — recover from poison to uphold L1 (emit never panics)
        match &part {
            StreamPart::ToolInvocation(data) if data.state == ToolInvocationState::Call => {
                recover_mutex(&self.pending_tool_calls, "pending_tool_calls")
                    .insert(data.id.clone());
            }
            StreamPart::ToolInvocation(data)
                if matches!(data.state, ToolInvocationState::Result { .. }) =>
            {
                let removed =
                    recover_mutex(&self.pending_tool_calls, "pending_tool_calls").remove(&data.id);
                debug_assert!(removed, "Orphan tool result: {}", data.id);
                if !removed {
                    tracing::warn!(tool_id = %data.id, "Orphan tool result — no matching call");
                }
            }
            StreamPart::ToolAgent(data) if data.state == ToolAgentState::Call => {
                recover_mutex(&self.pending_sub_agent_calls, "pending_sub_agent_calls")
                    .insert(data.invocation_id.clone());
            }
            StreamPart::ToolAgent(data) if data.state == ToolAgentState::Result => {
                let removed =
                    recover_mutex(&self.pending_sub_agent_calls, "pending_sub_agent_calls")
                        .remove(&data.invocation_id);
                debug_assert!(removed, "Orphan sub-agent result: {}", data.invocation_id);
                if !removed {
                    tracing::warn!(
                        invocation_id = %data.invocation_id,
                        "Orphan sub-agent result — no matching call"
                    );
                }
            }
            _ => {}
        }

        // Forward immediately (never blocks)
        self.inner.emit(part)
    }

    fn close(&self) {
        let pending_tools = recover_mutex(&self.pending_tool_calls, "pending_tool_calls");
        let pending_agents =
            recover_mutex(&self.pending_sub_agent_calls, "pending_sub_agent_calls");

        if !pending_tools.is_empty() {
            tracing::warn!(
                unresolved_calls = ?*pending_tools,
                "Stream closed with unresolved tool calls"
            );
        }
        if !pending_agents.is_empty() {
            tracing::warn!(
                unresolved_agents = ?*pending_agents,
                "Stream closed with unresolved sub-agent calls"
            );
        }

        self.inner.close();
    }

    fn is_open(&self) -> bool {
        self.inner.is_open()
    }
}

// =============================================================================
// TeeEventSink — sub-agent event scoping combinator
// =============================================================================

/// An EventSink combinator that forwards selected events to a parent sink
/// with agent-name scoping. All events go to the primary sink; a subset
/// (tool progress, data-flow-ui, approval events) is also forwarded to
/// the parent so a sub-agent's approval gate is visible to (and resolvable
/// by) the parent host's event stream.
///
/// This mirrors the Python `TeeEventSink` pattern for sub-agent transparency.
///
/// # Laws
///
/// - **L1 (Primary guarantee)**: Every event reaches the primary sink.
/// - **L2 (Selective forward)**: `ToolProgress`, `DataFlowUI`,
///   `ApprovalRequired`, `ApprovalDecision`, and `PlanStatusChange`
///   events reach the parent sink. Other variants stay primary-only.
/// - **L3 (Scoping)**: Forwarded `ToolProgress` events have `tool_name` prefixed
///   with the agent name (e.g., `"planner/draft_plan"`). Approval and
///   plan-status-change events are forwarded **unscoped** — the host needs
///   the original `ApprovalId` to call `respond_to_approval`, and remapping
///   IDs across the sub-agent boundary would break that correlation.
/// - **L4 (Non-blocking)**: Parent forward failures don't affect the primary sink.
pub struct TeeEventSink {
    primary: Arc<dyn EventSink>,
    parent: Arc<dyn EventSink>,
    agent_name: String,
}

impl TeeEventSink {
    /// Create a new TeeEventSink.
    ///
    /// - `primary`: The sub-agent's own sink (receives all events).
    /// - `parent`: The parent orchestrator's sink (receives scoped subset).
    /// - `agent_name`: The name used to prefix tool names (e.g., "planner").
    pub fn new(
        primary: Arc<dyn EventSink>,
        parent: Arc<dyn EventSink>,
        agent_name: impl Into<String>,
    ) -> Self {
        Self {
            primary,
            parent,
            agent_name: agent_name.into(),
        }
    }
}

impl EventSink for TeeEventSink {
    fn emit(&self, part: StreamPart) -> bool {
        let ok = self.primary.emit(part.clone());

        match part {
            StreamPart::ToolProgress(mut data) => {
                data.tool_name = format!("{}/{}", self.agent_name, data.tool_name);
                if self.parent.is_open() {
                    self.parent.emit(StreamPart::ToolProgress(data));
                }
            }
            StreamPart::DataFlowUI { .. } => {
                if self.parent.is_open() {
                    self.parent.emit(part);
                }
            }
            // Approval events forward unscoped so the host's
            // `respond_to_approval(id, decision)` resolves the same
            // `ApprovalId` the sub-agent's gate is awaiting on.
            StreamPart::ApprovalRequired { .. }
            | StreamPart::ApprovalDecision { .. }
            | StreamPart::PlanStatusChange { .. } => {
                if self.parent.is_open() {
                    self.parent.emit(part);
                }
            }
            _ => {}
        }

        ok
    }

    fn close(&self) {
        self.primary.close();
    }

    fn is_open(&self) -> bool {
        self.primary.is_open()
    }
}

// =============================================================================
// EventSource — read-side dual of EventSink
// =============================================================================

/// Read-side dual of [`EventSink`] for dynamic event subscription.
///
/// Enables late-binding of observers (e.g., attach a debugger mid-eval).
///
/// # Laws
///
/// - **L1 (Non-blocking subscribe)**: `subscribe()` returns immediately
/// - **L2 (Broadcast, best-effort)**: All subscribers receive the same events
///   *within buffer capacity*. A slow subscriber that falls behind the buffer
///   may miss the oldest events (lossy broadcast). Implementations must
///   preserve this guarantee for subscribers that keep up with the emit rate.
/// - **L3 (Order preservation)**: Events arrive in emit order per subscriber
///   (FIFO within the events that subscriber receives)
pub trait EventSource: Send + Sync {
    /// Subscribe to events. Returns a receiver that yields stream parts.
    ///
    /// The receiver is bounded; slow consumers may miss events (lossy broadcast).
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<StreamPart>;

    /// Current number of active subscribers.
    fn subscriber_count(&self) -> usize;
}

/// Combined sink + source backed by a `tokio::sync::broadcast` channel.
///
/// Implements both [`EventSink`] (write) and [`EventSource`] (read).
pub struct BroadcastEventChannel {
    sender: tokio::sync::broadcast::Sender<StreamPart>,
    open: std::sync::atomic::AtomicBool,
}

impl BroadcastEventChannel {
    /// Create a new broadcast channel with the given capacity.
    ///
    /// When the buffer is full, the oldest event is dropped (lossy broadcast).
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self {
            sender,
            open: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl EventSink for BroadcastEventChannel {
    fn emit(&self, part: StreamPart) -> bool {
        if !self.is_open() {
            return false;
        }
        // send returns Err only if there are no receivers, which is fine
        let _ = self.sender.send(part);
        true
    }

    fn close(&self) {
        self.open.store(false, std::sync::atomic::Ordering::Release);
    }

    fn is_open(&self) -> bool {
        self.open.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl EventSource for BroadcastEventChannel {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<StreamPart> {
        self.sender.subscribe()
    }

    fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Test helper: A simple in-memory sink for testing
    #[derive(Default)]
    struct TestSink {
        events: std::sync::Mutex<Vec<StreamPart>>,
        open: AtomicBool,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
                open: AtomicBool::new(true),
            }
        }
    }

    impl EventSink for TestSink {
        fn emit(&self, part: StreamPart) -> bool {
            if !self.is_open() {
                return false;
            }
            self.events.lock().unwrap().push(part);
            true
        }

        fn close(&self) {
            self.open.store(false, Ordering::SeqCst);
        }

        fn is_open(&self) -> bool {
            self.open.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn emit_stores_events() {
        let sink = TestSink::new();
        assert!(sink.emit(StreamPart::text("hello")));
        assert!(sink.emit(StreamPart::text("world")));

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn close_prevents_further_emits() {
        let sink = TestSink::new();
        assert!(sink.emit(StreamPart::text("before")));
        sink.close();

        assert!(!sink.emit(StreamPart::text("after")));
    }

    #[test]
    fn close_is_idempotent() {
        let sink = TestSink::new();
        sink.close();
        sink.close(); // Should not panic
        assert!(!sink.is_open());
    }

    #[test]
    fn extension_methods_work() {
        let sink = TestSink::new();
        assert!(sink.emit_text("hello"));
        assert!(sink.emit_error("oops"));
        assert!(sink.emit_tool_progress(
            "draft_plan",
            Some("call-1".into()),
            "Loading",
            0,
            3,
            None
        ));
        assert!(sink.emit_data_flow_ui("{\"dsl\":true}"));

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert!(
            matches!(&events[2], StreamPart::ToolProgress(data) if data.tool_name == "draft_plan")
        );
    }

    #[test]
    fn approval_extension_methods_emit_typed_variants() {
        use agent_fw_core::approval::ApprovalKind;
        use agent_fw_core::{ApprovalId, TenantId, ThreadId};

        let sink = TestSink::new();
        let req = ApprovalRequest {
            id: ApprovalId::new_unchecked("apr-1"),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: serde_json::json!({}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("acme"),
            thread_id: ThreadId::new_unchecked("th-1"),
            correlation_id: None,
        };
        assert!(sink.emit_approval_required(req));
        assert!(
            sink.emit_approval_decision(ApprovalDecision::approve(ApprovalId::new_unchecked(
                "apr-1"
            )))
        );
        assert!(sink.emit_plan_status_change("plan-1", "draft", "approved"));

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], StreamPart::ApprovalRequired { .. }));
        assert!(matches!(&events[1], StreamPart::ApprovalDecision { .. }));
        assert!(matches!(&events[2], StreamPart::PlanStatusChange { .. }));
    }

    #[test]
    fn can_use_as_dyn_trait() {
        let sink: Box<dyn EventSink> = Box::new(TestSink::new());
        assert!(sink.emit(StreamPart::text("dynamic dispatch")));
        sink.close();
        assert!(!sink.is_open());
    }

    // =========================================================================
    // ValidatingEventSink Tests
    // =========================================================================

    #[test]
    fn validating_sink_allows_matched_call_result() {
        let inner = Arc::new(TestSink::new());
        let sink = ValidatingEventSink::wrap(inner.clone());

        // Call then result — valid
        assert!(sink.emit(StreamPart::tool_call("id1", "test", serde_json::json!({}))));
        assert!(sink.emit(StreamPart::tool_result(
            "id1",
            "test",
            serde_json::json!({}),
            serde_json::json!("ok")
        )));

        sink.close(); // No warnings — all calls resolved

        // Verify events were forwarded
        let events = inner.events.lock().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn validating_sink_forwards_all_events() {
        let inner = Arc::new(TestSink::new());
        let sink = ValidatingEventSink::wrap(inner.clone());

        assert!(sink.emit(StreamPart::text("hello")));
        assert!(sink.emit(StreamPart::StepStart));
        assert!(sink.emit(StreamPart::error("oops")));

        let events = inner.events.lock().unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn validating_sink_tracks_sub_agent_calls() {
        let inner = Arc::new(TestSink::new());
        let sink = ValidatingEventSink::wrap(inner.clone());

        // Sub-agent call then result — valid
        assert!(sink.emit(StreamPart::sub_agent_call("planner", "inv-1")));
        assert!(sink.emit(StreamPart::sub_agent_result("planner", "inv-1")));

        sink.close(); // No warnings

        let events = inner.events.lock().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn validating_sink_warns_on_unresolved_close() {
        let inner = Arc::new(TestSink::new());
        let sink = ValidatingEventSink::wrap(inner.clone());

        // Emit a call with no matching result
        sink.emit(StreamPart::tool_call("id1", "test", serde_json::json!({})));
        sink.close(); // Logs warning about unresolved "id1"

        // Event was still forwarded
        let events = inner.events.lock().unwrap();
        assert_eq!(events.len(), 1);
    }

    // =========================================================================
    // BroadcastEventChannel Tests
    // =========================================================================

    #[test]
    fn broadcast_channel_emit_and_subscribe() {
        let channel = BroadcastEventChannel::new(16);
        let mut rx = channel.subscribe();

        assert!(channel.emit(StreamPart::text("hello")));
        assert!(channel.emit(StreamPart::text("world")));

        // Receiver should get both events
        let e1 = rx.try_recv().unwrap();
        let e2 = rx.try_recv().unwrap();
        assert!(matches!(e1, StreamPart::Text { .. }));
        assert!(matches!(e2, StreamPart::Text { .. }));
    }

    #[test]
    fn broadcast_channel_subscriber_count() {
        let channel = BroadcastEventChannel::new(16);
        assert_eq!(channel.subscriber_count(), 0);

        let _rx1 = channel.subscribe();
        assert_eq!(channel.subscriber_count(), 1);

        let _rx2 = channel.subscribe();
        assert_eq!(channel.subscriber_count(), 2);

        drop(_rx1);
        // subscriber_count may not update immediately, but conceptually decreases
    }

    #[test]
    fn broadcast_channel_close_stops_emit() {
        let channel = BroadcastEventChannel::new(16);
        assert!(channel.is_open());
        assert!(channel.emit(StreamPart::text("before")));

        channel.close();
        assert!(!channel.is_open());
        assert!(!channel.emit(StreamPart::text("after")));
    }

    #[test]
    fn broadcast_channel_late_subscriber() {
        let channel = BroadcastEventChannel::new(16);

        // Emit before any subscriber
        channel.emit(StreamPart::text("early"));

        // Late subscriber misses the early event
        let mut rx = channel.subscribe();
        channel.emit(StreamPart::text("late"));

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, StreamPart::Text { .. }));
    }

    #[test]
    fn validating_sink_delegates_is_open() {
        let inner = Arc::new(TestSink::new());
        let sink = ValidatingEventSink::wrap(inner.clone());

        assert!(sink.is_open());
        inner.close();
        assert!(!sink.is_open());
    }

    // =========================================================================
    // TeeEventSink Tests
    // =========================================================================

    #[test]
    fn tee_sink_all_events_reach_primary() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        tee.emit(StreamPart::text("hello"));
        tee.emit(StreamPart::tool_progress(
            "search",
            None,
            "Resolving",
            0,
            3,
            None,
        ));
        tee.emit(StreamPart::data_flow_ui("{\"dsl\":true}"));

        let primary_events = primary.events.lock().unwrap();
        assert_eq!(primary_events.len(), 3, "all events reach primary");
    }

    #[test]
    fn tee_sink_only_forwards_selected_types() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        tee.emit(StreamPart::text("hello")); // NOT forwarded
        tee.emit(StreamPart::tool_progress(
            "search",
            None,
            "Resolving",
            0,
            3,
            None,
        )); // forwarded
        tee.emit(StreamPart::data_flow_ui("{}")); // forwarded
        tee.emit(StreamPart::error("oops")); // NOT forwarded

        let parent_events = parent.events.lock().unwrap();
        assert_eq!(
            parent_events.len(),
            2,
            "only tool-progress and data-flow-ui forwarded"
        );
    }

    #[test]
    fn tee_sink_scopes_tool_progress_name() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        tee.emit(StreamPart::tool_progress(
            "draft_plan",
            Some("call-42".into()),
            "Building",
            0,
            2,
            None,
        ));

        let parent_events = parent.events.lock().unwrap();
        assert_eq!(parent_events.len(), 1);
        match &parent_events[0] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.tool_name, "planner/draft_plan");
                assert_eq!(data.tool_call_id.as_deref(), Some("call-42"));
            }
            other => panic!("Expected ToolProgress, got {:?}", other),
        }

        // Primary gets unscoped name
        let primary_events = primary.events.lock().unwrap();
        match &primary_events[0] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.tool_name, "draft_plan");
            }
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
    }

    #[test]
    fn tee_sink_data_flow_ui_forwarded_unscoped() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        tee.emit(StreamPart::data_flow_ui("{\"dsl\":\"test\"}"));

        let parent_events = parent.events.lock().unwrap();
        assert_eq!(parent_events.len(), 1);
        match &parent_events[0] {
            StreamPart::DataFlowUI { data } => {
                assert_eq!(data.dsl, "{\"dsl\":\"test\"}");
            }
            other => panic!("Expected DataFlowUI, got {:?}", other),
        }
    }

    #[test]
    fn tee_sink_close_closes_primary_only() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "agent");

        assert!(tee.is_open());
        tee.close();
        assert!(!tee.is_open());
        assert!(!primary.is_open());
        assert!(parent.is_open(), "parent should remain open");
    }

    #[test]
    fn tee_sink_skips_parent_forward_when_closed() {
        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "agent");

        parent.close(); // close parent before emitting

        tee.emit(StreamPart::tool_progress(
            "search", None, "Phase 1", 0, 1, None,
        ));

        // Primary still gets the event
        assert_eq!(primary.events.lock().unwrap().len(), 1);
        // Parent gets nothing (was closed)
        assert_eq!(parent.events.lock().unwrap().len(), 0);
    }

    // ─── Approval forwarding (pre-dispatch approval review fix) ────────────────────

    /// Sub-agent approval events must reach the parent host stream so it
    /// can render and resolve them. Forwarded **unscoped** — the
    /// `ApprovalId` is the resolution key.
    #[test]
    fn tee_sink_forwards_approval_events_unscoped() {
        use agent_fw_core::approval::{
            ApprovalDecision, ApprovalKind, ApprovalRequest, PlanStatusChange,
        };
        use agent_fw_core::{ApprovalId, TenantId, ThreadId};

        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        let req = ApprovalRequest {
            id: ApprovalId::new_unchecked("apr-xyz"),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: serde_json::json!({}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("acme"),
            thread_id: ThreadId::new_unchecked("th-1"),
            correlation_id: Some("tool_use_42".into()),
        };
        tee.emit(StreamPart::approval_required(req.clone()));
        tee.emit(StreamPart::approval_decision(ApprovalDecision::approve(
            ApprovalId::new_unchecked("apr-xyz"),
        )));
        tee.emit(StreamPart::PlanStatusChange {
            data: PlanStatusChange {
                plan_id: "plan-1".into(),
                from: "draft".into(),
                to: "pending_approval".into(),
            },
        });

        // Both sinks see all three events.
        assert_eq!(primary.events.lock().unwrap().len(), 3);
        let parent_events = parent.events.lock().unwrap();
        assert_eq!(parent_events.len(), 3);

        // The parent's ApprovalRequest must be byte-identical — same ApprovalId,
        // same target. No agent-name prefix was applied.
        match &parent_events[0] {
            StreamPart::ApprovalRequired { data } => {
                assert_eq!(data.id.as_str(), "apr-xyz");
                assert_eq!(data.target, "create_scenario");
                assert_eq!(data.correlation_id.as_deref(), Some("tool_use_42"));
            }
            other => panic!("expected ApprovalRequired, got {other:?}"),
        }
        match &parent_events[1] {
            StreamPart::ApprovalDecision { data } => {
                assert_eq!(data.id.as_str(), "apr-xyz");
            }
            other => panic!("expected ApprovalDecision, got {other:?}"),
        }
        match &parent_events[2] {
            StreamPart::PlanStatusChange { data } => {
                assert_eq!(data.plan_id, "plan-1");
                assert_eq!(data.from, "draft");
                assert_eq!(data.to, "pending_approval");
            }
            other => panic!("expected PlanStatusChange, got {other:?}"),
        }
    }

    /// Approval forwarding respects the parent's `is_open()` gate just
    /// like other forwarded variants do (no panic on closed parent).
    #[test]
    fn tee_sink_skips_approval_forward_when_parent_closed() {
        use agent_fw_core::approval::{ApprovalKind, ApprovalRequest};
        use agent_fw_core::{ApprovalId, TenantId, ThreadId};

        let primary = Arc::new(TestSink::new());
        let parent = Arc::new(TestSink::new());
        let tee = TeeEventSink::new(primary.clone(), parent.clone(), "planner");

        parent.close();
        tee.emit(StreamPart::approval_required(ApprovalRequest {
            id: ApprovalId::new_unchecked("apr-1"),
            kind: ApprovalKind::Tool,
            target: "t".into(),
            payload: serde_json::json!({}),
            glimpse: None,
            resource_id: TenantId::new_unchecked("acme"),
            thread_id: ThreadId::new_unchecked("th-1"),
            correlation_id: None,
        }));

        assert_eq!(primary.events.lock().unwrap().len(), 1);
        assert_eq!(parent.events.lock().unwrap().len(), 0);
    }
}
