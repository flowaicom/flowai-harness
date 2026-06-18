//! StreamBuilder + EventStream algebraic law test harnesses.
//!
//! # StreamBuilder Laws (Protocol Enforcement)
//!
//! - L1. Call Tracking: `emit_call(call)` ⟹ `call.id ∈ pending_calls`
//! - L2. Result Matching: `emit_result(result)` succeeds IFF `result.call_id ∈ pending_calls`
//! - L3. Completion: `finish()` succeeds IFF `pending_calls = ∅`
//! - L4. Causality: Valid streams have results only for emitted calls
//! - L5. Error Safety: Failed mutations leave the builder unchanged
//!
//! # EventStream Laws (Monoid)
//!
//! - M1. Identity: `EMPTY.concat(s) == s == s.concat(EMPTY)`
//! - M2. Associativity: `(a.concat(b)).concat(c) == a.concat(b.concat(c))`
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn stream_builder_satisfies_laws() {
//!     agent_fw_test::stream_builder_laws::test_all();
//! }
//! ```

use agent_fw_core::stream_builder::*;
use agent_fw_core::{FinishReason, StreamPart, TokenUsage};
use serde_json::json;

/// Run all deterministic StreamBuilder + EventStream laws.
pub fn test_all() {
    law_call_tracking();
    law_result_matching();
    law_completion();
    law_causality();
    law_error_safety();
    law_monoid_identity();
    law_monoid_associativity();
}

/// L1: emit_call adds the ID to pending_tool_calls.
pub fn law_call_tracking() {
    let call = ToolCall::new("c-1", "tool", json!({}));
    let mut builder = StreamBuilder::new();
    builder.emit_call(call).unwrap();
    assert!(
        builder.pending_tool_calls().contains("c-1"),
        "L1 violated: call ID not tracked after emit_call"
    );
}

/// L2: emit_result succeeds IFF call_id is pending.
pub fn law_result_matching() {
    let result = ToolResult::new("c-1", "tool", json!({}), json!("v"));

    // Without call: must fail
    let mut b1 = StreamBuilder::new();
    assert!(
        b1.emit_result(result.clone()).is_err(),
        "L2 violated: orphan result should fail"
    );

    // With call: must succeed
    let call = ToolCall::new("c-1", "tool", json!({}));
    let mut b2 = StreamBuilder::new();
    b2.emit_call(call).unwrap();
    assert!(
        b2.emit_result(result).is_ok(),
        "L2 violated: result with matching call should succeed"
    );
}

/// L3: finish succeeds IFF pending sets are empty.
pub fn law_completion() {
    let term = Termination::finish(FinishReason::Stop, TokenUsage::ZERO);

    // Empty: must succeed
    assert!(
        StreamBuilder::new().finish(term.clone()).is_ok(),
        "L3 violated: empty builder should finish"
    );

    // With pending tool call: must fail
    let call = ToolCall::new("c-1", "tool", json!({}));
    let mut b1 = StreamBuilder::new();
    b1.emit_call(call).unwrap();
    assert!(
        b1.finish(term.clone()).is_err(),
        "L3 violated: pending tool calls should prevent finish"
    );

    // With pending sub-agent call: must fail
    let sa = SubAgentCall::new("agent", "inv-1");
    let mut b2 = StreamBuilder::new();
    b2.emit_sub_agent_call(sa).unwrap();
    assert!(
        b2.finish(term).is_err(),
        "L3 violated: pending sub-agent calls should prevent finish"
    );
}

/// L4: Complete call/result cycle allows finish — causality preserved.
pub fn law_causality() {
    let call = ToolCall::new("c-1", "tool", json!({}));
    let result = ToolResult::new("c-1", "tool", json!({}), json!("done"));
    let term = Termination::finish(FinishReason::Stop, TokenUsage::ZERO);

    let mut builder = StreamBuilder::new();
    builder.emit_call(call).unwrap();
    builder.emit_result(result).unwrap();
    let stream = builder.finish(term).unwrap();

    // All events should be present: 1 call + 1 result
    assert_eq!(stream.events().len(), 2, "L4: expected 2 events");
    assert!(!stream.is_error());
}

/// L5: Failed mutations leave the builder unchanged.
pub fn law_error_safety() {
    let mut builder = StreamBuilder::new();
    builder.emit_text("before");
    let initial_count = builder.event_count();

    // Orphan result should fail but not corrupt state
    let _err = builder
        .emit_result(ToolResult::new("nope", "t", json!({}), json!(null)))
        .unwrap_err();
    assert_eq!(
        builder.event_count(),
        initial_count,
        "L5 violated: failed emit_result changed event count"
    );

    // Builder still usable after error
    builder.emit_text("after");
    assert_eq!(builder.event_count(), initial_count + 1);
}

/// M1: EMPTY.concat(s) == s == s.concat(EMPTY)
pub fn law_monoid_identity() {
    let s = EventStream::singleton(StreamPart::text("hello"));
    assert_eq!(
        EventStream::EMPTY.concat(s.clone()),
        s.clone(),
        "M1 violated: left identity"
    );
    assert_eq!(
        s.clone().concat(EventStream::EMPTY),
        s,
        "M1 violated: right identity"
    );
}

/// M2: (a.concat(b)).concat(c) == a.concat(b.concat(c))
pub fn law_monoid_associativity() {
    let a = EventStream::singleton(StreamPart::text("a"));
    let b = EventStream::singleton(StreamPart::text("b"));
    let c = EventStream::singleton(StreamPart::text("c"));

    let left = a.clone().concat(b.clone()).concat(c.clone());
    let right = a.concat(b.concat(c));
    assert_eq!(left, right, "M2 violated: associativity");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_laws_pass() {
        test_all();
    }
}
