//! `plans` toolkit — typed plan lifecycle helpers.
//!
//! Exposes three tools backed by the runtime-owned
//! [`PlanRegistry`](crate::plans::PlanRegistry):
//!
//! - `storePlan` validates planner output against a registered
//!   [`PlanSpec`](crate::PlanSpec), normalises the action sequence, and
//!   persists the plan in `Draft`. Delegates to
//!   [`PlanRegistry::propose`](crate::plans::PlanRegistry::propose).
//! - `getPlan` loads a persisted plan and returns its status + body.
//!   Delegates to [`PlanRegistry::load`](crate::plans::PlanRegistry::load).
//! - `executePlan` (runtime query assembly C4) runs the framework's
//!   [`GatedPlanExecutor`](agent_fw_plan::executor::GatedPlanExecutor)
//!   wrapped around a
//!   [`HydratingDispatcher`](crate::plans::HydratingDispatcher), driving
//!   the `Draft → Approved → Executed | Failed` transitions. Reads its
//!   approval store, customer action dispatcher and approver identity
//!   from a [`PlanExecutionContext`](crate::runtime::PlanExecutionContext)
//!   env extension.
//!
//! `listPlans` is intentionally not included — the registry has no
//! prefix-iteration capability and the ticket acceptance criteria do
//! not require it.

use std::sync::Arc;

use agent_fw_agent::approval::ApprovalContext;
use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_algebra::EventSinkExt;
use agent_fw_core::approval::ApprovalKind;
use agent_fw_core::PlanId;
use agent_fw_plan::executor::{GatedExecutionError, GatedPlanExecutor};
use agent_fw_plan::PlanStatus;
use agent_fw_tool::{ToolEnvironment, ToolSchema};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use super::{filter_by_config, ToolkitConfig, ToolkitError};
use crate::plans::{HydratingDispatcher, PlanProtocolError, PlanRegistry};
use crate::runtime::action::SharedActionDispatcher;
use crate::runtime::PlanExecutionContext;

// ─── storePlan ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, agent_fw_tool_macro::ToolSchema)]
#[serde(rename_all = "camelCase")]
struct StorePlanInput {
    /// Name of a registered [`PlanSpec`](crate::PlanSpec).
    #[schema(description = "Plan spec name matching a declared PlanSpec")]
    spec_name: String,
    /// Plan identifier minted by the caller (typically a fresh UUID).
    #[schema(description = "Identifier for this plan instance")]
    plan_id: String,
    /// Planner output. Must validate against the spec's JSON schema and
    /// carry a top-level `actions` array.
    #[schema(
        description = "Planner body: validates against PlanSpec.schema, must include `actions`"
    )]
    body: JsonValue,
}

#[derive(Debug, Clone, Deserialize, agent_fw_tool_macro::ToolSchema)]
#[serde(rename_all = "camelCase")]
struct GetPlanInput {
    /// Plan identifier returned by `storePlan`.
    #[schema(description = "Identifier of the plan to load")]
    plan_id: String,
}

pub(crate) struct StorePlanHandler {
    registry: Arc<PlanRegistry>,
}

impl StorePlanHandler {
    pub(crate) fn new(registry: Arc<PlanRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for StorePlanHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "storePlan".to_string(),
            description: "Validate planner output against a PlanSpec and persist a new Draft plan."
                .to_string(),
            input_schema: StorePlanInput::json_schema(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let input: StorePlanInput = match serde_json::from_value(input) {
            Ok(i) => i,
            Err(e) => return ToolCallResult::error(tool_use_id, format!("Invalid input: {e}")),
        };
        let plan_id = PlanId::new_unchecked(input.plan_id);
        match self
            .registry
            .propose(&input.spec_name, plan_id, input.body, env.resource_id())
            .await
        {
            Ok(plan) => match serde_json::to_value(&plan) {
                Ok(value) => ToolCallResult::success(tool_use_id, value),
                Err(e) => ToolCallResult::error(
                    tool_use_id,
                    format!("Failed to serialise stored plan: {e}"),
                ),
            },
            Err(e) => plan_error_to_result(tool_use_id, e),
        }
    }
}

// ─── getPlan ────────────────────────────────────────────────────────

pub(crate) struct GetPlanHandler {
    registry: Arc<PlanRegistry>,
}

impl GetPlanHandler {
    pub(crate) fn new(registry: Arc<PlanRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for GetPlanHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "getPlan".to_string(),
            description: "Load a persisted plan by id. Returns null for unknown ids.".to_string(),
            input_schema: GetPlanInput::json_schema(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let input: GetPlanInput = match serde_json::from_value(input) {
            Ok(i) => i,
            Err(e) => return ToolCallResult::error(tool_use_id, format!("Invalid input: {e}")),
        };
        let plan_id = PlanId::new_unchecked(input.plan_id);
        match self.registry.load(&plan_id, env.resource_id()).await {
            Ok(None) => ToolCallResult::success(tool_use_id, JsonValue::Null),
            Ok(Some(plan)) => match serde_json::to_value(&plan) {
                Ok(value) => ToolCallResult::success(tool_use_id, value),
                Err(e) => ToolCallResult::error(
                    tool_use_id,
                    format!("Failed to serialise loaded plan: {e}"),
                ),
            },
            Err(e) => plan_error_to_result(tool_use_id, e),
        }
    }
}

fn plan_error_to_result(tool_use_id: &str, err: PlanProtocolError) -> ToolCallResult {
    ToolCallResult::error(tool_use_id, err.to_string())
}

// ─── executePlan ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, agent_fw_tool_macro::ToolSchema)]
#[serde(rename_all = "camelCase")]
struct ExecutePlanInput {
    /// Plan identifier returned by `storePlan`.
    #[schema(description = "Identifier of the plan to execute")]
    plan_id: String,
}

pub(crate) struct ExecutePlanHandler {
    registry: Arc<PlanRegistry>,
}

impl ExecutePlanHandler {
    pub(crate) fn new(registry: Arc<PlanRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for ExecutePlanHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "executePlan".to_string(),
            description: "Execute a stored plan. Drives the framework approval gate \
                 (`Draft → Approved`) and dispatches the hydrated actions to \
                 the customer-supplied ActionDispatcher."
                .to_string(),
            input_schema: ExecutePlanInput::json_schema(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let input: ExecutePlanInput = match serde_json::from_value(input) {
            Ok(i) => i,
            Err(e) => return ToolCallResult::error(tool_use_id, format!("Invalid input: {e}")),
        };
        let ctx = match env.try_ext::<PlanExecutionContext>() {
            Ok(ctx) => ctx.clone(),
            Err(e) => return ToolCallResult::error(tool_use_id, e.to_string()),
        };
        let Some(thread_id) = env.tenant().thread_id().cloned() else {
            return ToolCallResult::error(
                tool_use_id,
                "executePlan requires a thread id on the tenant context",
            );
        };
        let plan_id = PlanId::new_unchecked(input.plan_id);
        let tenant = env.resource_id().clone();

        let plan = match self.registry.load(&plan_id, &tenant).await {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                return plan_error_to_result(
                    tool_use_id,
                    PlanProtocolError::NotFound {
                        plan_id: plan_id.clone(),
                    },
                );
            }
            Err(e) => return plan_error_to_result(tool_use_id, e),
        };

        if plan.status == PlanStatus::Draft {
            let payload = match serde_json::to_value(&plan) {
                Ok(value) => value,
                Err(e) => {
                    return ToolCallResult::error(
                        tool_use_id,
                        format!("Failed to serialise plan approval payload: {e}"),
                    );
                }
            };
            let approval_context = ApprovalContext {
                kind: ApprovalKind::Plan,
                target: plan_id.as_str(),
                input: &payload,
                tenant: &tenant,
            };
            if !ctx
                .approval_policy
                .resolve_plan("plan")
                .is_required(&approval_context)
            {
                match self
                    .registry
                    .approve_for_execution(&plan_id, &tenant, ctx.approver.clone())
                    .await
                {
                    Ok(true) => {
                        if !env.event_sink().emit_plan_status_change(
                            plan_id.as_str(),
                            "draft",
                            "approved",
                        ) {
                            tracing::warn!(
                                plan_id = plan_id.as_str(),
                                "event sink closed; auto-approval status transition lost to host"
                            );
                            return ToolCallResult::error(
                                tool_use_id,
                                format!(
                                    "Failed to emit plan status transition for '{}'",
                                    plan_id.as_str()
                                ),
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(e) => return plan_error_to_result(tool_use_id, e),
                }
            }
        }

        // Compose framework primitives: HydratingDispatcher (plan registry) wraps
        // the customer action dispatcher; GatedPlanExecutor (pre-dispatch approval)
        // wraps that with the approval gate. No logic is reimplemented —
        // each construction is one line of glue.
        let shared = SharedActionDispatcher(ctx.action_dispatcher.clone());
        let hydrating =
            HydratingDispatcher::new(shared, self.registry.references().clone(), tenant.clone());
        let gated = GatedPlanExecutor::new(
            env.kv().as_ref(),
            &tenant,
            &hydrating,
            "plan",
            env.event_sink().clone(),
            ctx.approval_store.clone(),
            ctx.approver.clone(),
            thread_id,
        );

        match gated.execute(&plan_id, &(), env.cancel()).await {
            Ok(result) => match execute_plan_result_value(&result) {
                Ok(value) => ToolCallResult::success(tool_use_id, value),
                Err(e) => ToolCallResult::error(
                    tool_use_id,
                    format!("Failed to serialise execution result: {e}"),
                ),
            },
            Err(GatedExecutionError::Rejected { plan_id, reason }) => {
                ToolCallResult::error(tool_use_id, format!("Plan {plan_id} rejected: {reason}"))
            }
            Err(GatedExecutionError::Revise {
                plan_id, partial, ..
            }) => ToolCallResult::success(
                tool_use_id,
                serde_json::json!({
                    "rejected": true,
                    "should_revise": true,
                    "plan_id": plan_id.as_str(),
                    "partial": partial,
                }),
            ),
            Err(GatedExecutionError::Cancelled { plan_id }) => ToolCallResult::error(
                tool_use_id,
                format!("Plan {plan_id} approval was cancelled"),
            ),
            Err(GatedExecutionError::EventSinkClosed { plan_id }) => ToolCallResult::error(
                tool_use_id,
                format!("Plan {plan_id} approval channel closed before resolution"),
            ),
            Err(GatedExecutionError::Approval(err)) => {
                ToolCallResult::error(tool_use_id, format!("Approval store error: {err}"))
            }
            Err(GatedExecutionError::Execution(inner)) => {
                ToolCallResult::error(tool_use_id, inner.to_string())
            }
        }
    }
}

fn execute_plan_result_value(
    result: &agent_fw_plan::ExecutionResult,
) -> Result<JsonValue, serde_json::Error> {
    let mut value = serde_json::to_value(result)?;
    if let Some(resolved_actions) = result
        .details
        .as_ref()
        .and_then(|details| details.get("resolvedActions"))
        .cloned()
    {
        if let Some(object) = value.as_object_mut() {
            object.entry("resolvedActions").or_insert(resolved_actions);
        }
    }
    Ok(value)
}

// ─── Toolkit entry point ────────────────────────────────────────────

pub(super) fn handlers(
    toolkit_id: &str,
    cfg: &ToolkitConfig,
    registry: Arc<PlanRegistry>,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    let handlers: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(StorePlanHandler::new(registry.clone())),
        Arc::new(GetPlanHandler::new(registry.clone())),
        Arc::new(ExecutePlanHandler::new(registry)),
    ];
    filter_by_config(toolkit_id, handlers, cfg)
}
