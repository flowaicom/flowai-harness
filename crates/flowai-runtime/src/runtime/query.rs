//! `Runtime::query` and `Runtime::run_specialist` driver (runtime query assembly C4).
//!
//! The driver assembles framework primitives — no orchestration logic is
//! reimplemented in the harness:
//!
//! 1. [`ChannelEventSink`](agent_fw_interpreter::ChannelEventSink) for the
//!    push → pull stream bridge.
//! 2. Per-agent [`ToolEnvironment`] composed via
//!    [`Runtime::dispatcher_for`](crate::Runtime::dispatcher_for) (default toolkit composition)
//!    then wrapped in [`ApprovalLayer`](agent_fw_agent::ApprovalLayer)
//!    using the shared `PendingApprovalStore` + compiled `ApprovalPolicy`.
//! 3. [`AgentOrchestrator`](agent_fw_agent::AgentOrchestrator) built with
//!    [`interpreters_per_agent`](agent_fw_agent::AgentOrchestratorBuilder::interpreters_per_agent)
//!    and
//!    [`dispatchers_per_agent`](agent_fw_agent::AgentOrchestratorBuilder::dispatchers_per_agent)
//!    so each sub-agent invocation sees its provider-specific interpreter and
//!    its own dispatcher.
//! 4. The orchestrator's `SubAgentInvoker::invoke` handles everything else:
//!    sub-agent thread-id derivation (G2), event emission, usage
//!    accounting, the streaming chat loop.

use std::collections::HashMap;
use std::sync::Arc;

use agent_fw_agent::approval::{ApprovalPolicy, ApprovalRule};
use agent_fw_algebra::sub_agent::{SubAgentInvoker, SubAgentRequest};
use agent_fw_algebra::{CancellationToken, EventSink};
use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogRef, CatalogToolEnvironmentExt, DataCatalog,
    JoinPath,
};
use agent_fw_core::StreamPart;
use agent_fw_interpreter::{ChannelEventSink, ErrorTargetDatabase};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;

use crate::runtime::invoker::LateBoundInvoker;
use crate::{
    AgentRole, AgentSpec, CancellableRuntimeEventStream, QueryRequest, Runtime, RuntimeError,
    SpecialistRequest,
};

#[derive(Clone, Default)]
struct RequestExecutionOptions {
    approval_policy_override: Option<Arc<ApprovalPolicy>>,
    deterministic_invocation_ids: bool,
}

impl RequestExecutionOptions {
    fn eval() -> Self {
        Self {
            approval_policy_override: Some(eval_bypass_approval_policy()),
            deterministic_invocation_ids: true,
        }
    }

    fn approval_policy_for(&self, runtime: &Runtime, agent: &str) -> Arc<ApprovalPolicy> {
        self.approval_policy_override
            .clone()
            .unwrap_or_else(|| runtime.approval_policy_for(agent))
    }
}

fn eval_bypass_approval_policy() -> Arc<ApprovalPolicy> {
    Arc::new(
        ApprovalPolicy::new()
            .with_default_plan_rule(ApprovalRule::Never)
            .with_default_tool_rule(ApprovalRule::Never),
    )
}

impl Runtime {
    pub(crate) fn run_eval_entry_agent_stream(
        &self,
        entry_agent: String,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
    ) -> crate::RuntimeEventStream {
        self.run_entry_agent_stream_cancellable_with_options(
            entry_agent,
            prompt,
            thread_id,
            RequestExecutionOptions::eval(),
        )
        .into_stream()
    }

    fn run_entry_agent_stream_cancellable_with_options(
        &self,
        entry_agent: String,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
        options: RequestExecutionOptions,
    ) -> CancellableRuntimeEventStream {
        let request_cancel = self.cancel_root.child();
        let stream = self.run_entry_agent_stream_with_options(
            entry_agent,
            prompt,
            thread_id,
            options,
            request_cancel.clone(),
        );
        CancellableRuntimeEventStream::new(stream, request_cancel)
    }

    fn run_entry_agent_stream_with_options(
        &self,
        entry_agent: String,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
        options: RequestExecutionOptions,
        request_cancel: CancellationToken,
    ) -> crate::RuntimeEventStream {
        let (channel, receiver) = ChannelEventSink::new(1024);
        let channel: Arc<dyn EventSink> = Arc::new(channel);

        // Build the per-request orchestrator. Errors here become a single
        // `StreamPart::Error` and the channel closes immediately.
        let orchestrator = match self.build_orchestrator_for_request(
            channel.clone(),
            &thread_id,
            &entry_agent,
            &options,
            &request_cancel,
        ) {
            Ok(o) => o,
            Err(e) => {
                channel.emit(StreamPart::error(e.to_string()));
                channel.close();
                return Box::pin(receiver);
            }
        };

        // Drive the entry agent off-thread. Errors flow as `StreamPart::Error`
        // so the consumer always sees a clean stream termination.
        let driver = orchestrator.clone();
        tokio::spawn(async move {
            let mut req = SubAgentRequest::new(entry_agent.clone(), prompt).with_current_thread();
            if options.deterministic_invocation_ids {
                req = req.with_invocation_id(format!("entry-{entry_agent}"));
            }
            if let Err(e) = driver.invoke(req).await {
                channel.emit(StreamPart::error(e.to_string()));
            }
            channel.close();
        });

        Box::pin(receiver)
    }

    fn build_orchestrator_for_request(
        &self,
        channel: Arc<dyn EventSink>,
        thread_id: &agent_fw_core::ThreadId,
        entry_agent: &str,
        options: &RequestExecutionOptions,
        request_cancel: &CancellationToken,
    ) -> Result<Arc<agent_fw_agent::AgentOrchestrator>, RuntimeError> {
        use agent_fw_agent::{AgentOrchestrator, AgentRegistration, ModelId};
        use agent_fw_core::tenant::TenantContext;

        // Map each AgentSpec to the framework's AgentRegistration.
        let registrations: Vec<AgentRegistration> = self
            .spec
            .agents
            .iter()
            .map(|a| AgentRegistration {
                name: a.name.clone(),
                model: ModelId::new(a.model.id.clone()),
                system_prompt: a.system_prompt.clone(),
                role: Some(a.role.to_agent_label()),
                stateful: a.stateful,
            })
            .collect();

        // runtime query assembly: late-bound `SubAgentInvoker` resolves the chicken-and-egg
        // between dispatcher composition and orchestrator construction. The
        // invoker Arc is installed on every per-agent env now; the actual
        // `Arc<AgentOrchestrator>` is set into it once `.build()` returns.
        // The spawn task in `run_entry_agent_stream` only calls
        // `orchestrator.invoke(...)` after `set` has happened, so any
        // `call_agent` tool the LLM emits reaches the real orchestrator.
        let late_bound = Arc::new(LateBoundInvoker::new());
        let invoker: Arc<dyn SubAgentInvoker> = late_bound.clone();

        // Build the per-agent dispatcher map (default toolkit composition C5 + B2 approval gate).
        // `Runtime::dispatcher_for` already applies the canonical
        // `guarded → approval → traced` framework stack, so this loop just
        // wires the result into the orchestrator's per-agent slot.
        let mut dispatchers: HashMap<String, Arc<dyn agent_fw_agent::ToolDispatcher>> =
            HashMap::new();
        for agent in &self.spec.agents {
            let env = self.tool_env_for_agent_with_options(
                agent,
                channel.clone(),
                thread_id.clone(),
                invoker.clone(),
                agent.name == entry_agent,
                options,
                request_cancel,
            );
            let approval_policy = options.approval_policy_for(self, &agent.name);
            if let Some(d) = self.dispatcher_for_with_policy(&agent.name, env, approval_policy)? {
                dispatchers.insert(agent.name.clone(), Arc::new(d));
            }
        }

        let tenant_ctx = TenantContext::new(self.tenant.clone()).with_thread(thread_id.clone());
        let (default_interpreter, agent_interpreters) = self.orchestrator_interpreters()?;

        let orchestrator = AgentOrchestrator::builder()
            .agents(registrations)
            .interpreter(default_interpreter)
            .interpreters_per_agent(agent_interpreters)
            .dispatchers_per_agent(dispatchers)
            .tenant(tenant_ctx)
            .event_sink(channel)
            .memory_store(self.agent_memory.clone())
            .cancel(request_cancel.child())
            .build()
            .map_err(RuntimeError::from)?;
        let orchestrator = Arc::new(orchestrator);

        // Install the freshly-built orchestrator into the late-bound
        // invoker as a `Weak` reference (see `LateBoundInvoker` docs —
        // a strong Arc back-edge would close a reference cycle through
        // the per-agent envs and the channel sink would never drop).
        // The returned `Arc<AgentOrchestrator>` is the only strong owner;
        // the runtime's spawn task holds it for the duration of the
        // request, which keeps the Weak upgradeable.
        let orchestrator_dyn: Arc<dyn SubAgentInvoker> = orchestrator.clone();
        let _ = late_bound.set(&orchestrator_dyn);
        debug_assert!(
            late_bound.has_agent(&self.spec.agents[0].name) || self.spec.agents.is_empty(),
            "LateBoundInvoker should resolve through the orchestrator after `set`",
        );

        Ok(orchestrator)
    }

    fn orchestrator_interpreters(
        &self,
    ) -> Result<
        (
            Arc<dyn agent_fw_agent::ChatInterpreter>,
            HashMap<String, Arc<dyn agent_fw_agent::ChatInterpreter>>,
        ),
        RuntimeError,
    > {
        crate::resolve_orchestrator_interpreters(&self.spec, &self.interpreter_providers)
    }

    pub(crate) fn tool_env_for_agent(
        &self,
        agent: &AgentSpec,
        sink: Arc<dyn EventSink>,
        parent_thread_id: agent_fw_core::ThreadId,
        sub_agents: Arc<dyn SubAgentInvoker>,
        use_parent_thread: bool,
    ) -> ToolEnvironment {
        let request_cancel = self.cancel_root.child();
        self.tool_env_for_agent_with_options(
            agent,
            sink,
            parent_thread_id,
            sub_agents,
            use_parent_thread,
            &RequestExecutionOptions::default(),
            &request_cancel,
        )
    }

    fn tool_env_for_agent_with_options(
        &self,
        agent: &AgentSpec,
        sink: Arc<dyn EventSink>,
        parent_thread_id: agent_fw_core::ThreadId,
        sub_agents: Arc<dyn SubAgentInvoker>,
        use_parent_thread: bool,
        options: &RequestExecutionOptions,
        request_cancel: &CancellationToken,
    ) -> ToolEnvironment {
        use agent_fw_core::tenant::TenantContext;

        let plan_ctx = crate::runtime::PlanExecutionContext {
            approval_policy: options.approval_policy_for(self, &agent.name),
            approval_store: self.approval_store.clone(),
            action_dispatcher: self.action_dispatcher.clone(),
            approver: self.approver.clone(),
        };
        let parent_tenant = TenantContext::new(self.tenant.clone()).with_thread(parent_thread_id);
        let tenant_ctx = if use_parent_thread {
            parent_tenant
        } else {
            // runtime query assembly review fix: derive the same per-agent thread id that
            // delegated `AgentOrchestrator::invoke` builds via G2, so approval
            // and plan events emitted from tools carry the sub-agent's thread
            // (e.g. `thread-1-executor`) instead of the parent request thread.
            parent_tenant.with_derived_thread(&agent.name)
        };
        let mut env = ToolEnvironment::builder()
            .kv_arc(self.kv.clone())
            .event_sink_arc(sink)
            .sub_agents_arc(sub_agents)
            .tenant_context(tenant_ctx)
            .cancel(request_cancel.child())
            .build()
            .with_ext::<crate::runtime::PlanExecutionContext>(Arc::new(plan_ctx))
            .with_ext::<crate::ReferenceRegistry>(self.references.clone());

        if let Some(context) = self.data_workspace_context.clone() {
            env = env.with_ext::<agent_fw_core::WorkspaceContext>(Arc::new(context));
        }

        if agent_uses_catalog_toolkit(agent) {
            let catalog = self
                .data_catalog
                .clone()
                .unwrap_or_else(|| Arc::new(MissingDataCatalog) as Arc<dyn DataCatalog>);
            env = env.with_catalog(catalog);
            if let Some(backend) = self.catalog_search_backend.clone() {
                env = env.with_catalog_search_backend(backend);
            }
        }

        if agent_uses_target_database_toolkit(agent) {
            let target_database = self.target_database.clone().unwrap_or_else(|| {
                Arc::new(ErrorTargetDatabase::new(
                    "TargetDatabase missing: pass data_environment.target_database or data_environment.target_database_url to flowai_harness.create_runtime(...)",
                )) as Arc<dyn agent_fw_algebra::TargetDatabase>
            });
            env = env.with_target_db(target_database);
        }

        env
    }
}

impl Runtime {
    /// Run a user query through the registered coordinator agent and
    /// stream framework [`StreamPart`]s back to the caller (runtime query assembly C4).
    pub fn query_impl(&self, request: QueryRequest) -> crate::RuntimeEventStream {
        self.query_cancellable_impl(request).into_stream()
    }

    pub(crate) fn query_cancellable_impl(
        &self,
        request: QueryRequest,
    ) -> CancellableRuntimeEventStream {
        if request.resource_id != self.tenant {
            return cancellable_error_stream(
                "resource_id mismatch with runtime tenant",
                self.cancel_root.child(),
            );
        }
        let Some(coordinator) = self
            .spec
            .agents
            .iter()
            .find(|a| a.role == AgentRole::Coordinator)
        else {
            return cancellable_error_stream(
                "no coordinator agent registered in the spec",
                self.cancel_root.child(),
            );
        };
        self.run_entry_agent_stream_cancellable_with_options(
            coordinator.name.clone(),
            request.prompt,
            request.thread_id,
            RequestExecutionOptions::default(),
        )
    }

    pub(crate) fn run_eval_role_stream(
        &self,
        role: AgentRole,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
    ) -> crate::RuntimeEventStream {
        let Some(agent_name) = self.agent_name_by_role(role).map(str::to_owned) else {
            return error_stream(&format!("no {role} agent registered in the spec"));
        };
        self.run_eval_entry_agent_stream(agent_name, prompt, thread_id)
    }

    #[cfg(test)]
    pub(crate) fn run_role_stream(
        &self,
        role: AgentRole,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
    ) -> crate::RuntimeEventStream {
        let Some(agent_name) = self.agent_name_by_role(role).map(str::to_owned) else {
            return error_stream(&format!("no {role} agent registered in the spec"));
        };
        self.run_entry_agent_stream_cancellable_with_options(
            agent_name,
            prompt,
            thread_id,
            RequestExecutionOptions::default(),
        )
        .into_stream()
    }

    pub(crate) fn run_eval_query_stream(
        &self,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
    ) -> crate::RuntimeEventStream {
        let Some(coordinator) = self
            .spec
            .agents
            .iter()
            .find(|a| a.role == AgentRole::Coordinator)
        else {
            return error_stream("no coordinator agent registered in the spec");
        };
        self.run_eval_entry_agent_stream(coordinator.name.clone(), prompt, thread_id)
    }

    pub(crate) fn run_eval_specialist_stream(
        &self,
        specialist: &str,
        prompt: String,
        thread_id: agent_fw_core::ThreadId,
    ) -> crate::RuntimeEventStream {
        let Some(agent) = self.spec.agents.iter().find(|a| a.name == specialist) else {
            return error_stream(&format!("specialist '{specialist}' not found"));
        };
        if agent.role != AgentRole::Specialist {
            return error_stream(&format!("agent '{specialist}' is not a specialist"));
        }
        self.run_eval_entry_agent_stream(specialist.to_string(), prompt, thread_id)
    }

    /// Directly invoke a specialist agent, skipping coordinator routing.
    pub fn run_specialist_impl(&self, request: SpecialistRequest) -> crate::RuntimeEventStream {
        self.run_specialist_cancellable_impl(request).into_stream()
    }

    pub(crate) fn run_specialist_cancellable_impl(
        &self,
        request: SpecialistRequest,
    ) -> CancellableRuntimeEventStream {
        // runtime query assembly review fix: mirror `query_impl`'s tenant defence. The
        // public API must not accept an arbitrary tenant from the
        // caller — `resourceId` is always derived from auth context.
        // Without this check `run_specialist` runs against the runtime
        // handle's tenant regardless of the request, breaking the
        // contract documented on `SpecialistRequest::resource_id`.
        if request.resource_id != self.tenant {
            return cancellable_error_stream(
                "resource_id mismatch with runtime tenant",
                self.cancel_root.child(),
            );
        }
        let Some(agent) = self
            .spec
            .agents
            .iter()
            .find(|a| a.name == request.specialist)
        else {
            return cancellable_error_stream(
                &format!("specialist '{}' not found", request.specialist),
                self.cancel_root.child(),
            );
        };
        if agent.role != AgentRole::Specialist {
            return cancellable_error_stream(
                &format!("agent '{}' is not a specialist", request.specialist),
                self.cancel_root.child(),
            );
        }
        let thread_id = request.thread_id.unwrap_or_else(|| {
            agent_fw_core::ThreadId::new_unchecked(format!("specialist-{}", uuid::Uuid::new_v4()))
        });
        self.run_entry_agent_stream_cancellable_with_options(
            request.specialist,
            request.prompt,
            thread_id,
            RequestExecutionOptions::default(),
        )
    }
}

fn error_stream(message: &str) -> crate::RuntimeEventStream {
    let part = StreamPart::error(message);
    Box::pin(futures::stream::once(async move { part }))
}

fn cancellable_error_stream(
    message: &str,
    cancel: CancellationToken,
) -> CancellableRuntimeEventStream {
    CancellableRuntimeEventStream::new(error_stream(message), cancel)
}

fn agent_uses_catalog_toolkit(agent: &AgentSpec) -> bool {
    agent.toolkits.iter().any(|toolkit| toolkit == "catalog")
}

fn agent_uses_target_database_toolkit(agent: &AgentSpec) -> bool {
    agent.toolkits.iter().any(|toolkit| toolkit == "catalog")
}

struct MissingDataCatalog;

const MISSING_CATALOG_MESSAGE: &str =
    "DataCatalog missing: pass data_environment.catalog to flowai_harness.create_runtime(...)";

impl MissingDataCatalog {
    fn err(&self) -> CatalogError {
        CatalogError::Unavailable(MISSING_CATALOG_MESSAGE.to_string())
    }
}

#[async_trait]
impl DataCatalog for MissingDataCatalog {
    async fn get_by_id(&self, _id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_by_ids(&self, _ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_by_qualified_name(
        &self,
        _kind: CatalogKind,
        _qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_by_name(
        &self,
        _kind: CatalogKind,
        _name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn list_by_type(
        &self,
        _kind: CatalogKind,
        _limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_related(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_related_reverse(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn resolve_ref(
        &self,
        _reference: &CatalogRef,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        Err(self.err())
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Err(self.err())
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Err(self.err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_request_options_disable_plan_and_tool_approvals() {
        let options = RequestExecutionOptions::eval();
        assert!(options.deterministic_invocation_ids);
        let policy = options
            .approval_policy_override
            .expect("eval options should carry an approval override");

        assert!(matches!(policy.resolve_plan("plan"), ApprovalRule::Never));
        assert!(matches!(
            policy.resolve_tool("executePlan"),
            ApprovalRule::Never
        ));
    }

    #[tokio::test]
    async fn missing_catalog_reverse_relation_lookup_is_actionable_error() {
        let err = MissingDataCatalog
            .get_related_reverse("column:public.orders.status", None)
            .await
            .expect_err("missing catalog reverse lookup should not return an empty graph");

        let message = err.to_string();
        assert!(message.contains("DataCatalog missing"));
        assert!(message.contains("data_environment.catalog"));
    }
}
