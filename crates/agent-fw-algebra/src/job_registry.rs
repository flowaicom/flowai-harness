//! JobRegistry algebra — tracking long-running background tasks.
//!
//! # Design
//!
//! `JobPhase` is a **join-semilattice** under the total ordering:
//!   `Queued ≤ Running ≤ Completed ≤ Failed ≤ Cancelled`
//!
//! Terminal states are **absorbing** — once a job reaches a terminal state,
//! `join` returns the maximum of the two terminals. This is commutative,
//! associative, and idempotent — a true semilattice.
//!
//! The `JobRegistry` trait provides the algebra; concrete implementations
//! (DashMap-backed, SQLite-backed, etc.) live in the interpreter layer.
//!
//! # Laws
//!
//! - **L1 (Register Totality)**: `register(id, kind)` always succeeds.
//!   If the job already exists and is non-terminal, returns `false` (no-op).
//!   If the job is terminal or absent, the entry is (re)created.
//! - **L2 (Phase Monotonicity)**: `advance_phase(id, p)` applies the
//!   join-semilattice operation. Phase can only advance, never regress.
//!   Terminal states are absorbing — `join(Completed, Failed) = Failed`.
//! - **L3 (Cleanup Safety)**: `cleanup_completed(max_age)` only removes
//!   entries whose phase is terminal.
//! - **L4 (Cancel Universality)**: `cancel(id)` transitions any non-terminal
//!   job to `Cancelled`, regardless of kind.
//! - **L5 (List Consistency)**: `list()` reflects the current registry state.

use serde::{Deserialize, Serialize};

// =============================================================================
// Pure Data Types
// =============================================================================

/// Pause state — boolean algebra under disjunction.
///
/// Forms a two-element lattice: `{Unpaused, Paused}` where `Paused` is ⊤
/// and `Unpaused` is ⊥ under `join` (OR).
///
/// Unlike `JobPhase`, pause is **reversible** (a group under XOR), not monotonic.
/// This is why it cannot be part of `JobPhase`'s join-semilattice — the product
/// type `JobState` preserves both structures independently.
///
/// # Laws
///
/// - **B1 (Join commutativity)**: `a.join(b) == b.join(a)`
/// - **B2 (Join idempotence)**: `a.join(a) == a`
/// - **B3 (Toggle involution)**: `a.toggle().toggle() == a`
/// - **B4 (Join absorption)**: `Paused.join(x) == Paused` for all `x`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseState {
    Unpaused = 0,
    Paused = 1,
}

impl PauseState {
    pub fn is_paused(self) -> bool {
        matches!(self, Self::Paused)
    }

    /// Boolean join (OR): `Paused` dominates.
    pub fn join(self, other: Self) -> Self {
        if self == Self::Paused || other == Self::Paused {
            Self::Paused
        } else {
            Self::Unpaused
        }
    }

    /// Boolean meet (AND): `Unpaused` dominates.
    pub fn meet(self, other: Self) -> Self {
        if self == Self::Unpaused || other == Self::Unpaused {
            Self::Unpaused
        } else {
            Self::Paused
        }
    }

    /// Toggle (group operation under XOR). Involution: `toggle(toggle(x)) == x`.
    pub fn toggle(self) -> Self {
        match self {
            Self::Unpaused => Self::Paused,
            Self::Paused => Self::Unpaused,
        }
    }
}

impl PartialOrd for PauseState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PauseState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

impl std::fmt::Display for PauseState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unpaused => write!(f, "unpaused"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

/// Product state: `JobPhase × PauseState`.
///
/// Captures the full observable state of a background job in a single value.
/// The phase component forms a join-semilattice (monotone). The pause component
/// forms a Boolean algebra (togglable). The product is ordered lexicographically:
/// phase takes priority, then pause state.
///
/// # Laws
///
/// - **P1 (Phase projection)**: `advance_phase(p).phase == old.phase.join(p)`
/// - **P2 (Pause orthogonality)**: `advance_phase(p).pause == old.pause`
/// - **P3 (Terminal absorption)**: `is_paused() == false` when `phase.is_terminal()`
/// - **P4 (Toggle involution)**: `toggle_pause().toggle_pause() == self` (non-terminal)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobState {
    pub phase: JobPhase,
    pub pause: PauseState,
}

impl JobState {
    pub fn new(phase: JobPhase, pause: PauseState) -> Self {
        Self { phase, pause }
    }

    pub fn queued() -> Self {
        Self {
            phase: JobPhase::Queued,
            pause: PauseState::Unpaused,
        }
    }

    pub fn running() -> Self {
        Self {
            phase: JobPhase::Running,
            pause: PauseState::Unpaused,
        }
    }

    /// Advance phase (semilattice join on phase dimension only).
    /// Pause state is preserved — phase advancement is orthogonal to pause.
    pub fn advance_phase(self, new_phase: JobPhase) -> Self {
        Self {
            phase: self.phase.join(new_phase),
            pause: self.pause,
        }
    }

    /// Toggle pause state. No-op when phase is terminal (P3: terminal absorption).
    pub fn toggle_pause(self) -> Self {
        if self.phase.is_terminal() {
            self
        } else {
            Self {
                phase: self.phase,
                pause: self.pause.toggle(),
            }
        }
    }

    /// Set pause state explicitly. No-op when phase is terminal (P3).
    pub fn set_paused(self, paused: bool) -> Self {
        if self.phase.is_terminal() {
            self
        } else {
            Self {
                phase: self.phase,
                pause: if paused {
                    PauseState::Paused
                } else {
                    PauseState::Unpaused
                },
            }
        }
    }

    pub fn is_terminal(self) -> bool {
        self.phase.is_terminal()
    }

    /// Paused only when non-terminal.
    pub fn is_paused(self) -> bool {
        !self.phase.is_terminal() && self.pause.is_paused()
    }

    /// Effectively paused: paused AND running.
    pub fn is_effectively_paused(self) -> bool {
        self.phase == JobPhase::Running && self.pause.is_paused()
    }
}

impl PartialOrd for JobState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JobState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.phase
            .cmp(&other.phase)
            .then(self.pause.cmp(&other.pause))
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_effectively_paused() {
            write!(f, "running (paused)")
        } else {
            write!(
                f,
                "{}",
                match self.phase {
                    JobPhase::Queued => "queued",
                    JobPhase::Running => "running",
                    JobPhase::Completed => "completed",
                    JobPhase::Failed => "failed",
                    JobPhase::Cancelled => "cancelled",
                }
            )
        }
    }
}

/// Kind of background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Eval,
    Ingestion,
    Profiling,
}

impl std::fmt::Display for JobKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobKind::Eval => write!(f, "eval"),
            JobKind::Ingestion => write!(f, "ingestion"),
            JobKind::Profiling => write!(f, "profiling"),
        }
    }
}

/// Phase of a background job (join-semilattice).
///
/// Total order: `Queued ≤ Running ≤ Completed ≤ Failed ≤ Cancelled`.
/// Terminal phases (`Completed`, `Failed`, `Cancelled`) are absorbing —
/// once reached, `join` can only advance within the terminal set, never
/// regress to a non-terminal phase.
///
/// The `PartialOrd`/`Ord` derive matches the semilattice ordering.
/// `join(a, b) = max(a, b)` — commutative, associative, idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
    Queued = 0,
    Running = 1,
    Completed = 2,
    Failed = 3,
    Cancelled = 4,
}

impl JobPhase {
    /// Whether this phase is terminal (absorbing element).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Join operation (semilattice): `max(self, other)`.
    ///
    /// Commutative, associative, idempotent — a true join-semilattice.
    /// Terminal states are absorbing: `join(Running, Completed) = Completed`,
    /// and terminal-vs-terminal resolves by the total order:
    /// `Completed < Failed < Cancelled`.
    ///
    /// ```text
    /// join(Completed, Failed)   = Failed     // max of two terminals
    /// join(Running, Completed)  = Completed  // advance to terminal
    /// join(Running, Queued)     = Running    // no regression
    /// ```
    pub fn join(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }
}

/// Serializable view of a job (for API responses).
///
/// `is_paused` is orthogonal to `phase` — a `Running` job can be paused.
/// Pause is a reversible toggle (group under XOR), not a semilattice element,
/// so it cannot be part of `JobPhase`. This field is volatile: after restart,
/// all jobs are unpaused (pause tokens don't survive process death, and
/// `fail_interrupted_jobs` makes it moot).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub kind: JobKind,
    pub phase: JobPhase,
    pub workspace_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub description: Option<String>,
    pub error_message: Option<String>,
    pub is_cancellable: bool,
    /// Whether the job is currently paused (volatile — not persisted).
    ///
    /// Orthogonal to `phase`: a `Running` job can be paused. Pause is a
    /// reversible toggle (`PauseToken`), not a monotonic semilattice
    /// transition. Default `false` for jobs that don't support pausing.
    #[serde(default)]
    pub is_paused: bool,
}

// =============================================================================
// Error
// =============================================================================

/// Error from job registry operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum JobRegistryError {
    #[error("job registry storage error: {0}")]
    Storage(String),
}

/// Error from cancel operations — explicit feedback instead of `bool`.
///
/// Distinguishes three failure modes:
/// - Structural: wrong ID (not found)
/// - Semantic: cancel on completed/failed/cancelled job (already terminal)
/// - Authorization: cross-workspace cancel attempt (wrong workspace)
///
/// This replaces the `bool` return from `cancel()` with a proper sum type,
/// making the failure reason observable to callers (logging, API responses).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CancelError {
    #[error("job {job_id} not found")]
    NotFound { job_id: String },

    #[error("job {job_id} already in terminal phase {phase:?}")]
    AlreadyTerminal { job_id: String, phase: JobPhase },

    #[error("job {job_id} belongs to a different workspace")]
    WrongWorkspace { job_id: String },
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_ordering() {
        assert!(JobPhase::Queued < JobPhase::Running);
        assert!(JobPhase::Running < JobPhase::Completed);
        assert!(JobPhase::Running < JobPhase::Failed);
        assert!(JobPhase::Running < JobPhase::Cancelled);
    }

    #[test]
    fn phase_join_semilattice() {
        // Commutativity (non-terminal)
        assert_eq!(
            JobPhase::Queued.join(JobPhase::Running),
            JobPhase::Running.join(JobPhase::Queued)
        );
        // Idempotence
        assert_eq!(JobPhase::Running.join(JobPhase::Running), JobPhase::Running);
        // Advance to terminal
        assert_eq!(
            JobPhase::Running.join(JobPhase::Completed),
            JobPhase::Completed
        );
    }

    #[test]
    fn phase_terminal_is_absorbing() {
        // Terminal × Terminal: max wins (true semilattice — commutative)
        assert_eq!(JobPhase::Completed.join(JobPhase::Failed), JobPhase::Failed);
        assert_eq!(
            JobPhase::Completed.join(JobPhase::Cancelled),
            JobPhase::Cancelled
        );
        assert_eq!(
            JobPhase::Failed.join(JobPhase::Cancelled),
            JobPhase::Cancelled
        );
        assert_eq!(
            JobPhase::Cancelled.join(JobPhase::Failed),
            JobPhase::Cancelled
        );
        // Terminal × Non-terminal: terminal absorbs
        assert_eq!(JobPhase::Failed.join(JobPhase::Running), JobPhase::Failed);
        assert_eq!(
            JobPhase::Cancelled.join(JobPhase::Queued),
            JobPhase::Cancelled
        );
    }

    #[test]
    fn phase_terminal_check() {
        assert!(!JobPhase::Queued.is_terminal());
        assert!(!JobPhase::Running.is_terminal());
        assert!(JobPhase::Completed.is_terminal());
        assert!(JobPhase::Failed.is_terminal());
        assert!(JobPhase::Cancelled.is_terminal());
    }

    #[test]
    fn job_kind_display() {
        assert_eq!(JobKind::Eval.to_string(), "eval");
        assert_eq!(JobKind::Ingestion.to_string(), "ingestion");
        assert_eq!(JobKind::Profiling.to_string(), "profiling");
    }

    // =========================================================================
    // PauseState tests (Boolean algebra)
    // =========================================================================

    /// B1: Join commutativity
    #[test]
    fn pause_state_join_commutative() {
        assert_eq!(
            PauseState::Unpaused.join(PauseState::Paused),
            PauseState::Paused.join(PauseState::Unpaused)
        );
    }

    /// B2: Join idempotence
    #[test]
    fn pause_state_join_idempotent() {
        assert_eq!(
            PauseState::Paused.join(PauseState::Paused),
            PauseState::Paused
        );
        assert_eq!(
            PauseState::Unpaused.join(PauseState::Unpaused),
            PauseState::Unpaused
        );
    }

    /// B3: Toggle involution
    #[test]
    fn pause_state_toggle_involution() {
        assert_eq!(PauseState::Unpaused.toggle().toggle(), PauseState::Unpaused);
        assert_eq!(PauseState::Paused.toggle().toggle(), PauseState::Paused);
    }

    /// B4: Paused is absorbing under join (⊤)
    #[test]
    fn pause_state_paused_is_top() {
        assert_eq!(
            PauseState::Paused.join(PauseState::Unpaused),
            PauseState::Paused
        );
        assert_eq!(
            PauseState::Paused.join(PauseState::Paused),
            PauseState::Paused
        );
    }

    /// Meet: Unpaused is absorbing under meet (⊥)
    #[test]
    fn pause_state_unpaused_is_bottom_under_meet() {
        assert_eq!(
            PauseState::Unpaused.meet(PauseState::Paused),
            PauseState::Unpaused
        );
        assert_eq!(
            PauseState::Paused.meet(PauseState::Paused),
            PauseState::Paused
        );
    }

    #[test]
    fn pause_state_display() {
        assert_eq!(PauseState::Unpaused.to_string(), "unpaused");
        assert_eq!(PauseState::Paused.to_string(), "paused");
    }

    // =========================================================================
    // JobState tests (Product lattice)
    // =========================================================================

    /// P1: Phase projection respects semilattice
    #[test]
    fn job_state_advance_phase_respects_semilattice() {
        let s = JobState::new(JobPhase::Running, PauseState::Paused);
        assert_eq!(
            s.advance_phase(JobPhase::Completed).phase,
            JobPhase::Completed
        );
        // Cannot regress
        assert_eq!(s.advance_phase(JobPhase::Queued).phase, JobPhase::Running);
    }

    /// P2: Phase advancement preserves pause state
    #[test]
    fn job_state_advance_preserves_pause() {
        let s = JobState::new(JobPhase::Running, PauseState::Paused);
        assert_eq!(
            s.advance_phase(JobPhase::Completed).pause,
            PauseState::Paused
        );
    }

    /// P3: Terminal absorption — is_paused returns false when terminal
    #[test]
    fn job_state_terminal_absorbs_pause() {
        let s = JobState::new(JobPhase::Completed, PauseState::Paused);
        assert!(!s.is_paused());
        assert!(s.toggle_pause() == s, "toggle on terminal is no-op");
        assert!(s.set_paused(true) == s, "set_paused on terminal is no-op");
    }

    /// P4: Toggle involution on non-terminal
    #[test]
    fn job_state_toggle_involution() {
        let s = JobState::running();
        assert_eq!(s.toggle_pause().toggle_pause(), s);
    }

    #[test]
    fn job_state_is_effectively_paused() {
        assert!(JobState::new(JobPhase::Running, PauseState::Paused).is_effectively_paused());
        assert!(!JobState::new(JobPhase::Queued, PauseState::Paused).is_effectively_paused());
        assert!(!JobState::new(JobPhase::Running, PauseState::Unpaused).is_effectively_paused());
        assert!(!JobState::new(JobPhase::Completed, PauseState::Paused).is_effectively_paused());
    }

    #[test]
    fn job_state_display() {
        assert_eq!(JobState::queued().to_string(), "queued");
        assert_eq!(JobState::running().to_string(), "running");
        assert_eq!(
            JobState::new(JobPhase::Running, PauseState::Paused).to_string(),
            "running (paused)"
        );
        assert_eq!(
            JobState::new(JobPhase::Completed, PauseState::Paused).to_string(),
            "completed"
        );
    }

    #[test]
    fn job_state_ordering() {
        let queued = JobState::queued();
        let running = JobState::running();
        let running_paused = JobState::new(JobPhase::Running, PauseState::Paused);
        let completed = JobState::new(JobPhase::Completed, PauseState::Unpaused);
        assert!(queued < running);
        assert!(running < running_paused); // same phase, Paused > Unpaused
        assert!(running_paused < completed); // phase dominates
    }

    #[test]
    fn job_state_serde_roundtrip() {
        let s = JobState::new(JobPhase::Running, PauseState::Paused);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["phase"], "running");
        assert_eq!(json["pause"], "paused");
        let parsed: JobState = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, s);
    }

    // =========================================================================
    // CancelError tests
    // =========================================================================

    #[test]
    fn cancel_error_display() {
        let e = CancelError::AlreadyTerminal {
            job_id: "j1".into(),
            phase: JobPhase::Completed,
        };
        assert!(e.to_string().contains("j1"));
        assert!(e.to_string().contains("terminal"));

        let e2 = CancelError::NotFound {
            job_id: "j2".into(),
        };
        assert!(e2.to_string().contains("not found"));
    }

    #[test]
    fn cancel_error_equality() {
        let a = CancelError::NotFound {
            job_id: "j1".into(),
        };
        let b = CancelError::NotFound {
            job_id: "j1".into(),
        };
        assert_eq!(a, b);
    }

    // =========================================================================
    // JobView tests
    // =========================================================================

    #[test]
    fn job_view_serde_roundtrip() {
        let view = JobView {
            id: "test-1".to_string(),
            kind: JobKind::Eval,
            phase: JobPhase::Running,
            workspace_id: "default".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:01Z".to_string(),
            description: Some("Test run".to_string()),
            error_message: None,
            is_cancellable: true,
            is_paused: false,
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["kind"], "eval");
        assert_eq!(json["phase"], "running");
        assert_eq!(json["workspaceId"], "default");
        assert_eq!(json["isCancellable"], true);
        assert_eq!(json["isPaused"], false);
        assert_eq!(json["errorMessage"], serde_json::Value::Null);

        let parsed: JobView = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.id, "test-1");
        assert_eq!(parsed.workspace_id, "default");
    }
}
