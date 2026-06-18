//! Fluent plan builder.
//!
//! Provides a type-safe, ergonomic way to assemble plans with descriptions,
//! actions, and domain-specific context.
//!
//! # Laws
//!
//! - **L1 (Non-empty actions)**: `build()` fails with `EmptyActions` if
//!   no actions were provided (enforced at build time, not construction).
//!
//! - **L2 (Context merge)**: Multiple `context_entry` calls accumulate;
//!   later calls with the same key overwrite earlier ones (last-write-wins).

use agent_fw_core::{PlanId, TenantId};
use thiserror::Error;

use crate::action::{action_seq_from_vec, ActionSeq};
use crate::context::PlanContext;
use crate::plan::{create_plan, Plan};

/// Errors from plan building.
#[derive(Debug, Error)]
pub enum PlanBuildError {
    #[error("Missing required field: {0}")]
    MissingField(&'static str),
    #[error("Empty action list — plans require at least one action")]
    EmptyActions,
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Fluent builder for Plan<A>.
///
/// Handles the common pattern of assembling a plan with:
/// - Actions (required, must be non-empty)
/// - Description (optional)
/// - Arbitrary context entries (optional)
///
/// # Example
///
/// ```ignore
/// let plan = PlanBuilder::new(plan_id, tenant)
///     .description("Increase prices for premium beverages")
///     .actions(vec![price_action])
///     .build()?;
/// ```
#[derive(Debug)]
pub struct PlanBuilder<A> {
    id: PlanId,
    owner: TenantId,
    action_seq: Option<ActionSeq<A>>,
    raw_actions: Option<Vec<A>>,
    description: Option<String>,
    context: PlanContext,
}

impl<A> PlanBuilder<A> {
    /// Start building a plan.
    pub fn new(id: PlanId, owner: TenantId) -> Self {
        Self {
            id,
            owner,
            action_seq: None,
            raw_actions: None,
            description: None,
            context: PlanContext::new(),
        }
    }

    /// Set the plan description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set actions from a Vec. Validation (non-empty) deferred to `build()`.
    pub fn actions(mut self, actions: Vec<A>) -> Self {
        self.raw_actions = Some(actions);
        self
    }

    /// Set actions from a pre-built ActionSeq (always valid).
    ///
    /// Takes priority over `raw_actions` if both are set.
    pub fn action_seq(mut self, seq: ActionSeq<A>) -> Self {
        self.action_seq = Some(seq);
        self
    }

    /// Add an arbitrary key-value pair to the plan context.
    pub fn context_entry(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.context.set(key, value);
        self
    }

    /// Add a typed context entry using a phantom-typed key.
    ///
    /// Pairs with [`PlanContext::get_typed`] — the type parameter `T`
    /// ensures write and read agree on the value's shape.
    ///
    /// # Panics
    ///
    /// Panics if `T`'s `Serialize` impl fails. This is a programmer bug
    /// (broken Serialize impl), not a runtime condition — so it's appropriate
    /// to panic rather than break the fluent chain with `Result`.
    pub fn typed_context_entry<T: serde::Serialize>(
        mut self,
        key: &crate::context::ContextKey<T>,
        value: &T,
    ) -> Self {
        self.context.set_typed(key, value).expect(
            "Serialize impl produced an error — this is a bug in the type's Serialize impl",
        );
        self
    }

    /// Build the plan. All validation happens here.
    ///
    /// Resolution order: `action_seq` (pre-validated) wins over `raw_actions`
    /// (validated here). Fails if neither was provided, or if `raw_actions`
    /// was empty.
    ///
    /// The resulting plan is in Draft status.
    pub fn build(self) -> Result<Plan<A>, PlanBuildError> {
        let actions = if let Some(seq) = self.action_seq {
            seq
        } else if let Some(raw) = self.raw_actions {
            action_seq_from_vec(raw).ok_or(PlanBuildError::EmptyActions)?
        } else {
            return Err(PlanBuildError::MissingField("actions"));
        };
        let mut plan = create_plan(self.id, self.owner, actions);
        plan.description = self.description;
        plan.context = self.context;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::single_action;
    use crate::plan::PlanStatus;

    #[test]
    fn builder_simple_plan() {
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .description("test plan")
            .actions(vec!["action_a".to_string()])
            .build()
            .unwrap();

        assert_eq!(plan.description.as_deref(), Some("test plan"));
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.status, PlanStatus::Draft);
    }

    #[test]
    fn builder_empty_actions_error() {
        let result =
            PlanBuilder::<String>::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
                .actions(vec![])
                .build();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PlanBuildError::EmptyActions));
    }

    #[test]
    fn builder_empty_actions_does_not_fail_until_build() {
        // .actions(vec![]) is infallible — error deferred to build()
        let _builder =
            PlanBuilder::<String>::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
                .actions(vec![])
                .description("deferred validation");
        // No error yet — would panic if actions() returned Result
    }

    #[test]
    fn builder_action_seq_takes_priority() {
        let seq = single_action("from_seq".to_string());
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .actions(vec!["from_vec".to_string()])
            .action_seq(seq)
            .build()
            .unwrap();

        assert_eq!(plan.actions.to_vec(), vec!["from_seq"]);
    }

    #[test]
    fn builder_no_actions_error() {
        let result =
            PlanBuilder::<String>::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
                .description("no actions")
                .build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PlanBuildError::MissingField("actions")
        ));
    }

    #[test]
    fn builder_context_entry() {
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .context_entry("strategy", serde_json::json!("premium_uplift"))
            .context_entry("region", serde_json::json!("US"))
            .actions(vec!["act".to_string()])
            .build()
            .unwrap();

        assert_eq!(
            plan.context.get("strategy"),
            Some(&serde_json::json!("premium_uplift"))
        );
        assert_eq!(plan.context.get("region"), Some(&serde_json::json!("US")));
    }

    #[test]
    fn builder_action_seq() {
        let seq = single_action("x".to_string());
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .action_seq(seq)
            .build()
            .unwrap();

        assert_eq!(plan.actions.len(), 1);
    }

    #[test]
    fn builder_plan_is_draft() {
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .actions(vec!["a".to_string()])
            .build()
            .unwrap();

        assert_eq!(plan.status, PlanStatus::Draft);
        assert!(plan.can_approve());
    }

    #[test]
    fn builder_context_merge() {
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .context_entry("key", serde_json::json!("first"))
            .context_entry("key", serde_json::json!("second"))
            .actions(vec!["a".to_string()])
            .build()
            .unwrap();

        // L3: last-write-wins
        assert_eq!(plan.context.get("key"), Some(&serde_json::json!("second")));
    }

    #[test]
    fn builder_multiple_actions() {
        let plan = PlanBuilder::new(PlanId::new_unchecked("p1"), TenantId::new_unchecked("t1"))
            .actions(vec!["a".to_string(), "b".to_string(), "c".to_string()])
            .build()
            .unwrap();

        assert_eq!(plan.actions.len(), 3);
        assert_eq!(plan.actions.to_vec(), vec!["a", "b", "c"]);
    }
}
