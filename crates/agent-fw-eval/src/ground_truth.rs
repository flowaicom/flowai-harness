//! Ground truth types for evaluation.
//!
//! The framework owns only the generic ground-truth envelope. Domain-specific
//! contracts, including action payload semantics, belong in consuming crates and
//! should be carried through [`GroundTruth::Structured`].

use serde::{Deserialize, Deserializer, Serialize};

// =============================================================================
// NonEmptyText — serde-safe non-empty string
// =============================================================================

/// A non-empty, non-whitespace-only string.
///
/// Enforced at construction AND deserialization. The inner `String` is
/// private — callers use [`as_str()`](NonEmptyText::as_str) for read access.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NonEmptyText(String);

impl NonEmptyText {
    /// Construct from a string. Returns `None` if empty or whitespace-only.
    pub(crate) fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.trim().is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for NonEmptyText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        NonEmptyText::new(s)
            .ok_or_else(|| serde::de::Error::custom("text must not be empty or whitespace-only"))
    }
}

// =============================================================================
// GroundTruth Enum
// =============================================================================

/// Generic ground truth envelope.
///
/// # Invariants
///
/// - `Text` always has a non-empty `text` field after validation.
/// - `Structured` holds opaque domain-specific JSON.
///
/// # Vacuous truth policy
///
/// When ground truth is `None` or `Text` with no structured expectations,
/// scorers should yield perfect scores. This policy is encoded in
/// [`has_structured_expectations`](GroundTruth::has_structured_expectations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GroundTruth {
    /// Legacy free-form text.
    ///
    /// The `text` field is guaranteed non-empty at both construction time
    /// (via `GroundTruth::text()`) and deserialization (via `NonEmptyText`).
    #[serde(rename_all = "camelCase")]
    Text { text: NonEmptyText },

    /// Domain-specific structured ground truth (opaque to the framework).
    #[serde(rename_all = "camelCase")]
    Structured { data: serde_json::Value },
}

impl GroundTruth {
    /// Canonical variant name.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Structured { .. } => "structured",
        }
    }

    /// Create a `Text` ground truth from a string.
    ///
    /// Returns `None` if the text is empty or whitespace-only.
    pub fn text(s: impl Into<String>) -> Option<Self> {
        NonEmptyText::new(s).map(|text| Self::Text { text })
    }

    /// Create a `Structured` ground truth from JSON.
    pub fn structured(data: serde_json::Value) -> Self {
        Self::Structured { data }
    }

    /// Whether this ground truth has structured expectations.
    ///
    /// This is the single source of truth for the vacuous truth policy:
    /// when `false`, scorers should yield perfect scores.
    pub fn has_structured_expectations(&self) -> bool {
        match self {
            Self::Text { .. } => false,
            Self::Structured { data } => !data.is_null(),
        }
    }

    /// Extract the text content (if Text variant).
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    /// Extract the structured data (if Structured variant).
    pub fn as_structured(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Structured { data } => Some(data),
            _ => None,
        }
    }

    /// Convert from a legacy optional text field.
    pub fn from_legacy(text: Option<String>) -> Option<Self> {
        text.and_then(|t| Self::text(t))
    }

    /// Convert to framework format (for serialization).
    pub fn to_framework(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Convert from framework format (for deserialization).
    pub fn from_framework(value: serde_json::Value) -> Option<Self> {
        serde_json::from_value(value).ok()
    }

    /// Validate the ground truth structure.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        Ok(())
    }

    /// Count of expected actions.
    ///
    /// The framework no longer interprets action contracts, so this is always
    /// `0`. Runtime crates can expose domain-specific counts after decoding
    /// [`GroundTruth::Structured`].
    pub fn action_count(&self) -> usize {
        0
    }

    /// Count of explicit groups.
    ///
    /// The framework no longer interprets grouped action contracts, so this is
    /// always `0`.
    pub fn group_count(&self) -> usize {
        0
    }

    /// Human-readable summary for authoring tools and UIs.
    pub fn summary(&self) -> String {
        match self {
            Self::Text { text } => format!("Text ground truth ({} chars)", text.as_str().len()),
            Self::Structured { data } => {
                let key_count = data.as_object().map(|obj| obj.len()).unwrap_or(0);
                format!("Structured ground truth ({} top-level keys)", key_count)
            }
        }
    }

    /// Check if ground truth uses content-addressed products (fingerprints).
    ///
    /// The framework no longer owns product-specific ground-truth fields.
    pub fn has_fingerprints(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_rejects_empty() {
        assert!(GroundTruth::text("").is_none());
        assert!(GroundTruth::text("   ").is_none());
    }

    #[test]
    fn text_accepts_nonempty() {
        let gt = GroundTruth::text("expected output").unwrap();
        assert_eq!(gt.as_text(), Some("expected output"));
        assert!(!gt.has_structured_expectations());
    }

    #[test]
    fn structured_has_expectations() {
        let gt = GroundTruth::structured(serde_json::json!({
            "actions": [{"type": "price_change"}]
        }));
        assert!(gt.has_structured_expectations());
        assert!(gt.as_text().is_none());
        assert_eq!(gt.action_count(), 0);
    }

    #[test]
    fn structured_null_is_vacuous() {
        let gt = GroundTruth::structured(serde_json::Value::Null);
        assert!(!gt.has_structured_expectations());
    }

    #[test]
    fn serde_text_rejects_empty() {
        let json = r#"{"kind":"text","text":""}"#;
        let result = serde_json::from_str::<GroundTruth>(json);
        assert!(result.is_err(), "deserialization should reject empty text");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty or whitespace"),
            "error should mention non-empty constraint: {err}"
        );
    }

    #[test]
    fn serde_text_rejects_whitespace() {
        let json = r#"{"kind":"text","text":"   "}"#;
        let result = serde_json::from_str::<GroundTruth>(json);
        assert!(
            result.is_err(),
            "deserialization should reject whitespace-only text"
        );
    }

    #[test]
    fn validate_variants() {
        assert!(GroundTruth::text("hello").unwrap().validate().is_ok());
        assert!(GroundTruth::structured(serde_json::Value::Null)
            .validate()
            .is_ok());
        assert!(GroundTruth::structured(serde_json::json!({"key": "value"}))
            .validate()
            .is_ok());
    }

    #[test]
    fn rejects_removed_flat_and_multi_group_variants() {
        let flat =
            r#"{"kind":"flat","expectedActions":[{"type":"price_change","payload":{"value":1}}]}"#;
        let multi_group = r#"{"kind":"multiGroup","groups":[{"name":"g","expectedActions":[{"type":"price_change","payload":{"value":1}}]}]}"#;

        assert!(serde_json::from_str::<GroundTruth>(flat).is_err());
        assert!(serde_json::from_str::<GroundTruth>(multi_group).is_err());
    }

    #[test]
    fn to_from_framework_roundtrip() {
        let gt = GroundTruth::structured(serde_json::json!({"kind": "flat"}));
        let framework = gt.to_framework();
        let parsed = GroundTruth::from_framework(framework).unwrap();
        assert_eq!(gt, parsed);
    }

    #[test]
    fn python_text_deserializes() {
        let json = r#"{"kind": "text", "text": "Expected output"}"#;
        let gt: GroundTruth = serde_json::from_str(json).unwrap();
        assert_eq!(gt.as_text(), Some("Expected output"));
    }
}
