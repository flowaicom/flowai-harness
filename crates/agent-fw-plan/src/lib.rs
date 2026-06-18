//! Generic plan state machine, sweep axis system, and card algebra.
//!
//! # Stories
//!
//! This crate tells four independent stories. Import what you need:
//!
//! ## Story 1: Plan Lifecycle (always available)
//!
//! [`Plan<A>`], [`ActionSeq<A>`], [`PlanBuilder`], [`PlanContext`] — the
//! state machine. Generic over your domain's action type. Start with
//! the [`prelude`].
//!
//! ## Story 2: Sweep & Metrics (always available)
//!
//! [`MetricPoint`] (commutative monoid), [`SweepRange`], [`SweepMetric`]
//! — pure math for parameter sensitivity analysis. No plans required.
//!
//! ## Story 3: Presentation (always available)
//!
//! [`CardAlg`] (tagless final) — rendering plans for humans. Multiple
//! interpreters: JSON, plain text. `flow_ui` and `glimpse` reserve neutral
//! projection boundaries for future Studio/harness presentation work.
//!
//! ## Story 4: Infrastructure (always available)
//!
//! [`persist_plan`], [`PlanExecutor`], [`ActionDispatcher`] — KV-backed
//! plan persistence and execution lifecycle.
//!
//! # Architecture
//!
//! The plan module sits between core types and the tool/service layer.
//! It depends only on `agent-fw-core` for ID types.
//!
//! Users parameterize `Plan<A>` with their domain-specific action type,
//! implement `SweepMetric<E, A>` for domain metrics, and choose `CardAlg`
//! interpreters for rendering.
//!
//! Product, scope, pricing, and legacy FlowGen plan-building APIs are not part
//! of the generic framework surface.

// ── Story 1: Plan Lifecycle ─────────────────────────────────────────
pub mod action;
pub mod builder;
pub mod context;
pub mod plan;

// ── Story 2: Sweep & Metrics ────────────────────────────────────────
pub mod sweep;
pub mod sweep_runner;

// ── Story 3: Presentation ───────────────────────────────────────────
pub mod card;
#[cfg(feature = "tool-presentation")]
pub mod card_presentation;
pub mod flow_ui;
pub mod glimpse;

// ── Story 4: Infrastructure ─────────────────────────────────────────
pub mod executor;
pub mod persist;
pub mod tool_dispatcher;

// ── Prelude ─────────────────────────────────────────────────────────

/// Curated re-exports for newcomers.
///
/// Contains the Story 1 surface: plan lifecycle, actions, builder, and executor.
/// Import with `use agent_fw_plan::prelude::*;` to get started quickly.
pub mod prelude {
    pub use crate::action::{action_seq_from_vec, single_action, ActionSeq};
    pub use crate::builder::{PlanBuildError, PlanBuilder};
    pub use crate::executor::{ActionDispatcher, PlanExecutor};
    #[allow(deprecated)]
    pub use crate::plan::{
        create_plan, Plan, PlanOutcome, PlanStatus, Rejected, TerminalOutcome, TransitionError,
    };
}

// ─── Re-exports: Story 1 (Plan Lifecycle) ────────────────────────────

#[allow(deprecated)]
pub use plan::{
    create_plan, create_plan_with_context, ExecutionResult, Plan, PlanError, PlanOutcome,
    PlanStatus, Rejected, TerminalOutcome, TransitionError,
};

pub use action::{
    action_seq_from_vec, append_action, concat_actions, single_action, ActionSeq, PlanGroup,
};

pub use builder::{PlanBuildError, PlanBuilder};

pub use context::{ContextError, ContextKey, PlanContext, PlanContextProjection};

// ─── Re-exports: Story 2 (Sweep & Metrics) ──────────────────────────

pub use sweep::{
    build_description, compute_sweep_summary, find_breakeven, ActionValue, MetricPoint, SweepAxis,
    SweepAxisType, SweepMetric, SweepPoint, SweepRange, SweepSummary, SweepTarget,
    MAX_SWEEP_POINTS,
};

pub use sweep_runner::{
    comparison_sweep, comparison_sweep_streaming, grid_sweep, grid_sweep_streaming,
    parameter_sweep, parameter_sweep_streaming, NullSweepObserver, SweepGroup, SweepObserver,
    SweepSlice,
};

// ─── Re-exports: Story 3 (Presentation) ─────────────────────────────

pub use card::{
    ButtonVariant, CalloutVariant, CardAlg, JsonCard, JsonSweepCard, PlainText, SeriesHighlight,
    SeriesPoint,
};
pub use flow_ui::{PlanUiFact, PlanUiProjection};
pub use glimpse::{PlanGlimpse, PlanGlimpseReference};

#[cfg(feature = "tool-presentation")]
pub use card_presentation::{CardOutputFields, CardPresentation};

// ─── Re-exports: Story 4 (Infrastructure) ───────────────────────────

pub use persist::{
    approve_plan_in_kv, approve_plan_observed, complete_or_recover_in_kv,
    complete_or_recover_observed, complete_plan_in_kv, complete_plan_observed, fail_plan_in_kv,
    fail_plan_observed, load_plan, load_plan_with_prefix, persist_plan, persist_plan_with_prefix,
    plan_key, start_plan_in_kv, start_plan_observed, ComposedPlanObserver, LoggingPlanObserver,
    NullPlanObserver, PlanPersistError, PlanStore, PlanTransitionObserver, PLAN_PREFIX, PLAN_TTL,
};

pub use executor::{ActionDispatcher, PlanExecutionError, PlanExecutor};
