//! Pure data types for pre-dispatch approval (pre-dispatch approval).
//!
//! These types are framework-generic and free of async/IO concerns —
//! they live in `agent-fw-core` so the `PendingApprovalStore` algebra
//! (in `agent-fw-algebra`) can reference them without violating the
//! crate layering.
//!
//! The closure-bearing rule/policy primitives (`ApprovalRule`,
//! `ApprovalPolicy`, `ApprovalContext`) and the `ApprovalLayer` consumer
//! live in `agent-fw-agent::approval` because they couple to the tool
//! environment and the `ToolHandler` middleware stack.
//!
//! # Non-negotiable invariant
//!
//! A tool or executor action that requires approval **must not be invoked**
//! until a corresponding [`ApprovalDecision`] with
//! [`ApprovalOutcome::Approve`] has been recorded. This is the pre-dispatch approval
//! acceptance test: a counter-tool wrapped in the approval gate must show
//! `count == 0` while a decision is pending.
//!
//! # Algebraic laws (data layer)
//!
//! - **L1 (Schema purity)**: Constructing an [`ApprovalRequest`] or
//!   [`ApprovalDecision`] has no side effects.
//! - **L2 (Outcome dispatch totality)**: [`ApprovalOutcome::is_approve`]
//!   is the unique discriminator of "do we invoke inner?".
//! - **L3 (Serde roundtrip)**: every type roundtrips through serde without
//!   loss.

use serde::{Deserialize, Serialize};

use crate::id::{ApprovalId, TenantId, ThreadId};

// ─── ApprovalKind ────────────────────────────────────────────────────

/// Where on the agent's lifecycle approval is required.
///
/// `Tool` is checked inside `ApprovalLayer` (pre-`ToolHandler::handle`).
/// `Plan` is checked inside `GatedPlanExecutor` (pre-`PlanExecutor::execute`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Tool,
    Plan,
}

impl std::fmt::Display for ApprovalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ApprovalKind::Tool => "tool",
            ApprovalKind::Plan => "plan",
        };
        write!(f, "{s}")
    }
}

// ─── ApprovalRequest ─────────────────────────────────────────────────

/// A pending approval request emitted to the host via
/// `StreamPart::ApprovalRequired`.
///
/// The host renders this (e.g., as a card), waits for the user, and calls
/// `runtime.respond_to_approval(id, decision)` to resolve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub kind: ApprovalKind,
    /// Tool name (Tool kind) or plan id (Plan kind).
    pub target: String,
    /// Tool input args (Tool kind) or plan body (Plan kind).
    pub payload: serde_json::Value,
    /// Optional small preview of the underlying data (uses the existing
    /// `Glimpse` shape from `agent-fw-plan` when available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glimpse: Option<serde_json::Value>,
    /// Tenant scope. **Never** sourced from input.
    pub resource_id: TenantId,
    pub thread_id: ThreadId,
    /// Stable correlation back to the originating call:
    /// - **Tool kind**: the LLM's `tool_use_id` (`target` carries the tool *name*).
    /// - **Plan kind**: `plan_id.as_str()` (same string `target` carries, surfaced
    ///   here for symmetric host-side lookup).
    ///
    /// The primary `id` is a fresh UUID per attempt so a Rejected/Revised
    /// approval can be re-issued without colliding with the store's
    /// permanent resolved-set. Hosts use this field to correlate retries
    /// with the underlying conversation entity. Optional + serde-skipped
    /// when absent for backward-compatible deserialisation of older bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ─── ApprovalOutcome / ApprovalDecision ──────────────────────────────

/// What the host sends back.
///
/// Tool approvals collapse `Revise` to `Reject` (alpha scope) — only
/// plan approvals get the full three-way decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approve,
    Reject,
    /// Re-invoke the planner / re-shape the request with a partial.
    /// Meaningful for plans; tools treat as Reject.
    Revise {
        partial: serde_json::Value,
    },
}

impl ApprovalOutcome {
    /// Whether this outcome should result in invoking the gated action.
    /// Only `Approve` is treated as a green light.
    pub fn is_approve(&self) -> bool {
        matches!(self, ApprovalOutcome::Approve)
    }
}

/// The host-side resolution of an [`ApprovalRequest`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    pub id: ApprovalId,
    pub outcome: ApprovalOutcome,
    /// Human-readable reason (surfaces in `tool_result` content for
    /// the LLM and in the SSE `approval_decision` event for the host).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

impl ApprovalDecision {
    pub fn approve(id: ApprovalId) -> Self {
        Self {
            id,
            outcome: ApprovalOutcome::Approve,
            feedback: None,
        }
    }

    pub fn reject(id: ApprovalId, feedback: impl Into<String>) -> Self {
        Self {
            id,
            outcome: ApprovalOutcome::Reject,
            feedback: Some(feedback.into()),
        }
    }

    pub fn revise(id: ApprovalId, partial: serde_json::Value) -> Self {
        Self {
            id,
            outcome: ApprovalOutcome::Revise { partial },
            feedback: None,
        }
    }
}

// ─── PlanStatusChange ───────────────────────────────────────────────

/// Emitted on the SSE stream when a plan transitions, including the
/// optional `pending_approval` display alias that has no corresponding
/// `PlanStatus` variant (plan stays `Draft` while the gate is open).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStatusChange {
    pub plan_id: String,
    /// Previous canonical status string (`"draft"` etc.) or display alias.
    pub from: String,
    /// New canonical status string or display alias.
    pub to: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tid() -> TenantId {
        TenantId::new_unchecked("acme")
    }

    fn thid() -> ThreadId {
        ThreadId::new_unchecked("thread-1")
    }

    fn aid() -> ApprovalId {
        ApprovalId::new_unchecked("apr-1")
    }

    // ── L2: Outcome dispatch totality ───────────────────────────────

    #[test]
    fn only_approve_returns_true() {
        assert!(ApprovalOutcome::Approve.is_approve());
        assert!(!ApprovalOutcome::Reject.is_approve());
        assert!(!ApprovalOutcome::Revise { partial: json!({}) }.is_approve());
    }

    // ── Decision constructors ───────────────────────────────────────

    #[test]
    fn approve_constructor() {
        let d = ApprovalDecision::approve(aid());
        assert!(d.outcome.is_approve());
        assert!(d.feedback.is_none());
    }

    #[test]
    fn reject_constructor_carries_feedback() {
        let d = ApprovalDecision::reject(aid(), "unsafe");
        assert!(!d.outcome.is_approve());
        assert_eq!(d.feedback.as_deref(), Some("unsafe"));
    }

    #[test]
    fn revise_constructor_carries_partial() {
        let d = ApprovalDecision::revise(aid(), json!({"price": 9.99}));
        match d.outcome {
            ApprovalOutcome::Revise { partial } => assert_eq!(partial["price"], 9.99),
            _ => panic!("expected Revise"),
        }
    }

    // ── L3: Serde roundtrip ─────────────────────────────────────────

    #[test]
    fn approval_request_serde_roundtrip() {
        let req = ApprovalRequest {
            id: aid(),
            kind: ApprovalKind::Tool,
            target: "create_scenario".into(),
            payload: json!({"plan_ref": "p-1"}),
            glimpse: Some(json!({"summary": "5 products"})),
            resource_id: tid(),
            thread_id: thid(),
            correlation_id: Some("tool_use_xyz".into()),
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: ApprovalRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.id, req.id);
        assert_eq!(parsed.kind, req.kind);
        assert_eq!(parsed.target, req.target);
        assert_eq!(parsed.correlation_id.as_deref(), Some("tool_use_xyz"));
    }

    /// `correlation_id: None` is omitted on the wire and re-deserialises cleanly.
    #[test]
    fn approval_request_correlation_id_optional() {
        let req = ApprovalRequest {
            id: aid(),
            kind: ApprovalKind::Tool,
            target: "t".into(),
            payload: json!({}),
            glimpse: None,
            resource_id: tid(),
            thread_id: thid(),
            correlation_id: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(
            !s.contains("correlationId"),
            "None correlation must not appear on the wire: {s}"
        );
        let parsed: ApprovalRequest = serde_json::from_str(&s).unwrap();
        assert!(parsed.correlation_id.is_none());

        // Backward-compat: legacy bodies without the field still parse.
        let legacy = r#"{"id":"apr-1","kind":"tool","target":"t","payload":{},"resourceId":"acme","threadId":"thread-1"}"#;
        let legacy_req: ApprovalRequest = serde_json::from_str(legacy).unwrap();
        assert!(legacy_req.correlation_id.is_none());
    }

    #[test]
    fn approval_decision_serde_roundtrip() {
        let cases = vec![
            ApprovalDecision::approve(aid()),
            ApprovalDecision::reject(aid(), "no"),
            ApprovalDecision::revise(aid(), json!({"price": 9.99})),
        ];
        for d in cases {
            let s = serde_json::to_string(&d).unwrap();
            let parsed: ApprovalDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(parsed.id, d.id);
            assert_eq!(parsed.outcome.is_approve(), d.outcome.is_approve());
        }
    }

    #[test]
    fn plan_status_change_serde_roundtrip() {
        let event = PlanStatusChange {
            plan_id: "plan-1".into(),
            from: "draft".into(),
            to: "pending_approval".into(),
        };
        let s = serde_json::to_string(&event).unwrap();
        let parsed: PlanStatusChange = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn approval_kind_display() {
        assert_eq!(ApprovalKind::Tool.to_string(), "tool");
        assert_eq!(ApprovalKind::Plan.to_string(), "plan");
    }
}
