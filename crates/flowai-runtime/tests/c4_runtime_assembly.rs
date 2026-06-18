//! C4 (runtime query assembly) end-to-end integration tests for the flowai-runtime.
//!
//! These exercise the harness primitives wired together:
//!
//! - `Runtime::respond_to_approval` against the shared `PendingApprovalStore`
//!   used by both the framework tool gate (`ApprovalLayer`) and plan gate
//!   (`GatedPlanExecutor`).
//! - `executePlan` toolkit handler running `GatedPlanExecutor +
//!   HydratingDispatcher` against a recording `ActionDispatcher`, to prove
//!   reference hydration reaches the customer dispatcher.
//! - The per-request orchestrator wiring is covered by lib-level tests
//!   (`query_streams_sub_agent_call_for_coordinator`, `run_specialist_*`).

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use agent_fw_agent::{ChatInterpreter, ChatProgram, ToolDispatcher};
use agent_fw_algebra::approval::ApprovalAwait;
use agent_fw_algebra::kv_store::KVStoreExt;
use agent_fw_algebra::testing::{NullEventSink, RecordingEventSink};
use agent_fw_algebra::{CancellationToken, EventSink, KVStore};
use agent_fw_core::approval::{ApprovalKind, ApprovalRequest};
use agent_fw_core::stream_part::FinishReason;
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::usage::TokenUsage;
use agent_fw_core::{ApprovalId, PlanId, StreamPart, TenantId, TestCaseId, ThreadId};
use agent_fw_eval::{
    EvalConfig, EvalEventBus, EvalMode, EvalOrchestrator, EvalPlan, EvalRun, EvalStatus,
    EvalTestCase, GroundTruth, ResolvedModelConfig, SampleExecutionError, SampleExecutor,
    SampleExecutorOutput, SampleInput, StandardAggregator, TestCaseSource, TokenUsageSummary,
    TrajectoryMode, ValidatedEvalConfig,
};
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_plan::{ActionDispatcher, ActionSeq, ExecutionResult, PlanStatus};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use futures::{stream, Stream};
use serde_json::json;
use tokio::sync::Mutex;

use flowai_runtime::runtime::PlanExecutionContext;
use flowai_runtime::{
    AgentRole, AgentSpec, ApprovalDecision, ApprovalOutcome, ApprovalPolicies, ApprovalPolicyPatch,
    ApprovalRule, ArtifactRef, HarnessAction, HarnessActionContext, HarnessActionError, ModelSpec,
    PlanDisplayAlias, PlanSpec, ProviderConfig, ReferenceSpec, Runtime, RuntimeDeps, RuntimeError,
    RuntimeSpec, TenantIdentity, ToolkitSpec,
};

// ─── Mock interpreter (zero LLM, valid stream contract) ────────────────

struct NoopInterpreter;

impl ChatInterpreter for NoopInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        Box::pin(stream::iter(vec![
            StreamPart::StepStart,
            StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
        ]))
    }
}

struct ClosedEventSink;

impl EventSink for ClosedEventSink {
    fn emit(&self, _part: StreamPart) -> bool {
        false
    }

    fn close(&self) {}

    fn is_open(&self) -> bool {
        false
    }
}

struct ScriptedEvalExecutor;

#[async_trait]
impl SampleExecutor for ScriptedEvalExecutor {
    async fn execute(
        &self,
        input: SampleInput,
        _model_config: &ResolvedModelConfig,
        _timeout: Option<std::time::Duration>,
    ) -> Result<SampleExecutorOutput, SampleExecutionError> {
        Ok(SampleExecutorOutput {
            actual_trajectory: input.test_case.expected_trajectory.clone(),
            captured_tool_calls: vec![],
            duration_ms: 15,
            token_usage: TokenUsageSummary::new(12, 8, 0, 0),
            error: None,
            thread_id: Some(format!("eval-{}", input.sample_index)),
            extra: Some(flowai_runtime::eval::resolved_actions_extra(&[
                flowai_runtime::eval::ResolvedAction::new(
                    "price_change",
                    serde_json::json!({
                        "changeType": "absolute",
                        "value": 10.0,
                        "productIds": ["sku-1"],
                        "context": { "channels": ["ONLINE"] },
                    }),
                ),
            ])),
            latency: None,
        })
    }
}

// ─── Recording ActionDispatcher (captures hydrated refs + actions) ─────

#[derive(Clone, Default)]
struct RecordingActionDispatcher {
    calls: Arc<Mutex<Vec<RecordedDispatch>>>,
}

#[derive(Clone, Debug)]
struct RecordedDispatch {
    actions: Vec<HarnessAction>,
    resolved_refs: HashMap<ArtifactRef, serde_json::Value>,
}

#[async_trait]
impl ActionDispatcher for RecordingActionDispatcher {
    type Action = HarnessAction;
    type Context = HarnessActionContext;
    type Error = HarnessActionError;

    async fn dispatch(
        &self,
        actions: &ActionSeq<HarnessAction>,
        ctx: &HarnessActionContext,
    ) -> Result<ExecutionResult, HarnessActionError> {
        self.calls.lock().await.push(RecordedDispatch {
            actions: actions.iter().cloned().collect(),
            resolved_refs: ctx.resolved_refs.clone(),
        });
        Ok(ExecutionResult {
            entities_affected: actions.len(),
            summary: Some("recorded".to_string()),
            details: Some(json!({
                "resolvedActions": actions
                    .iter()
                    .map(|action| json!({
                        "type": action.kind,
                        "changeType": action
                            .payload
                            .get("change_type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("absolute"),
                        "value": action
                            .payload
                            .get("new_price")
                            .and_then(|value| value.as_f64())
                            .unwrap_or(0.0),
                        "entityIds": action
                            .payload
                            .get("product_id")
                            .and_then(|value| value.as_str())
                            .map(|id| vec![id.to_string()])
                            .unwrap_or_default(),
                    }))
                    .collect::<Vec<_>>()
            })),
        })
    }
}

// ─── Fixtures ──────────────────────────────────────────────────────────

fn spec_with_scenario_plan() -> RuntimeSpec {
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    let mut spec = RuntimeSpec {
        tenant: TenantIdentity::new("tenant-1", "v1"),
        agents: vec![
            AgentSpec::new(
                "coordinator",
                AgentRole::Coordinator,
                ModelSpec::new("claude-sonnet-4-6"),
                "You coordinate.",
            ),
            AgentSpec::new(
                "planner",
                AgentRole::Planner,
                ModelSpec::new("claude-sonnet-4-6"),
                "You plan.",
            ),
            AgentSpec::new(
                "executor",
                AgentRole::Executor,
                ModelSpec::new("claude-haiku-4-5"),
                "You execute.",
            ),
        ],
        references: vec![ReferenceSpec {
            name: "ProductSet".to_string(),
            schema: json!({"type": "object"}),
            ttl_ms: Some(60_000),
        }],
        plans: vec![PlanSpec {
            name: "ScenarioPlan".to_string(),
            schema: json!({"type": "object"}),
            display_aliases: vec![PlanDisplayAlias {
                status: PlanStatus::Draft,
                alias: "pending_approval".to_string(),
            }],
        }],
        toolkits: vec![],
        approval_policies: ApprovalPolicies {
            plans: ApprovalRule::Always,
            tools: ApprovalRule::Never,
        },
        approval_overrides: Default::default(),
        storage_factories: Default::default(),
        providers,
    };
    spec.agents[0].routes = vec!["planner".to_string(), "executor".to_string()];
    spec
}

fn deps_with_dispatcher(
    kv: Arc<DashMapKVStore>,
    action_dispatcher: Arc<RecordingActionDispatcher>,
) -> RuntimeDeps {
    // Cast the concrete recorder into the harness's trait-object alias.
    let dispatcher: Arc<flowai_runtime::HarnessActionDispatcher> = action_dispatcher;
    RuntimeDeps::new(
        Arc::new(NoopInterpreter),
        Arc::new(NullEventSink) as Arc<dyn EventSink>,
        TenantContext::new(TenantId::new_unchecked("tenant-1")),
        kv,
    )
    .with_action_dispatcher(dispatcher)
}

// ─── T5: unknown approval id ──────────────────────────────────────────

#[tokio::test]
async fn respond_to_approval_unknown_id_returns_not_found() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let runtime = Runtime::new(
        spec_with_scenario_plan(),
        deps_with_dispatcher(kv, recorder),
    )
    .expect("runtime builds");

    let err = runtime
        .respond_to_approval(ApprovalDecision {
            approval_id: "does-not-exist".to_string(),
            outcome: ApprovalOutcome::Approve,
            feedback: None,
            partial: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RuntimeError::ApprovalNotFound(ref s) if s == "does-not-exist"
    ));
}

// ─── T3 (slim): respond_to_approval resolves a registered awaiter ─────

#[tokio::test]
async fn respond_to_approval_resolves_pending_awaiter() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let runtime = Arc::new(
        Runtime::new(
            spec_with_scenario_plan(),
            deps_with_dispatcher(kv, recorder),
        )
        .expect("runtime builds"),
    );

    let req = ApprovalRequest {
        id: ApprovalId::new_unchecked("apr-T3"),
        kind: ApprovalKind::Tool,
        target: "tool-x".to_string(),
        payload: json!({}),
        glimpse: None,
        resource_id: TenantId::new_unchecked("tenant-1"),
        thread_id: ThreadId::new_unchecked("thread-1"),
        correlation_id: None,
    };
    let awaiter: ApprovalAwait = runtime
        .approval_store()
        .register(req)
        .await
        .expect("register pending");

    // Resolve from a different task; the awaiter must wake with Approve.
    let runtime_clone = runtime.clone();
    let resolver = tokio::spawn(async move {
        runtime_clone
            .respond_to_approval(ApprovalDecision {
                approval_id: "apr-T3".to_string(),
                outcome: ApprovalOutcome::Approve,
                feedback: None,
                partial: None,
            })
            .await
    });

    let decision = awaiter.await.expect("awaiter resolves with Ok");
    assert!(decision.outcome.is_approve());
    resolver
        .await
        .expect("join")
        .expect("respond_to_approval ok");
}

// ─── T6: AlreadyResolved on duplicate respond ─────────────────────────

#[tokio::test]
async fn respond_to_approval_twice_surfaces_already_resolved() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let runtime = Arc::new(
        Runtime::new(
            spec_with_scenario_plan(),
            deps_with_dispatcher(kv, recorder),
        )
        .expect("runtime builds"),
    );

    let req = ApprovalRequest {
        id: ApprovalId::new_unchecked("apr-T6"),
        kind: ApprovalKind::Tool,
        target: "tool-y".to_string(),
        payload: json!({}),
        glimpse: None,
        resource_id: TenantId::new_unchecked("tenant-1"),
        thread_id: ThreadId::new_unchecked("thread-1"),
        correlation_id: None,
    };
    let awaiter = runtime
        .approval_store()
        .register(req)
        .await
        .expect("register pending");

    // Drive the first respond_to_approval to completion before issuing
    // the second. Awaiting the awaiter alone is insufficient: the oneshot
    // fires inside `awaiters.resolve(...)` while `KvPendingApprovalStore`
    // still has KV writethrough + resolved-set updates queued. Joining
    // the spawn handle guarantees the store reached its terminal state
    // before we probe AlreadyResolved.
    let runtime_a = runtime.clone();
    let first = tokio::spawn(async move {
        runtime_a
            .respond_to_approval(ApprovalDecision {
                approval_id: "apr-T6".to_string(),
                outcome: ApprovalOutcome::Approve,
                feedback: None,
                partial: None,
            })
            .await
    });
    let _ = awaiter.await.expect("first decision lands");
    first
        .await
        .expect("join first respond_to_approval")
        .expect("first respond_to_approval should succeed");

    let second = runtime
        .respond_to_approval(ApprovalDecision {
            approval_id: "apr-T6".to_string(),
            outcome: ApprovalOutcome::Approve,
            feedback: None,
            partial: None,
        })
        .await;

    assert!(
        matches!(second, Err(RuntimeError::ApprovalAlreadyResolved(_))),
        "second respond_to_approval should error: {second:?}",
    );
}

// ─── T7: pre-approved plan dispatches with hydrated references ────────

#[tokio::test]
async fn execute_plan_dispatches_with_hydrated_refs_when_pre_approved() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let runtime = Runtime::new(
        spec_with_scenario_plan(),
        deps_with_dispatcher(kv.clone(), recorder.clone()),
    )
    .expect("runtime builds");
    let tenant = TenantId::new_unchecked("tenant-1");

    // 1. Create a reference that the plan will carry.
    let product_set = runtime
        .references()
        .create(
            "ProductSet",
            json!({"product_ids": ["a", "b"]}),
            json!({"n": 2}),
            &tenant,
        )
        .await
        .expect("create reference");

    // 2. Propose a plan whose action references that ProductSet.
    let plan_id = PlanId::new_unchecked("plan-T7");
    runtime
        .propose_plan(
            "ScenarioPlan",
            plan_id.clone(),
            json!({
                "actions": [{
                    "kind": "price_change",
                    "product_id": "a",
                    "new_price": 9.99,
                    "references": [{"kind": "ProductSet", "id": product_set.id}],
                }],
                "rationale": "test",
            }),
        )
        .await
        .expect("propose plan");

    // 3. Pre-approve the plan directly so executePlan bypasses the gate.
    //    `GatedPlanExecutor` short-circuits non-`Draft` plans (H6).
    let plan = runtime
        .plan(&plan_id)
        .await
        .expect("load plan")
        .expect("plan exists");
    let approved = plan
        .approve(agent_fw_core::UserId::new_unchecked("test"))
        .expect("approve transition");
    let kv_dyn: &dyn KVStore = kv.as_ref();
    let key = agent_fw_plan::persist::plan_key("plan", &plan_id);
    kv_dyn
        .put::<agent_fw_plan::Plan<HarnessAction>>(tenant.as_str(), &key, &approved, None)
        .await
        .expect("persist approved plan");

    // 4. Build an executor-agent ToolEnvironment with the PlanExecutionContext
    //    extension and dispatch executePlan.
    let env = ToolEnvironment::builder()
        .kv_arc(kv.clone() as Arc<dyn agent_fw_algebra::KVStore>)
        .event_sink_arc(Arc::new(NullEventSink) as Arc<dyn EventSink>)
        .tenant_context(
            TenantContext::new(tenant.clone()).with_thread(ThreadId::new_unchecked("thread-T7")),
        )
        .build()
        .with_ext::<PlanExecutionContext>(Arc::new(PlanExecutionContext {
            approval_policy: runtime.approval_policy_for("executor"),
            approval_store: runtime.approval_store().clone(),
            action_dispatcher: Arc::clone(&recorder)
                as Arc<flowai_runtime::HarnessActionDispatcher>,
            approver: agent_fw_core::UserId::new_unchecked("test"),
        }));

    let dispatcher = runtime
        .dispatcher_for("executor", env)
        .expect("compose ok")
        .expect("executor has dispatcher");
    let result = dispatcher
        .dispatch("executePlan", "tu-T7", json!({"planId": plan_id.as_str()}))
        .await;
    assert!(!result.is_error, "executePlan errored: {result:?}");
    assert_eq!(
        result.content["resolvedActions"], result.content["details"]["resolvedActions"],
        "executePlan should expose resolvedActions at the top level for eval projection",
    );
    assert_eq!(result.content["resolvedActions"][0]["type"], "price_change");
    assert_eq!(
        result.content["resolvedActions"][0]["entityIds"],
        json!(["a"])
    );

    // 5. Confirm the recording dispatcher saw the action AND the hydrated ref.
    let calls = recorder.calls.lock().await.clone();
    assert_eq!(calls.len(), 1, "executor should dispatch exactly once");
    let call = &calls[0];
    assert_eq!(call.actions.len(), 1);
    assert_eq!(call.actions[0].kind, "price_change");
    let hydrated = call
        .resolved_refs
        .get(&product_set)
        .expect("ProductSet ref must be hydrated before dispatch");
    assert_eq!(hydrated, &json!({"product_ids": ["a", "b"]}));
}

#[tokio::test]
async fn execute_plan_agent_plan_override_never_skips_gate_and_runs_immediately() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let mut spec = spec_with_scenario_plan();
    spec.approval_overrides.agents.insert(
        "executor".to_string(),
        ApprovalPolicyPatch {
            plans: Some(ApprovalRule::Never),
            tools: None,
        },
    );
    let runtime = Runtime::new(spec, deps_with_dispatcher(kv.clone(), recorder.clone()))
        .expect("runtime builds");
    let tenant = TenantId::new_unchecked("tenant-1");

    let product_set = runtime
        .references()
        .create(
            "ProductSet",
            json!({"product_ids": ["a", "b"]}),
            json!({"n": 2}),
            &tenant,
        )
        .await
        .expect("create reference");
    let plan_id = PlanId::new_unchecked("plan-no-gate");
    runtime
        .propose_plan(
            "ScenarioPlan",
            plan_id.clone(),
            json!({
                "actions": [{
                    "kind": "price_change",
                    "product_id": "a",
                    "new_price": 9.99,
                    "references": [{"kind": "ProductSet", "id": product_set.id}],
                }],
                "rationale": "no gate test",
            }),
        )
        .await
        .expect("propose plan");

    let recorder_sink = Arc::new(RecordingEventSink::new());
    let env = ToolEnvironment::builder()
        .kv_arc(kv.clone() as Arc<dyn KVStore>)
        .event_sink_arc(recorder_sink.clone() as Arc<dyn EventSink>)
        .tenant_context(
            TenantContext::new(tenant.clone())
                .with_thread(ThreadId::new_unchecked("thread-no-gate")),
        )
        .build()
        .with_ext::<PlanExecutionContext>(Arc::new(PlanExecutionContext {
            approval_policy: runtime.approval_policy_for("executor"),
            approval_store: runtime.approval_store().clone(),
            action_dispatcher: Arc::clone(&recorder)
                as Arc<flowai_runtime::HarnessActionDispatcher>,
            approver: agent_fw_core::UserId::new_unchecked("test"),
        }));

    let dispatcher = runtime
        .dispatcher_for("executor", env)
        .expect("compose ok")
        .expect("executor has dispatcher");
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        dispatcher.dispatch(
            "executePlan",
            "tu-no-gate",
            json!({"planId": plan_id.as_str()}),
        ),
    )
    .await
    .expect("executePlan should not block on approval");
    assert!(!result.is_error, "executePlan errored: {result:?}");

    let calls = recorder.calls.lock().await.clone();
    assert_eq!(calls.len(), 1, "executor should dispatch exactly once");
    assert!(
        !recorder_sink
            .events()
            .iter()
            .any(|ev| matches!(ev, StreamPart::ApprovalRequired { .. })),
        "plan approval override should skip approval_required",
    );
}

#[tokio::test]
async fn execute_plan_auto_approval_fails_if_status_transition_event_is_dropped() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let mut spec = spec_with_scenario_plan();
    spec.approval_overrides.agents.insert(
        "executor".to_string(),
        ApprovalPolicyPatch {
            plans: Some(ApprovalRule::Never),
            tools: None,
        },
    );
    let runtime = Runtime::new(spec, deps_with_dispatcher(kv.clone(), recorder.clone()))
        .expect("runtime builds");
    let tenant = TenantId::new_unchecked("tenant-1");

    let product_set = runtime
        .references()
        .create(
            "ProductSet",
            json!({"product_ids": ["a", "b"]}),
            json!({"n": 2}),
            &tenant,
        )
        .await
        .expect("create reference");
    let plan_id = PlanId::new_unchecked("plan-no-gate-closed-sink");
    runtime
        .propose_plan(
            "ScenarioPlan",
            plan_id.clone(),
            json!({
                "actions": [{
                    "kind": "price_change",
                    "product_id": "a",
                    "new_price": 9.99,
                    "references": [{"kind": "ProductSet", "id": product_set.id}],
                }],
                "rationale": "closed sink test",
            }),
        )
        .await
        .expect("propose plan");

    let env = ToolEnvironment::builder()
        .kv_arc(kv.clone() as Arc<dyn KVStore>)
        .event_sink_arc(Arc::new(ClosedEventSink) as Arc<dyn EventSink>)
        .tenant_context(
            TenantContext::new(tenant.clone())
                .with_thread(ThreadId::new_unchecked("thread-closed-sink")),
        )
        .build()
        .with_ext::<PlanExecutionContext>(Arc::new(PlanExecutionContext {
            approval_policy: runtime.approval_policy_for("executor"),
            approval_store: runtime.approval_store().clone(),
            action_dispatcher: Arc::clone(&recorder)
                as Arc<flowai_runtime::HarnessActionDispatcher>,
            approver: agent_fw_core::UserId::new_unchecked("test"),
        }));

    let dispatcher = runtime
        .dispatcher_for("executor", env)
        .expect("compose ok")
        .expect("executor has dispatcher");
    let result = dispatcher
        .dispatch(
            "executePlan",
            "tu-closed-sink",
            json!({"planId": plan_id.as_str()}),
        )
        .await;
    assert!(result.is_error, "executePlan should error: {result:?}");
    assert!(
        result
            .content
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .contains("Failed to emit plan status transition"),
        "unexpected error content: {:?}",
        result.content
    );
    assert!(
        recorder.calls.lock().await.is_empty(),
        "execution must not continue after dropping the auto-approval status transition"
    );
}

// ─── T-gate: full Draft → respond_to_approval → executor runs flow ────

#[tokio::test]
async fn execute_plan_blocks_on_gate_and_resumes_after_respond_to_approval() {
    let kv = Arc::new(DashMapKVStore::new());
    let recorder = Arc::new(RecordingActionDispatcher::default());
    let runtime = Arc::new(
        Runtime::new(
            spec_with_scenario_plan(),
            deps_with_dispatcher(kv.clone(), recorder.clone()),
        )
        .expect("runtime builds"),
    );
    let tenant = TenantId::new_unchecked("tenant-1");

    // 1. Propose a Draft plan whose action references a real ProductSet.
    let product_set = runtime
        .references()
        .create(
            "ProductSet",
            json!({"product_ids": ["a", "b"]}),
            json!({"n": 2}),
            &tenant,
        )
        .await
        .expect("create reference");
    let plan_id = PlanId::new_unchecked("plan-gate");
    runtime
        .propose_plan(
            "ScenarioPlan",
            plan_id.clone(),
            json!({
                "actions": [{
                    "kind": "price_change",
                    "product_id": "a",
                    "new_price": 9.99,
                    "references": [{"kind": "ProductSet", "id": product_set.id}],
                }],
                "rationale": "gate test",
            }),
        )
        .await
        .expect("propose plan");

    // 2. Wire a recording event sink onto the executor's env so we can
    //    extract the approval id the GatedPlanExecutor emits.
    let recorder_sink = Arc::new(RecordingEventSink::new());
    let env = ToolEnvironment::builder()
        .kv_arc(kv.clone() as Arc<dyn KVStore>)
        .event_sink_arc(recorder_sink.clone() as Arc<dyn EventSink>)
        .tenant_context(
            TenantContext::new(tenant.clone()).with_thread(ThreadId::new_unchecked("thread-gate")),
        )
        .build()
        .with_ext::<PlanExecutionContext>(Arc::new(PlanExecutionContext {
            approval_policy: runtime.approval_policy_for("executor"),
            approval_store: runtime.approval_store().clone(),
            action_dispatcher: Arc::clone(&recorder)
                as Arc<flowai_runtime::HarnessActionDispatcher>,
            approver: agent_fw_core::UserId::new_unchecked("test"),
        }));

    let dispatcher = runtime
        .dispatcher_for("executor", env)
        .expect("compose ok")
        .expect("executor has dispatcher");

    // 3. Spawn the executePlan dispatch on a task — it will block on the
    //    GatedPlanExecutor until we resolve the approval.
    let recorder_calls = recorder.calls.clone();
    let plan_id_str = plan_id.as_str().to_string();
    let dispatch_task = tokio::spawn(async move {
        dispatcher
            .dispatch("executePlan", "tu-gate", json!({"planId": plan_id_str}))
            .await
    });

    // 4. Poll the event sink for an `approval_required` event with the plan
    //    id. The framework gate emits it before the awaiter blocks.
    let approval_id = poll_approval_id(&recorder_sink).await;

    // 5. Counter-tool invariant: the customer action dispatcher must NOT
    //    have run yet — the inner executor is gated behind the approval.
    assert!(
        recorder_calls.lock().await.is_empty(),
        "action dispatcher must stay quiet while approval is pending",
    );

    // 6. Resolve the approval. The gate transitions Draft → Approved and
    //    invokes the HydratingDispatcher → recording action dispatcher.
    runtime
        .respond_to_approval(ApprovalDecision {
            approval_id,
            outcome: ApprovalOutcome::Approve,
            feedback: None,
            partial: None,
        })
        .await
        .expect("respond_to_approval should resolve the awaiter");

    // 7. The dispatch task now completes successfully.
    let result = dispatch_task.await.expect("dispatch task join");
    assert!(!result.is_error, "executePlan errored: {result:?}");

    // 8. Confirm the customer action dispatcher saw the hydrated ref.
    let calls = recorder_calls.lock().await.clone();
    assert_eq!(calls.len(), 1, "executor should dispatch exactly once");
    let hydrated = calls[0]
        .resolved_refs
        .get(&product_set)
        .expect("ProductSet ref must be hydrated before dispatch");
    assert_eq!(hydrated, &json!({"product_ids": ["a", "b"]}));
}

/// Poll the recording sink for the next `ApprovalRequired` event and return
/// its id. Fails fast after a short timeout so a misconfigured fixture
/// doesn't hang CI indefinitely.
async fn poll_approval_id(sink: &Arc<RecordingEventSink>) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        for ev in sink.events() {
            if let StreamPart::ApprovalRequired { data } = ev {
                return data.id.as_str().to_string();
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for ApprovalRequired event");
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// ─── T12: RuntimeSpec JSON round-trip parity ───────────────────────────

#[test]
fn runtime_spec_round_trips_through_json() {
    let spec = spec_with_scenario_plan();
    let value = serde_json::to_value(&spec).expect("serialise");
    let back: RuntimeSpec = serde_json::from_value(value.clone()).expect("deserialise");
    assert_eq!(back, spec, "RuntimeSpec must round-trip losslessly");

    // The canonical fixture also serialises to a known shape — host
    // libraries (Python, TypeScript) re-check this byte-for-byte.
    assert!(value["providers"]["anthropic"].is_object());
    assert_eq!(
        value["plans"][0]["displayAliases"][0]["alias"],
        "pending_approval"
    );
}

// ─── T-callAgent: coordinator delegates to sub-agent via call_agent ───
//
// End-to-end proof that the runtime wires the framework's
// `CallAgentHandler` correctly:
//
// 1. The runtime declares an `agents` toolkit on the coordinator only.
// 2. A scripted interpreter for the coordinator emits a single
//    `call_agent(agent: "planner", prompt: ...)` tool invocation.
// 3. The framework's `CallAgentHandler` reads `env.sub_agents()` — which
//    the runtime installs as the late-bound `Arc<AgentOrchestrator>` — and
//    invokes the planner sub-agent.
// 4. The orchestrator's `SubAgentInvoker::invoke` emits the planner's
//    `sub_agent_call` / `sub_agent_result` framing on the stream.
// 5. The scripted interpreter for the planner just finishes (no further
//    delegation), so control returns to the coordinator, which finishes.
//
// Asserting both `sub_agent_call(coordinator)` AND `sub_agent_call(planner)`
// appear in the stream proves the late-bound wiring resolves correctly.

use agent_fw_agent::ToolCallResult;
use futures::StreamExt;
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

/// Per-call action a `ScriptedInterpreter` performs. The script is
/// shared across all per-agent interpreter clones (one per agent, set
/// by G1's `with_tool_dispatcher`), and consumed in FIFO order — so the
/// test queues actions in the order the orchestrator will invoke agents.
enum ScriptAction {
    /// Dispatch a tool through the interpreter's attached dispatcher,
    /// then emit `Finish`. Mirrors the LLM-tool-use shape without
    /// requiring a real Rig interpreter.
    CallTool {
        tool_name: String,
        args: serde_json::Value,
    },
    /// Just emit `StepStart` + `Finish`. Used for sub-agents that
    /// shouldn't do anything beyond returning.
    Finish,
}

/// Minimal `ChatInterpreter` that pulls one `ScriptAction` per invoke
/// from a shared queue. Overrides `with_tool_dispatcher` so the
/// orchestrator's G1 dispatcher attachment works the same way it does
/// for the real Rig interpreters.
struct ScriptedInterpreter {
    actions: Arc<StdMutex<VecDeque<ScriptAction>>>,
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl ScriptedInterpreter {
    fn new(actions: Vec<ScriptAction>) -> Self {
        Self {
            actions: Arc::new(StdMutex::new(VecDeque::from(actions))),
            dispatcher: None,
        }
    }
}

impl ChatInterpreter for ScriptedInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        // Spawn the dispatch off-thread and stream the two framing
        // events through an mpsc receiver. Keeps the stream-yielding
        // shape but lets us `.await` the dispatcher between yields
        // without pulling in `async-stream`.
        let action = self.actions.lock().expect("not poisoned").pop_front();
        let dispatcher = self.dispatcher.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamPart>(4);
        tokio::spawn(async move {
            let _ = tx.send(StreamPart::StepStart).await;
            if let Some(ScriptAction::CallTool { tool_name, args }) = action {
                if let Some(d) = dispatcher {
                    let _: ToolCallResult = d.dispatch(&tool_name, "tu-script", args).await;
                }
            }
            let _ = tx
                .send(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO))
                .await;
        });
        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        // Clone the shared action queue Arc so all per-agent clones pull
        // from the same FIFO — this is what makes the scripted
        // coordinator→planner sequence deterministic.
        Some(Arc::new(ScriptedInterpreter {
            actions: self.actions.clone(),
            dispatcher: Some(dispatcher),
        }))
    }
}

#[derive(Clone, Debug)]
struct ProviderInterpreterCall {
    provider: String,
    model: String,
    tool_names: Vec<String>,
}

/// Records which provider-specific interpreter executed a program and which
/// per-agent dispatcher was attached to that interpreter.
struct ProviderRecordingInterpreter {
    provider: String,
    calls: Arc<StdMutex<Vec<ProviderInterpreterCall>>>,
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl ProviderRecordingInterpreter {
    fn new(
        provider: impl Into<String>,
        calls: Arc<StdMutex<Vec<ProviderInterpreterCall>>>,
    ) -> Self {
        Self {
            provider: provider.into(),
            calls,
            dispatcher: None,
        }
    }
}

impl ChatInterpreter for ProviderRecordingInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let tool_names = self
            .dispatcher
            .as_ref()
            .map(|dispatcher| {
                dispatcher
                    .tool_definitions()
                    .into_iter()
                    .map(|definition| definition.name)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.calls
            .lock()
            .expect("not poisoned")
            .push(ProviderInterpreterCall {
                provider: self.provider.clone(),
                model: program.model().as_str().to_string(),
                tool_names,
            });
        Box::pin(stream::iter(vec![
            StreamPart::StepStart,
            StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
        ]))
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        Some(Arc::new(Self {
            provider: self.provider.clone(),
            calls: self.calls.clone(),
            dispatcher: Some(dispatcher),
        }))
    }
}

fn mixed_provider_specialist_spec() -> RuntimeSpec {
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    providers.insert(
        "bedrock".to_string(),
        ProviderConfig::new(json!({"region": "us-east-1"})),
    );
    providers.insert(
        "openai-compatible".to_string(),
        ProviderConfig::new(json!({"baseUrl": "https://api.openai.test/v1"})),
    );

    let mut anthropic_agent = AgentSpec::new(
        "anthropic_agent",
        AgentRole::Specialist,
        ModelSpec::new("claude-sonnet-4-6"),
        "Use Anthropic.",
    );
    anthropic_agent.toolkits = vec!["agents".to_string()];

    let mut bedrock_agent = AgentSpec::new(
        "bedrock_agent",
        AgentRole::Specialist,
        ModelSpec::new("anthropic.claude-3-5-sonnet-20240620-v1:0"),
        "Use Bedrock.",
    );
    bedrock_agent.toolkits = vec!["references".to_string()];

    let mut openai_agent = AgentSpec::new(
        "openai_agent",
        AgentRole::Specialist,
        ModelSpec::new("gpt-4o-mini"),
        "Use OpenAI-compatible.",
    );
    openai_agent.toolkits = vec!["plans".to_string()];

    RuntimeSpec {
        tenant: TenantIdentity::new("tenant-1", "v1"),
        agents: vec![anthropic_agent, bedrock_agent, openai_agent],
        references: vec![ReferenceSpec {
            name: "ProductSet".to_string(),
            schema: json!({"type": "object"}),
            ttl_ms: None,
        }],
        plans: vec![PlanSpec {
            name: "ScenarioPlan".to_string(),
            schema: json!({"type": "object"}),
            display_aliases: vec![],
        }],
        toolkits: vec![
            ToolkitSpec {
                id: "agents".to_string(),
                config: serde_json::Value::Null,
            },
            ToolkitSpec {
                id: "references".to_string(),
                config: serde_json::Value::Null,
            },
            ToolkitSpec {
                id: "plans".to_string(),
                config: serde_json::Value::Null,
            },
        ],
        approval_policies: ApprovalPolicies {
            plans: ApprovalRule::Never,
            tools: ApprovalRule::Never,
        },
        approval_overrides: Default::default(),
        storage_factories: Default::default(),
        providers,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_routes_agents_to_provider_interpreters_and_keeps_their_dispatchers() {
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let kv = Arc::new(DashMapKVStore::new());
    let deps = RuntimeDeps::new(
        Arc::new(ProviderRecordingInterpreter::new(
            "anthropic",
            calls.clone(),
        )),
        Arc::new(NullEventSink) as Arc<dyn EventSink>,
        TenantContext::new(TenantId::new_unchecked("tenant-1")),
        kv,
    )
    .with_interpreter_provider(
        "bedrock",
        Arc::new(ProviderRecordingInterpreter::new("bedrock", calls.clone())),
    )
    .with_interpreter_provider(
        "openai-compatible",
        Arc::new(ProviderRecordingInterpreter::new(
            "openai-compatible",
            calls.clone(),
        )),
    );
    let runtime = Runtime::new(mixed_provider_specialist_spec(), deps).expect("runtime builds");

    for specialist in ["anthropic_agent", "bedrock_agent", "openai_agent"] {
        let parts = runtime
            .run_specialist(flowai_runtime::SpecialistRequest {
                specialist: specialist.to_string(),
                prompt: "hello".to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id: Some(ThreadId::new_unchecked(format!("thread-{specialist}"))),
            })
            .collect::<Vec<_>>()
            .await;
        assert!(
            parts
                .iter()
                .all(|part| !matches!(part, StreamPart::Error { .. })),
            "{specialist} emitted an error stream: {parts:?}",
        );
    }

    let by_model: HashMap<String, ProviderInterpreterCall> = calls
        .lock()
        .expect("not poisoned")
        .iter()
        .cloned()
        .map(|call| (call.model.clone(), call))
        .collect();

    let anthropic = by_model
        .get("claude-sonnet-4-6")
        .expect("anthropic call recorded");
    assert_eq!(anthropic.provider, "anthropic");
    assert!(anthropic.tool_names.iter().any(|name| name == "call_agent"));

    let bedrock = by_model
        .get("anthropic.claude-3-5-sonnet-20240620-v1:0")
        .expect("bedrock call recorded");
    assert_eq!(bedrock.provider, "bedrock");
    assert!(bedrock.tool_names.iter().any(|name| name == "resolveRef"));

    let openai = by_model
        .get("gpt-4o-mini")
        .expect("openai-compatible call recorded");
    assert_eq!(openai.provider, "openai-compatible");
    assert!(openai.tool_names.iter().any(|name| name == "getPlan"));
    assert!(!openai.tool_names.iter().any(|name| name == "storePlan"));
}

#[test]
fn runtime_new_rejects_declared_provider_without_registered_interpreter() {
    let mut spec = mixed_provider_specialist_spec();
    spec.agents = vec![spec
        .agents
        .into_iter()
        .find(|agent| agent.name == "openai_agent")
        .expect("openai fixture agent")];

    let err = Runtime::new(
        spec,
        RuntimeDeps::new(
            Arc::new(NoopInterpreter),
            Arc::new(NullEventSink) as Arc<dyn EventSink>,
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            Arc::new(DashMapKVStore::new()),
        ),
    )
    .err()
    .expect("expected missing provider interpreter");

    assert!(matches!(
        err,
        RuntimeError::ProviderInterpreterMissing {
            ref agent,
            ref provider,
            ref model,
        } if agent == "openai_agent" && provider == "openai-compatible" && model == "gpt-4o-mini"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_call_agent_delegates_to_planner_via_runtime() {
    // 1. Spec: coordinator (with `agents` toolkit) + planner (no
    //    toolkit). The planner doesn't need any tool — it just needs to
    //    be reachable via call_agent.
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    let mut coordinator = AgentSpec::new(
        "coordinator",
        AgentRole::Coordinator,
        ModelSpec::new("claude-sonnet-4-6"),
        "You coordinate.",
    );
    coordinator.toolkits = vec!["agents".to_string()];
    coordinator.routes = vec!["planner".to_string()];
    let planner = AgentSpec::new(
        "planner",
        AgentRole::Planner,
        ModelSpec::new("claude-sonnet-4-6"),
        "You plan.",
    );

    let spec = RuntimeSpec {
        tenant: TenantIdentity::new("tenant-1", "v1"),
        agents: vec![coordinator, planner],
        references: vec![],
        plans: vec![],
        toolkits: vec![ToolkitSpec {
            id: "agents".to_string(),
            config: serde_json::Value::Null,
        }],
        approval_policies: ApprovalPolicies {
            plans: ApprovalRule::Never,
            tools: ApprovalRule::Never,
        },
        approval_overrides: Default::default(),
        storage_factories: Default::default(),
        providers,
    };

    // 2. Scripted interpreter: coordinator emits a call_agent to
    //    planner, planner just finishes. Order matters — the queue is
    //    FIFO and the orchestrator invokes the coordinator first.
    let interpreter = Arc::new(ScriptedInterpreter::new(vec![
        ScriptAction::CallTool {
            tool_name: "call_agent".to_string(),
            args: json!({"agent": "planner", "prompt": "build a plan"}),
        },
        ScriptAction::Finish,
    ]));

    let kv = Arc::new(DashMapKVStore::new());
    let deps = RuntimeDeps::new(
        interpreter,
        Arc::new(NullEventSink) as Arc<dyn EventSink>,
        TenantContext::new(TenantId::new_unchecked("tenant-1")),
        kv,
    );

    let runtime = Runtime::new(spec, deps).expect("runtime builds");

    // 3. Drive the coordinator. The stream should contain BOTH
    //    sub_agent_call(coordinator) and sub_agent_call(planner) —
    //    proving the late-bound invoker resolved through the orchestrator.
    //    Collect with a 3s overall timeout and a 1s per-item timeout so a
    //    wiring regression fails fast with a diagnostic of what arrived.
    let mut stream = runtime.query(flowai_runtime::QueryRequest {
        prompt: "Plan something for me".to_string(),
        resource_id: TenantId::new_unchecked("tenant-1"),
        thread_id: ThreadId::new_unchecked("thread-1"),
        resume: None,
    });
    let mut parts: Vec<StreamPart> = Vec::new();
    let overall_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let remaining = overall_deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            panic!("overall 3s timeout — stream hung after collecting parts: {parts:?}",);
        }
        let next = tokio::time::timeout(
            remaining.min(std::time::Duration::from_secs(1)),
            stream.next(),
        )
        .await;
        match next {
            Ok(Some(part)) => parts.push(part),
            Ok(None) => break,
            Err(_) => panic!(
                "per-step timeout — stream stalled with {} parts collected: {parts:?}",
                parts.len()
            ),
        }
    }

    let names: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolAgent(data) => Some(data.agent_name.clone()),
            _ => None,
        })
        .collect();

    assert!(
        names.iter().any(|n| n == "coordinator"),
        "stream missing coordinator framing: {names:?}",
    );
    assert!(
        names.iter().any(|n| n == "planner"),
        "stream missing planner framing — call_agent didn't reach the orchestrator. \
         Got: {names:?}. parts: {parts:?}",
    );
}

#[derive(Clone, Debug)]
struct ObservedProgram {
    thread_id: Option<String>,
    messages: Vec<(String, String)>,
}

struct StatefulProbeInterpreter {
    actions: Arc<StdMutex<VecDeque<ScriptAction>>>,
    observed: Arc<StdMutex<Vec<ObservedProgram>>>,
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl StatefulProbeInterpreter {
    fn new(actions: Vec<ScriptAction>, observed: Arc<StdMutex<Vec<ObservedProgram>>>) -> Self {
        Self {
            actions: Arc::new(StdMutex::new(VecDeque::from(actions))),
            observed,
            dispatcher: None,
        }
    }
}

impl ChatInterpreter for StatefulProbeInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let messages = program
            .conversation()
            .messages()
            .iter()
            .map(|message| (format!("{:?}", message.role), message.content.clone()))
            .collect::<Vec<_>>();
        let reply = messages
            .iter()
            .rev()
            .find_map(|(role, content)| {
                if role == "User" {
                    Some(format!("reply: {content}"))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "reply".to_string());
        self.observed
            .lock()
            .expect("not poisoned")
            .push(ObservedProgram {
                thread_id: program
                    .tenant()
                    .thread_id()
                    .map(|thread| thread.as_str().to_string()),
                messages,
            });

        let action = self.actions.lock().expect("not poisoned").pop_front();
        let dispatcher = self.dispatcher.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamPart>(8);
        tokio::spawn(async move {
            let _ = tx.send(StreamPart::StepStart).await;
            if let Some(ScriptAction::CallTool { tool_name, args }) = action {
                if let Some(d) = dispatcher {
                    let _: ToolCallResult = d.dispatch(&tool_name, "tu-script", args).await;
                }
            }
            let _ = tx.send(StreamPart::text(reply)).await;
            let _ = tx
                .send(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO))
                .await;
        });
        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        Some(Arc::new(Self {
            actions: self.actions.clone(),
            observed: self.observed.clone(),
            dispatcher: Some(dispatcher),
        }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stateful_planner_loads_child_thread_history_across_runtime_queries() {
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    let mut coordinator = AgentSpec::new(
        "coordinator",
        AgentRole::Coordinator,
        ModelSpec::new("claude-sonnet-4-6"),
        "You coordinate.",
    );
    coordinator.toolkits = vec!["agents".to_string()];
    coordinator.routes = vec!["planner".to_string()];
    let planner = AgentSpec::new(
        "planner",
        AgentRole::Planner,
        ModelSpec::new("claude-sonnet-4-6"),
        "You plan.",
    );

    let spec = RuntimeSpec {
        tenant: TenantIdentity::new("tenant-1", "v1"),
        agents: vec![coordinator, planner],
        references: vec![],
        plans: vec![],
        toolkits: vec![ToolkitSpec {
            id: "agents".to_string(),
            config: serde_json::Value::Null,
        }],
        approval_policies: ApprovalPolicies {
            plans: ApprovalRule::Never,
            tools: ApprovalRule::Never,
        },
        approval_overrides: Default::default(),
        storage_factories: Default::default(),
        providers,
    };

    let observed = Arc::new(StdMutex::new(Vec::new()));
    let interpreter = Arc::new(StatefulProbeInterpreter::new(
        vec![
            ScriptAction::CallTool {
                tool_name: "call_agent".to_string(),
                args: json!({"agent": "planner", "prompt": "build first"}),
            },
            ScriptAction::Finish,
            ScriptAction::CallTool {
                tool_name: "call_agent".to_string(),
                args: json!({"agent": "planner", "prompt": "build second"}),
            },
            ScriptAction::Finish,
        ],
        observed.clone(),
    ));
    let runtime = Runtime::new(
        spec,
        RuntimeDeps::new(
            interpreter,
            Arc::new(NullEventSink) as Arc<dyn EventSink>,
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            Arc::new(DashMapKVStore::new()),
        ),
    )
    .expect("runtime builds");

    for prompt in ["first query", "second query"] {
        let _: Vec<StreamPart> = runtime
            .query(flowai_runtime::QueryRequest {
                prompt: prompt.to_string(),
                resource_id: TenantId::new_unchecked("tenant-1"),
                thread_id: ThreadId::new_unchecked("thread-1"),
                resume: None,
            })
            .collect()
            .await;
    }

    let observed = observed.lock().expect("not poisoned").clone();
    assert_eq!(
        observed
            .iter()
            .map(|program| program.thread_id.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("thread-1"),
            Some("thread-1-planner"),
            Some("thread-1"),
            Some("thread-1-planner"),
        ]
    );
    assert_eq!(
        observed[1].messages,
        vec![
            ("System".to_string(), "You plan.".to_string()),
            ("User".to_string(), "build first".to_string()),
        ]
    );
    assert_eq!(
        observed[3].messages,
        vec![
            ("System".to_string(), "You plan.".to_string()),
            ("User".to_string(), "build first".to_string()),
            ("Assistant".to_string(), "reply: build first".to_string()),
            ("User".to_string(), "build second".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_toolkit_blocks_targets_outside_declared_routes() {
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderConfig::new(json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
    );
    let mut coordinator = AgentSpec::new(
        "coordinator",
        AgentRole::Coordinator,
        ModelSpec::new("claude-sonnet-4-6"),
        "You coordinate.",
    );
    coordinator.toolkits = vec!["agents".to_string()];
    coordinator.routes = vec!["planner".to_string()];
    let planner = AgentSpec::new(
        "planner",
        AgentRole::Planner,
        ModelSpec::new("claude-sonnet-4-6"),
        "You plan.",
    );
    let executor = AgentSpec::new(
        "executor",
        AgentRole::Executor,
        ModelSpec::new("claude-sonnet-4-6"),
        "You execute.",
    );

    let spec = RuntimeSpec {
        tenant: TenantIdentity::new("tenant-1", "v1"),
        agents: vec![coordinator, planner, executor],
        references: vec![],
        plans: vec![],
        toolkits: vec![ToolkitSpec {
            id: "agents".to_string(),
            config: serde_json::Value::Null,
        }],
        approval_policies: ApprovalPolicies {
            plans: ApprovalRule::Never,
            tools: ApprovalRule::Never,
        },
        approval_overrides: Default::default(),
        storage_factories: Default::default(),
        providers,
    };

    let interpreter = Arc::new(ScriptedInterpreter::new(vec![ScriptAction::CallTool {
        tool_name: "call_agent".to_string(),
        args: json!({"agent": "executor", "prompt": "execute anyway"}),
    }]));

    let runtime = Runtime::new(
        spec,
        RuntimeDeps::new(
            interpreter,
            Arc::new(NullEventSink) as Arc<dyn EventSink>,
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            Arc::new(DashMapKVStore::new()),
        ),
    )
    .expect("runtime builds");

    let parts: Vec<StreamPart> = runtime
        .query(flowai_runtime::QueryRequest {
            prompt: "Plan something for me".to_string(),
            resource_id: TenantId::new_unchecked("tenant-1"),
            thread_id: ThreadId::new_unchecked("thread-1"),
            resume: None,
        })
        .collect()
        .await;

    let names: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolAgent(data) => Some(data.agent_name.clone()),
            _ => None,
        })
        .collect();

    assert!(
        names.iter().any(|n| n == "coordinator"),
        "coordinator should still run: {names:?}",
    );
    assert!(
        !names.iter().any(|n| n == "executor"),
        "executor must not run when it is outside coordinator.routes: {names:?}; parts: {parts:?}",
    );
}

#[tokio::test]
async fn eval_plan_assembly_uses_harness_injected_scorer_end_to_end() {
    let orchestrator = EvalOrchestrator::new(
        Arc::new(ScriptedEvalExecutor),
        Arc::new(StandardAggregator),
        Arc::new(EvalEventBus::new(32)),
        Arc::new(DashMapKVStore::new()),
        CancellationToken::new(),
        agent_fw_algebra::PauseToken::new(),
    );

    let config = EvalConfig {
        mode: EvalMode::Sequential,
        test_case_source: TestCaseSource::Set("set-c4".into()),
        samples_per_case: 1,
        pass_threshold: 0.5,
        concurrency: 1,
        k_values: vec![1],
        timeout_per_sample_secs: Some(30),
        ..Default::default()
    };
    let validated = ValidatedEvalConfig::validate(config.clone()).expect("config validates");
    let scorer =
        flowai_runtime::eval_scorer_for_mode(EvalMode::Sequential, None).expect("scorer builds");

    let summary = orchestrator
        .run(EvalPlan {
            run: EvalRun::new(config),
            test_cases: vec![EvalTestCase {
                id: TestCaseId::new_unchecked("tc-c4-eval"),
                tags: vec![],
                input: "raise online price".into(),
                expected_trajectory: vec!["draft_plan".into()],
                trajectory_mode: TrajectoryMode::Unordered,
                ground_truth: Some(GroundTruth::structured(serde_json::json!({
                    "kind": "flat",
                    "executedActions": [
                        {
                            "type": "price_change",
                            "payload": {
                                "changeType": "absolute",
                                "value": 10.0,
                                "productIds": ["sku-1"],
                                "context": { "channels": ["ONLINE"] }
                            }
                        }
                    ]
                }))),
                final_response: None,
                source_thread_id: None,
            }],
            scorer,
            model_config: ResolvedModelConfig {
                provider: "test".into(),
                model: "stub".into(),
            },
            config: validated,
            tenant: TenantId::new_unchecked("tenant-1"),
        })
        .await
        .expect("eval run succeeds");

    match summary.status {
        EvalStatus::Completed { summary } => {
            assert_eq!(summary.total_test_cases, 1);
            assert_eq!(summary.passed, 1);
            assert_eq!(summary.aggregate_score, 1.0);
        }
        other => panic!("expected completed status, got {other:?}"),
    }
}
