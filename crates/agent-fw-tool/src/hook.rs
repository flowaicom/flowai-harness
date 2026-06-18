//! Hook channel for bidirectional state between tools and the event bridge.
//!
//! Tools never interact with `HookChannel` directly. The hook writes `tool_call_id`
//! before tool execution and reads `pending_card` after tool results.

use std::sync::{Arc, Mutex};

/// Bidirectional channel between tool execution and the event bridge.
///
/// Grouping these fields isolates hook machinery from core tool capabilities.
#[derive(Clone)]
pub struct HookChannel {
    /// Current tool call ID, set by the hook before tool execution.
    /// Shared across clones so ProgressEmitter can read it during tool::call().
    tool_call_id: Arc<Mutex<Option<String>>>,
    /// Pending card data buffered by tools for post-tool-result emission.
    /// Written by tool::call(), read by hook::on_tool_result() to emit AFTER the tool_result event.
    pending_card: Arc<Mutex<Option<CommandCardPayload>>>,
}

/// Card + summary buffered during tool execution for post-tool-result emission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandCardPayload {
    pub display_summary: Option<String>,
    pub approval_dsl: Option<String>,
}

impl HookChannel {
    /// Create a new empty hook channel.
    pub fn new() -> Self {
        Self {
            tool_call_id: Arc::new(Mutex::new(None)),
            pending_card: Arc::new(Mutex::new(None)),
        }
    }

    /// Read the current tool call ID (set by the hook before tool execution).
    pub fn current_tool_call_id(&self) -> Option<String> {
        self.tool_call_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Set the current tool call ID (called by hook.on_tool_call).
    pub fn set_current_tool_call_id(&self, id: Option<String>) {
        *self.tool_call_id.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }

    /// Get a shared reference to the tool_call_id cell.
    pub fn tool_call_id_cell(&self) -> Arc<Mutex<Option<String>>> {
        Arc::clone(&self.tool_call_id)
    }

    /// Buffer a card + summary for post-tool-result emission by the hook.
    pub fn buffer_card(&self, display_summary: Option<String>, approval_dsl: Option<String>) {
        *self.pending_card.lock().unwrap_or_else(|e| e.into_inner()) = Some(CommandCardPayload {
            display_summary,
            approval_dsl,
        });
    }

    /// Take the pending card (if any). Returns None if no card was buffered.
    pub fn take_pending_card(&self) -> Option<CommandCardPayload> {
        self.pending_card
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Get a shared reference to the pending_card cell.
    pub fn pending_card_cell(&self) -> Arc<Mutex<Option<CommandCardPayload>>> {
        Arc::clone(&self.pending_card)
    }
}

impl Default for HookChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_no_tool_call_id() {
        let hook = HookChannel::new();
        assert!(hook.current_tool_call_id().is_none());
    }

    #[test]
    fn set_and_get_tool_call_id() {
        let hook = HookChannel::new();
        hook.set_current_tool_call_id(Some("call-123".to_string()));
        assert_eq!(hook.current_tool_call_id(), Some("call-123".to_string()));
    }

    #[test]
    fn clear_tool_call_id() {
        let hook = HookChannel::new();
        hook.set_current_tool_call_id(Some("call-123".to_string()));
        hook.set_current_tool_call_id(None);
        assert!(hook.current_tool_call_id().is_none());
    }

    #[test]
    fn buffer_and_take_card() {
        let hook = HookChannel::new();
        hook.buffer_card(Some("summary".to_string()), Some("dsl".to_string()));

        let card = hook.take_pending_card();
        assert!(card.is_some());

        let card = card.unwrap();
        assert_eq!(card.display_summary, Some("summary".to_string()));
        assert_eq!(card.approval_dsl, Some("dsl".to_string()));

        // Second take returns None (it was consumed)
        assert!(hook.take_pending_card().is_none());
    }

    #[test]
    fn clone_shares_state() {
        let hook = HookChannel::new();
        let hook2 = hook.clone();

        hook.set_current_tool_call_id(Some("shared".to_string()));
        assert_eq!(hook2.current_tool_call_id(), Some("shared".to_string()));
    }
}
