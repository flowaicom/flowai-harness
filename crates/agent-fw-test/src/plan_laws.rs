//! Plan state machine law test harnesses.
//!
//! Verifies that `Plan<A>` transition functions satisfy:
//!
//! - **P1 (Monotonicity)**: Status only moves forward through the DAG.
//! - **P2 (Terminality)**: Executed and Failed are absorbing states.
//! - **P3 (Valid transitions)**: Only the documented transitions succeed.
//! - **P4 (Invalid transitions rejected)**: All other transitions fail.
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn plan_laws() {
//!     agent_fw_test::plan_laws::test_all();
//! }
//! ```

use agent_fw_core::{PlanId, TenantId, UserId};
use agent_fw_plan::{
    create_plan, single_action, ExecutionResult, Plan, PlanError, PlanStatus, Rejected,
    TransitionError,
};

fn make_plan() -> Plan<String> {
    create_plan(
        PlanId::new_unchecked("law-test"),
        TenantId::new_unchecked("tenant"),
        single_action("test-action".to_string()),
    )
}

fn user() -> UserId {
    UserId::new_unchecked("test-user")
}

/// Run all plan transition laws.
pub fn test_all() {
    law_draft_to_approved();
    law_approved_to_executing();
    law_executing_to_executed();
    law_executing_to_failed();
    law_no_skip_draft_to_executing();
    law_no_skip_draft_to_executed();
    law_no_reverse_approved_to_draft();
    law_terminal_executed_absorbing();
    law_terminal_failed_absorbing();
    law_monotonicity_happy_path();
    law_status_is_terminal_consistency();
}

// ─── P3: Valid transitions succeed ────────────────────────────────────

fn law_draft_to_approved() {
    let plan = make_plan();
    assert_eq!(plan.status, PlanStatus::Draft);
    let plan = plan.approve(user()).unwrap();
    assert_eq!(plan.status, PlanStatus::Approved);
}

fn law_approved_to_executing() {
    let plan = make_plan().approve(user()).unwrap();
    let plan = plan.start().unwrap();
    assert_eq!(plan.status, PlanStatus::Executing);
}

fn law_executing_to_executed() {
    let plan = make_plan().approve(user()).unwrap().start().unwrap();
    let plan = plan.complete(ExecutionResult::default()).unwrap();
    assert_eq!(plan.status, PlanStatus::Executed);
}

fn law_executing_to_failed() {
    let plan = make_plan().approve(user()).unwrap().start().unwrap();
    let plan = plan.fail(PlanError::new("test")).unwrap();
    assert_eq!(plan.status, PlanStatus::Failed);
}

// ─── P4: Invalid transitions rejected ─────────────────────────────────

fn law_no_skip_draft_to_executing() {
    let plan = make_plan();
    let Rejected { error, .. } = plan.start().unwrap_err();
    assert!(matches!(error, TransitionError::InvalidState { .. }));
}

fn law_no_skip_draft_to_executed() {
    let plan = make_plan();
    let Rejected { error, .. } = plan.complete(ExecutionResult::default()).unwrap_err();
    assert!(matches!(error, TransitionError::InvalidState { .. }));
}

fn law_no_reverse_approved_to_draft() {
    // Approved plan cannot go back to Draft (no such transition exists)
    let plan = make_plan().approve(user()).unwrap();
    // Only start() is valid from Approved; approve() should fail
    let Rejected { error, .. } = plan.approve(user()).unwrap_err();
    assert!(matches!(error, TransitionError::InvalidState { .. }));
}

// ─── P2: Terminal states are absorbing ────────────────────────────────

fn law_terminal_executed_absorbing() {
    let plan = make_plan()
        .approve(user())
        .unwrap()
        .start()
        .unwrap()
        .complete(ExecutionResult::default())
        .unwrap();

    assert!(plan.is_terminal());
    // Rejected<A> returns the plan — no Clone needed
    let Rejected { plan, .. } = plan.approve(user()).unwrap_err();
    let Rejected { plan, .. } = plan.start().unwrap_err();
    let Rejected { plan, .. } = plan.complete(ExecutionResult::default()).unwrap_err();
    let Rejected { plan: _, .. } = plan.fail(PlanError::new("x")).unwrap_err();
}

fn law_terminal_failed_absorbing() {
    let plan = make_plan()
        .approve(user())
        .unwrap()
        .start()
        .unwrap()
        .fail(PlanError::new("x"))
        .unwrap();

    assert!(plan.is_terminal());
    let Rejected { plan, .. } = plan.approve(user()).unwrap_err();
    let Rejected { plan, .. } = plan.start().unwrap_err();
    let Rejected { plan, .. } = plan.complete(ExecutionResult::default()).unwrap_err();
    let Rejected { plan: _, .. } = plan.fail(PlanError::new("y")).unwrap_err();
}

// ─── P1: Monotonicity ─────────────────────────────────────────────────

fn law_monotonicity_happy_path() {
    let statuses: Vec<PlanStatus> = {
        let plan = make_plan();
        let s1 = plan.status;
        let plan = plan.approve(user()).unwrap();
        let s2 = plan.status;
        let plan = plan.start().unwrap();
        let s3 = plan.status;
        let plan = plan.complete(ExecutionResult::default()).unwrap();
        let s4 = plan.status;
        vec![s1, s2, s3, s4]
    };

    // Status only moves forward
    assert_eq!(
        statuses,
        vec![
            PlanStatus::Draft,
            PlanStatus::Approved,
            PlanStatus::Executing,
            PlanStatus::Executed,
        ]
    );
}

// ─── Consistency ──────────────────────────────────────────────────────

fn law_status_is_terminal_consistency() {
    for status in [
        PlanStatus::Draft,
        PlanStatus::Approved,
        PlanStatus::Executing,
        PlanStatus::Executed,
        PlanStatus::Failed,
    ] {
        let expected = matches!(status, PlanStatus::Executed | PlanStatus::Failed);
        assert_eq!(
            status.is_terminal(),
            expected,
            "is_terminal() inconsistent for {status:?}"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Hegel-based exhaustive verification
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod hegel_laws {
    use super::*;
    use hegel::generators;

    // ─── Why Clone is structurally required ─────────────────────────
    //
    // Plan<A> transitions consume `self` (move semantics) — `approve`,
    // `start`, `complete`, `fail` all take `self` by value and return
    // `Result<Plan<A>, TransitionError>`. Crucially, `TransitionError`
    // does NOT return the consumed plan on failure.
    //
    // This means: to test that a transition *fails* while still keeping
    // the plan alive for further assertions, we must clone before each
    // attempted transition (e.g. `plan.clone().approve(user())`).
    //
    // Plan<String> is cheap to clone (a few small heap allocations),
    // and the clone-before-test pattern enforces "no stale references"
    // — once a plan transitions, the old binding is consumed, preventing
    // accidental use of outdated state (make
    // illegal states unrepresentable at the type level).
    // ────────────────────────────────────────────────────────────────

    /// Ordinal for monotonicity checks (no Ord on PlanStatus by design).
    fn status_ordinal(s: PlanStatus) -> u8 {
        match s {
            PlanStatus::Draft => 0,
            PlanStatus::Approved => 1,
            PlanStatus::Executing => 2,
            PlanStatus::Executed => 3,
            PlanStatus::Failed => 3, // same terminal rank
        }
    }

    /// Arbitrary transition that can be applied to any plan.
    #[derive(Clone, Debug)]
    enum PlanTransition {
        Approve,
        Start,
        Complete,
        Fail,
    }

    fn apply_transition(
        plan: Plan<String>,
        t: &PlanTransition,
    ) -> Result<Plan<String>, Rejected<String>> {
        match t {
            PlanTransition::Approve => plan.approve(user()),
            PlanTransition::Start => plan.start(),
            PlanTransition::Complete => plan.complete(ExecutionResult::default()),
            PlanTransition::Fail => plan.fail(PlanError::new("proptest")),
        }
    }

    fn draw_transitions(tc: &hegel::TestCase) -> Vec<PlanTransition> {
        let all = vec![
            PlanTransition::Approve,
            PlanTransition::Start,
            PlanTransition::Complete,
            PlanTransition::Fail,
        ];
        let len: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(19));
        (0..len)
            .map(|_| tc.draw(generators::sampled_from(all.clone())))
            .collect()
    }

    /// L1 (Totality): Any sequence of transitions applied to a fresh plan never panics.
    #[hegel::test]
    fn hegel_totality(tc: hegel::TestCase) {
        let transitions = draw_transitions(&tc);
        let mut plan = make_plan();
        for t in &transitions {
            match apply_transition(plan, t) {
                Ok(next) => plan = next,
                Err(rejected) => plan = rejected.plan,
            }
        }
    }

    /// L2 (Monotonicity): Status ordinal never decreases through successful transitions.
    #[hegel::test]
    fn hegel_monotonicity(tc: hegel::TestCase) {
        let transitions = draw_transitions(&tc);
        tc.assume(!transitions.is_empty());
        let mut plan = make_plan();
        let mut prev_ordinal = status_ordinal(plan.status);
        for t in &transitions {
            match apply_transition(plan, t) {
                Ok(next) => {
                    let next_ordinal = status_ordinal(next.status);
                    assert!(
                        next_ordinal >= prev_ordinal,
                        "Status went backwards: ord {} -> {:?} (ord {})",
                        prev_ordinal,
                        next.status,
                        next_ordinal
                    );
                    prev_ordinal = next_ordinal;
                    plan = next;
                }
                Err(rejected) => plan = rejected.plan,
            }
        }
    }

    /// L3 (Terminality): Once in Executed or Failed, all further transitions return Err.
    #[hegel::test]
    fn hegel_terminality(tc: hegel::TestCase) {
        let prefix = draw_transitions(&tc);
        let suffix = draw_transitions(&tc);
        tc.assume(!suffix.is_empty());
        let mut plan = make_plan();
        for t in &prefix {
            match apply_transition(plan, t) {
                Ok(next) => plan = next,
                Err(rejected) => plan = rejected.plan,
            }
        }
        if plan.status.is_terminal() {
            for t in &suffix {
                let rejected = apply_transition(plan, t).unwrap_err();
                plan = rejected.plan;
            }
        }
    }

    /// L4 (Determinism): Same transition sequence from same initial state produces same final state.
    #[hegel::test]
    fn hegel_determinism(tc: hegel::TestCase) {
        let transitions = draw_transitions(&tc);
        let run = |transitions: &[PlanTransition]| -> PlanStatus {
            let mut plan = make_plan();
            for t in transitions {
                match apply_transition(plan, t) {
                    Ok(next) => plan = next,
                    Err(rejected) => plan = rejected.plan,
                }
            }
            plan.status
        };
        let s1 = run(&transitions);
        let s2 = run(&transitions);
        assert_eq!(s1, s2, "Non-deterministic plan state machine");
    }
}
