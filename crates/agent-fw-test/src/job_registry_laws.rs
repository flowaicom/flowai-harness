//! JobRegistry algebraic law test harnesses.
//!
//! Verifies the **join-semilattice** properties of `JobPhase` documented
//! in `agent_fw_algebra::job_registry`.
//!
//! `join(a, b) = max(a, b)` under the total order
//! `Queued ≤ Running ≤ Completed ≤ Failed ≤ Cancelled`.
//!
//! # Semilattice Laws
//!
//! - **S1 (Idempotence)**: `p.join(p) == p` for all phases
//! - **S2 (Commutativity)**: `a.join(b) == b.join(a)` for all pairs
//! - **S3 (Associativity)**: `(a.join(b)).join(c) == a.join(b.join(c))` for all triples
//! - **S4 (Absorbing terminal)**: `t.join(nt) == t` for terminal `t`, non-terminal `nt`
//! - **S5 (Monotonicity)**: For `a <= b`, `a.join(b) == b`
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn job_phase_laws() {
//!     agent_fw_test::job_registry_laws::test_all();
//! }
//! ```

use agent_fw_algebra::job_registry::JobPhase;

/// All phases for exhaustive enumeration.
const ALL_PHASES: [JobPhase; 5] = [
    JobPhase::Queued,
    JobPhase::Running,
    JobPhase::Completed,
    JobPhase::Failed,
    JobPhase::Cancelled,
];

/// Non-terminal phases.
const NON_TERMINAL: [JobPhase; 2] = [JobPhase::Queued, JobPhase::Running];

/// Terminal phases.
const TERMINAL: [JobPhase; 3] = [JobPhase::Completed, JobPhase::Failed, JobPhase::Cancelled];

/// Run all JobPhase semilattice laws.
pub fn test_all() {
    law_idempotence();
    law_commutativity_exhaustive();
    law_associativity_exhaustive();
    law_absorbing_terminal();
    law_monotonicity();
    law_is_terminal_consistency();
}

// ─── S1: Idempotence ────────────────────────────────────────────────

/// `p.join(p) == p` for every phase.
fn law_idempotence() {
    for p in ALL_PHASES {
        assert_eq!(
            p.join(p),
            p,
            "S1 (Idempotence): {p:?}.join({p:?}) must equal {p:?}"
        );
    }
}

// ─── S2: Commutativity (exhaustive) ─────────────────────────────────

/// `a.join(b) == b.join(a)` for ALL pairs — true semilattice.
fn law_commutativity_exhaustive() {
    for a in ALL_PHASES {
        for b in ALL_PHASES {
            assert_eq!(
                a.join(b),
                b.join(a),
                "S2 (Commutativity): {a:?}.join({b:?}) must equal {b:?}.join({a:?})"
            );
        }
    }
}

// ─── S3: Associativity (exhaustive) ─────────────────────────────────

/// `(a.join(b)).join(c) == a.join(b.join(c))` for all triples.
///
/// Exhaustive over all 5^3 = 125 combinations.
fn law_associativity_exhaustive() {
    for a in ALL_PHASES {
        for b in ALL_PHASES {
            for c in ALL_PHASES {
                let left = a.join(b).join(c);
                let right = a.join(b.join(c));
                assert_eq!(
                    left, right,
                    "S3 (Associativity): ({a:?}.join({b:?})).join({c:?}) != {a:?}.join({b:?}.join({c:?}))"
                );
            }
        }
    }
}

// ─── S4: Absorbing terminal ─────────────────────────────────────────

/// Terminal joined with non-terminal yields the terminal.
fn law_absorbing_terminal() {
    for t in TERMINAL {
        for nt in NON_TERMINAL {
            assert_eq!(
                t.join(nt),
                t,
                "S4 (Absorbing): {t:?}.join({nt:?}) must stay {t:?}"
            );
            assert_eq!(
                nt.join(t),
                t,
                "S4 (Absorbing, commuted): {nt:?}.join({t:?}) must advance to {t:?}"
            );
        }
    }
}

// ─── S5: Monotonicity ───────────────────────────────────────────────

/// For `a <= b`, `a.join(b) == b` (join is max).
fn law_monotonicity() {
    for a in ALL_PHASES {
        for b in ALL_PHASES {
            if a <= b {
                assert_eq!(
                    a.join(b),
                    b,
                    "S5 (Monotonicity): {a:?} <= {b:?} so {a:?}.join({b:?}) must equal {b:?}"
                );
            }
        }
    }
}

// ─── Consistency ────────────────────────────────────────────────────

/// `is_terminal()` matches exactly the terminal variants.
fn law_is_terminal_consistency() {
    for p in NON_TERMINAL {
        assert!(!p.is_terminal(), "is_terminal() must be false for {p:?}");
    }
    for t in TERMINAL {
        assert!(t.is_terminal(), "is_terminal() must be true for {t:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_job_phase_semilattice_laws_hold() {
        test_all();
    }

    /// Verify the join truth table explicitly for documentation.
    #[test]
    fn join_truth_table() {
        // Non-terminal × Non-terminal: max
        assert_eq!(JobPhase::Queued.join(JobPhase::Queued), JobPhase::Queued);
        assert_eq!(JobPhase::Queued.join(JobPhase::Running), JobPhase::Running);
        assert_eq!(JobPhase::Running.join(JobPhase::Queued), JobPhase::Running);
        assert_eq!(JobPhase::Running.join(JobPhase::Running), JobPhase::Running);

        // Non-terminal × Terminal: terminal wins
        assert_eq!(
            JobPhase::Queued.join(JobPhase::Completed),
            JobPhase::Completed
        );
        assert_eq!(JobPhase::Running.join(JobPhase::Failed), JobPhase::Failed);

        // Terminal × Terminal: max (commutative)
        assert_eq!(JobPhase::Completed.join(JobPhase::Failed), JobPhase::Failed);
        assert_eq!(JobPhase::Failed.join(JobPhase::Completed), JobPhase::Failed);
        assert_eq!(
            JobPhase::Completed.join(JobPhase::Cancelled),
            JobPhase::Cancelled
        );
        assert_eq!(
            JobPhase::Cancelled.join(JobPhase::Running),
            JobPhase::Cancelled
        );
    }
}
