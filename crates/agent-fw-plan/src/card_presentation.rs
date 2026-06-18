//! Deferred card emission for tool results.
//!
//! When a live EventSink is wired, cards are emitted directly via SSE
//! and cleared from tool output (avoiding LLM context pollution).
//! When no sink exists (tests, non-interactive), cards stay in output
//! for the hook fallback path.
//!
//! # Laws
//!
//! - After `emit_via(live_sink)`: `approval_dsl = None`, `display_summary = None`, `emitted = true`
//! - After `emit_via(noop)`: `approval_dsl = Some`, `display_summary = Some`, `emitted = false`
//! - Idempotent: calling `emit_via` twice is safe (second call is no-op since fields are None)

use agent_fw_core::StreamPart;
use agent_fw_tool::ProgressEmitter;

/// Output fields for tool results after card emission.
#[derive(Clone, Debug, Default)]
pub struct CardOutputFields {
    /// DSL string for the approval card (None if emitted via sink).
    pub approval_dsl: Option<String>,
    /// Human-readable summary text (None if emitted via sink).
    pub display_summary: Option<String>,
    /// Whether the card was emitted directly via EventSink.
    pub emitted: bool,
}

/// Card emission strategy: ready → emit (if live sink) → output fields.
///
/// Encapsulates the decision of whether to emit via SSE or include in tool output.
pub struct CardPresentation {
    approval_dsl: Option<String>,
    display_summary: Option<String>,
    emitted: bool,
}

impl CardPresentation {
    /// Create with DSL and summary text ready for emission.
    pub fn ready(dsl: String, summary: String) -> Self {
        Self {
            approval_dsl: Some(dsl),
            display_summary: Some(summary),
            emitted: false,
        }
    }

    /// Create an empty presentation (no card to emit).
    pub fn none() -> Self {
        Self {
            approval_dsl: None,
            display_summary: None,
            emitted: false,
        }
    }

    /// Attempt emission via ProgressEmitter.
    ///
    /// If sink is live, emits Text + DataFlowUI events and clears fields.
    /// If no sink, retains fields for hook fallback.
    ///
    /// `card_fn`: converts DSL string into a StreamPart (domain-specific).
    /// This avoids coupling the framework to a specific card format.
    pub fn emit_via<F>(mut self, progress: &ProgressEmitter, card_fn: F) -> Self
    where
        F: FnOnce(String) -> StreamPart,
    {
        if self.approval_dsl.is_some() && progress.has_sink() {
            progress.emit_card_with(
                self.approval_dsl.take().expect("is_some() checked"),
                self.display_summary.take(),
                card_fn,
            );
            self.emitted = true;
        }
        self
    }

    /// Extract output fields for tool result serialization.
    pub fn into_output_fields(self) -> CardOutputFields {
        CardOutputFields {
            approval_dsl: self.approval_dsl,
            display_summary: self.display_summary,
            emitted: self.emitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::EventSink;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct CollectingSink {
        events: Mutex<Vec<StreamPart>>,
        open: AtomicBool,
    }

    impl CollectingSink {
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

    impl EventSink for CollectingSink {
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

    fn test_card_fn(dsl: String) -> StreamPart {
        StreamPart::data_flow_ui(dsl)
    }

    #[test]
    fn emit_via_live_sink_clears_fields() {
        let sink = Arc::new(CollectingSink::new());
        let progress = ProgressEmitter::new(sink.clone(), "test", None, 1);

        let card = CardPresentation::ready("dsl-content".into(), "Summary text".into());
        let card = card.emit_via(&progress, test_card_fn);
        let output = card.into_output_fields();

        assert!(output.approval_dsl.is_none());
        assert!(output.display_summary.is_none());
        assert!(output.emitted);

        let events = sink.events();
        assert_eq!(events.len(), 2); // text + data_flow_ui
    }

    #[test]
    fn emit_via_noop_retains_fields() {
        let progress = ProgressEmitter::noop();

        let card = CardPresentation::ready("dsl".into(), "summary".into());
        let card = card.emit_via(&progress, test_card_fn);
        let output = card.into_output_fields();

        assert_eq!(output.approval_dsl.as_deref(), Some("dsl"));
        assert_eq!(output.display_summary.as_deref(), Some("summary"));
        assert!(!output.emitted);
    }

    #[test]
    fn none_has_empty_fields() {
        let card = CardPresentation::none();
        let output = card.into_output_fields();

        assert!(output.approval_dsl.is_none());
        assert!(output.display_summary.is_none());
        assert!(!output.emitted);
    }

    #[test]
    fn emit_via_idempotent() {
        let sink = Arc::new(CollectingSink::new());
        let progress = ProgressEmitter::new(sink.clone(), "test", None, 1);

        let card = CardPresentation::ready("dsl".into(), "summary".into());
        let card = card.emit_via(&progress, test_card_fn);
        // Second emit_via — fields are already None, so nothing happens
        let card = card.emit_via(&progress, test_card_fn);
        let output = card.into_output_fields();

        assert!(output.emitted);
        let events = sink.events();
        assert_eq!(events.len(), 2); // Only emitted once
    }

    #[test]
    fn none_emit_via_is_noop() {
        let sink = Arc::new(CollectingSink::new());
        let progress = ProgressEmitter::new(sink.clone(), "test", None, 1);

        let card = CardPresentation::none();
        let card = card.emit_via(&progress, test_card_fn);
        let output = card.into_output_fields();

        assert!(!output.emitted);
        assert!(sink.events().is_empty());
    }
}
