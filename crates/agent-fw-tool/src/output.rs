//! Typed tool output with optional UI channels.
//!
//! `ToolOutput` provides a typed alternative to returning raw `serde_json::Value`
//! from tool handlers. Implementors declare which UI channels (approval DSL,
//! display summary) their output carries, and `to_tool_result()` handles
//! serialization + channel injection.
//!
//! # Backward Compatibility
//!
//! The blanket `impl ToolOutput for serde_json::Value` means existing handlers
//! that return `Value` continue to work unchanged.
//!
//! # Laws
//!
//! - **O1 (Content preservation)**: `to_tool_result()` produces JSON that
//!   includes all fields from `Serialize` plus any UI channels.
//! - **O2 (Channel injection)**: If `approval_dsl()` returns `Some(dsl)`,
//!   then `to_tool_result()["approvalDsl"] == dsl`.
//! - **O3 (Backward compat)**: `Value::to_tool_result()` returns the value unchanged.

use serde::Serialize;

/// Typed tool output with optional UI channels.
///
/// Implementors provide domain-typed output structs that serialize to JSON.
/// The `to_tool_result()` method injects UI channels (`approvalDsl`,
/// `displaySummary`) into the serialized JSON, keeping the handler code
/// focused on domain computation rather than JSON key manipulation.
///
/// # Example
///
/// ```ignore
/// #[derive(Serialize)]
/// #[serde(rename_all = "camelCase")]
/// struct MyToolResult {
///     plan_id: String,
///     summary: String,
///     #[serde(skip)]
///     card_dsl: String,
/// }
///
/// impl ToolOutput for MyToolResult {
///     fn approval_dsl(&self) -> Option<&str> { Some(&self.card_dsl) }
/// }
///
/// // In the handler:
/// let result = MyToolResult { ... };
/// Ok(result.to_tool_result()?)
/// ```
pub trait ToolOutput: Serialize {
    /// Approval DSL for frontend card rendering.
    fn approval_dsl(&self) -> Option<&str> {
        None
    }

    /// Display summary for chat UI.
    fn display_summary(&self) -> Option<&str> {
        None
    }

    /// Serialize this output to a JSON `Value`, injecting UI channels.
    ///
    /// The default implementation:
    /// 1. Serializes `self` via `serde_json::to_value()`
    /// 2. Injects `approvalDsl` if `approval_dsl()` returns `Some`
    /// 3. Injects `displaySummary` if `display_summary()` returns `Some`
    fn to_tool_result(&self) -> Result<serde_json::Value, serde_json::Error> {
        let mut value = serde_json::to_value(self)?;
        if let Some(dsl) = self.approval_dsl() {
            value["approvalDsl"] = serde_json::Value::String(dsl.to_string());
        }
        if let Some(summary) = self.display_summary() {
            value["displaySummary"] = serde_json::Value::String(summary.to_string());
        }
        Ok(value)
    }
}

/// Blanket implementation: raw `serde_json::Value` is a `ToolOutput` with no UI channels.
///
/// Existing handlers returning `Value` work unchanged. The `to_tool_result()`
/// returns the value as-is (no serialization round-trip, no channel injection).
impl ToolOutput for serde_json::Value {
    fn to_tool_result(&self) -> Result<serde_json::Value, serde_json::Error> {
        Ok(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Serialize)]
    struct PlainOutput {
        count: usize,
    }

    impl ToolOutput for PlainOutput {}

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RichOutput {
        plan_id: String,
        #[serde(skip)]
        card: String,
        #[serde(skip)]
        summary: String,
    }

    impl ToolOutput for RichOutput {
        fn approval_dsl(&self) -> Option<&str> {
            Some(&self.card)
        }
        fn display_summary(&self) -> Option<&str> {
            Some(&self.summary)
        }
    }

    // O1: Content preservation
    #[test]
    fn plain_output_preserves_content() {
        let output = PlainOutput { count: 42 };
        let result = output.to_tool_result().unwrap();
        assert_eq!(result["count"], 42);
        assert!(result.get("approvalDsl").is_none());
        assert!(result.get("displaySummary").is_none());
    }

    // O2: Channel injection
    #[test]
    fn rich_output_injects_channels() {
        let output = RichOutput {
            plan_id: "plan-123".into(),
            card: "<card>test</card>".into(),
            summary: "Applied 3 actions".into(),
        };
        let result = output.to_tool_result().unwrap();
        assert_eq!(result["planId"], "plan-123");
        assert_eq!(result["approvalDsl"], "<card>test</card>");
        assert_eq!(result["displaySummary"], "Applied 3 actions");
    }

    // O3: Backward compat — Value passthrough
    #[test]
    fn value_passthrough() {
        let value = json!({"planId": "plan-1", "approvalDsl": "existing"});
        let result = value.to_tool_result().unwrap();
        assert_eq!(result, value);
    }

    #[test]
    fn skip_fields_not_serialized() {
        let output = RichOutput {
            plan_id: "plan-456".into(),
            card: "dsl".into(),
            summary: "summary".into(),
        };
        let result = output.to_tool_result().unwrap();
        // card and summary are #[serde(skip)] — they appear only via channel injection
        assert!(result.get("card").is_none());
        assert!(result.get("summary").is_none());
        // But they ARE injected as approvalDsl / displaySummary
        assert_eq!(result["approvalDsl"], "dsl");
        assert_eq!(result["displaySummary"], "summary");
    }
}
