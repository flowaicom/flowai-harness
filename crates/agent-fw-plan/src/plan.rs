//! Plan state machine.
//!
//! A `Plan<A>` is a lifecycle-managed container for a sequence of actions
//! to be applied to some set of entities. The type parameter `A` represents
//! the domain-specific action type (e.g., allocation actions, campaign changes).
//!
//! # State Machine
//!
//! ```text
//!    Draft ──approve──▶ Approved ──start──▶ Executing ──┬─complete─▶ Executed
//!                                                       └───fail───▶ Failed
//! ```
//!
//! All transitions are validated at runtime. Invalid transitions return
//! `TransitionError`.
//!
//! # Properties
//!
//! - **P1 (Monotonicity)**: Status only moves forward through the DAG.
//! - **P2 (Terminality)**: `Executed` and `Failed` are absorbing states.
//! - **P3 (Timestamps)**: Each transition records its timestamp; timestamps
//!   are monotonically non-decreasing.
//! - **P4 (Audit trail)**: `approved_by` is set exactly once on `approve`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use agent_fw_core::{PlanId, TenantId, UserId};

use crate::action::ActionSeq;
use crate::context::PlanContext;

// ─── Status ───────────────────────────────────────────────────────────

/// Lifecycle status of a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Draft,
    Approved,
    Executing,
    Executed,
    Failed,
}

impl PlanStatus {
    /// Terminal states are absorbing — no further transitions are allowed.
    pub fn is_terminal(self) -> bool {
        matches!(self, PlanStatus::Executed | PlanStatus::Failed)
    }
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PlanStatus::Draft => "draft",
            PlanStatus::Approved => "approved",
            PlanStatus::Executing => "executing",
            PlanStatus::Executed => "executed",
            PlanStatus::Failed => "failed",
        };
        write!(f, "{s}")
    }
}

// ─── Outcome ──────────────────────────────────────────────────────────

/// Result of plan execution. Domain-agnostic container.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    /// Number of entities affected.
    pub entities_affected: usize,
    /// Human-readable summary of what happened.
    pub summary: Option<String>,
    /// Structured details (domain-specific, e.g., per-entity changes).
    pub details: Option<serde_json::Value>,
}

/// Structured error for plan failures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanError {
    pub message: String,
    pub code: Option<String>,
    pub details: Option<serde_json::Value>,
}

impl PlanError {
    /// Create a plan error with message only (code=None, details=None).
    ///
    /// For the full case, use a struct literal:
    /// ```ignore
    /// PlanError { message: "msg".into(), code: Some("CODE".into()), details: None }
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            details: None,
        }
    }
}

/// Terminal outcome of plan execution.
///
/// Represents the *presence* of an outcome. Absence is encoded as
/// `Option<TerminalOutcome>` — not as a `Pending` variant.
///
/// # P5 (Outcome consistency)
///
/// `outcome.is_some()` iff `status.is_terminal()`. Enforced by
/// construction: only `complete()` and `fail()` set `outcome` to `Some`,
/// and they also set terminal status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum TerminalOutcome {
    /// Plan executed successfully.
    Executed(ExecutionResult),
    /// Plan execution failed.
    Failed(PlanError),
}

// ─── Serde migration: read legacy PlanOutcome, write Option<TerminalOutcome> ─

/// Wire format for backward-compatible deserialization.
///
/// Reads the legacy `{"state":"pending"}` as `None`, and
/// `{"state":"executed",...}` / `{"state":"failed",...}` as `Some`.
/// New plans serialize as `Option<TerminalOutcome>` directly.
#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum OutcomeWire {
    Pending,
    Executed(ExecutionResult),
    Failed(PlanError),
}

pub(crate) fn deserialize_outcome<'de, D>(
    deserializer: D,
) -> Result<Option<TerminalOutcome>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // New format: `null` → None. Legacy format: `{"state":"pending"}` → None.
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(v) => {
            let wire: OutcomeWire = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
            Ok(match wire {
                OutcomeWire::Pending => None,
                OutcomeWire::Executed(r) => Some(TerminalOutcome::Executed(r)),
                OutcomeWire::Failed(e) => Some(TerminalOutcome::Failed(e)),
            })
        }
    }
}

// ─── Deprecated alias for downstream migration ──────────────────────

/// Deprecated: use `Option<TerminalOutcome>` instead.
///
/// This type alias exists only to ease migration. It will be removed
/// in a future release.
#[deprecated(note = "Use Option<TerminalOutcome> instead")]
pub type PlanOutcome = TerminalOutcome;

// ─── TransitionError ──────────────────────────────────────────────────

/// Error returned when a plan transition is invalid.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("invalid transition: expected {expected}, got {actual}")]
    InvalidState {
        expected: PlanStatus,
        actual: PlanStatus,
    },
    #[error("plan is in terminal state: {0}")]
    TerminalState(PlanStatus),
}

// ─── Rejected<A> ─────────────────────────────────────────────────────

/// A failed transition that returns the unconsumed plan.
///
/// When a plan transition fails, `Rejected<A>` carries both the original
/// plan (unmodified) and the transition error. This eliminates the need
/// for `Clone` workarounds in test code and makes the API honest about
/// what happens on failure: the plan is returned, not lost.
///
/// # Ergonomic conversions
///
/// - `From<Rejected<A>> for TransitionError` — discard plan, keep error
/// - `Rejected::into_parts()` — destructure into `(Plan<A>, TransitionError)`
#[derive(Clone, Debug)]
pub struct Rejected<A> {
    pub plan: Plan<A>,
    pub error: TransitionError,
}

impl<A> Rejected<A> {
    /// Destructure into plan and error.
    pub fn into_parts(self) -> (Plan<A>, TransitionError) {
        (self.plan, self.error)
    }
}

impl<A> From<Rejected<A>> for TransitionError {
    fn from(r: Rejected<A>) -> Self {
        r.error
    }
}

// ─── Plan<A> ──────────────────────────────────────────────────────────

/// A lifecycle-managed plan generic over action type `A`.
///
/// The plan carries:
/// - Identity: `id` + `owner` (tenant isolation)
/// - Actions: non-empty `ActionSeq<A>` defining what to do
/// - Context: opaque domain metadata (entity refs, scope, etc.)
/// - Lifecycle: status + timestamps + audit + outcome
///
/// State transitions consume `self` and return the new state, enforcing
/// that callers cannot hold a reference to a plan in a stale state.
///
/// # Design note — timestamp fields
///
/// The four `Option<DateTime<Utc>>` fields could be a single
/// `Vec<(PlanStatus, DateTime<Utc>)>` timeline. However, a Vec allows
/// duplicate entries and non-monotonic timestamps — trading one set of
/// invariant violations for another. The typestate alternative destroys
/// object safety and serde compatibility. The current encoding is
/// pragmatic: each field is O(1) accessible, and P1–P3 monotonicity
/// is enforced by the transition methods + verified by plan_laws.
/// Use [`timeline()`](Plan::timeline) for an ordered event view.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan<A> {
    pub id: PlanId,
    pub owner: TenantId,
    pub actions: ActionSeq<A>,
    pub status: PlanStatus,
    pub description: Option<String>,
    #[serde(default)]
    pub context: PlanContext,
    pub created_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub approved_by: Option<UserId>,
    #[serde(default, deserialize_with = "deserialize_outcome")]
    pub outcome: Option<TerminalOutcome>,
}

impl<A> Plan<A> {
    // ─── Queries ──────────────────────────────────────────────────

    pub fn can_approve(&self) -> bool {
        self.status == PlanStatus::Draft
    }

    pub fn can_start(&self) -> bool {
        self.status == PlanStatus::Approved
    }

    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    // ─── Transitions ──────────────────────────────────────────────
    //
    // All transitions consume `self` and return either the new state
    // or a `Rejected<A>` that carries the unmodified plan + error.
    // This eliminates the need for Clone workarounds in tests.

    /// Transition Draft → Approved.
    pub fn approve(mut self, by: UserId) -> Result<Self, Rejected<A>> {
        if self.status != PlanStatus::Draft {
            return Err(Rejected {
                error: TransitionError::InvalidState {
                    expected: PlanStatus::Draft,
                    actual: self.status,
                },
                plan: self,
            });
        }
        self.status = PlanStatus::Approved;
        self.approved_at = Some(Utc::now());
        self.approved_by = Some(by);
        Ok(self)
    }

    /// Transition Approved → Executing.
    pub fn start(mut self) -> Result<Self, Rejected<A>> {
        if self.status != PlanStatus::Approved {
            return Err(Rejected {
                error: TransitionError::InvalidState {
                    expected: PlanStatus::Approved,
                    actual: self.status,
                },
                plan: self,
            });
        }
        self.status = PlanStatus::Executing;
        self.started_at = Some(Utc::now());
        Ok(self)
    }

    /// Transition Executing → Executed.
    pub fn complete(mut self, result: ExecutionResult) -> Result<Self, Rejected<A>> {
        if self.status != PlanStatus::Executing {
            return Err(Rejected {
                error: TransitionError::InvalidState {
                    expected: PlanStatus::Executing,
                    actual: self.status,
                },
                plan: self,
            });
        }
        self.status = PlanStatus::Executed;
        self.completed_at = Some(Utc::now());
        self.outcome = Some(TerminalOutcome::Executed(result));
        Ok(self)
    }

    /// Transition Executing → Failed.
    pub fn fail(mut self, error: PlanError) -> Result<Self, Rejected<A>> {
        if self.status != PlanStatus::Executing {
            return Err(Rejected {
                error: TransitionError::InvalidState {
                    expected: PlanStatus::Executing,
                    actual: self.status,
                },
                plan: self,
            });
        }
        self.status = PlanStatus::Failed;
        self.failed_at = Some(Utc::now());
        self.outcome = Some(TerminalOutcome::Failed(error));
        Ok(self)
    }

    // ─── Views ───────────────────────────────────────────────────────

    /// View the plan's timestamp timeline as a sequence of events.
    ///
    /// Returns timestamps in state-machine order. Only includes
    /// timestamps that are set (i.e., states the plan has passed through).
    ///
    /// # Law
    ///
    /// - **TL1 (Monotonicity)**: `timeline().windows(2).all(|(a, b)| a.1 <= b.1)`
    pub fn timeline(&self) -> Vec<(PlanStatus, DateTime<Utc>)> {
        let mut events = vec![(PlanStatus::Draft, self.created_at)];
        if let Some(t) = self.approved_at {
            events.push((PlanStatus::Approved, t));
        }
        if let Some(t) = self.started_at {
            events.push((PlanStatus::Executing, t));
        }
        if let Some(t) = self.completed_at {
            events.push((PlanStatus::Executed, t));
        }
        if let Some(t) = self.failed_at {
            events.push((PlanStatus::Failed, t));
        }
        events
    }
}

// ─── Smart Constructors ───────────────────────────────────────────────

/// Create a new plan in Draft status.
pub fn create_plan<A>(id: PlanId, owner: TenantId, actions: ActionSeq<A>) -> Plan<A> {
    Plan {
        id,
        owner,
        actions,
        status: PlanStatus::Draft,
        description: None,
        context: PlanContext::new(),
        created_at: Utc::now(),
        approved_at: None,
        started_at: None,
        completed_at: None,
        failed_at: None,
        approved_by: None,
        outcome: None,
    }
}

/// Create a new plan with description and context.
pub fn create_plan_with_context<A>(
    id: PlanId,
    owner: TenantId,
    actions: ActionSeq<A>,
    description: String,
    context: PlanContext,
) -> Plan<A> {
    let mut plan = create_plan(id, owner, actions);
    plan.description = Some(description);
    plan.context = context;
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::single_action;

    fn test_plan() -> Plan<String> {
        create_plan(
            PlanId::new_unchecked("plan-1"),
            TenantId::new_unchecked("tenant-1"),
            single_action("increase_price_10%".to_string()),
        )
    }

    // ─── Status Tests ─────────────────────────────────────────────

    #[test]
    fn create_plan_starts_in_draft() {
        let plan = test_plan();
        assert_eq!(plan.status, PlanStatus::Draft);
        assert!(!plan.is_terminal());
        assert!(plan.can_approve());
        assert!(!plan.can_start());
    }

    #[test]
    fn status_is_terminal() {
        assert!(!PlanStatus::Draft.is_terminal());
        assert!(!PlanStatus::Approved.is_terminal());
        assert!(!PlanStatus::Executing.is_terminal());
        assert!(PlanStatus::Executed.is_terminal());
        assert!(PlanStatus::Failed.is_terminal());
    }

    #[test]
    fn status_display() {
        assert_eq!(PlanStatus::Draft.to_string(), "draft");
        assert_eq!(PlanStatus::Executing.to_string(), "executing");
    }

    // ─── Transition Tests ─────────────────────────────────────────

    #[test]
    fn approve_transitions_to_approved() {
        let plan = test_plan();
        let user = UserId::new_unchecked("user-1");
        let plan = plan.approve(user.clone()).unwrap();
        assert_eq!(plan.status, PlanStatus::Approved);
        assert!(plan.approved_at.is_some());
        assert_eq!(plan.approved_by, Some(user));
    }

    #[test]
    fn cannot_approve_non_draft() {
        let plan = test_plan();
        let user = UserId::new_unchecked("user-1");
        let plan = plan.approve(user.clone()).unwrap();
        let Rejected { error, .. } = plan.approve(user).unwrap_err();
        assert_eq!(
            error,
            TransitionError::InvalidState {
                expected: PlanStatus::Draft,
                actual: PlanStatus::Approved,
            }
        );
    }

    #[test]
    fn start_transitions_to_executing() {
        let plan = test_plan();
        let plan = plan.approve(UserId::new_unchecked("u")).unwrap();
        let plan = plan.start().unwrap();
        assert_eq!(plan.status, PlanStatus::Executing);
        assert!(plan.started_at.is_some());
    }

    #[test]
    fn cannot_start_non_approved() {
        let plan = test_plan();
        let Rejected { error, .. } = plan.start().unwrap_err();
        assert_eq!(
            error,
            TransitionError::InvalidState {
                expected: PlanStatus::Approved,
                actual: PlanStatus::Draft,
            }
        );
    }

    #[test]
    fn complete_transitions_to_executed() {
        let plan = test_plan();
        let plan = plan.approve(UserId::new_unchecked("u")).unwrap();
        let plan = plan.start().unwrap();
        let plan = plan
            .complete(ExecutionResult {
                entities_affected: 42,
                summary: Some("Done".into()),
                details: None,
            })
            .unwrap();
        assert_eq!(plan.status, PlanStatus::Executed);
        assert!(plan.is_terminal());
        assert!(plan.completed_at.is_some());
        assert!(matches!(plan.outcome, Some(TerminalOutcome::Executed(_))));
    }

    #[test]
    fn fail_transitions_to_failed() {
        let plan = test_plan();
        let plan = plan.approve(UserId::new_unchecked("u")).unwrap();
        let plan = plan.start().unwrap();
        let plan = plan
            .fail(PlanError {
                message: "out of memory".into(),
                code: Some("OOM".into()),
                details: None,
            })
            .unwrap();
        assert_eq!(plan.status, PlanStatus::Failed);
        assert!(plan.is_terminal());
        assert!(plan.failed_at.is_some());
        match &plan.outcome {
            Some(TerminalOutcome::Failed(e)) => {
                assert_eq!(e.message, "out of memory");
                assert_eq!(e.code.as_deref(), Some("OOM"));
            }
            _ => panic!("expected Failed outcome"),
        }
    }

    #[test]
    fn cannot_complete_non_executing() {
        let plan = test_plan();
        let Rejected { error, .. } = plan.complete(ExecutionResult::default()).unwrap_err();
        assert_eq!(
            error,
            TransitionError::InvalidState {
                expected: PlanStatus::Executing,
                actual: PlanStatus::Draft,
            }
        );
    }

    #[test]
    fn cannot_fail_non_executing() {
        let plan = test_plan();
        let Rejected { error, .. } = plan.fail(PlanError::new("x")).unwrap_err();
        assert_eq!(
            error,
            TransitionError::InvalidState {
                expected: PlanStatus::Executing,
                actual: PlanStatus::Draft,
            }
        );
    }

    // ─── Outcome Tests ────────────────────────────────────────────

    #[test]
    fn outcome_none_by_default() {
        let plan = test_plan();
        assert!(plan.outcome.is_none());
    }

    // ─── Context Tests ────────────────────────────────────────────

    #[test]
    fn create_with_context() {
        let mut ctx = PlanContext::new();
        ctx.set("entity_set_id", serde_json::json!("set-abc"));

        let plan = create_plan_with_context(
            PlanId::new_unchecked("p"),
            TenantId::new_unchecked("t"),
            single_action("action"),
            "test plan".into(),
            ctx,
        );
        assert_eq!(plan.description.as_deref(), Some("test plan"));
        assert_eq!(
            plan.context.get("entity_set_id"),
            Some(&serde_json::json!("set-abc"))
        );
    }

    // ─── Serde Tests ──────────────────────────────────────────────

    #[test]
    fn plan_serde_roundtrip() {
        let plan = test_plan();
        let json = serde_json::to_string(&plan).unwrap();
        let parsed: Plan<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, plan.id);
        assert_eq!(parsed.status, plan.status);
        assert_eq!(parsed.actions.to_vec(), plan.actions.to_vec());
    }

    #[test]
    fn terminal_outcome_serde_roundtrip() {
        let outcomes: Vec<Option<TerminalOutcome>> = vec![
            None,
            Some(TerminalOutcome::Executed(ExecutionResult {
                entities_affected: 10,
                summary: Some("ok".into()),
                details: None,
            })),
            Some(TerminalOutcome::Failed(PlanError {
                message: "bad".into(),
                code: Some("E01".into()),
                details: None,
            })),
        ];
        for outcome in &outcomes {
            let json = serde_json::to_string(outcome).unwrap();
            let parsed: Option<TerminalOutcome> = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, *outcome);
        }
    }

    #[test]
    fn plan_status_serde_roundtrip() {
        let statuses = [
            PlanStatus::Draft,
            PlanStatus::Approved,
            PlanStatus::Executing,
            PlanStatus::Executed,
            PlanStatus::Failed,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let parsed: PlanStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }

    // ─── Full Lifecycle Test ──────────────────────────────────────

    #[test]
    fn full_lifecycle_happy_path() {
        let plan = test_plan();
        assert_eq!(plan.status, PlanStatus::Draft);

        let plan = plan.approve(UserId::new_unchecked("admin")).unwrap();
        assert_eq!(plan.status, PlanStatus::Approved);

        let plan = plan.start().unwrap();
        assert_eq!(plan.status, PlanStatus::Executing);

        let plan = plan
            .complete(ExecutionResult {
                entities_affected: 100,
                summary: Some("Applied price changes".into()),
                details: None,
            })
            .unwrap();
        assert_eq!(plan.status, PlanStatus::Executed);
        assert!(plan.is_terminal());
    }

    #[test]
    fn full_lifecycle_failure_path() {
        let plan = test_plan();
        let plan = plan.approve(UserId::new_unchecked("admin")).unwrap();
        let plan = plan.start().unwrap();
        let plan = plan
            .fail(PlanError {
                message: "database timeout".into(),
                code: Some("TIMEOUT".into()),
                details: None,
            })
            .unwrap();
        assert_eq!(plan.status, PlanStatus::Failed);
        assert!(plan.is_terminal());
    }
}
