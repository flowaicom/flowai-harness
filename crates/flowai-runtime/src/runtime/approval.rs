//! Approval-policy compilation and decision translation (runtime query assembly C4).
//!
//! The runtime spec carries serialisable approval DTOs ([`ApprovalPolicies`],
//! [`crate::ApprovalRule`], [`crate::ApprovalDecision`]) so SDKs can ship a
//! `RuntimeSpec` over JSON. The framework consumes effect-side variants
//! ([`agent_fw_agent::ApprovalPolicy`], [`agent_fw_core::approval::ApprovalDecision`]).
//! This module translates between the two without inventing any new
//! semantics — runtime data → framework data, period.
//!
//! `compile_policy` and `into_core_decision` together back
//! [`Runtime::respond_to_approval`](crate::Runtime::respond_to_approval).

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_fw_agent::approval::{ApprovalPolicy, ApprovalPredicate, ApprovalRule as FrameworkRule};
use agent_fw_algebra::approval::ApprovalError;
use agent_fw_core::approval::{ApprovalDecision as CoreDecision, ApprovalOutcome as CoreOutcome};
use agent_fw_core::ApprovalId;

use crate::{
    ApprovalDecision as RuntimeDecision, ApprovalOutcome as RuntimeOutcome, ApprovalPolicies,
    ApprovalRule as RuntimeRule, HostToolBinding, RuntimeError, RuntimeSpec,
};

/// Host-registered dynamic approval predicates keyed by runtime-spec id.
pub type ApprovalPredicateRegistry = BTreeMap<String, ApprovalPredicate>;

/// Compile the runtime-spec floor ([`ApprovalPolicies`]) into the
/// framework-side [`ApprovalPolicy`] consumed by [`agent_fw_agent::ApprovalLayer`]
/// and [`agent_fw_plan::executor::GatedPlanExecutor`].
///
/// Dynamic rules must resolve through `predicates`. Missing dynamic ids are
/// rejected at runtime construction instead of being collapsed to a permanent
/// conservative rule, because that silently changes the host's policy.
pub fn compile_policy(
    policies: &ApprovalPolicies,
    predicates: &ApprovalPredicateRegistry,
) -> Result<ApprovalPolicy, ApprovalPolicyError> {
    Ok(ApprovalPolicy::new()
        .with_default_tool_rule(compile_rule(&policies.tools, predicates)?)
        .with_default_plan_rule(compile_rule(&policies.plans, predicates)?))
}

/// Compile one effective framework approval policy per agent.
pub fn compile_agent_policies(
    spec: &RuntimeSpec,
    host_tools: &BTreeMap<String, Vec<HostToolBinding>>,
    predicates: &ApprovalPredicateRegistry,
) -> Result<BTreeMap<String, Arc<ApprovalPolicy>>, ApprovalPolicyError> {
    let mut result = BTreeMap::new();
    for agent in &spec.agents {
        let patch = spec.approval_overrides.agents.get(&agent.name);
        let plan_rule = patch
            .and_then(|patch| patch.plans.as_ref())
            .unwrap_or(&spec.approval_policies.plans);
        let tool_rule = patch
            .and_then(|patch| patch.tools.as_ref())
            .unwrap_or(&spec.approval_policies.tools);

        let mut policy = ApprovalPolicy::new()
            .with_default_tool_rule(compile_rule(tool_rule, predicates)?)
            .with_default_plan_rule(compile_rule(plan_rule, predicates)?);

        if let Some(bindings) = host_tools.get(&agent.name) {
            for binding in bindings {
                if let Some(rule) = &binding.approval {
                    policy = policy.with_tool(
                        binding.handler.definition().name,
                        compile_rule(rule, predicates)?,
                    );
                }
            }
        }

        if let Some(tool_overrides) = spec.approval_overrides.tools.get(&agent.name) {
            for (tool, rule) in tool_overrides {
                policy = policy.with_tool(tool.clone(), compile_rule(rule, predicates)?);
            }
        }

        result.insert(agent.name.clone(), Arc::new(policy));
    }
    Ok(result)
}

/// Compile one serialisable runtime approval rule into the framework rule.
pub fn compile_rule(
    rule: &RuntimeRule,
    predicates: &ApprovalPredicateRegistry,
) -> Result<FrameworkRule, ApprovalPolicyError> {
    match rule {
        RuntimeRule::Never => Ok(FrameworkRule::Never),
        RuntimeRule::Always => Ok(FrameworkRule::Always),
        RuntimeRule::Dynamic(name) => predicates
            .get(name)
            .cloned()
            .map(FrameworkRule::Dynamic)
            .ok_or_else(|| ApprovalPolicyError::UnregisteredDynamicPredicate {
                name: name.clone(),
            }),
    }
}

/// Errors surfaced while compiling serialisable approval policy data.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ApprovalPolicyError {
    /// A dynamic rule referenced a predicate id the host did not register.
    #[error("dynamic approval predicate '{name}' is not registered")]
    UnregisteredDynamicPredicate { name: String },
}

/// Translate the runtime-spec [`crate::ApprovalDecision`] DTO into the
/// framework-side [`agent_fw_core::approval::ApprovalDecision`] consumed by
/// [`agent_fw_algebra::approval::PendingApprovalStore::resolve`].
///
/// The runtime DTO carries an optional `partial` JSON value that maps to the
/// `Revise { partial }` variant of [`CoreOutcome`]; missing partial is sent
/// as `JsonValue::Null` to keep the planner's revise loop schema-validating
/// instead of silently re-running.
pub fn into_core_decision(decision: &RuntimeDecision) -> CoreDecision {
    let outcome = match &decision.outcome {
        RuntimeOutcome::Approve => CoreOutcome::Approve,
        RuntimeOutcome::Reject => CoreOutcome::Reject,
        RuntimeOutcome::Revise => CoreOutcome::Revise {
            partial: decision.partial.clone().unwrap_or(serde_json::Value::Null),
        },
    };
    CoreDecision {
        id: ApprovalId::new_unchecked(decision.approval_id.clone()),
        outcome,
        feedback: decision.feedback.clone(),
    }
}

/// Map a `PendingApprovalStore` error into the runtime's public error enum.
pub fn map_approval_error(err: ApprovalError) -> RuntimeError {
    match err {
        ApprovalError::NotFound(id) => RuntimeError::ApprovalNotFound(id.as_str().to_string()),
        ApprovalError::AlreadyResolved(id) => {
            RuntimeError::ApprovalAlreadyResolved(id.as_str().to_string())
        }
        other => RuntimeError::Approval(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolHandler};
    use serde_json::json;

    struct TestTool(&'static str);

    #[async_trait::async_trait]
    impl ToolHandler for TestTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.to_string(),
                description: "test tool".to_string(),
                input_schema: json!({}),
            }
        }

        async fn handle(
            &self,
            tool_use_id: &str,
            _input: serde_json::Value,
            _env: &agent_fw_tool::ToolEnvironment,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, json!({}))
        }
    }

    #[test]
    fn compile_policy_maps_runtime_rules_to_framework_rules() {
        let pol = ApprovalPolicies {
            plans: RuntimeRule::Always,
            tools: RuntimeRule::Never,
        };
        let compiled = compile_policy(&pol, &Default::default()).expect("static rules compile");
        assert!(matches!(
            compiled.resolve_plan("any"),
            FrameworkRule::Always
        ));
        assert!(matches!(compiled.resolve_tool("any"), FrameworkRule::Never));
    }

    #[test]
    fn compile_agent_policies_layers_runtime_agent_host_and_tool_overrides() {
        let mut spec = RuntimeSpec::minimal("tenant-1", "v1");
        spec.agents = vec![
            crate::AgentSpec::new(
                "coordinator",
                crate::AgentRole::Coordinator,
                crate::ModelSpec::new("claude-sonnet-4-6"),
                "coordinate",
            ),
            crate::AgentSpec::new(
                "executor",
                crate::AgentRole::Executor,
                crate::ModelSpec::new("claude-sonnet-4-6"),
                "execute",
            ),
        ];
        spec.approval_policies = ApprovalPolicies {
            plans: RuntimeRule::Always,
            tools: RuntimeRule::Always,
        };
        spec.approval_overrides.agents.insert(
            "executor".to_string(),
            crate::ApprovalPolicyPatch {
                plans: Some(RuntimeRule::Never),
                tools: Some(RuntimeRule::Never),
            },
        );
        spec.approval_overrides
            .tools
            .entry("executor".to_string())
            .or_default()
            .insert("execute_query".to_string(), RuntimeRule::Never);

        let mut host_tools: BTreeMap<String, Vec<HostToolBinding>> = BTreeMap::new();
        host_tools.insert(
            "executor".to_string(),
            vec![
                HostToolBinding::new(Arc::new(TestTool("host_only")))
                    .with_approval(RuntimeRule::Always),
                HostToolBinding::new(Arc::new(TestTool("execute_query")))
                    .with_approval(RuntimeRule::Always),
            ],
        );

        let policies = compile_agent_policies(&spec, &host_tools, &Default::default())
            .expect("policies compile");
        let coordinator = policies.get("coordinator").expect("coordinator policy");
        let executor = policies.get("executor").expect("executor policy");

        assert!(matches!(
            coordinator.resolve_plan("plan"),
            FrameworkRule::Always
        ));
        assert!(matches!(
            coordinator.resolve_tool("any_tool"),
            FrameworkRule::Always
        ));
        assert!(matches!(
            executor.resolve_plan("plan"),
            FrameworkRule::Never
        ));
        assert!(matches!(
            executor.resolve_tool("unlisted_tool"),
            FrameworkRule::Never
        ));
        assert!(matches!(
            executor.resolve_tool("host_only"),
            FrameworkRule::Always
        ));
        assert!(matches!(
            executor.resolve_tool("execute_query"),
            FrameworkRule::Never
        ));
    }

    #[test]
    fn compile_policy_rejects_unregistered_dynamic_predicates() {
        let pol = ApprovalPolicies {
            plans: RuntimeRule::Dynamic("my_predicate".to_string()),
            tools: RuntimeRule::Dynamic("other".to_string()),
        };
        let err = compile_policy(&pol, &Default::default())
            .expect_err("dynamic predicates need a registry entry");
        assert!(
            err.to_string().contains("my_predicate") || err.to_string().contains("other"),
            "error should name the missing predicate, got: {err}"
        );
    }

    #[test]
    fn compile_policy_uses_registered_dynamic_predicate() {
        let pol = ApprovalPolicies {
            plans: RuntimeRule::Dynamic("needs_plan_approval".to_string()),
            tools: RuntimeRule::Never,
        };
        let mut predicates = ApprovalPredicateRegistry::new();
        predicates.insert(
            "needs_plan_approval".to_string(),
            std::sync::Arc::new(|ctx| ctx.target == "risky_plan"),
        );

        let compiled = compile_policy(&pol, &predicates).expect("predicate registered");
        let tenant = agent_fw_core::TenantId::new_unchecked("acme");
        let safe = agent_fw_agent::approval::ApprovalContext {
            kind: agent_fw_core::approval::ApprovalKind::Plan,
            target: "safe_plan",
            input: &json!({}),
            tenant: &tenant,
        };
        let risky = agent_fw_agent::approval::ApprovalContext {
            kind: agent_fw_core::approval::ApprovalKind::Plan,
            target: "risky_plan",
            input: &json!({}),
            tenant: &tenant,
        };
        assert!(!compiled.resolve_plan("safe_plan").is_required(&safe));
        assert!(compiled.resolve_plan("risky_plan").is_required(&risky));
    }

    #[test]
    fn into_core_decision_preserves_id_and_outcome() {
        let cases = [
            (
                RuntimeDecision {
                    approval_id: "apr-1".to_string(),
                    outcome: RuntimeOutcome::Approve,
                    partial: None,
                    feedback: None,
                },
                "approve",
            ),
            (
                RuntimeDecision {
                    approval_id: "apr-2".to_string(),
                    outcome: RuntimeOutcome::Reject,
                    partial: None,
                    feedback: Some("nope".to_string()),
                },
                "reject",
            ),
            (
                RuntimeDecision {
                    approval_id: "apr-3".to_string(),
                    outcome: RuntimeOutcome::Revise,
                    partial: Some(json!({"new_price": 9.99})),
                    feedback: None,
                },
                "revise",
            ),
        ];

        for (input, expected) in cases {
            let core = into_core_decision(&input);
            assert_eq!(core.id.as_str(), input.approval_id);
            match (expected, &core.outcome) {
                ("approve", CoreOutcome::Approve) => {}
                ("reject", CoreOutcome::Reject) => {}
                ("revise", CoreOutcome::Revise { partial }) => {
                    assert_eq!(partial, &json!({"new_price": 9.99}));
                }
                _ => panic!("outcome mismatch for {expected}: {:?}", core.outcome),
            }
        }
    }

    #[test]
    fn revise_without_partial_sends_null() {
        let core = into_core_decision(&RuntimeDecision {
            approval_id: "apr-x".to_string(),
            outcome: RuntimeOutcome::Revise,
            partial: None,
            feedback: None,
        });
        match core.outcome {
            CoreOutcome::Revise { partial } => assert_eq!(partial, serde_json::Value::Null),
            other => panic!("expected Revise, got {other:?}"),
        }
    }

    #[test]
    fn approval_error_mapping_distinguishes_not_found_from_already_resolved() {
        let id = ApprovalId::new_unchecked("apr-1");

        match map_approval_error(ApprovalError::NotFound(id.clone())) {
            RuntimeError::ApprovalNotFound(s) => assert_eq!(s, "apr-1"),
            other => panic!("expected ApprovalNotFound, got {other:?}"),
        }
        match map_approval_error(ApprovalError::AlreadyResolved(id.clone())) {
            RuntimeError::ApprovalAlreadyResolved(s) => assert_eq!(s, "apr-1"),
            other => panic!("expected ApprovalAlreadyResolved, got {other:?}"),
        }
        match map_approval_error(ApprovalError::Storage("disk full".to_string())) {
            RuntimeError::Approval(s) => assert!(s.contains("disk full")),
            other => panic!("expected Approval(_), got {other:?}"),
        }
    }
}
