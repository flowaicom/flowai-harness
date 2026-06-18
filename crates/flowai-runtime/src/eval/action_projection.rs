//! Projection helpers for action-oriented eval payloads.
//!
//! Runtimes can surface resolved actions either directly in `output.extra` or
//! nested inside captured tool-call results. This module normalizes those
//! runtime-specific payloads into the generic harness action shape.

use agent_fw_eval::{CapturedToolCall, RawSampleOutput, SampleExecutorOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const RESOLVED_ACTIONS_KEY: &str = "resolvedActions";
const PLANNED_ACTIONS_KEY: &str = "plannedActions";
const STORE_PLAN_TOOL: &str = "storePlan";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub payload: JsonValue,
}

impl ResolvedAction {
    pub fn new(action_type: impl Into<String>, payload: JsonValue) -> Self {
        Self {
            action_type: action_type.into(),
            payload,
        }
    }
}

pub fn extract_resolved_actions(
    output: &RawSampleOutput,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    extract_resolved_actions_from_extra(output.extra.as_ref())
}

/// Extract planned actions a scorer should compare against, from
/// `extra.plannedActions` populated by the sample executor.
pub fn extract_planned_actions(
    output: &RawSampleOutput,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    let actions = output
        .extra
        .as_ref()
        .and_then(|value| value.get(PLANNED_ACTIONS_KEY))
        .map(|value| serde_json::from_value::<Vec<ResolvedAction>>(value.clone()))
        .transpose()?;
    Ok(actions.unwrap_or_default())
}

pub fn extract_resolved_actions_from_sample(
    output: &SampleExecutorOutput,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    if let Ok(actions) = extract_resolved_actions_from_extra(output.extra.as_ref()) {
        if !actions.is_empty() {
            return Ok(actions);
        }
    }

    Ok(project_from_captured_tool_calls(&output.captured_tool_calls).unwrap_or_default())
}

pub fn resolved_actions_extra(actions: &[ResolvedAction]) -> JsonValue {
    serde_json::json!({
        RESOLVED_ACTIONS_KEY: actions,
    })
}

pub fn project_from_captured_tool_calls(
    captured_tool_calls: &[CapturedToolCall],
) -> Option<Vec<ResolvedAction>> {
    captured_tool_calls.iter().rev().find_map(|call| {
        let result = call.result.as_ref()?;
        decode_resolved_actions_value(result)
    })
}

fn extract_resolved_actions_from_extra(
    extra: Option<&JsonValue>,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    let actions = extra
        .and_then(|value| value.get(RESOLVED_ACTIONS_KEY))
        .map(decode_resolved_actions_value_lossy)
        .transpose()?;
    Ok(actions.unwrap_or_default())
}

fn decode_resolved_actions_value_lossy(
    value: &JsonValue,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    if let Some(actions) = decode_resolved_actions_value(value) {
        return Ok(actions);
    }
    serde_json::from_value(value.clone())
}

fn decode_resolved_actions_value(value: &JsonValue) -> Option<Vec<ResolvedAction>> {
    value
        .get(RESOLVED_ACTIONS_KEY)
        .and_then(|nested| serde_json::from_value(nested.clone()).ok())
        .or_else(|| serde_json::from_value(value.clone()).ok())
}

/// Build the `extra` payload that carries projected planned actions.
pub fn planned_actions_extra(actions: &[ResolvedAction]) -> JsonValue {
    serde_json::json!({
        PLANNED_ACTIONS_KEY: actions,
    })
}

/// Extract planned actions for a sample, preferring an explicit
/// `extra.plannedActions` and otherwise projecting from the stored plan emitted
/// by the planner's `storePlan` tool call.
pub fn extract_planned_actions_from_sample(
    output: &SampleExecutorOutput,
) -> Result<Vec<ResolvedAction>, serde_json::Error> {
    let from_extra = output
        .extra
        .as_ref()
        .and_then(|value| value.get(PLANNED_ACTIONS_KEY))
        .map(|value| serde_json::from_value::<Vec<ResolvedAction>>(value.clone()))
        .transpose()?;

    if let Some(actions) = from_extra {
        if !actions.is_empty() {
            return Ok(actions);
        }
    }

    Ok(project_planned_from_captured_tool_calls(&output.captured_tool_calls).unwrap_or_default())
}

/// Project planned actions from the most recent `storePlan` tool result.
pub fn project_planned_from_captured_tool_calls(
    captured_tool_calls: &[CapturedToolCall],
) -> Option<Vec<ResolvedAction>> {
    captured_tool_calls
        .iter()
        .rev()
        .filter(|call| call.tool == STORE_PLAN_TOOL)
        .find_map(|call| project_planned_from_stored_plan(call.result.as_ref()?))
}

/// Project planned actions out of a serialized stored `Plan` value.
///
/// Deserializes against the canonical `ActionSeq<HarnessAction>` wire shape so a
/// future change to either type breaks here rather than silently yielding no
/// planned actions.
fn project_planned_from_stored_plan(plan: &JsonValue) -> Option<Vec<ResolvedAction>> {
    let actions = plan.get("actions")?;
    let seq: agent_fw_plan::ActionSeq<crate::HarnessAction> =
        serde_json::from_value(actions.clone()).ok()?;
    Some(
        seq.iter()
            .map(|action| ResolvedAction::new(action.kind.clone(), action.payload.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_resolved_actions_from_output_extra() {
        let output = RawSampleOutput::with_extra(
            vec!["executePlan".into()],
            resolved_actions_extra(&[ResolvedAction::new(
                "send_email",
                serde_json::json!({ "template": "stockout_warning" }),
            )]),
        );

        let actions = extract_resolved_actions(&output).expect("actions should decode");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, "send_email");
        assert_eq!(actions[0].payload["template"], "stockout_warning");
    }

    #[test]
    fn projects_resolved_actions_from_captured_tool_calls() {
        let captured = vec![CapturedToolCall {
            tool: "executePlan".into(),
            tool_call_id: Some("tool-1".into()),
            args: serde_json::json!({ "ok": true }),
            result: Some(resolved_actions_extra(&[ResolvedAction::new(
                "create_ticket",
                serde_json::json!({ "priority": "high" }),
            )])),
        }];

        let projected =
            project_from_captured_tool_calls(&captured).expect("actions should project");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].payload["priority"], "high");
    }

    #[test]
    fn projects_resolved_actions_from_execute_plan_result_details_passthrough() {
        let captured = vec![CapturedToolCall {
            tool: "executePlan".into(),
            tool_call_id: Some("tool-1".into()),
            args: serde_json::json!({ "planId": "plan-1" }),
            result: Some(serde_json::json!({
                "entitiesAffected": 1,
                "summary": "recorded",
                "details": {
                    "resolvedActions": [{
                        "type": "price_change",
                        "payload": {
                            "changeType": "absolute",
                            "value": 9.99,
                            "productIds": ["p1"]
                        }
                    }]
                },
                "resolvedActions": [{
                    "type": "price_change",
                    "payload": {
                        "changeType": "absolute",
                        "value": 9.99,
                        "productIds": ["p1"]
                    }
                }]
            })),
        }];

        let projected =
            project_from_captured_tool_calls(&captured).expect("actions should project");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].action_type, "price_change");
        assert_eq!(
            projected[0].payload["productIds"],
            serde_json::json!(["p1"])
        );
    }

    #[test]
    fn projects_planned_actions_from_store_plan_result() {
        let captured = vec![CapturedToolCall {
            tool: "storePlan".into(),
            tool_call_id: Some("tool-1".into()),
            args: serde_json::json!({ "planId": "plan-1" }),
            result: Some(serde_json::json!({
                "id": "plan-1",
                "status": "draft",
                "actions": {
                    "head": {
                        "kind": "price_change",
                        "payload": { "scopeId": "s1", "value": 10.0 },
                        "references": []
                    },
                    "tail": [
                        {
                            "kind": "availability_change",
                            "payload": { "scopeId": "s2" }
                        }
                    ]
                }
            })),
        }];

        let planned =
            project_planned_from_captured_tool_calls(&captured).expect("planned actions project");
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].action_type, "price_change");
        assert_eq!(planned[0].payload["scopeId"], "s1");
        assert_eq!(planned[1].action_type, "availability_change");
    }

    #[test]
    fn planned_projection_ignores_non_store_plan_tool_calls() {
        let captured = vec![CapturedToolCall {
            tool: "executePlan".into(),
            tool_call_id: Some("tool-1".into()),
            args: serde_json::json!({ "planId": "plan-1" }),
            result: Some(resolved_actions_extra(&[ResolvedAction::new(
                "price_change",
                serde_json::json!({ "value": 10.0 }),
            )])),
        }];

        assert!(project_planned_from_captured_tool_calls(&captured).is_none());
    }

    #[test]
    fn extract_planned_actions_prefers_explicit_extra() {
        let output = SampleExecutorOutput {
            actual_trajectory: vec!["storePlan".into()],
            captured_tool_calls: vec![],
            duration_ms: 0,
            token_usage: agent_fw_eval::TokenUsageSummary::new(0, 0, 0, 0),
            error: None,
            thread_id: None,
            extra: Some(planned_actions_extra(&[ResolvedAction::new(
                "price_change",
                serde_json::json!({ "value": 10.0 }),
            )])),
            latency: None,
        };

        let planned = extract_planned_actions_from_sample(&output).expect("planned actions decode");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action_type, "price_change");
    }

    #[test]
    fn rejects_old_top_level_action_fields() {
        let raw = serde_json::json!([{
            "type": "price_change",
            "changeType": "absolute",
            "value": 7.5
        }]);

        let error = serde_json::from_value::<Vec<ResolvedAction>>(raw)
            .expect_err("old action shape should not decode");
        assert!(error.to_string().contains("unknown field"));
    }
}
