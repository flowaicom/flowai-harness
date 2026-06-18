from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Annotated, Any, Callable, Literal, TypedDict

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    field_validator,
    model_serializer,
    model_validator,
)
from pydantic.alias_generators import to_camel

from flowai_harness import _internal
from flowai_harness._native import call_native
from flowai_harness.plans import PlanSpec
from flowai_harness.prompts import augment_prompt_tools
from flowai_harness.references import ReferenceSpec
from flowai_harness.tenant import TenantIdentity
from flowai_harness.tools import ToolSpec

AgentRole = Literal[
    "coordinator",
    "planner",
    "executor",
    "specialist",
]

_DEFAULT_STATEFUL_BY_ROLE: dict[str, bool] = {
    "coordinator": True,
    "planner": True,
    "executor": False,
    "specialist": False,
}

Runtime = _internal.PyRuntime


class TestingConfig(TypedDict):
    """Deterministic native runtime test configuration.

    Passing ``testing=TestingConfig(mock_response=...)`` to
    ``create_runtime(...)`` runs the deterministic mock interpreter, which
    emits ``mock_response`` as the model output instead of calling a
    provider. ``testing`` is mutually exclusive with a non-default
    ``interpreter``.

    Keys:
        mock_response: Text emitted by the mock interpreter for every model
            turn. Required.
    """

    mock_response: str


class DataEnvironmentConfig(TypedDict, total=False):
    """Rust data dependencies attached to built-in toolkit dispatch.

    All keys are optional and accept snake_case or camelCase spellings.

    Keys:
        tenant_id: Tenant id the environment is pinned to. When set it must
            match the runtime tenant ``resource_id``.
        workspace_id: Workspace id scope for stored data.
        kv: KV store descriptor. Supported kinds: ``memory``, ``sqlite``,
            ``postgres``, ``redis``.
        target_database: Target database descriptor for agent data queries.
            Supported kinds: ``sqlite``, ``postgres``. Mutually exclusive
            with ``target_database_url``.
        target_database_url: Connection URL shorthand for the target
            database.
        target_database_schema: Schema name used for target database
            introspection.
        catalog: Data catalog store descriptor. Supported kinds: ``empty``,
            ``inline``, ``sqlite``, ``postgres``.
        catalog_search: Catalog fuzzy-search index configuration with
            ``index_path`` and optional ``rebuild_on_start`` /
            ``write_through`` flags.
    """

    tenant_id: str
    workspace_id: str
    kv: Mapping[str, Any]
    target_database: Mapping[str, Any]
    target_database_url: str
    target_database_schema: str
    catalog: Mapping[str, Any]
    catalog_search: Mapping[str, Any]


class _StorageModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
        hide_input_in_errors=True,
    )


class _RemoteUrlStorageModel(_StorageModel):
    url: str | None = None
    url_env: str | None = None

    @model_validator(mode="after")
    def _validate_url_source(self) -> _RemoteUrlStorageModel:
        if self.url and self.url_env:
            raise ValueError("accepts either url or url_env, not both")
        if not self.url and not self.url_env:
            raise ValueError("requires url_env or url")
        return self


class _MemoryKvStorage(_StorageModel):
    kind: Literal["memory"]


class _SqliteKvStorage(_StorageModel):
    kind: Literal["sqlite"]
    url: str = Field(min_length=1)
    ensure_schema: bool = False


class _PostgresKvStorage(_RemoteUrlStorageModel):
    kind: Literal["postgres"]
    table: str | None = None
    ensure_schema: bool = False


class _RedisKvStorage(_RemoteUrlStorageModel):
    kind: Literal["redis"]
    prefix: str | None = None


class _InlineCatalogStorage(_StorageModel):
    kind: Literal["inline"]
    entries: list[dict[str, Any]] = Field(default_factory=list)


class _EmptyCatalogStorage(_StorageModel):
    kind: Literal["empty"]


class _SqliteCatalogStorage(_StorageModel):
    kind: Literal["sqlite"]
    url: str = Field(min_length=1)
    ensure_schema: bool = False


class _PostgresCatalogStorage(_RemoteUrlStorageModel):
    kind: Literal["postgres"]
    ensure_schema: bool = False


class _CatalogSearchConfig(_StorageModel):
    index_path: str = Field(min_length=1)
    rebuild_on_start: bool = False
    write_through: bool = False


class _SqliteTargetDatabaseStorage(_StorageModel):
    kind: Literal["sqlite"]
    url: str = Field(min_length=1)


class _PostgresTargetDatabaseStorage(_RemoteUrlStorageModel):
    kind: Literal["postgres"]
    schema_: str | None = Field(default=None, alias="schema")


_KvStorage = Annotated[
    _MemoryKvStorage | _SqliteKvStorage | _PostgresKvStorage | _RedisKvStorage,
    Field(discriminator="kind"),
]
_CatalogStorage = Annotated[
    _InlineCatalogStorage
    | _EmptyCatalogStorage
    | _SqliteCatalogStorage
    | _PostgresCatalogStorage,
    Field(discriminator="kind"),
]
_TargetDatabaseStorage = Annotated[
    _SqliteTargetDatabaseStorage | _PostgresTargetDatabaseStorage,
    Field(discriminator="kind"),
]


class _DataEnvironmentSpec(_StorageModel):
    tenant_id: str | None = None
    workspace_id: str | None = None
    kv: _KvStorage | None = None
    target_database: _TargetDatabaseStorage | None = None
    target_database_url: str | None = None
    target_database_schema: str | None = None
    catalog: _CatalogStorage | None = None
    catalog_search: _CatalogSearchConfig | None = None

    @model_validator(mode="after")
    def _validate_target_database_legacy_conflict(self) -> _DataEnvironmentSpec:
        if self.target_database is not None and self.target_database_url is not None:
            raise ValueError(
                "data_environment accepts either 'target_database' or "
                "'target_database_url', not both"
            )
        return self


class ModelSpec(BaseModel):
    """Per-agent model selection.

    A plain model id string is accepted anywhere a ``ModelSpec`` is expected
    and is coerced to ``ModelSpec(id=...)``.
    """

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    id: str = Field(description="Provider model identifier, e.g. a model id string.")
    provider: str | None = Field(
        default=None,
        description=(
            "Provider name from the runtime `providers` mapping. None routes to "
            "the default provider."
        ),
    )

    @model_validator(mode="before")
    @classmethod
    def _coerce_from_string(cls, value: Any) -> Any:
        if isinstance(value, str):
            return {"id": value}
        return value


class ApprovalPolicies(BaseModel):
    """Runtime-level approval policy floor.

    Each channel accepts ``"never"``, ``"always"``, or
    ``{"kind": "dynamic", "value": predicate_id}`` and is normalized to the
    wire shape ``{"kind": ...}``.
    """

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    plans: dict[str, Any] = Field(
        default_factory=lambda: {"kind": "always"},
        description='Approval rule for plan approval gates. Defaults to {"kind": "always"}.',
    )
    tools: dict[str, Any] = Field(
        default_factory=lambda: {"kind": "never"},
        description='Approval rule for tool-call approval gates. Defaults to {"kind": "never"}.',
    )

    @field_validator("plans", "tools", mode="before")
    @classmethod
    def _normalize_approval_rule(cls, value: Any) -> dict[str, Any]:
        return _approval_rule_to_wire(value)


class ApprovalPolicyPatch(BaseModel):
    """Partial agent-level approval override.

    Missing channels inherit from the runtime-level approval policy. Each
    channel accepts ``"never"``, ``"always"``, or
    ``{"kind": "dynamic", "value": predicate_id}``.
    """

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    plans: dict[str, Any] | None = Field(
        default=None,
        description="Approval rule override for plan approval gates. None inherits the runtime floor.",
    )
    tools: dict[str, Any] | None = Field(
        default=None,
        description="Approval rule override for tool-call approval gates. None inherits the runtime floor.",
    )

    @field_validator("plans", "tools", mode="before")
    @classmethod
    def _normalize_approval_rule(cls, value: Any) -> dict[str, Any] | None:
        if value is None:
            return None
        return _approval_rule_to_wire(value)

    @model_serializer(mode="wrap")
    def _serialize_policy_patch(
        self,
        handler: Callable[[ApprovalPolicyPatch], dict[str, Any]],
    ) -> dict[str, Any]:
        data = handler(self)
        data = {key: value for key, value in data.items() if value is not None}
        return data


class ApprovalOverrides(BaseModel):
    """Hierarchical approval overrides scoped by agent and tool.

    Every agent named in ``agents`` or ``tools`` must be registered in the
    same runtime spec; ``RuntimeSpec`` validation rejects unknown names.
    """

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    agents: dict[str, ApprovalPolicyPatch] = Field(
        default_factory=dict,
        description="Per-agent approval policy patches keyed by agent name.",
    )
    tools: dict[str, dict[str, dict[str, Any]]] = Field(
        default_factory=dict,
        description=(
            "Per-tool approval rules keyed by agent name, then tool name. Each "
            "rule is normalized to the wire shape {'kind': ...}."
        ),
    )

    @field_validator("tools", mode="before")
    @classmethod
    def _normalize_tool_overrides(cls, value: Any) -> dict[str, dict[str, dict[str, Any]]]:
        if value is None:
            return {}
        if not isinstance(value, Mapping):
            raise TypeError("approval override tools must be a mapping")
        result: dict[str, dict[str, dict[str, Any]]] = {}
        for agent, tools in value.items():
            if not isinstance(agent, str) or agent == "":
                raise ValueError("approval override agent names must be non-empty strings")
            if not isinstance(tools, Mapping):
                raise TypeError("approval override tool entries must be mappings")
            result[agent] = {}
            for tool, rule in tools.items():
                if not isinstance(tool, str) or tool == "":
                    raise ValueError("approval override tool names must be non-empty strings")
                result[agent][tool] = _approval_rule_to_wire(rule)
        return result


class AgentSpec(BaseModel):
    """Agent registration compiled by `flowai-runtime` into an orchestrator agent."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    name: str = Field(
        min_length=1,
        description="Unique agent name within the runtime spec.",
    )
    role: AgentRole = Field(
        description="Agent role: coordinator, planner, executor, or specialist.",
    )
    stateful: bool = Field(
        description=(
            "Whether the agent keeps thread state across turns. Defaults by "
            "role: true for coordinator and planner, false otherwise."
        ),
    )
    model: ModelSpec = Field(
        description="Model selection. A plain model id string is coerced to ModelSpec.",
    )
    system_prompt: str = Field(
        description="System prompt text. Toolkit and bound-tool rows are merged into its `# Tools` section at assembly.",
    )
    routes: list[str] = Field(
        default_factory=list,
        description="Agent names this agent can hand off to. Coordinators require at least one.",
    )
    toolkits: list[str] = Field(
        default_factory=list,
        description="Built-in toolkit ids attached to this agent.",
    )
    max_turns: int | None = Field(
        default=None,
        ge=1,
        description="Maximum orchestration turns, or None for the runtime default.",
    )
    plan: PlanSpec | None = Field(
        default=None,
        exclude=True,
        description="Plan schema for planner/executor agents. Auto-attached to the runtime spec; excluded from the wire spec.",
    )
    tools: tuple[ToolSpec, ...] = Field(
        default_factory=tuple,
        exclude=True,
        description="Tool bindings attached directly to this agent. Excluded from the wire spec.",
    )
    approval_policies: ApprovalPolicyPatch | None = Field(
        default=None,
        exclude=True,
        description="Agent-level approval override collected into RuntimeSpec.approval_overrides. Excluded from the wire spec.",
    )
    tool_approval_policies: dict[str, dict[str, Any]] = Field(
        default_factory=dict,
        exclude=True,
        description="Per-tool approval rules keyed by tool name. Excluded from the wire spec.",
    )
    prompt_cache_key: str | None = Field(
        default=None,
        exclude=True,
        description=(
            "Deterministic SHA-256 fingerprint of the rendered prompt, used "
            "for prompt change detection and traceability. Excluded from the "
            "wire spec."
        ),
    )

    @model_validator(mode="before")
    @classmethod
    def _default_stateful_for_role(cls, value: Any) -> Any:
        if not isinstance(value, Mapping):
            return value
        data = dict(value)
        role = data.get("role")
        if data.get("stateful") is None and role in _DEFAULT_STATEFUL_BY_ROLE:
            data["stateful"] = _DEFAULT_STATEFUL_BY_ROLE[str(role)]
        return data

    @model_validator(mode="after")
    def _validate_role_contracts(self) -> AgentSpec:
        if self.role == "coordinator" and not self.routes:
            raise ValueError("coordinator agents require at least one route")
        return self

    @model_serializer(mode="wrap")
    def _serialize_agent_spec(
        self,
        handler: Callable[[AgentSpec], dict[str, Any]],
    ) -> dict[str, Any]:
        data = handler(self)
        if self.max_turns is None:
            data.pop("maxTurns", None)
            data.pop("max_turns", None)
        return data


class ToolkitSpec(BaseModel):
    """Toolkit declaration by stable identifier."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    id: str = Field(
        min_length=1,
        description="Stable built-in toolkit identifier, e.g. 'agents', 'plans', 'references', 'catalog'.",
    )
    config: Any | None = Field(
        default=None,
        description="Optional toolkit-specific configuration value.",
    )


class StorageFactorySpec(BaseModel):
    """Host-provided store factory descriptor."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    kind: str = Field(
        description="Store factory implementation identifier supplied by the host.",
    )
    config: Any | None = Field(
        default=None,
        description="Optional factory-specific configuration passed to the store implementation.",
    )


class StorageFactories(BaseModel):
    """Store factory descriptions supplied by the host language facade."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    kv: StorageFactorySpec | None = Field(
        default=None,
        description="Factory for runtime KV state such as references, plans, approval audit, and caches.",
    )
    plans: StorageFactorySpec | None = Field(
        default=None,
        description="Factory for plan lifecycle storage when supplied separately from the runtime KV store.",
    )
    memory: StorageFactorySpec | None = Field(
        default=None,
        description="Factory for persisted agent memory when supplied by the host runtime.",
    )


class RuntimeSpec(BaseModel):
    """Canonical pure runtime specification consumed by `flowai-runtime`."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    tenant: TenantIdentity = Field(
        description="Tenant identity the runtime executes under.",
    )
    agents: list[AgentSpec] = Field(
        default_factory=list,
        description="Registered agents. Names must be unique; at most one coordinator.",
    )
    references: list[ReferenceSpec] = Field(
        default_factory=list,
        description="Named typed reference declarations available to the runtime.",
    )
    plans: list[PlanSpec] = Field(
        default_factory=list,
        description="Plan schemas. Plans declared on agents are auto-attached.",
    )
    toolkits: list[ToolkitSpec] = Field(
        default_factory=list,
        description="Built-in toolkit declarations. Toolkit ids referenced by agents are auto-attached.",
    )
    approval_policies: ApprovalPolicies = Field(
        default_factory=ApprovalPolicies,
        description="Runtime-wide approval floor for the plans and tools channels.",
    )
    approval_overrides: ApprovalOverrides = Field(
        default_factory=ApprovalOverrides,
        description="Per-agent and per-tool approval overrides layered on the floor.",
    )
    storage_factories: StorageFactories = Field(
        default_factory=StorageFactories,
        description="Host-provided store factory descriptors for kv, plans, and memory.",
    )
    providers: dict[str, Any] = Field(
        default_factory=dict,
        description=(
            "Provider configuration keyed by provider name, e.g. "
            '{"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}}.'
        ),
    )
    tool_bindings: tuple[ToolSpec, ...] = Field(
        default_factory=tuple,
        exclude=True,
        description="Runtime-level tool bindings with Python handlers. Excluded from the wire spec.",
    )

    @model_serializer(mode="wrap")
    def _serialize_runtime_spec(
        self,
        handler: Callable[[RuntimeSpec], dict[str, Any]],
    ) -> dict[str, Any]:
        data = handler(self)
        key = "approvalOverrides" if "approvalOverrides" in data else "approval_overrides"
        if data.get(key) == {"agents": {}, "tools": {}}:
            data.pop(key, None)
        return data

    @model_validator(mode="after")
    def _validate_agent_routes(self) -> RuntimeSpec:
        names: set[str] = set()
        coordinator_count = 0
        for agent in self.agents:
            if agent.name in names:
                raise ValueError(f"duplicate agent name '{agent.name}'")
            names.add(agent.name)
            if agent.role == "coordinator":
                coordinator_count += 1

        if coordinator_count > 1:
            raise ValueError("multiple coordinator agents are not supported")

        for agent in self.agents:
            seen_routes: set[str] = set()
            for route in agent.routes:
                if route == agent.name:
                    raise ValueError(f"agent '{agent.name}' cannot route to itself")
                if route in seen_routes:
                    raise ValueError(f"agent '{agent.name}' declares duplicate route '{route}'")
                seen_routes.add(route)
                if route not in names:
                    raise ValueError(
                        f"agent '{agent.name}' declares unknown route target '{route}'"
                    )

        override_agents = set(self.approval_overrides.agents) | set(self.approval_overrides.tools)
        unknown_override_agents = override_agents - names
        if unknown_override_agents:
            raise ValueError(
                "approval_overrides references unknown agent(s): "
                f"{sorted(unknown_override_agents)}"
            )

        return self


def define_runtime(
    tenant: TenantIdentity | Mapping[str, Any],
    *,
    agents: list[AgentSpec | Mapping[str, Any]] | None = None,
    references: list[ReferenceSpec | Mapping[str, Any]] | None = None,
    plans: list[PlanSpec | Mapping[str, Any]] | None = None,
    toolkits: list[ToolkitSpec | Mapping[str, Any]] | None = None,
    approval_policies: ApprovalPolicies | Mapping[str, Any] | None = None,
    approval_overrides: ApprovalOverrides | Mapping[str, Any] | None = None,
    storage_factories: StorageFactories | Mapping[str, Any] | None = None,
    providers: Mapping[str, Any] | None = None,
    tool_bindings: list[ToolSpec] | None = None,
) -> RuntimeSpec:
    """Create a validated Flow AI runtime spec value.

    Collects tenant identity, agents, references, plans, toolkits, approval
    policy, storage descriptors, and provider config into one pure data spec.
    Plans, toolkits, and tool bindings declared on agents are auto-attached,
    and toolkit/agent tool rows are merged into each agent's prompt.

    Args:
        tenant: ``TenantIdentity`` or mapping with ``resource_id`` and
            ``version``.
        agents: ``AgentSpec`` values or mappings validated as such.
        references: ``ReferenceSpec`` values or mappings.
        plans: ``PlanSpec`` values or mappings. Plans attached to agents are
            appended automatically when not listed.
        toolkits: ``ToolkitSpec`` values or mappings. Toolkit ids referenced
            by agents are appended automatically when not listed.
        approval_policies: Runtime-wide approval floor. When omitted, it is
            derived from the coordinator's ``approval`` patch applied on top
            of the defaults (plans ``always``, tools ``never``).
        approval_overrides: Per-agent/per-tool approval overrides. When
            omitted, they are collected from each agent's ``approval`` and
            ``tool_approvals`` declarations.
        storage_factories: Host-provided store factory descriptors.
        providers: Provider configuration keyed by provider name.
        tool_bindings: Runtime-level ``ToolSpec`` bindings. Agent-attached
            tools are appended automatically.

    Returns:
        A frozen, validated ``RuntimeSpec``.

    Raises:
        pydantic.ValidationError: On duplicate agent names, more than one
            coordinator, unknown / duplicate / self-referencing routes,
            approval overrides naming unknown agents, or more than one
            coordinator supplying ``approval_policies``.
    """

    agent_specs = _normalize_agents(agents or [])
    approval_value = (
        approval_policies
        if approval_policies is not None
        else _approval_policies_from_coordinator(agent_specs)
    )
    approval_override_value = (
        approval_overrides
        if approval_overrides is not None
        else _approval_overrides_from_agents(
            agent_specs,
            include_coordinator_agent_policy=approval_policies is not None,
        )
    )
    if not isinstance(approval_override_value, ApprovalOverrides):
        approval_override_value = ApprovalOverrides.model_validate(approval_override_value)
    _validate_approval_overrides_known_agents(approval_override_value, agent_specs)

    toolkit_specs = _with_agent_toolkits(_normalize_toolkits(toolkits or []), agent_specs)
    agent_specs = _with_agent_prompt_tools(agent_specs, toolkit_specs)
    plan_specs = _with_agent_plans(_normalize_plans(plans or []), agent_specs)
    tool_binding_specs = _with_agent_tools(tuple(tool_bindings or ()), agent_specs)

    return RuntimeSpec(
        tenant=tenant,
        agents=agent_specs,
        references=references or [],
        plans=plan_specs,
        toolkits=toolkit_specs,
        approval_policies=approval_value,
        approval_overrides=approval_override_value,
        storage_factories=storage_factories or StorageFactories(),
        providers=dict(providers or {}),
        tool_bindings=tool_binding_specs,
    )


def create_runtime(
    spec: RuntimeSpec | Mapping[str, Any],
    *,
    tool_bindings: list[ToolSpec] | None = None,
    services: Mapping[str, Any] | None = None,
    approval_predicates: Mapping[str, Callable[..., bool]] | None = None,
    action_dispatcher: Callable[..., Any] | None = None,
    event_hooks: list[Callable[..., Any]] | None = None,
    data_environment: DataEnvironmentConfig | Mapping[str, Any] | None = None,
    target_database_url: str | None = None,
    testing: TestingConfig | None = None,
    interpreter: Literal["noop", "scripted", "anthropic"] = "noop",
) -> Runtime:
    """Create an executable runtime handle from a validated spec.

    Call this after ``define_runtime(...)`` when the runtime should start
    handling work. The returned ``Runtime`` is the live object used to stream
    coordinator responses, run specialists, execute evals, manage references
    and approvals, inspect traces, and expose agent tools over MCP.

    The optional arguments attach host capabilities to this runtime instance:
    Python tool handlers, host services available to tool callbacks, dynamic
    approval predicates, an action dispatcher, event hooks, data-environment
    storage/query backends, and deterministic testing behavior. The runtime
    executes under ``spec.tenant.resource_id``; there is no per-call tenant
    override.

    Args:
        spec: ``RuntimeSpec`` or mapping validated as one.
        tool_bindings: Additional ``ToolSpec`` values with Python handlers.
            Agent-attached tools are registered automatically; every tool
            bound to an agent must carry a handler.
        services: Host service objects exposed to Python tool handlers via
            the tool context (``ctx.<name>``, ``ctx["<name>"]``). Keys must
            be non-empty strings and must not use the reserved names
            ``tool_use_id``, ``services``, or ``references``.
        approval_predicates: Dynamic approval predicates keyed by predicate
            id, for tools whose approval is ``{"kind": "dynamic"}`` without
            an attached ``approval_handler``.
        action_dispatcher: Callable that receives executor business actions
            for host-side dispatch.
        event_hooks: Callables invoked for each runtime event during
            streaming.
        data_environment: Rust-owned data dependencies (kv store, target
            database, catalog, catalog search) consumed by built-in
            toolkits. See
            <a href="#flowai_harness.runtime.DataEnvironmentConfig">DataEnvironmentConfig</a>.
        target_database_url: Shorthand for
            ``data_environment["target_database_url"]``. Conflicts with an
            explicit ``target_database`` descriptor or a differing
            ``target_database_url`` value.
        testing: ``TestingConfig`` with ``mock_response``. See
            <a href="#flowai_harness.runtime.TestingConfig">TestingConfig</a>.
            Runs the deterministic mock interpreter; mutually exclusive with
            a non-default ``interpreter``.
        interpreter: Model interpreter key: ``"noop"`` (default, no
            provider), ``"scripted"`` (deterministic scripted replay), or
            ``"anthropic"`` (live provider).

    Returns:
        Native handle returned by the Rust extension. Use the handle to start
            coordinator or specialist runs, run or stream evals, create and
            resolve references, respond to approval gates, inspect traces, and
            expose runtime tools over MCP. See
            <a href="#runtime-handle">Runtime Handle</a>.

    Raises:
        ValueError: If a dynamic approval predicate is not registered, an
            agent tool binding has no Python handler, ``testing`` is combined
            with a non-default ``interpreter``, the testing config is
            malformed, ``target_database_url`` conflicts with
            ``data_environment``, or a service key is reserved.
        TypeError: If ``services`` or data-environment values have invalid
            types.
        pydantic.ValidationError: If ``spec`` or ``data_environment`` fail
            validation.
    """

    runtime_spec = spec if isinstance(spec, RuntimeSpec) else RuntimeSpec.model_validate(spec)
    runtime_spec = _with_runtime_prompt_tools(runtime_spec)
    resource_id = runtime_spec.tenant.resource_id
    data_environment_wire = _normalize_data_environment(
        data_environment,
        target_database_url,
        resource_id,
    )
    service_context = _normalize_services(services)

    all_tools = _with_agent_tools(tuple(tool_bindings or ()), runtime_spec.agents)
    tool_by_key = {_tool_binding_key(tool): tool for tool in all_tools}

    agent_tools = {
        agent.name: [_tool_binding_key(tool) for tool in agent.tools]
        for agent in runtime_spec.agents
        if agent.tools
    }
    tool_callbacks: dict[str, Callable[..., Any]] = {}
    approval_callbacks: dict[str, Callable[..., bool]] = dict(approval_predicates or {})

    for tool in all_tools:
        if tool.approval.get("kind") == "dynamic":
            dynamic_id = str(tool.approval.get("value"))
            if tool.approval_handler is not None:
                approval_callbacks[dynamic_id] = tool.approval_handler
            elif dynamic_id not in approval_callbacks:
                raise ValueError(f"dynamic approval predicate '{dynamic_id}' is not registered")

    for agent_name, binding_keys in agent_tools.items():
        for binding_key in binding_keys:
            tool = tool_by_key[binding_key]
            if tool.handler is None:
                raise ValueError(
                    f"tool binding '{binding_key}' for agent '{agent_name}' has no Python handler"
                )
            tool_callbacks[binding_key] = tool.handler

    tool_specs = [
        {
            "bindingId": binding_key,
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "approval": tool.approval,
        }
        for binding_key, tool in tool_by_key.items()
    ]

    interpreter_key, mock_response = _runtime_interpreter_config(interpreter, testing)

    return call_native(
        _internal.create_runtime,
        json.dumps(runtime_spec.model_dump(by_alias=True, mode="json")),
        resource_id,
        json.dumps(agent_tools),
        json.dumps(tool_specs),
        tool_callbacks,
        approval_callbacks,
        action_dispatcher,
        list(event_hooks or ()),
        interpreter_key,
        mock_response,
        service_context,
        json.dumps(data_environment_wire) if data_environment_wire is not None else None,
    )


def _normalize_data_environment(
    data_environment: DataEnvironmentConfig | Mapping[str, Any] | None,
    target_database_url: str | None,
    runtime_resource_id: str,
) -> dict[str, Any] | None:
    data = dict(data_environment or {})

    if target_database_url is not None:
        existing_target_urls = [
            data[key]
            for key in ("target_database_url", "targetDatabaseUrl")
            if key in data
        ]
        if existing_target_urls and any(
            value != target_database_url for value in existing_target_urls
        ):
            raise ValueError(
                "target_database_url conflicts with data_environment['target_database_url']"
            )
        if "target_database" in data or "targetDatabase" in data:
            raise ValueError(
                "target_database_url conflicts with data_environment['target_database']"
            )
        data["target_database_url"] = target_database_url

    target_database_url_value = _data_environment_alias_value(
        data, "target_database_url", "targetDatabaseUrl"
    )
    if target_database_url_value is not None and not isinstance(
        target_database_url_value, str
    ):
        raise TypeError("data_environment['target_database_url'] must be a string")
    target_database_schema_value = _data_environment_alias_value(
        data, "target_database_schema", "targetDatabaseSchema"
    )
    if target_database_schema_value is not None and not isinstance(
        target_database_schema_value, str
    ):
        raise TypeError("data_environment['target_database_schema'] must be a string")
    tenant_id_value = _data_environment_alias_value(data, "tenant_id", "tenantId")
    _validate_optional_data_environment_id(tenant_id_value, "tenant_id")
    if tenant_id_value is not None and tenant_id_value != runtime_resource_id:
        raise ValueError(
            "data_environment.tenant_id must match the runtime tenant "
            f"'{runtime_resource_id}'"
        )
    workspace_id_value = _data_environment_alias_value(data, "workspace_id", "workspaceId")
    _validate_optional_data_environment_id(workspace_id_value, "workspace_id")

    target_database_descriptor = _data_environment_alias_value(
        data, "target_database", "targetDatabase"
    )
    if target_database_descriptor is not None:
        _validate_storage_descriptor(
            target_database_descriptor,
            key="target_database",
            supported={"sqlite", "postgres"},
        )
    if "kv" in data:
        _validate_storage_descriptor(
            data["kv"],
            key="kv",
            supported={"memory", "sqlite", "postgres", "redis"},
        )
    if "catalog" in data:
        _validate_storage_descriptor(
            data["catalog"],
            key="catalog",
            supported={"empty", "inline", "sqlite", "postgres"},
        )
    catalog_search_descriptor = _data_environment_alias_value(
        data, "catalog_search", "catalogSearch"
    )
    if catalog_search_descriptor is not None and not isinstance(
        catalog_search_descriptor, Mapping
    ):
        raise TypeError("data_environment['catalog_search'] must be a mapping")

    if not data:
        return None
    spec = _DataEnvironmentSpec.model_validate(data)
    return spec.model_dump(by_alias=True, exclude_none=True, mode="json")


def normalize_data_environment(
    data_environment: DataEnvironmentConfig | Mapping[str, Any] | None,
    target_database_url: str | None = None,
    *,
    runtime_resource_id: str,
) -> dict[str, Any] | None:
    """Validate and normalize a data environment without constructing a runtime.

    Args:
        data_environment: ``DataEnvironmentConfig`` or mapping; snake_case
            and camelCase keys are both accepted. See
            <a href="#flowai_harness.runtime.DataEnvironmentConfig">DataEnvironmentConfig</a>.
        target_database_url: Shorthand for
            ``data_environment["target_database_url"]``.
        runtime_resource_id: The runtime tenant the environment must agree
            with; a data environment that pins a different ``tenant_id`` is
            rejected.

    Returns:
        CamelCase mapping passed to the Rust runtime, or ``None`` when no
            environment data is supplied. When present, the mapping can contain
            ``tenantId``, ``workspaceId``, ``kv``, ``targetDatabase``,
            ``targetDatabaseUrl``, ``targetDatabaseSchema``, ``catalog``, and
            ``catalogSearch``. See the
            <a href="#flowai_harness.runtime.DataEnvironmentConfig">DataEnvironmentConfig</a>
            table for key meanings.

    Raises:
        ValueError: If ``target_database_url`` conflicts with the data
            environment, ``tenant_id`` does not match
            ``runtime_resource_id``, or a storage descriptor has an
            unsupported ``kind``.
        TypeError: If a value has an invalid type.
    """

    return _normalize_data_environment(
        data_environment, target_database_url, runtime_resource_id
    )


def _normalize_services(services: Mapping[str, Any] | None) -> dict[str, Any]:
    if services is None:
        return {}
    if not isinstance(services, Mapping):
        raise TypeError("services must be a mapping from service name to Python object")
    reserved = {"tool_use_id", "services", "references"}
    result: dict[str, Any] = {}
    for name, service in services.items():
        if not isinstance(name, str) or name == "":
            raise TypeError("services keys must be non-empty strings")
        if name in reserved:
            raise ValueError(f"services key '{name}' is reserved by the runtime tool context")
        result[name] = service
    return result


def _data_environment_alias_value(
    data: Mapping[str, Any],
    snake_key: str,
    camel_key: str,
) -> Any | None:
    if snake_key in data:
        return data[snake_key]
    if camel_key in data:
        return data[camel_key]
    return None


def _validate_optional_data_environment_id(value: Any | None, key: str) -> None:
    if value is None:
        return
    if not isinstance(value, str):
        raise TypeError(f"data_environment['{key}'] must be a string")
    if not value.strip():
        raise ValueError(f"data_environment['{key}'] must not be blank")


def _validate_storage_descriptor(
    value: Any,
    *,
    key: str,
    supported: set[str],
) -> None:
    if not isinstance(value, Mapping):
        raise TypeError(f"data_environment['{key}'] must be a mapping")
    kind = value.get("kind")
    if not isinstance(kind, str):
        raise TypeError(f"data_environment['{key}']['kind'] must be a string")
    if kind not in supported:
        expected = ", ".join(sorted(supported))
        raise ValueError(
            f"data_environment['{key}'] does not support kind '{kind}'; "
            f"expected one of: {expected}"
        )


def _runtime_interpreter_config(
    interpreter: Literal["noop", "scripted", "anthropic"],
    testing: TestingConfig | None,
) -> tuple[str, str | None]:
    if testing is None:
        return interpreter, None

    if interpreter != "noop":
        raise ValueError(
            "create_runtime accepts either testing or a non-default interpreter, not both"
        )

    data = dict(testing)
    unknown = sorted(set(data) - {"mock_response"})
    if unknown:
        raise ValueError(
            f"unsupported testing config keys: {unknown}; expected ['mock_response']"
        )
    if "mock_response" not in data:
        raise ValueError("testing config requires 'mock_response'")

    response = data["mock_response"]
    if not isinstance(response, str):
        raise TypeError("testing['mock_response'] must be a string")
    return "noop", response


def _approval_rule_to_wire(value: Any) -> dict[str, Any]:
    if value is None:
        raise ValueError("approval rule must not be null")
    if isinstance(value, str):
        if value in {"never", "always"}:
            return {"kind": value}
        raise ValueError("approval rule must be 'never', 'always', or {'kind': 'dynamic'}")
    if isinstance(value, Mapping):
        kind = value.get("kind")
        if kind in {"never", "always"}:
            return {"kind": kind}
        if kind == "dynamic":
            dynamic_value = value.get("value")
            if not isinstance(dynamic_value, str) or dynamic_value == "":
                raise ValueError("dynamic approval rule requires a non-empty value")
            return {"kind": "dynamic", "value": dynamic_value}
    raise ValueError("approval rule must be 'never', 'always', or {'kind': 'dynamic'}")


def _normalize_agents(values: list[AgentSpec | Mapping[str, Any]]) -> list[AgentSpec]:
    return [
        value if isinstance(value, AgentSpec) else AgentSpec.model_validate(value)
        for value in values
    ]


def _normalize_plans(values: list[PlanSpec | Mapping[str, Any]]) -> list[PlanSpec]:
    return [
        value if isinstance(value, PlanSpec) else PlanSpec.model_validate(value)
        for value in values
    ]


def _normalize_toolkits(values: list[ToolkitSpec | Mapping[str, Any]]) -> list[ToolkitSpec]:
    return [
        value if isinstance(value, ToolkitSpec) else ToolkitSpec.model_validate(value)
        for value in values
    ]


def _with_agent_plans(plans: list[PlanSpec], agents: list[AgentSpec]) -> list[PlanSpec]:
    result = list(plans)
    names = {plan.name for plan in result}
    for agent in agents:
        if agent.plan is not None and agent.plan.name not in names:
            result.append(agent.plan)
            names.add(agent.plan.name)
    return result


def _with_agent_toolkits(
    toolkits: list[ToolkitSpec],
    agents: list[AgentSpec],
) -> list[ToolkitSpec]:
    result = list(toolkits)
    ids = {toolkit.id for toolkit in result}
    for agent in agents:
        for toolkit_id in agent.toolkits:
            if toolkit_id not in ids:
                result.append(ToolkitSpec(id=toolkit_id))
                ids.add(toolkit_id)
    return result


def _with_agent_tools(
    tools: tuple[ToolSpec, ...],
    agents: list[AgentSpec],
) -> tuple[ToolSpec, ...]:
    result = list(tools)
    ids = {_tool_binding_key(tool) for tool in result}
    for agent in agents:
        for tool in agent.tools:
            key = _tool_binding_key(tool)
            if key not in ids:
                result.append(tool)
                ids.add(key)
    return tuple(result)


def _with_runtime_prompt_tools(runtime: RuntimeSpec) -> RuntimeSpec:
    agents = _with_agent_prompt_tools(runtime.agents, runtime.toolkits)
    if agents == runtime.agents:
        return runtime
    return runtime.model_copy(update={"agents": agents})


def _with_agent_prompt_tools(
    agents: list[AgentSpec],
    toolkits: list[ToolkitSpec],
) -> list[AgentSpec]:
    toolkit_by_id = {toolkit.id: toolkit for toolkit in toolkits}
    result: list[AgentSpec] = []
    for agent in agents:
        prompt_tools: list[Any] = []
        _extend_prompt_tools(prompt_tools, _role_default_prompt_tools(agent))
        for toolkit_id in agent.toolkits:
            toolkit = toolkit_by_id.get(toolkit_id)
            if toolkit is None:
                continue
            _extend_prompt_tools(prompt_tools, _toolkit_prompt_tools(toolkit, agent))
        _extend_prompt_tools(prompt_tools, agent.tools)

        if not prompt_tools:
            result.append(agent)
            continue

        prompt = augment_prompt_tools(agent.system_prompt, prompt_tools)
        result.append(
            agent.model_copy(
                update={
                    "system_prompt": prompt.text,
                    "prompt_cache_key": prompt.cache_key,
                }
            )
        )
    return result


def _role_default_prompt_tools(agent: AgentSpec) -> list[dict[str, Any]]:
    if agent.role == "coordinator" and agent.routes:
        return _toolkit_prompt_tools(ToolkitSpec(id="agents"), agent)
    if agent.role == "planner":
        return _toolkit_prompt_tools(
            ToolkitSpec(id="plans", config={"tools": ["storePlan", "getPlan"]}),
            agent,
        )
    if agent.role == "executor":
        return [
            *_toolkit_prompt_tools(
                ToolkitSpec(id="plans", config={"tools": ["getPlan", "executePlan"]}),
                agent,
            ),
            *_toolkit_prompt_tools(ToolkitSpec(id="references"), agent),
        ]
    return []


def _extend_prompt_tools(target: list[Any], tools: Iterable[Any]) -> None:
    names = {name for tool in target if isinstance((name := _prompt_tool_name(tool)), str)}
    for tool in tools:
        name = _prompt_tool_name(tool)
        if isinstance(name, str):
            if name in names:
                continue
            names.add(name)
        target.append(tool)


def _prompt_tool_name(tool: Any) -> str | None:
    name = getattr(tool, "name", None)
    if name is None and isinstance(tool, Mapping):
        name = tool.get("name")
    return name if isinstance(name, str) else None


def _toolkit_prompt_tools(toolkit: ToolkitSpec, agent: AgentSpec) -> list[dict[str, Any]]:
    toolkit = _role_scoped_prompt_toolkit(toolkit, agent)
    toolkit_json = json.dumps(toolkit.model_dump(by_alias=True, mode="json"))
    agent_json = json.dumps(agent.model_dump(by_alias=True, mode="json"))
    definitions = json.loads(_internal.describe_toolkit_tools(toolkit_json, agent_json))
    return [
        {
            "name": definition["name"],
            "description": definition.get("description", ""),
            "approval": "runtime",
        }
        for definition in definitions
    ]


def _role_scoped_prompt_toolkit(toolkit: ToolkitSpec, agent: AgentSpec) -> ToolkitSpec:
    if toolkit.id != "plans":
        return toolkit
    allowed = _plan_tools_for_role(agent.role)
    config = toolkit.config
    if config is None:
        return toolkit.model_copy(update={"config": {"tools": allowed}})
    if not isinstance(config, Mapping):
        return toolkit

    tools = config.get("tools")
    if tools is None:
        return toolkit.model_copy(update={"config": {**config, "tools": allowed}})
    if isinstance(tools, list):
        forbidden = [tool for tool in tools if isinstance(tool, str) and tool not in allowed]
        if forbidden:
            raise ValueError(
                "toolkit 'plans' tools are scoped by agent role; "
                f"{agent.role} agents cannot use {forbidden}"
            )
    return toolkit


def _plan_tools_for_role(role: AgentRole) -> list[str]:
    if role == "planner":
        return ["storePlan", "getPlan"]
    if role == "executor":
        return ["getPlan", "executePlan"]
    return ["getPlan"]


def _tool_binding_key(tool: ToolSpec) -> str:
    return tool.binding_id or tool.name


def _approval_policies_from_coordinator(agents: list[AgentSpec]) -> ApprovalPolicies:
    policies = [
        agent.approval_policies
        for agent in agents
        if agent.role == "coordinator" and agent.approval_policies is not None
    ]
    if len(policies) > 1:
        _raise_runtime_validation_error(
            "only one coordinator can provide approval_policies; pass approval_policies explicitly"
        )
    if policies:
        return _apply_approval_patch(ApprovalPolicies(), policies[0])
    return ApprovalPolicies()


def _approval_overrides_from_agents(
    agents: list[AgentSpec],
    *,
    include_coordinator_agent_policy: bool,
) -> ApprovalOverrides:
    agent_overrides: dict[str, ApprovalPolicyPatch] = {}
    tool_overrides: dict[str, dict[str, dict[str, Any]]] = {}

    for agent in agents:
        if (
            agent.approval_policies is not None
            and (include_coordinator_agent_policy or agent.role != "coordinator")
        ):
            agent_overrides[agent.name] = agent.approval_policies

        scoped_tools: dict[str, dict[str, Any]] = {}
        for tool in agent.tools:
            scoped_tools[_tool_binding_key(tool)] = tool.approval
        scoped_tools.update(agent.tool_approval_policies)
        if scoped_tools:
            tool_overrides[agent.name] = scoped_tools

    return ApprovalOverrides(agents=agent_overrides, tools=tool_overrides)


def _apply_approval_patch(
    base: ApprovalPolicies,
    patch: ApprovalPolicyPatch,
) -> ApprovalPolicies:
    return ApprovalPolicies(
        plans=patch.plans if patch.plans is not None else base.plans,
        tools=patch.tools if patch.tools is not None else base.tools,
    )


def _validate_approval_overrides_known_agents(
    overrides: ApprovalOverrides,
    agents: list[AgentSpec],
) -> None:
    names = {agent.name for agent in agents}
    unknown = (set(overrides.agents) | set(overrides.tools)) - names
    if unknown:
        _raise_runtime_validation_error(
            f"approval_overrides references unknown agent(s): {sorted(unknown)}"
        )


def _raise_runtime_validation_error(message: str) -> None:
    from pydantic import ValidationError

    raise ValidationError.from_exception_data(
        "RuntimeSpec",
        [
            {
                "type": "value_error",
                "loc": ("approval_policies",),
                "input": None,
                "ctx": {"error": ValueError(message)},
            }
        ],
    )
