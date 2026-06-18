//! Approval rules and policy registry — the closure-bearing half of pre-dispatch approval.
//!
//! Pure data types (`ApprovalRequest`, `ApprovalDecision`, `ApprovalOutcome`,
//! `ApprovalKind`) live in [`agent_fw_core::approval`] so the
//! `PendingApprovalStore` algebra (in `agent-fw-algebra`) can reference
//! them without violating crate layering. This module owns the runtime
//! abstractions that the framework's `ApprovalLayer` consumes:
//!
//! - [`ApprovalRule`] — `Never | Always | Dynamic(predicate)` per-target rule.
//! - [`ApprovalContext`] — borrowed view passed to a `Dynamic` predicate.
//! - [`ApprovalPolicy`] — per-tool / per-plan registry with default floor.
//!
//! The pure data types are re-exported for convenience.
//!
//! # Algebraic laws
//!
//! - **L1 (Rule totality)**: [`ApprovalRule::is_required`] never panics.
//! - **L2 (Policy determinism)**: Given the same name input,
//!   [`ApprovalPolicy::resolve_tool`] returns the same rule reference on
//!   each call. `Dynamic` evaluation may then differ per call — that is
//!   by design.

use std::collections::HashMap;
use std::sync::Arc;

use agent_fw_core::TenantId;

pub use agent_fw_core::approval::{
    ApprovalDecision, ApprovalKind, ApprovalOutcome, ApprovalRequest, PlanStatusChange,
};

// ─── ApprovalRule ────────────────────────────────────────────────────

/// Context passed to a [`ApprovalRule::Dynamic`] predicate.
///
/// Borrowed fields — no allocation per call.
pub struct ApprovalContext<'a> {
    pub kind: ApprovalKind,
    pub target: &'a str,
    pub input: &'a serde_json::Value,
    pub tenant: &'a TenantId,
}

/// Predicate used by [`ApprovalRule::Dynamic`].
///
/// Kept as a type alias so the trait bounds (`Fn + Send + Sync`) appear
/// once and the closure body stays readable at the call site.
pub type ApprovalPredicate =
    Arc<dyn for<'a> Fn(&ApprovalContext<'a>) -> bool + Send + Sync + 'static>;

/// When is approval required for a given target?
///
/// The variants mirror the public SDK contract from `flowai-harness`:
/// `"never" | "always" | (args, ctx) => bool`.
#[derive(Clone)]
pub enum ApprovalRule {
    /// Auto-allowed — gate delegates directly to the inner handler/executor.
    Never,
    /// Always paused for human review.
    Always,
    /// Predicate over `(target, input, tenant)` decides per-call.
    Dynamic(ApprovalPredicate),
}

impl ApprovalRule {
    /// Convenience constructor for the dynamic variant.
    pub fn dynamic<F>(predicate: F) -> Self
    where
        F: for<'a> Fn(&ApprovalContext<'a>) -> bool + Send + Sync + 'static,
    {
        ApprovalRule::Dynamic(Arc::new(predicate))
    }

    /// Evaluate the rule. `Never` is always `false`; `Always` is always
    /// `true`; `Dynamic` runs its predicate.
    pub fn is_required(&self, ctx: &ApprovalContext<'_>) -> bool {
        match self {
            ApprovalRule::Never => false,
            ApprovalRule::Always => true,
            ApprovalRule::Dynamic(p) => p(ctx),
        }
    }
}

impl Default for ApprovalRule {
    fn default() -> Self {
        ApprovalRule::Never
    }
}

impl std::fmt::Debug for ApprovalRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalRule::Never => f.write_str("Never"),
            ApprovalRule::Always => f.write_str("Always"),
            ApprovalRule::Dynamic(_) => f.write_str("Dynamic(<closure>)"),
        }
    }
}

// ─── ApprovalPolicy ──────────────────────────────────────────────────

/// Per-tool and per-plan approval rule registry.
///
/// The harness builder populates this from `coordinator.approval` (the
/// floor) overlaid with per-tool `defineTool({ approval: ... })` settings.
/// Tool settings can **raise** the floor but never lower it; the public
/// SDK enforces that asymmetry — this struct just records the result.
#[derive(Clone, Debug, Default)]
pub struct ApprovalPolicy {
    /// Floor for any tool not in `tools`.
    default_tool_rule: ApprovalRule,
    /// Floor for any plan kind not in `plans`.
    default_plan_rule: ApprovalRule,
    /// Per-tool overrides (key: tool name from `ToolDefinition::name`).
    tools: HashMap<String, ApprovalRule>,
    /// Per-plan overrides (key: plan kind/name).
    plans: HashMap<String, ApprovalRule>,
}

impl ApprovalPolicy {
    /// Create an empty policy (everything `Never`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: set the default rule applied to tools without overrides.
    pub fn with_default_tool_rule(mut self, rule: ApprovalRule) -> Self {
        self.default_tool_rule = rule;
        self
    }

    /// Builder: set the default rule applied to plans without overrides.
    pub fn with_default_plan_rule(mut self, rule: ApprovalRule) -> Self {
        self.default_plan_rule = rule;
        self
    }

    /// Builder: register a per-tool override.
    pub fn with_tool(mut self, name: impl Into<String>, rule: ApprovalRule) -> Self {
        self.tools.insert(name.into(), rule);
        self
    }

    /// Builder: register a per-plan override.
    pub fn with_plan(mut self, name: impl Into<String>, rule: ApprovalRule) -> Self {
        self.plans.insert(name.into(), rule);
        self
    }

    /// Resolve the rule for a tool call.
    pub fn resolve_tool(&self, name: &str) -> &ApprovalRule {
        self.tools.get(name).unwrap_or(&self.default_tool_rule)
    }

    /// Resolve the rule for a plan execution.
    pub fn resolve_plan(&self, kind: &str) -> &ApprovalRule {
        self.plans.get(kind).unwrap_or(&self.default_plan_rule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tid() -> TenantId {
        TenantId::new_unchecked("acme")
    }

    // ── L1: Rule totality ───────────────────────────────────────────

    #[test]
    fn rule_never_is_required_is_false() {
        let r = ApprovalRule::Never;
        let ctx = ApprovalContext {
            kind: ApprovalKind::Tool,
            target: "x",
            input: &json!({}),
            tenant: &tid(),
        };
        assert!(!r.is_required(&ctx));
    }

    #[test]
    fn rule_always_is_required_is_true() {
        let r = ApprovalRule::Always;
        let ctx = ApprovalContext {
            kind: ApprovalKind::Tool,
            target: "x",
            input: &json!({}),
            tenant: &tid(),
        };
        assert!(r.is_required(&ctx));
    }

    #[test]
    fn rule_dynamic_runs_predicate() {
        let r = ApprovalRule::dynamic(|ctx| {
            ctx.target == "danger" && ctx.input.get("force") == Some(&json!(true))
        });
        let safe = ApprovalContext {
            kind: ApprovalKind::Tool,
            target: "danger",
            input: &json!({"force": false}),
            tenant: &tid(),
        };
        let danger = ApprovalContext {
            kind: ApprovalKind::Tool,
            target: "danger",
            input: &json!({"force": true}),
            tenant: &tid(),
        };
        assert!(!r.is_required(&safe));
        assert!(r.is_required(&danger));
    }

    // ── L2: Policy floor + override semantics ───────────────────────

    #[test]
    fn policy_default_falls_back_to_never() {
        let p = ApprovalPolicy::new();
        assert!(matches!(p.resolve_tool("anything"), ApprovalRule::Never));
        assert!(matches!(p.resolve_plan("anything"), ApprovalRule::Never));
    }

    #[test]
    fn policy_default_tool_rule_applies_to_unknowns() {
        let p = ApprovalPolicy::new().with_default_tool_rule(ApprovalRule::Always);
        assert!(matches!(p.resolve_tool("unknown"), ApprovalRule::Always));
    }

    #[test]
    fn policy_tool_override_beats_default() {
        let p = ApprovalPolicy::new()
            .with_default_tool_rule(ApprovalRule::Always)
            .with_tool("safe_tool", ApprovalRule::Never);
        assert!(matches!(p.resolve_tool("safe_tool"), ApprovalRule::Never));
        assert!(matches!(p.resolve_tool("other"), ApprovalRule::Always));
    }

    #[test]
    fn policy_plan_override_beats_default() {
        let p = ApprovalPolicy::new()
            .with_default_plan_rule(ApprovalRule::Always)
            .with_plan("low_risk", ApprovalRule::Never);
        assert!(matches!(p.resolve_plan("low_risk"), ApprovalRule::Never));
        assert!(matches!(p.resolve_plan("other"), ApprovalRule::Always));
    }
}
