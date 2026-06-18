//! Zero-cost progress emitter for long-running tools.
//!
//! `ProgressEmitter` bridges tool-internal phase boundaries to the SSE stream
//! via `EventSink`. Tests use `ProgressEmitter::noop()` (zero-cost, no sink).
//!
//! # Laws
//!
//! - **Monotonicity** (structural): `advance()` auto-increments — the phase
//!   index can only go up. No caller can violate ordering.
//! - **Bounds**: calling `advance()` more than `total_phases` times panics
//!   (programmer error, caught in all builds).
//! - **Noop transparency**: `ProgressEmitter::noop().advance(...)` is a no-op.
//! - **Idempotency on UI state**: the frontend overwrites the label on each
//!   phase index, so duplicate indices are harmless.

use agent_fw_algebra::EventSink;
use agent_fw_core::StreamPart;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Sentinel value indicating no phase has been emitted yet.
const NO_PHASE: u8 = u8::MAX;

/// Sentinel value indicating total_phases is not yet set (lazy mode).
const LAZY_TOTAL: u8 = 0;

/// Emits `StreamPart::ToolProgress` events during tool execution.
///
/// Constructed per tool invocation. When `sink` is `None` (tests, non-interactive),
/// all methods are no-ops with zero allocation.
///
/// # Lazy Mode
///
/// Use [`ProgressEmitter::lazy()`] when the total number of phases is unknown at
/// construction time (e.g., constraint expansion where group count varies).
/// Call [`set_total_phases()`](ProgressEmitter::set_total_phases) before the first
/// `advance()`:
///
/// ```ignore
/// let progress = env.lazy_progress_tracker("draft_plan", Some(call_id));
/// progress.set_total_phases(groups.len() as u8 * 2);
/// for group in &groups {
///     progress.advance(&format!("Resolving {}", group.name), None);
///     progress.advance(&format!("Building {}", group.name), None);
/// }
/// ```
pub struct ProgressEmitter {
    sink: Option<Arc<dyn EventSink>>,
    tool_name: String,
    /// Tool call ID from rig-core, enables precise progress -> tool invocation correlation.
    tool_call_id: Option<String>,
    total_phases: AtomicU8,
    /// Tracks the last emitted phase index for auto-increment.
    /// `NO_PHASE` (u8::MAX) means no phase emitted yet.
    last_phase: AtomicU8,
}

impl ProgressEmitter {
    /// Create a no-op emitter (for tests and non-interactive contexts).
    ///
    /// All `advance()` calls become no-ops. Zero allocation.
    pub fn noop() -> Self {
        Self {
            sink: None,
            tool_name: String::new(),
            tool_call_id: None,
            total_phases: AtomicU8::new(LAZY_TOTAL),
            last_phase: AtomicU8::new(NO_PHASE),
        }
    }

    /// Create a live emitter that pushes `ToolProgress` events to the `EventSink`.
    ///
    /// `tool_call_id` enables precise correlation when the same tool runs concurrently.
    /// Pass `None` when the call ID is unavailable (e.g., legacy code paths).
    pub fn new(
        sink: Arc<dyn EventSink>,
        tool_name: impl Into<String>,
        tool_call_id: Option<String>,
        total_phases: u8,
    ) -> Self {
        Self {
            sink: Some(sink),
            tool_name: tool_name.into(),
            tool_call_id,
            total_phases: AtomicU8::new(total_phases),
            last_phase: AtomicU8::new(NO_PHASE),
        }
    }

    /// Create a lazy emitter where `total_phases` is set later.
    ///
    /// Must call [`set_total_phases()`](Self::set_total_phases) before the first
    /// `advance()`. Panics if `advance()` is called before total is set.
    ///
    /// Use this for dynamic phase counts (e.g., constraint expansion where
    /// the number of groups varies per invocation).
    pub fn lazy(
        sink: Arc<dyn EventSink>,
        tool_name: impl Into<String>,
        tool_call_id: Option<String>,
    ) -> Self {
        Self {
            sink: Some(sink),
            tool_name: tool_name.into(),
            tool_call_id,
            total_phases: AtomicU8::new(LAZY_TOTAL),
            last_phase: AtomicU8::new(NO_PHASE),
        }
    }

    /// Set total phases lazily (for dynamic phase counts).
    ///
    /// Must be called before the first `advance()`. Can be called again to
    /// update the total for subsequent events (e.g., if more groups are
    /// discovered mid-execution).
    pub fn set_total_phases(&self, total: u8) {
        self.total_phases.store(total, Ordering::Relaxed);
    }

    /// Emit the next phase transition (auto-incrementing).
    ///
    /// Monotonicity is structural — the counter only goes up.
    ///
    /// # Arguments
    ///
    /// * `label` — Human-readable phase label (e.g., "Resolving entities")
    /// * `milestone` — Optional structured data (e.g., `{"matched": 142}`)
    ///
    /// # Panics
    ///
    /// Panics if called more times than `total_phases`. This is a programmer
    /// error (declared N phases but advanced N+1 times).
    ///
    /// Also panics if total_phases was never set (lazy mode).
    pub fn advance(&self, label: &str, milestone: Option<serde_json::Value>) {
        if self.sink.is_none() {
            return;
        }

        let prev = self.last_phase.fetch_add(1, Ordering::Relaxed);
        let index = if prev == NO_PHASE { 0 } else { prev + 1 };

        let total = self.total_phases.load(Ordering::Relaxed);

        // Lazy mode check
        assert!(
            total > LAZY_TOTAL,
            "ProgressEmitter({}): total_phases not set. Call set_total_phases() before advance().",
            self.tool_name,
        );

        // Bounds check — make overflow unrepresentable
        assert!(
            index < total,
            "ProgressEmitter: phase {index} exceeds total_phases {}",
            total,
        );

        if let Some(ref sink) = self.sink {
            let part = StreamPart::tool_progress(
                &self.tool_name,
                self.tool_call_id.clone(),
                label,
                index,
                total,
                milestone,
            );
            let _ = sink.emit(part);
        }
    }

    /// Emit a phase transition with an explicit index.
    ///
    /// **Deprecated**: Use `advance()` instead, which auto-increments and
    /// enforces monotonicity structurally.
    #[deprecated(since = "0.2.0", note = "use advance() for structural monotonicity")]
    pub fn phase(&self, index: u8, label: &str, milestone: Option<serde_json::Value>) {
        // Verify caller-provided index is consistent with auto-increment
        let prev = self.last_phase.load(Ordering::Relaxed);
        if prev != NO_PHASE {
            assert!(
                index >= prev,
                "ProgressEmitter monotonicity violation: phase {} after phase {}",
                index,
                prev
            );
        }
        self.last_phase.store(index, Ordering::Relaxed);

        let total = self.total_phases.load(Ordering::Relaxed);
        if let Some(ref sink) = self.sink {
            let part = StreamPart::tool_progress(
                &self.tool_name,
                self.tool_call_id.clone(),
                label,
                index,
                total,
                milestone,
            );
            let _ = sink.emit(part);
        }
    }

    /// Whether a live sink is wired (false for noop emitters).
    ///
    /// Callers use this to decide whether to **move** card data into
    /// `emit_card` (avoiding a clone) or retain it for the fallback path.
    pub fn has_sink(&self) -> bool {
        self.sink.is_some()
    }

    /// Emit a pre-computed card + summary directly via EventSink.
    ///
    /// `card_fn` converts the DSL string into a `StreamPart`. This avoids
    /// coupling the framework to a specific card format (e.g., CommandCard DSL).
    ///
    /// **Consumes** `dsl` and `summary` — caller should only call this
    /// after checking `has_sink()` so the values can be moved, not cloned.
    pub fn emit_card_with<F>(&self, dsl: String, summary: Option<String>, card_fn: F)
    where
        F: FnOnce(String) -> StreamPart,
    {
        if let Some(ref sink) = self.sink {
            if let Some(summary) = summary {
                let _ = sink.emit(StreamPart::text(&summary));
            }
            let _ = sink.emit(card_fn(dsl));
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(deprecated)]
    use super::*;
    use agent_fw_core::StreamPart;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// Test sink that collects events.
    struct TestSink {
        events: Mutex<Vec<StreamPart>>,
        open: AtomicBool,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                open: AtomicBool::new(true),
            }
        }

        fn events(&self) -> Vec<StreamPart> {
            self.events.lock().unwrap().clone()
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
    fn noop_emits_nothing() {
        let emitter = ProgressEmitter::noop();
        emitter.advance("ignored", None);
        emitter.advance("still ignored", Some(serde_json::json!({"ok": true})));
    }

    #[test]
    fn advance_auto_increments() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink.clone(), "draft_plan", None, 4);

        emitter.advance("Detecting schema", None);
        emitter.advance("Resolving entities", None);
        emitter.advance("Storing plan", Some(serde_json::json!({"matched": 142})));
        emitter.advance("Generating approval", None);

        let events = sink.events();
        assert_eq!(events.len(), 4);

        // Verify auto-increment
        for (i, event) in events.iter().enumerate() {
            match event {
                StreamPart::ToolProgress(data) => {
                    assert_eq!(data.phase_index, i as u8);
                    assert_eq!(data.total_phases, 4);
                }
                other => panic!("Expected ToolProgress, got {:?}", other),
            }
        }

        // Verify milestone on third event
        match &events[2] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.label, "Storing plan");
                let milestone = data.milestone.as_ref().unwrap();
                assert_eq!(milestone["matched"], 142);
            }
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
    }

    #[test]
    #[should_panic(expected = "exceeds total_phases")]
    fn advance_beyond_total_panics() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink, "tool", None, 2);

        emitter.advance("Phase 0", None);
        emitter.advance("Phase 1", None);
        emitter.advance("Phase 2 — boom!", None); // exceeds total_phases=2
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_phase_still_works() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink.clone(), "draft_plan", None, 4);

        emitter.phase(0, "Detecting schema", None);
        emitter.phase(1, "Resolving entities", None);
        emitter.phase(2, "Storing plan", Some(serde_json::json!({"matched": 142})));
        emitter.phase(3, "Generating approval", None);

        let events = sink.events();
        assert_eq!(events.len(), 4);

        // Verify first event
        match &events[0] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.tool_name, "draft_plan");
                assert_eq!(data.label, "Detecting schema");
                assert_eq!(data.phase_index, 0);
                assert_eq!(data.total_phases, 4);
                assert!(data.milestone.is_none());
            }
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
    }

    #[test]
    #[allow(deprecated)]
    fn same_phase_index_is_allowed() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink.clone(), "draft_plan", None, 3);

        emitter.phase(1, "first", None);
        emitter.phase(1, "updated label", None);

        let events = sink.events();
        assert_eq!(events.len(), 2);
    }

    #[test]
    #[allow(deprecated)]
    #[should_panic(expected = "monotonicity violation")]
    fn decreasing_phase_index_panics() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink, "draft_plan", None, 4);

        emitter.phase(2, "later", None);
        emitter.phase(1, "earlier", None); // violation!
    }

    #[test]
    fn closed_sink_does_not_panic() {
        let sink = Arc::new(TestSink::new());
        sink.close();
        let emitter = ProgressEmitter::new(sink.clone(), "draft_plan", None, 4);

        emitter.advance("test", None);

        let events = sink.events();
        assert!(events.is_empty());
    }

    #[test]
    fn emit_card_with_closure() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::new(sink.clone(), "tool", None, 1);

        emitter.emit_card_with(
            r#"{"type":"card"}"#.into(),
            Some("Summary text".into()),
            |dsl| StreamPart::data_flow_ui(dsl),
        );

        let events = sink.events();
        assert_eq!(events.len(), 2);
        // First event: summary text
        match &events[0] {
            StreamPart::Text { text } => assert_eq!(text, "Summary text"),
            other => panic!("Expected Text, got {:?}", other),
        }
        // Second event: data flow UI
        match &events[1] {
            StreamPart::DataFlowUI { data } => {
                assert_eq!(data.dsl, r#"{"type":"card"}"#);
            }
            other => panic!("Expected DataFlowUI, got {:?}", other),
        }
    }

    #[test]
    fn emit_card_noop_emitter() {
        let emitter = ProgressEmitter::noop();
        // Should not panic
        emitter.emit_card_with("dsl".into(), Some("summary".into()), |dsl| {
            StreamPart::data_flow_ui(dsl)
        });
    }

    #[test]
    fn has_sink_returns_correct_value() {
        let noop = ProgressEmitter::noop();
        assert!(!noop.has_sink());

        let sink = Arc::new(TestSink::new());
        let live = ProgressEmitter::new(sink, "tool", None, 1);
        assert!(live.has_sink());
    }

    // =========================================================================
    // Lazy ProgressEmitter Tests
    // =========================================================================

    #[test]
    fn lazy_emitter_works_after_set_total() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::lazy(sink.clone(), "draft_plan", None);

        emitter.set_total_phases(3);
        emitter.advance("Phase 0", None);
        emitter.advance("Phase 1", None);
        emitter.advance("Phase 2", None);

        let events = sink.events();
        assert_eq!(events.len(), 3);

        // Verify all events have total_phases=3
        for event in &events {
            match event {
                StreamPart::ToolProgress(data) => {
                    assert_eq!(data.total_phases, 3);
                }
                other => panic!("Expected ToolProgress, got {:?}", other),
            }
        }
    }

    #[test]
    #[should_panic(expected = "total_phases not set")]
    fn lazy_emitter_panics_without_set_total() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::lazy(sink, "draft_plan", None);

        emitter.advance("Phase 0", None); // panics — total not set
    }

    #[test]
    fn lazy_emitter_can_update_total() {
        let sink = Arc::new(TestSink::new());
        let emitter = ProgressEmitter::lazy(sink.clone(), "draft_plan", None);

        // Start with 2 phases
        emitter.set_total_phases(2);
        emitter.advance("Phase 0", None);

        // Discover more work — increase to 4
        emitter.set_total_phases(4);
        emitter.advance("Phase 1", None);
        emitter.advance("Phase 2", None);
        emitter.advance("Phase 3", None);

        let events = sink.events();
        assert_eq!(events.len(), 4);

        // First event has total_phases=2, rest have total_phases=4
        match &events[0] {
            StreamPart::ToolProgress(data) => assert_eq!(data.total_phases, 2),
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
        match &events[1] {
            StreamPart::ToolProgress(data) => assert_eq!(data.total_phases, 4),
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
    }

    #[test]
    fn lazy_emitter_with_tool_call_id() {
        let sink = Arc::new(TestSink::new());
        let emitter =
            ProgressEmitter::lazy(sink.clone(), "draft_plan", Some("toolu_abc123".to_string()));

        emitter.set_total_phases(1);
        emitter.advance("Working", None);

        match &sink.events()[0] {
            StreamPart::ToolProgress(data) => {
                assert_eq!(data.tool_call_id.as_deref(), Some("toolu_abc123"));
            }
            other => panic!("Expected ToolProgress, got {:?}", other),
        }
    }
}
