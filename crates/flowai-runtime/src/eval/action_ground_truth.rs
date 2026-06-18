//! Harness-owned action ground truth normalization.
//!
//! The framework only sees an opaque `GroundTruth` envelope. This module owns
//! the Flow AI harness interpretation for action-oriented eval payloads while
//! keeping the payload itself generic. Actions are identified by `action_type`
//! plus exact JSON `payload`; domain concepts such as products, scope, value,
//! and change type live inside that payload.
//!
//! Expected actions are declared in two source buckets:
//!
//! - `planned` — scored against business actions projected from the stored plan
//! - `executed` — scored against business actions resolved by `executePlan`
//!
//! Presence of a bucket signals intent to score that source. At least one bucket
//! must be non-empty when action ground truth is declared.

use agent_fw_eval::GroundTruth as FrameworkGroundTruth;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionPayloadMatchMode {
    /// Expected payload must be a deep subset of the actual payload.
    Subset,
    /// Expected payload must equal the actual payload exactly.
    Exact,
}

impl Default for ActionPayloadMatchMode {
    fn default() -> Self {
        Self::Exact
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub payload: JsonValue,
}

/// Normalized action ground truth, split by the source each bucket scores.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionGroundTruth {
    /// Expected actions scored against the stored plan.
    pub planned: Vec<ExpectedAction>,
    /// Expected actions scored against the execution result.
    pub executed: Vec<ExpectedAction>,
    /// How expected action payloads are matched against actual action payloads.
    pub payload_match: ActionPayloadMatchMode,
}

impl ActionGroundTruth {
    pub fn planned_actions(&self) -> &[ExpectedAction] {
        &self.planned
    }

    pub fn executed_actions(&self) -> &[ExpectedAction] {
        &self.executed
    }

    /// True when neither bucket declares an expected action.
    pub fn is_empty(&self) -> bool {
        self.planned.is_empty() && self.executed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroundTruthNormalizationError {
    #[error("unsupported structured ground truth payload (expected kind text or flat): {0}")]
    StructuredDecode(String),
    #[error("action ground truth must declare at least one of plannedActions or executedActions")]
    EmptyActionGroundTruth,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StructuredGroundTruthWire {
    #[serde(rename_all = "camelCase")]
    Text {
        #[allow(dead_code)]
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Flat {
        #[serde(default)]
        planned_actions: Vec<ExpectedAction>,
        #[serde(default)]
        executed_actions: Vec<ExpectedAction>,
        #[serde(default)]
        payload_match: ActionPayloadMatchMode,
    },
}

pub fn normalize_ground_truth(
    ground_truth: Option<&FrameworkGroundTruth>,
) -> Result<Option<ActionGroundTruth>, GroundTruthNormalizationError> {
    let Some(ground_truth) = ground_truth else {
        return Ok(None);
    };

    match ground_truth {
        FrameworkGroundTruth::Text { .. } => Ok(None),
        FrameworkGroundTruth::Structured { data } => normalize_structured_payload(data),
    }
}

/// Expected actions for the planned source, if action ground truth is present.
pub fn extract_planned_expected_actions(
    ground_truth: Option<&FrameworkGroundTruth>,
) -> Result<Option<Vec<ExpectedAction>>, GroundTruthNormalizationError> {
    Ok(normalize_ground_truth(ground_truth)?.map(|normalized| normalized.planned))
}

/// Expected actions for the executed source, if action ground truth is present.
pub fn extract_executed_expected_actions(
    ground_truth: Option<&FrameworkGroundTruth>,
) -> Result<Option<Vec<ExpectedAction>>, GroundTruthNormalizationError> {
    Ok(normalize_ground_truth(ground_truth)?.map(|normalized| normalized.executed))
}

fn normalize_structured_payload(
    data: &JsonValue,
) -> Result<Option<ActionGroundTruth>, GroundTruthNormalizationError> {
    if data.is_null() {
        return Ok(None);
    }

    let wire: StructuredGroundTruthWire = serde_json::from_value(data.clone())
        .map_err(|error| GroundTruthNormalizationError::StructuredDecode(error.to_string()))?;

    Ok(match wire {
        StructuredGroundTruthWire::Text { .. } => None,
        StructuredGroundTruthWire::Flat {
            planned_actions,
            executed_actions,
            payload_match,
        } => {
            let normalized = ActionGroundTruth {
                planned: planned_actions,
                executed: executed_actions,
                payload_match,
            };
            if normalized.is_empty() {
                return Err(GroundTruthNormalizationError::EmptyActionGroundTruth);
            }
            Some(normalized)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_executed_action_bucket() {
        let structured = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "executedActions": [
                {
                    "type": "send_email",
                    "payload": {
                        "template": "stockout_warning",
                        "recipient": "ops@example.com"
                    }
                }
            ]
        }));

        let normalized = normalize_ground_truth(Some(&structured))
            .expect("ground truth should normalize")
            .expect("ground truth should be present");
        assert!(normalized.planned.is_empty());
        assert_eq!(normalized.executed.len(), 1);
        assert_eq!(normalized.executed[0].action_type, "send_email");
        assert_eq!(
            normalized.executed[0].payload["template"],
            serde_json::json!("stockout_warning")
        );
        assert_eq!(normalized.payload_match, ActionPayloadMatchMode::Exact);
    }

    #[test]
    fn normalizes_both_action_buckets() {
        let structured = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "payloadMatch": "exact",
            "plannedActions": [
                { "type": "price_change", "payload": { "scopeId": "s1" } }
            ],
            "executedActions": [
                { "type": "price_change", "payload": { "productIds": ["p1"] } }
            ]
        }));

        let normalized = normalize_ground_truth(Some(&structured))
            .expect("ground truth should normalize")
            .expect("ground truth should be present");
        assert_eq!(normalized.planned.len(), 1);
        assert_eq!(normalized.planned[0].payload["scopeId"], "s1");
        assert_eq!(normalized.executed.len(), 1);
        assert_eq!(normalized.executed[0].payload["productIds"][0], "p1");
        assert_eq!(normalized.payload_match, ActionPayloadMatchMode::Exact);
    }

    #[test]
    fn empty_action_buckets_are_rejected() {
        let structured = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "flat"
        }));

        let error = normalize_ground_truth(Some(&structured))
            .expect_err("empty buckets should be rejected");
        assert!(matches!(
            error,
            GroundTruthNormalizationError::EmptyActionGroundTruth
        ));
    }

    #[test]
    fn legacy_expected_actions_key_is_unsupported() {
        // The legacy `expectedActions` key is no longer recognized; it is ignored
        // and the now-empty buckets are rejected.
        let structured = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "expectedActions": [
                { "type": "send_email", "payload": {} }
            ]
        }));

        let error = normalize_ground_truth(Some(&structured))
            .expect_err("legacy expectedActions key should not produce a scored ground truth");
        assert!(matches!(
            error,
            GroundTruthNormalizationError::EmptyActionGroundTruth
        ));
    }

    #[test]
    fn text_variants_are_vacuous() {
        let text = FrameworkGroundTruth::text("summary").expect("text should build");
        assert!(normalize_ground_truth(Some(&text))
            .expect("text should normalize")
            .is_none());

        let structured_text = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "text",
            "text": "summary"
        }));
        assert!(normalize_ground_truth(Some(&structured_text))
            .expect("structured text should normalize")
            .is_none());
    }

    #[test]
    fn unknown_structured_kind_is_rejected() {
        let unknown = FrameworkGroundTruth::structured(serde_json::json!({
            "kind": "domainSpecific",
            "payload": {}
        }));

        let error = normalize_ground_truth(Some(&unknown)).expect_err("kind should be rejected");
        assert!(error
            .to_string()
            .contains("unsupported structured ground truth payload"));
    }
}
