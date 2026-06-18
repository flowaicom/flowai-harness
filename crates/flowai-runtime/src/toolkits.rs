//! Default toolkits (default toolkit composition C5 + runtime query assembly C4).
//!
//! Wires stable symbolic-ID toolkits onto in-tree / framework backings:
//!
//! | Toolkit ID        | Backing                                                |
//! | ----------------- | ------------------------------------------------------ |
//! | `catalog`         | `agent-fw-catalog-tools` seven-tool surface            |
//! | `references`      | [`ReferenceRegistry`] resolve / glimpse helpers (C2)   |
//! | `plans`           | [`PlanRegistry`] propose / load / execute helpers (C3) |
//! | `agents`          | [`CallAgentHandler`](agent_fw_agent::CallAgentHandler) — coordinator-side sub-agent delegation (runtime query assembly C4) |
//!
//! Each toolkit exposes a `Vec<Arc<dyn ToolHandler>>` builder. The
//! runtime never stores a long-lived dispatcher — it would have to bake
//! in a partial [`ToolEnvironment`] without the per-request
//! `DataCatalog` / `TargetDatabase` extensions, producing a dispatcher
//! that looks ready but errors at the first SDL tool call.
//!
//! Instead, [`Runtime::new`](crate::Runtime::new) does a one-shot
//! validation pass — catching unknown toolkit IDs, malformed config, and
//! cross-toolkit name collisions at startup through
//! [`ComposedDispatcher::validate_no_collisions`] — and discards the
//! result. The C4 runner then calls
//! [`Runtime::dispatcher_for`](crate::Runtime::dispatcher_for) per
//! request with an env that carries the catalog and target-db for the
//! tenant in flight.
//!
//! Toolkit narrowing: a [`ToolkitSpec`]'s `config` is parsed into
//! [`ToolkitConfig`]. Setting `tools: ["execute_query"]` exposes only that
//! tool; an unknown name surfaces as [`ToolkitError::UnknownTool`].

use std::collections::HashSet;
use std::sync::Arc;

use agent_fw_agent::{ComposedDispatcher, ToolDefinition, ToolHandler};
use agent_fw_tool::{ToolCollision, ToolEnvironment};
use serde::Deserialize;

use crate::plans::PlanRegistry;
use crate::references::ReferenceRegistry;
use crate::{AgentRole, AgentSpec, ToolkitSpec};

mod agents;
mod catalog_surface;
mod plans;
mod refs;

/// Per-toolkit configuration parsed from [`ToolkitSpec::config`].
///
/// Currently supports one knob: narrowing the exposed tool set. Future
/// fields can be added without breaking callers because the struct is
/// `serde(default)` everywhere.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolkitConfig {
    /// Narrow the toolkit to a subset of its tool names. `None` exposes
    /// every tool the backing crate provides; `Some([...])` rejects any
    /// name the toolkit does not declare via
    /// [`ToolHandler::definition`](agent_fw_agent::ToolHandler::definition).
    #[serde(default)]
    pub tools: Option<Vec<String>>,

    /// Internal/operator-only switch for catalog-search diagnostics.
    ///
    /// Customer-facing runtime toolkits redact diagnostics by default so
    /// agents do not see search implementation details.
    #[serde(default)]
    pub expose_diagnostics: bool,
}

/// Errors surfaced while resolving or composing toolkits.
#[derive(Debug, thiserror::Error)]
pub enum ToolkitError {
    /// The toolkit ID does not match any built-in (`catalog`, `references`,
    /// `plans`, or `agents`).
    #[error("unknown toolkit id: {0}")]
    UnknownToolkit(String),

    /// `config.tools` named a tool that this toolkit does not provide.
    #[error("toolkit '{toolkit}' has no tool named '{name}'")]
    UnknownTool {
        /// Toolkit ID under which the unknown name was requested.
        toolkit: String,
        /// Requested tool name.
        name: String,
    },

    /// `config.tools` named a tool that exists in the toolkit but is not
    /// available to this agent role.
    #[error("tool '{name}' from toolkit '{toolkit}' is not available for {role} agents")]
    ForbiddenTool {
        /// Toolkit ID under which the forbidden name was requested.
        toolkit: String,
        /// Requested tool name.
        name: String,
        /// Agent role whose scoped toolkit surface rejected the tool.
        role: AgentRole,
    },

    /// [`ToolkitSpec::config`] failed to deserialize into
    /// [`ToolkitConfig`].
    #[error("toolkit '{toolkit}' config is malformed: {error}")]
    Config {
        /// Toolkit ID whose config failed to parse.
        toolkit: String,
        /// Serde error message.
        error: String,
    },

    /// One or more tool names appeared in multiple toolkits for the
    /// same agent. Surfaced by
    /// [`ComposedDispatcher::validate_no_collisions`].
    ///
    /// Extension validation (required env types like `DataCatalog`,
    /// `TargetDatabase`) is deferred to dispatch time: the runtime
    /// composes dispatchers before the C4 runner attaches the
    /// per-request env that carries those extensions. Handlers fall
    /// back to `try_ext()`-style errors at dispatch when the env is
    /// genuinely missing a dependency.
    #[error("dispatcher collisions: {0:?}")]
    Collisions(Vec<ToolCollision>),
}

/// Resolve a single toolkit spec into a handler set.
///
/// `references` and `plans` toolkits receive the runtime-owned registries
/// directly so the handlers do not have to thread them through the env.
pub(crate) fn resolve_toolkit(
    spec: &ToolkitSpec,
    agent: &AgentSpec,
    references: &Arc<ReferenceRegistry>,
    plans: &Arc<PlanRegistry>,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    // `null` or absent config is normalised to defaults — most toolkit
    // declarations carry no tuning and we don't want to force every
    // facade to emit an explicit empty object.
    let cfg: ToolkitConfig = if spec.config.is_null() {
        ToolkitConfig::default()
    } else {
        serde_json::from_value(spec.config.clone()).map_err(|e| ToolkitError::Config {
            toolkit: spec.id.clone(),
            error: e.to_string(),
        })?
    };
    match spec.id.as_str() {
        "catalog" => catalog_surface::handlers(&spec.id, &cfg),
        "references" => refs::handlers(&spec.id, &cfg, references.clone()),
        "plans" => plans::handlers(
            &spec.id,
            &role_scoped_plan_config(&spec.id, &cfg, agent.role)?,
            plans.clone(),
        ),
        "agents" => agents::handlers(&spec.id, &cfg, agent.routes.clone()),
        other => Err(ToolkitError::UnknownToolkit(other.to_string())),
    }
}

/// Describe the tools exposed by a toolkit without constructing a dispatcher.
///
/// This is used by language facades to render prompt-visible tool
/// descriptions from the same handler definitions that runtime dispatch uses.
/// The helper still runs the normal toolkit config parser and narrowing logic,
/// so prompt rendering fails on the same unknown toolkit/tool names as runtime
/// dispatcher composition.
pub fn describe_toolkit_tools(
    spec: &ToolkitSpec,
    agent: &AgentSpec,
) -> Result<Vec<ToolDefinition>, ToolkitError> {
    let kv = Arc::new(agent_fw_interpreter::DashMapKVStore::new());
    let references = Arc::new(
        ReferenceRegistry::new(Vec::new(), kv.clone())
            .expect("empty reference registry should be valid"),
    );
    let plans = Arc::new(
        PlanRegistry::new(Vec::new(), kv, references.clone())
            .expect("empty plan registry should be valid"),
    );
    Ok(resolve_toolkit(spec, agent, &references, &plans)?
        .into_iter()
        .map(|handler| handler.definition())
        .collect())
}

/// Build the per-agent [`ComposedDispatcher`] from every toolkit the
/// agent lists in [`AgentSpec::toolkits`].
///
/// Returns [`ToolkitError::UnknownToolkit`] if the agent references an ID
/// that is not declared in [`RuntimeSpec::toolkits`](crate::RuntimeSpec::toolkits).
/// Cross-toolkit name collisions surface as [`ToolkitError::Collisions`].
/// Extension validation is deferred — the runtime composes dispatchers
/// before the C4 runner attaches the per-request env that carries
/// `DataCatalog` / `TargetDatabase`. Handlers surface missing
/// extensions at dispatch time via `try_ext` / `try_catalog`.
pub(crate) fn compose_agent_dispatcher(
    agent: &AgentSpec,
    available: &[ToolkitSpec],
    references: &Arc<ReferenceRegistry>,
    plans: &Arc<PlanRegistry>,
    env: ToolEnvironment,
) -> Result<ComposedDispatcher, ToolkitError> {
    let mut dispatcher = ComposedDispatcher::new(env);
    let default_handlers = role_default_handlers(agent, references, plans)?;
    let default_names: HashSet<String> = default_handlers
        .iter()
        .map(|handler| handler.definition().name)
        .collect();
    for handler in default_handlers {
        dispatcher.add_handler(handler);
    }

    for toolkit_id in &agent.toolkits {
        let spec = available
            .iter()
            .find(|t| &t.id == toolkit_id)
            .ok_or_else(|| ToolkitError::UnknownToolkit(toolkit_id.clone()))?;
        let handlers = resolve_toolkit(spec, agent, references, plans)?;
        dispatcher = dispatcher.with_handlers(
            handlers
                .into_iter()
                .filter(|handler| !default_names.contains(handler.definition().name.as_str())),
        );
    }
    dispatcher
        .validate_no_collisions()
        .map_err(ToolkitError::Collisions)?;
    Ok(dispatcher)
}

fn role_default_handlers(
    agent: &AgentSpec,
    references: &Arc<ReferenceRegistry>,
    plans: &Arc<PlanRegistry>,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    match agent.role {
        AgentRole::Coordinator if !agent.routes.is_empty() => {
            agents::handlers("agents", &ToolkitConfig::default(), agent.routes.clone())
        }
        AgentRole::Planner => plans::handlers(
            "plans",
            &tool_subset(["storePlan", "getPlan"]),
            plans.clone(),
        ),
        AgentRole::Executor => {
            let mut handlers = plans::handlers(
                "plans",
                &tool_subset(["getPlan", "executePlan"]),
                plans.clone(),
            )?;
            handlers.extend(refs::handlers(
                "references",
                &ToolkitConfig::default(),
                references.clone(),
            )?);
            Ok(handlers)
        }
        AgentRole::Coordinator | AgentRole::Specialist => Ok(vec![]),
    }
}

fn tool_subset<const N: usize>(tools: [&str; N]) -> ToolkitConfig {
    tool_subset_from_slice(&tools, false)
}

fn tool_subset_from_slice(tools: &[&str], expose_diagnostics: bool) -> ToolkitConfig {
    ToolkitConfig {
        tools: Some(tools.iter().copied().map(str::to_string).collect()),
        expose_diagnostics,
    }
}

fn role_scoped_plan_config(
    toolkit_id: &str,
    cfg: &ToolkitConfig,
    role: AgentRole,
) -> Result<ToolkitConfig, ToolkitError> {
    let allowed = plan_tools_for_role(role);
    let Some(requested) = cfg.tools.as_ref() else {
        return Ok(tool_subset_from_slice(allowed, cfg.expose_diagnostics));
    };
    for name in requested {
        if !allowed.contains(&name.as_str()) {
            return Err(ToolkitError::ForbiddenTool {
                toolkit: toolkit_id.to_string(),
                name: name.clone(),
                role,
            });
        }
    }
    Ok(ToolkitConfig {
        tools: Some(requested.clone()),
        expose_diagnostics: cfg.expose_diagnostics,
    })
}

fn plan_tools_for_role(role: AgentRole) -> &'static [&'static str] {
    match role {
        AgentRole::Planner => &["storePlan", "getPlan"],
        AgentRole::Executor => &["getPlan", "executePlan"],
        AgentRole::Coordinator | AgentRole::Specialist => &["getPlan"],
    }
}

/// Filter a handler set by [`ToolkitConfig::tools`].
///
/// Returns [`ToolkitError::UnknownTool`] if `cfg.tools` contains a name
/// none of the supplied handlers declares.
pub(crate) fn filter_by_config(
    toolkit_id: &str,
    handlers: Vec<Arc<dyn ToolHandler>>,
    cfg: &ToolkitConfig,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    let Some(allowed) = &cfg.tools else {
        return Ok(handlers);
    };
    let available: std::collections::HashSet<String> =
        handlers.iter().map(|h| h.definition().name).collect();
    for name in allowed {
        if !available.contains(name) {
            return Err(ToolkitError::UnknownTool {
                toolkit: toolkit_id.to_string(),
                name: name.clone(),
            });
        }
    }
    let allow_set: std::collections::HashSet<&str> = allowed.iter().map(String::as_str).collect();
    Ok(handlers
        .into_iter()
        .filter(|h| allow_set.contains(h.definition().name.as_str()))
        .collect())
}

#[cfg(test)]
mod tests {
    //! default toolkit composition acceptance tests for the default toolkits.
    //!
    //! Each toolkit's per-tool behaviour is exercised via the same
    //! [`ComposedDispatcher`] surface a C4 runner will call. Catalog /
    //! target-db extensions are attached per test via the dispatcher
    //! env so [`ExecuteQueryHandler`] reaches its AST-level read-only
    //! validation.
    use super::*;
    use std::sync::Arc;

    use agent_fw_agent::ToolDispatcher;
    use agent_fw_algebra::testing::NullEventSink;
    use agent_fw_catalog::{
        CatalogError, CatalogSearchBackend, CatalogSearchHealth, CatalogSearchRequest,
        CatalogSearchResults, CatalogToolEnvironmentExt, DataCatalog,
    };
    use agent_fw_core::tenant::TenantContext;
    use agent_fw_core::{PlanId, TenantId};
    use agent_fw_interpreter::mock_catalog::MockCatalog;
    use agent_fw_interpreter::{DashMapKVStore, MockTargetDatabase};
    use agent_fw_tool::ToolEnvironment;
    use serde_json::json;

    use crate::{
        AgentSpec, ApprovalPolicies, ModelSpec, PlanSpec, ProviderConfig, ProviderRegistry,
        ReferenceSpec, Runtime, RuntimeDeps, RuntimeError, RuntimeSpec, StorageFactories,
        TenantIdentity, ToolkitSpec,
    };

    fn core_toolkits() -> Vec<ToolkitSpec> {
        vec![
            ToolkitSpec {
                id: "catalog".to_string(),
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
        ]
    }

    fn catalog_toolkit() -> ToolkitSpec {
        ToolkitSpec {
            id: "catalog".to_string(),
            config: serde_json::Value::Null,
        }
    }

    fn executor_with_catalog_toolkit() -> AgentSpec {
        let mut agent = AgentSpec::new(
            "executor",
            crate::AgentRole::Executor,
            ModelSpec::new("claude-haiku-4-5"),
            "Executes plans.",
        );
        agent.toolkits = vec!["catalog".to_string()];
        agent
    }

    fn planner_without_toolkits() -> AgentSpec {
        AgentSpec::new(
            "planner",
            crate::AgentRole::Planner,
            ModelSpec::new("claude-sonnet-4-6"),
            "Produces plans.",
        )
    }

    fn planner_with_plans_toolkit() -> AgentSpec {
        let mut agent = AgentSpec::new(
            "planner",
            crate::AgentRole::Planner,
            ModelSpec::new("claude-sonnet-4-6"),
            "Produces plans.",
        );
        agent.toolkits = vec!["plans".to_string()];
        agent
    }

    fn specialist_with_catalog() -> AgentSpec {
        let mut agent = AgentSpec::new(
            "catalog_reader",
            crate::AgentRole::Specialist,
            ModelSpec::new("claude-haiku-4-5"),
            "Reads the data catalog.",
        );
        agent.toolkits = vec!["catalog".to_string()];
        agent
    }

    fn coordinator_no_toolkits() -> AgentSpec {
        let mut agent = AgentSpec::new(
            "coordinator",
            crate::AgentRole::Coordinator,
            ModelSpec::new("claude-sonnet-4-6"),
            "Coordinates.",
        );
        agent.routes = vec!["executor".to_string()];
        agent
    }

    fn product_set_reference_spec() -> ReferenceSpec {
        ReferenceSpec {
            name: "ProductSet".to_string(),
            schema: json!({
                "type": "object",
                "required": ["product_ids"],
                "properties": {
                    "product_ids": {"type": "array", "items": {"type": "string"}}
                }
            }),
            ttl_ms: None,
        }
    }

    fn scenario_plan_spec() -> PlanSpec {
        PlanSpec {
            name: "ScenarioPlan".to_string(),
            schema: json!({
                "type": "object",
                "required": ["actions"],
                "properties": {
                    "actions": {
                        "type": "array",
                        "minItems": 1,
                        "items": {"type": "object", "required": ["kind"]}
                    }
                }
            }),
            display_aliases: vec![],
        }
    }

    fn runtime_spec_with_core_toolkits() -> RuntimeSpec {
        // The two agents both use `claude-sonnet-4-6`, which the
        // `runtime::providers` id-family router maps to `anthropic`.
        // `Runtime::new` rejects unsupported provider/model combinations
        // (runtime query assembly eager validation), so declare anthropic explicitly.
        let mut providers = ProviderRegistry::default();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new(serde_json::json!({"apiKeyEnv": "ANTHROPIC_API_KEY"})),
        );
        RuntimeSpec {
            tenant: TenantIdentity::new("tenant-1", "v1"),
            agents: vec![coordinator_no_toolkits(), executor_with_catalog_toolkit()],
            references: vec![product_set_reference_spec()],
            plans: vec![scenario_plan_spec()],
            toolkits: core_toolkits(),
            approval_policies: ApprovalPolicies::default(),
            approval_overrides: Default::default(),
            storage_factories: StorageFactories::default(),
            providers,
        }
    }

    struct ReadyCatalogSearchBackend;

    #[async_trait::async_trait]
    impl CatalogSearchBackend for ReadyCatalogSearchBackend {
        async fn search(
            &self,
            _scope: &agent_fw_catalog::CatalogScope,
            _request: CatalogSearchRequest,
        ) -> Result<CatalogSearchResults, CatalogError> {
            Ok(CatalogSearchResults {
                hits: vec![],
                facets: Default::default(),
                has_more: false,
                next_cursor: None,
                candidate_count: 0,
                warnings: vec![],
            })
        }

        async fn health(
            &self,
            _scope: &agent_fw_catalog::CatalogScope,
        ) -> Result<CatalogSearchHealth, CatalogError> {
            Ok(CatalogSearchHealth::Ready {
                indexed_entries: 0,
                projection_version: 1,
            })
        }
    }

    fn deps_for(kv: Arc<DashMapKVStore>) -> RuntimeDeps {
        deps_without_catalog_search(kv)
            .with_catalog_search_backend(Arc::new(ReadyCatalogSearchBackend))
    }

    fn deps_without_catalog_search(kv: Arc<DashMapKVStore>) -> RuntimeDeps {
        use agent_fw_agent::{ChatInterpreter, ChatProgram};
        use agent_fw_core::stream_part::FinishReason;
        use agent_fw_core::usage::TokenUsage;
        use agent_fw_core::StreamPart;
        use futures::stream;
        use std::pin::Pin;

        struct NoopInterpreter;
        impl ChatInterpreter for NoopInterpreter {
            fn interpret(
                &self,
                _program: ChatProgram,
                _cancel: agent_fw_algebra::CancellationToken,
            ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
                Box::pin(stream::iter(vec![
                    StreamPart::StepStart,
                    StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
                ]))
            }
        }

        RuntimeDeps::new(
            Arc::new(NoopInterpreter),
            Arc::new(NullEventSink),
            TenantContext::new(TenantId::new_unchecked("tenant-1")),
            kv,
        )
    }

    /// Build a per-request env that satisfies every C5 toolkit's
    /// extension requirements — catalog, target-db, kv.
    fn dispatch_env(kv: Arc<DashMapKVStore>) -> ToolEnvironment {
        dispatch_env_with_catalog(kv, Arc::new(MockCatalog::new()))
    }

    fn dispatch_env_with_catalog(
        kv: Arc<DashMapKVStore>,
        catalog: Arc<dyn DataCatalog>,
    ) -> ToolEnvironment {
        let target_db: Arc<dyn agent_fw_algebra::TargetDatabase> =
            Arc::new(MockTargetDatabase::new());
        ToolEnvironment::builder()
            .kv_arc(kv)
            .event_sink_arc(Arc::new(NullEventSink) as Arc<dyn agent_fw_algebra::EventSink>)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("tenant-1")))
            .build()
            .with_catalog(catalog)
            .with_target_db(target_db)
    }

    #[test]
    fn catalog_toolkit_describes_exact_seven_contract_tools() {
        let definitions = describe_toolkit_tools(&catalog_toolkit(), &specialist_with_catalog())
            .expect("describes catalog toolkit");
        let names: Vec<String> = definitions
            .into_iter()
            .map(|definition| definition.name)
            .collect();

        assert_eq!(
            names,
            vec![
                "search_catalog",
                "get_catalog_entities",
                "list_schema_fields",
                "get_catalog_relations",
                "get_relation_paths_between",
                "sample_table_data",
                "execute_query",
            ]
        );
    }

    #[test]
    fn catalog_toolkit_requires_catalog_search_backend_at_runtime_build() {
        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(
            runtime_spec_with_core_toolkits(),
            deps_without_catalog_search(kv),
        )
        .err()
        .expect("catalog toolkit should require catalog_search at runtime build");

        let message = err.to_string();
        assert!(message.contains("catalog_search"));
        assert!(message.contains("catalog"));
    }

    #[test]
    fn narrowed_catalog_toolkit_requires_catalog_search_backend_at_runtime_build() {
        let kv = Arc::new(DashMapKVStore::new());
        let mut spec = runtime_spec_with_core_toolkits();
        spec.toolkits[0].config = json!({"tools": ["execute_query"]});

        let err = Runtime::new(spec, deps_without_catalog_search(kv))
            .err()
            .expect("narrowed catalog toolkit should still require catalog_search");

        let message = err.to_string();
        assert!(message.contains("catalog_search"));
        assert!(message.contains("catalog"));
    }

    #[test]
    fn legacy_catalog_toolkit_ids_are_no_longer_public() {
        for legacy_id in ["semantic-search", "warehouse-read", "knowledge"] {
            let err = describe_toolkit_tools(
                &ToolkitSpec {
                    id: legacy_id.to_string(),
                    config: serde_json::Value::Null,
                },
                &specialist_with_catalog(),
            )
            .err()
            .expect("legacy catalog toolkit id should be rejected");
            match err {
                ToolkitError::UnknownToolkit(id) => assert_eq!(id, legacy_id),
                other => panic!("expected UnknownToolkit for {legacy_id}, got {other:?}"),
            }
        }
    }

    #[test]
    fn plans_toolkit_describes_role_scoped_tools() {
        let plans_spec = ToolkitSpec {
            id: "plans".to_string(),
            config: serde_json::Value::Null,
        };

        let names: Vec<String> = describe_toolkit_tools(&plans_spec, &planner_with_plans_toolkit())
            .expect("describes plans toolkit")
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(names, vec!["storePlan", "getPlan"]);

        let names: Vec<String> =
            describe_toolkit_tools(&plans_spec, &executor_with_catalog_toolkit())
                .expect("describes executor plans toolkit")
                .into_iter()
                .map(|definition| definition.name)
                .collect();
        assert_eq!(names, vec!["getPlan", "executePlan"]);

        let mut specialist = specialist_with_catalog();
        specialist.toolkits = vec!["plans".to_string()];
        let names: Vec<String> = describe_toolkit_tools(&plans_spec, &specialist)
            .expect("describes specialist plans toolkit")
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(names, vec!["getPlan"]);
    }

    #[test]
    fn role_default_tools_compose_without_toolkit_selection() {
        let kv = Arc::new(DashMapKVStore::new());
        let references = Arc::new(
            ReferenceRegistry::new(vec![product_set_reference_spec()], kv.clone())
                .expect("reference registry"),
        );
        let plans = Arc::new(
            PlanRegistry::new(vec![scenario_plan_spec()], kv.clone(), references.clone())
                .expect("plan registry"),
        );
        let planner = planner_without_toolkits();
        let dispatcher = compose_agent_dispatcher(
            &planner,
            &core_toolkits(),
            &references,
            &plans,
            dispatch_env(kv.clone()),
        )
        .expect("planner defaults compose");

        let names: std::collections::HashSet<String> = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();

        assert_eq!(names.len(), 2);
        assert!(names.contains("storePlan"));
        assert!(names.contains("getPlan"));
        assert!(!names.contains("executePlan"));
    }

    #[test]
    fn explicit_plans_toolkit_respects_planner_scope_without_colliding() {
        let kv = Arc::new(DashMapKVStore::new());
        let references = Arc::new(
            ReferenceRegistry::new(vec![product_set_reference_spec()], kv.clone())
                .expect("reference registry"),
        );
        let plans = Arc::new(
            PlanRegistry::new(vec![scenario_plan_spec()], kv.clone(), references.clone())
                .expect("plan registry"),
        );
        let planner = planner_with_plans_toolkit();
        let dispatcher = compose_agent_dispatcher(
            &planner,
            &core_toolkits(),
            &references,
            &plans,
            dispatch_env(kv.clone()),
        )
        .expect("explicit plans compose");

        let names: std::collections::HashSet<String> = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();

        assert_eq!(names.len(), 2);
        assert!(names.contains("storePlan"));
        assert!(names.contains("getPlan"));
        assert!(!names.contains("executePlan"));
    }

    #[test]
    fn explicit_plans_toolkit_respects_executor_scope_without_colliding() {
        let kv = Arc::new(DashMapKVStore::new());
        let references = Arc::new(
            ReferenceRegistry::new(vec![product_set_reference_spec()], kv.clone())
                .expect("reference registry"),
        );
        let plans = Arc::new(
            PlanRegistry::new(vec![scenario_plan_spec()], kv.clone(), references.clone())
                .expect("plan registry"),
        );
        let mut executor = executor_with_catalog_toolkit();
        executor.toolkits = vec!["plans".to_string()];
        let dispatcher = compose_agent_dispatcher(
            &executor,
            &core_toolkits(),
            &references,
            &plans,
            dispatch_env(kv.clone()),
        )
        .expect("explicit plans compose");

        let names: std::collections::HashSet<String> = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();

        assert_eq!(names.len(), 4);
        assert!(!names.contains("storePlan"));
        assert!(names.contains("getPlan"));
        assert!(names.contains("executePlan"));
        assert!(names.contains("resolveRef"));
        assert!(names.contains("glimpseRef"));
    }

    #[test]
    fn explicit_plans_toolkit_rejects_tools_forbidden_by_role() {
        let mut spec = runtime_spec_with_core_toolkits();
        spec.agents[1].toolkits = vec!["plans".to_string()];
        spec.toolkits[2].config = json!({"tools": ["storePlan"]});

        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(spec, deps_for(kv))
            .err()
            .expect("expected RuntimeError");
        match err {
            RuntimeError::Toolkit(ToolkitError::ForbiddenTool {
                toolkit,
                name,
                role,
            }) => {
                assert_eq!(toolkit, "plans");
                assert_eq!(name, "storePlan");
                assert_eq!(role, AgentRole::Executor);
            }
            other => panic!("expected ForbiddenTool, got {other:?}"),
        }
    }

    #[test]
    fn core_toolkits_compose_into_executor_dispatcher() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");

        // Production call shape: caller supplies the per-request env
        // with the extensions every selected toolkit needs.
        let executor_dispatcher = runtime
            .dispatcher_for("executor", dispatch_env(kv))
            .expect("compose ok")
            .expect("executor has dispatcher");
        // Explicit catalog (7) + executor defaults:
        // references (2) + plans (2: getPlan, executePlan) = 11 handlers.
        let names: std::collections::HashSet<String> = executor_dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names.len(), 11, "expected 11 handlers, got {}", names.len());
        // Sanity-check that one tool from each toolkit is present.
        assert!(names.contains("search_catalog"));
        assert!(names.contains("sample_table_data"));
        assert!(names.contains("execute_query"));
        assert!(names.contains("resolveRef"));
        assert!(names.contains("getPlan"));
        assert!(names.contains("executePlan"));
        assert!(!names.contains("storePlan"));
    }

    // ─── Acceptance #6: per-agent isolation ─────────────────────────

    #[test]
    fn coordinator_without_explicit_toolkits_gets_route_dispatcher() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("coordinator", dispatch_env(kv))
            .expect("compose ok");
        let dispatcher = dispatcher.expect("coordinator has call_agent dispatcher");
        let names: std::collections::HashSet<String> = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(
            names,
            std::collections::HashSet::from(["call_agent".to_string()])
        );
    }

    #[test]
    fn unknown_agent_name_returns_none() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("nobody", dispatch_env(kv))
            .expect("compose ok");
        assert!(dispatcher.is_none());
    }

    // ─── Acceptance #2: execute_query rejects writes ────────────────

    #[tokio::test]
    async fn execute_query_rejects_write_with_clear_error() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("executor", dispatch_env(kv))
            .expect("compose ok")
            .expect("executor has dispatcher");

        let result = dispatcher
            .dispatch("execute_query", "tu-1", json!({"sql": "DROP TABLE foo"}))
            .await;

        assert!(result.is_error, "expected error result, got: {:?}", result);
        let err_text = result.content["error"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        // ReadOnlyQuery::parse rejection wording. Different sqlparser
        // versions phrase this slightly differently — we accept any
        // wording that references the read-only restriction.
        let mentions_readonly = err_text.to_lowercase().contains("select")
            || err_text.to_lowercase().contains("read-only")
            || err_text.to_lowercase().contains("readonly")
            || err_text.to_lowercase().contains("mutation")
            || err_text.to_lowercase().contains("only");
        assert!(
            mentions_readonly,
            "execute_query write rejection should mention the read-only restriction; got: {err_text}"
        );
    }

    // Regression test for the per-request env contract: a caller who
    // passes an env without `DataCatalog` / `TargetDatabase` must still
    // get a clean ToolCallResult::error from the catalog tools rather
    // than a panic. This is the failure mode that motivated the
    // `dispatcher_for` API in the first place.
    #[tokio::test]
    async fn catalog_tools_surface_clean_errors_when_env_lacks_extensions() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");

        // Bare env — no catalog, no target_db.
        let bare_env = ToolEnvironment::builder()
            .kv_arc(kv)
            .event_sink_arc(Arc::new(NullEventSink) as Arc<dyn agent_fw_algebra::EventSink>)
            .tenant_context(TenantContext::new(TenantId::new_unchecked("tenant-1")))
            .build();
        let dispatcher = runtime
            .dispatcher_for("executor", bare_env)
            .expect("compose ok")
            .expect("executor has dispatcher");

        // A catalog call surfaces a clean error rather than panicking.
        let result = dispatcher
            .dispatch(
                "search_catalog",
                "tu-bare",
                json!({"query": "anything", "limit": 5}),
            )
            .await;
        assert!(result.is_error, "expected error when env lacks catalog");
    }

    // ─── Acceptance #3: resolveRef round-trip ───────────────────────

    #[tokio::test]
    async fn resolve_ref_round_trips_through_references_toolkit() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("executor", dispatch_env(kv))
            .expect("compose ok")
            .expect("executor has dispatcher");

        let value = json!({"product_ids": ["a", "b", "c"]});
        let glimpse = json!({"n_products": 3});
        let tenant = TenantId::new_unchecked("tenant-1");
        let artifact = runtime
            .references()
            .create("ProductSet", value.clone(), glimpse.clone(), &tenant)
            .await
            .expect("registry create");

        let result = dispatcher
            .dispatch(
                "resolveRef",
                "tu-2",
                json!({"kind": artifact.kind, "id": artifact.id}),
            )
            .await;
        assert!(!result.is_error, "resolveRef errored: {:?}", result);
        assert_eq!(result.content["value"], value);
        assert_eq!(result.content["glimpse"], glimpse);
        assert_eq!(result.content["kind"], "ProductSet");
    }

    // ─── Acceptance #4: getPlan round-trip with status ──────────────

    #[tokio::test]
    async fn get_plan_round_trips_with_draft_status() {
        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(runtime_spec_with_core_toolkits(), deps_for(kv.clone()))
            .expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("executor", dispatch_env(kv))
            .expect("compose ok")
            .expect("executor has dispatcher");

        let plan_id = PlanId::new_unchecked("scenario-1");
        let body = json!({
            "actions": [
                {"kind": "noop", "label": "L"}
            ],
            "rationale": "test"
        });
        runtime
            .propose_plan("ScenarioPlan", plan_id.clone(), body.clone())
            .await
            .expect("propose plan");

        let result = dispatcher
            .dispatch("getPlan", "tu-3", json!({"planId": plan_id.as_str()}))
            .await;
        assert!(!result.is_error, "getPlan errored: {:?}", result);
        assert_eq!(result.content["status"], "draft");
        // ActionSeq<A> serialises as `{"head": A, "tail": [A...]}` — see
        // `agent-fw-plan/src/action.rs`. The first action lives under
        // `actions.head`.
        assert_eq!(result.content["actions"]["head"]["kind"], "noop");
        assert_eq!(result.content["context"]["rationale"], "test");

        // Bonus: unknown id returns null without error.
        let miss = dispatcher
            .dispatch("getPlan", "tu-4", json!({"planId": "nope"}))
            .await;
        assert!(!miss.is_error);
        assert!(miss.content.is_null());
    }

    // ─── Acceptance #5: collision detection ─────────────────────────

    #[test]
    fn cross_toolkit_name_collision_is_rejected_at_build() {
        // Synthesize a clash by listing `catalog` twice with the same
        // tool subset — second insertion records a collision.
        let mut spec = runtime_spec_with_core_toolkits();
        spec.agents[1].toolkits.push("catalog".to_string());

        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(spec, deps_for(kv))
            .err()
            .expect("expected RuntimeError");
        match err {
            RuntimeError::Toolkit(ToolkitError::Collisions(collisions)) => {
                assert!(
                    collisions
                        .iter()
                        .any(|c| c.tool_name == "search_catalog" || c.tool_name == "execute_query"),
                    "expected catalog toolkit collision: {collisions:?}"
                );
            }
            other => panic!("expected ToolkitError::Collisions, got {other:?}"),
        }
    }

    // ─── Composition input validation ───────────────────────────────

    #[test]
    fn unknown_toolkit_id_in_agent_selection_errors() {
        let mut spec = runtime_spec_with_core_toolkits();
        spec.agents[1].toolkits = vec!["does-not-exist".to_string()];

        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(spec, deps_for(kv))
            .err()
            .expect("expected RuntimeError");
        match err {
            RuntimeError::Toolkit(ToolkitError::UnknownToolkit(id)) => {
                assert_eq!(id, "does-not-exist");
            }
            other => panic!("expected UnknownToolkit, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_name_in_config_errors() {
        let mut spec = runtime_spec_with_core_toolkits();
        // Override catalog's config to name a non-existent tool.
        spec.toolkits[0].config = json!({"tools": ["execute_nope"]});

        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(spec, deps_for(kv))
            .err()
            .expect("expected RuntimeError");
        match err {
            RuntimeError::Toolkit(ToolkitError::UnknownTool { toolkit, name }) => {
                assert_eq!(toolkit, "catalog");
                assert_eq!(name, "execute_nope");
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn config_narrowing_filters_handler_set() {
        let mut spec = runtime_spec_with_core_toolkits();
        spec.toolkits[0].config = json!({"tools": ["execute_query"]});

        let kv = Arc::new(DashMapKVStore::new());
        let runtime = Runtime::new(spec, deps_for(kv.clone())).expect("runtime composes");
        let dispatcher = runtime
            .dispatcher_for("executor", dispatch_env(kv))
            .expect("compose ok")
            .expect("dispatcher");
        let names: std::collections::HashSet<String> = dispatcher
            .tool_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(names.contains("execute_query"));
        assert!(!names.contains("search_catalog"));
        assert!(!names.contains("sample_table_data"));
    }

    #[test]
    fn describe_toolkit_tools_uses_runtime_definitions_and_config() {
        let agent = executor_with_catalog_toolkit();
        let spec = ToolkitSpec {
            id: "catalog".to_string(),
            config: json!({"tools": ["execute_query"]}),
        };

        let definitions = describe_toolkit_tools(&spec, &agent).expect("describes toolkit");
        let names: Vec<String> = definitions
            .into_iter()
            .map(|definition| definition.name)
            .collect();

        assert_eq!(names, vec!["execute_query"]);
    }

    #[test]
    fn describe_agents_toolkit_uses_agent_routes() {
        let mut coordinator = coordinator_no_toolkits();
        coordinator.routes = vec!["executor".to_string()];
        let spec = ToolkitSpec {
            id: "agents".to_string(),
            config: serde_json::Value::Null,
        };

        let definitions = describe_toolkit_tools(&spec, &coordinator).expect("describes agents");

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "call_agent");
    }

    #[test]
    fn malformed_toolkit_config_errors() {
        let mut spec = runtime_spec_with_core_toolkits();
        // `tools` must be an array; passing a string trips the serde
        // deserialiser and surfaces as ToolkitError::Config.
        spec.toolkits[0].config = json!({"tools": "not-an-array"});

        let kv = Arc::new(DashMapKVStore::new());
        let err = Runtime::new(spec, deps_for(kv))
            .err()
            .expect("expected RuntimeError");
        match err {
            RuntimeError::Toolkit(ToolkitError::Config { toolkit, .. }) => {
                assert_eq!(toolkit, "catalog");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
