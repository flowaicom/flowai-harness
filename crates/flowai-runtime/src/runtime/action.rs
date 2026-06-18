//! Noop action dispatcher used as the default when [`RuntimeDeps`] doesn't
//! supply one (runtime query assembly). Python adapter wires the real Python-backed adapter via the
//! same trait shape.
//!
//! Also exposes [`SharedActionDispatcher`], a thin wrapper that lets the
//! `executePlan` handler build a
//! [`HydratingDispatcher`](crate::plans::HydratingDispatcher) over an
//! `Arc<HarnessActionDispatcher>` trait object (which is not itself an
//! `ActionDispatcher` impl thanks to the orphan rules).

use std::sync::Arc;

use agent_fw_plan::{ActionDispatcher, ActionSeq, ExecutionResult};
use async_trait::async_trait;

use crate::{HarnessAction, HarnessActionContext, HarnessActionDispatcher, HarnessActionError};

/// Noop [`ActionDispatcher`] returning an empty [`ExecutionResult`].
///
/// The runtime uses this as the fallback when no host-supplied dispatcher
/// is registered through [`RuntimeDeps::action_dispatcher`](crate::RuntimeDeps::action_dispatcher).
/// It lets the rest of the harness wire through the plan-approval gate and
/// reference-hydration path without forcing every binding to ship a real
/// dispatcher before plan execution becomes useful.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopActionDispatcher;

#[async_trait]
impl ActionDispatcher for NoopActionDispatcher {
    type Action = HarnessAction;
    type Context = HarnessActionContext;
    type Error = HarnessActionError;

    async fn dispatch(
        &self,
        _actions: &ActionSeq<HarnessAction>,
        _ctx: &HarnessActionContext,
    ) -> Result<ExecutionResult, HarnessActionError> {
        Ok(ExecutionResult {
            entities_affected: 0,
            summary: None,
            details: None,
        })
    }
}

/// Thin newtype wrapping an `Arc<HarnessActionDispatcher>` so it satisfies
/// the [`ActionDispatcher`] trait bound consumed by
/// [`HydratingDispatcher<D>`](crate::plans::HydratingDispatcher). The
/// orphan rules forbid implementing the trait directly for
/// `Arc<dyn ActionDispatcher<…>>` in this crate, so the harness wraps
/// once at the `executePlan` handler boundary.
pub struct SharedActionDispatcher(pub Arc<HarnessActionDispatcher>);

#[async_trait]
impl ActionDispatcher for SharedActionDispatcher {
    type Action = HarnessAction;
    type Context = HarnessActionContext;
    type Error = HarnessActionError;

    async fn dispatch(
        &self,
        actions: &ActionSeq<HarnessAction>,
        ctx: &HarnessActionContext,
    ) -> Result<ExecutionResult, HarnessActionError> {
        let result = self.0.dispatch(actions, ctx).await?;
        Ok(with_resolved_actions(result, actions))
    }
}

fn with_resolved_actions(
    mut result: ExecutionResult,
    actions: &ActionSeq<HarnessAction>,
) -> ExecutionResult {
    let resolved_actions = actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "type": action.kind.clone(),
                "payload": action.payload.clone(),
            })
        })
        .collect::<Vec<_>>();

    match result.details {
        Some(serde_json::Value::Object(mut details)) => {
            details
                .entry("resolvedActions")
                .or_insert_with(|| serde_json::json!(resolved_actions));
            result.details = Some(serde_json::Value::Object(details));
        }
        Some(details) => {
            result.details = Some(serde_json::json!({
                "result": details,
                "resolvedActions": resolved_actions,
            }));
        }
        None => {
            result.details = Some(serde_json::json!({
                "resolvedActions": resolved_actions,
            }));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactRef;
    use agent_fw_plan::action::single_action;
    use std::collections::HashMap;

    #[tokio::test]
    async fn noop_dispatcher_returns_empty_result() {
        let dispatcher = NoopActionDispatcher;
        let actions = single_action(HarnessAction {
            kind: "noop".to_string(),
            payload: serde_json::json!({}),
            references: vec![ArtifactRef {
                kind: "kind".to_string(),
                id: "id".to_string(),
            }],
        });
        let ctx = HarnessActionContext {
            resolved_refs: HashMap::new(),
        };
        let result = dispatcher.dispatch(&actions, &ctx).await.expect("dispatch");
        assert_eq!(result.entities_affected, 0);
        assert!(result.summary.is_none());
        assert!(result.details.is_none());
    }

    #[test]
    fn resolved_actions_are_added_to_execution_details() {
        let actions = single_action(HarnessAction {
            kind: "record_counter".to_string(),
            payload: serde_json::json!({ "message": "record eval action" }),
            references: vec![],
        });
        let result = ExecutionResult {
            entities_affected: 1,
            summary: Some("ok".to_string()),
            details: None,
        };

        let result = with_resolved_actions(result, &actions);

        assert_eq!(
            result.details.expect("details")["resolvedActions"][0],
            serde_json::json!({
                "type": "record_counter",
                "payload": { "message": "record eval action" },
            })
        );
    }
}
