//! ToolOutput algebraic law test harness.
//!
//! Verifies that `ToolOutput` implementations satisfy:
//!
//! - **O1 (Content preservation)**: `to_tool_result()` JSON includes all Serialize fields
//! - **O2 (Channel injection)**: `approval_dsl() == Some(dsl)` implies `result["approvalDsl"] == dsl`
//! - **O3 (Backward compat)**: `serde_json::Value::to_tool_result()` is identity
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn tool_output_laws() {
//!     agent_fw_test::tool_output_laws::test_all();
//! }
//! ```

use agent_fw_tool::output::ToolOutput;
use serde::Serialize;

/// Run all tool output laws.
pub fn test_all() {
    law_content_preservation();
    law_channel_injection();
    law_backward_compat();
}

// ── Test output types ────────────────────────────────────────────────

#[derive(Serialize)]
struct PlainTestOutput {
    count: usize,
    label: String,
}

impl ToolOutput for PlainTestOutput {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RichTestOutput {
    plan_id: String,
    action_count: usize,
    #[serde(skip)]
    card_dsl: String,
    #[serde(skip)]
    display: String,
}

impl ToolOutput for RichTestOutput {
    fn approval_dsl(&self) -> Option<&str> {
        Some(&self.card_dsl)
    }
    fn display_summary(&self) -> Option<&str> {
        Some(&self.display)
    }
}

// ── Laws ─────────────────────────────────────────────────────────────

/// O1 (Content preservation): `to_tool_result()` includes all serializable fields.
pub fn law_content_preservation() {
    let output = PlainTestOutput {
        count: 42,
        label: "test".to_string(),
    };
    let result = output.to_tool_result().unwrap();

    assert_eq!(result["count"], 42, "O1: count field must be present");
    assert_eq!(result["label"], "test", "O1: label field must be present");
    // No channels should be injected for PlainTestOutput
    assert!(
        result.get("approvalDsl").is_none(),
        "O1: no approvalDsl for plain output"
    );
    assert!(
        result.get("displaySummary").is_none(),
        "O1: no displaySummary for plain output"
    );
}

/// O2 (Channel injection): UI channels appear in the result JSON.
pub fn law_channel_injection() {
    let output = RichTestOutput {
        plan_id: "plan-abc".to_string(),
        action_count: 3,
        card_dsl: "<card>approval</card>".to_string(),
        display: "Applied 3 price changes".to_string(),
    };
    let result = output.to_tool_result().unwrap();

    // Domain fields present
    assert_eq!(result["planId"], "plan-abc", "O2: planId must be present");
    assert_eq!(result["actionCount"], 3, "O2: actionCount must be present");

    // Channels injected
    assert_eq!(
        result["approvalDsl"], "<card>approval</card>",
        "O2: approvalDsl must match"
    );
    assert_eq!(
        result["displaySummary"], "Applied 3 price changes",
        "O2: displaySummary must match"
    );

    // Skipped fields not leaked
    assert!(
        result.get("card_dsl").is_none(),
        "O2: skipped field must not appear"
    );
    assert!(
        result.get("cardDsl").is_none(),
        "O2: skipped field must not appear (camelCase)"
    );
}

/// O3 (Backward compat): `Value::to_tool_result()` returns the value unchanged.
pub fn law_backward_compat() {
    let value = serde_json::json!({
        "planId": "plan-1",
        "approvalDsl": "existing-dsl",
        "extra": [1, 2, 3],
    });
    let result = value.to_tool_result().unwrap();
    assert_eq!(result, value, "O3: Value must pass through unchanged");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_laws() {
        test_all();
    }
}
