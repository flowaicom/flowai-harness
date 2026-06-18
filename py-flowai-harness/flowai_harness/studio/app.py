from __future__ import annotations

import json
import threading
from collections.abc import Callable, Mapping
from types import MappingProxyType
from typing import Any

from flowai_harness.runtime import RuntimeSpec, create_runtime, normalize_data_environment

STUDIO_API_VERSION = "harness-studio/v1"

LOCAL_BASE_CAPABILITIES: tuple[str, ...] = (
    "runtime.inspect",
    "chat.stream",
    "approvals.decide",
    "runs.list",
    "traces.read",
    "tests.manage",
    "evals.run",
)

LOCAL_DATA_CAPABILITIES: tuple[str, ...] = (
    "data.sources",
    "data.profile",
    "knowledge.ingest",
)

LOCAL_CAPABILITIES: tuple[str, ...] = LOCAL_BASE_CAPABILITIES

KNOWN_CAPABILITIES: tuple[tuple[str, str], ...] = (
    ("runtime.inspect", "local"),
    ("chat.stream", "local"),
    ("tools.inspect", "local"),
    ("approvals.decide", "local"),
    ("runs.list", "local"),
    ("traces.read", "local"),
    ("data.sources", "local"),
    ("data.profile", "local"),
    ("knowledge.ingest", "local"),
    ("tests.manage", "local"),
    ("evals.run", "local"),
    ("settings.local", "local"),
    ("enterprise.orgAdmin", "enterprise"),
    ("enterprise.rbac", "enterprise"),
    ("enterprise.auditLogs", "enterprise"),
    ("enterprise.secrets", "enterprise"),
    ("enterprise.deployments", "enterprise"),
    ("enterprise.usage", "enterprise"),
    ("enterprise.policyControls", "enterprise"),
    ("enterprise.workspaceGovernance", "enterprise"),
)


RuntimeFactory = Callable[[], Any]


class WorkspaceRuntimeBinding:
    """One Studio workspace bound to one FlowAI runtime spec and factory."""

    def __init__(
        self,
        *,
        runtime_spec: RuntimeSpec | Mapping[str, Any],
        runtime_factory: RuntimeFactory | None = None,
        runtime: Any | None = None,
        workspace_key: str | None = None,
        display_name: str | None = None,
        description: str | None = None,
        resource_id: str | None = None,
        metadata: Mapping[str, Any] | None = None,
        capabilities: list[str] | tuple[str, ...] | None = None,
        data_environment: Mapping[str, Any] | None = None,
        target_database_url: str | None = None,
    ) -> None:
        if runtime_factory is not None and runtime is not None:
            raise ValueError("workspace runtime accepts either runtime_factory or runtime, not both")
        spec = (
            runtime_spec
            if isinstance(runtime_spec, RuntimeSpec)
            else RuntimeSpec.model_validate(runtime_spec)
        )
        data_environment_wire = normalize_data_environment(
            data_environment,
            target_database_url,
            runtime_resource_id=resource_id or spec.tenant.resource_id,
        )
        self.workspace_key = _optional_non_empty("workspace_key", workspace_key)
        self.display_name = display_name or _default_display_name(self.workspace_key)
        self.description = description
        self.runtime_spec = spec
        self.data_environment = data_environment_wire
        self.runtime_factory = runtime_factory or (
            lambda: create_runtime(spec, data_environment=data_environment_wire)
        )
        self.resource_id = resource_id or spec.tenant.resource_id
        self.metadata = MappingProxyType(dict(metadata or {}))
        self.capabilities = tuple(
            capabilities
            or (
                LOCAL_BASE_CAPABILITIES + LOCAL_DATA_CAPABILITIES
                if data_environment_wire is not None
                else LOCAL_BASE_CAPABILITIES
            )
        )
        self._runtime = runtime
        self._runtime_constructed = runtime is not None
        self._lock = threading.Lock()

    def with_workspace_key(self, workspace_key: str) -> WorkspaceRuntimeBinding:
        key = _required_non_empty("workspace_key", workspace_key)
        if self.workspace_key is not None and self.workspace_key != key:
            raise ValueError(
                f"workspace binding key mismatch: binding has {self.workspace_key!r}, "
                f"registry uses {key!r}"
            )
        self.workspace_key = key
        if self.display_name is None:
            self.display_name = _default_display_name(key)
        return self

    def get_runtime(self) -> Any:
        """Construct and cache the runtime handle for this workspace."""

        if self._runtime_constructed:
            return self._runtime
        with self._lock:
            if not self._runtime_constructed:
                self._runtime = self.runtime_factory()
                self._runtime_constructed = True
        return self._runtime

    @property
    def runtime_constructed(self) -> bool:
        return self._runtime_constructed

    @property
    def has_data_environment(self) -> bool:
        return self.data_environment is not None

    def data_environment_json(self) -> str:
        if self.data_environment is None:
            raise ValueError("workspace runtime binding has no data_environment")
        return json.dumps(self.data_environment, sort_keys=True)

    def workspace_summary(self) -> dict[str, Any]:
        return {
            "workspaceKey": self.workspace_key,
            "displayName": self.display_name,
            "description": self.description,
            "status": "ready",
            "resourceId": self.resource_id,
            "metadata": dict(self.metadata),
            "capabilities": list(self.capabilities),
        }

    def capability_registry(self) -> dict[str, Any]:
        enabled = set(self.capabilities)
        capabilities: list[dict[str, Any]] = []
        for capability_id, scope in KNOWN_CAPABILITIES:
            entry: dict[str, Any] = {
                "id": capability_id,
                "enabled": capability_id in enabled,
                "scope": scope,
            }
            if entry["enabled"] is False:
                entry["reason"] = (
                    "Capability is not implemented by this local M1 Studio server."
                    if scope == "local"
                    else "Enterprise control plane is not configured."
                )
            capabilities.append(entry)
        return {"workspaceKey": self.workspace_key, "capabilities": capabilities}

    def runtime_summary(self) -> dict[str, Any]:
        spec = self.runtime_spec
        return {
            "workspaceKey": self.workspace_key,
            "tenant": {
                "tenantId": spec.tenant.resource_id,
                "version": spec.tenant.version,
            },
            "agents": [_agent_summary(agent, entrypoint=_is_entrypoint(agent)) for agent in spec.agents],
            "providers": [_provider_summary(name, config) for name, config in spec.providers.items()],
            "plans": [
                {"name": plan.name, "schemaRef": f"runtime://plans/{plan.name}/schema"}
                for plan in spec.plans
            ],
            "references": [
                {
                    "name": reference.name,
                    "ttlMs": reference.ttl_ms,
                    "schemaRef": f"runtime://references/{reference.name}/schema",
                }
                for reference in spec.references
            ],
        }

    def agents_response(self) -> dict[str, Any]:
        return {
            "workspaceKey": self.workspace_key,
            "agents": [_agent_summary(agent, entrypoint=_is_entrypoint(agent)) for agent in self.runtime_spec.agents],
        }

    def eval_capabilities_response(self) -> dict[str, Any]:
        return {
            "workspaceKey": self.workspace_key,
            "modes": _eval_capabilities(self.runtime_spec.agents),
        }


class FlowAIApp:
    """Studio-visible registry of workspace runtime bindings."""

    def __init__(
        self,
        *,
        name: str,
        workspaces: Mapping[str, WorkspaceRuntimeBinding],
        default_workspace: str = "default",
        description: str | None = None,
        metadata: Mapping[str, Any] | None = None,
    ) -> None:
        self.name = _required_non_empty("name", name)
        self.app_id = self.name
        self.default_workspace = _required_non_empty("default_workspace", default_workspace)
        if not workspaces:
            raise ValueError("define_app requires at least one workspace runtime binding")
        normalized: dict[str, WorkspaceRuntimeBinding] = {}
        for key, binding in workspaces.items():
            workspace_key = _required_non_empty("workspace key", key)
            if workspace_key in normalized:
                raise ValueError(f"duplicate workspace key {workspace_key!r}")
            if not isinstance(binding, WorkspaceRuntimeBinding):
                raise TypeError("workspaces values must be WorkspaceRuntimeBinding instances")
            normalized[workspace_key] = binding.with_workspace_key(workspace_key)
        if self.default_workspace not in normalized:
            raise ValueError(
                f"default_workspace {self.default_workspace!r} is not registered"
            )
        self.workspaces = MappingProxyType(normalized)
        self.description = description
        self.metadata = MappingProxyType(dict(metadata or {}))

    def workspace(self, workspace_key: str) -> WorkspaceRuntimeBinding:
        key = _required_non_empty("workspace_key", workspace_key)
        try:
            return self.workspaces[key]
        except KeyError as exc:
            raise KeyError(f"workspace {key!r} is not registered") from exc

    def default_binding(self) -> WorkspaceRuntimeBinding:
        return self.workspace(self.default_workspace)

    def workspaces_response(self) -> dict[str, Any]:
        return {
            "defaultWorkspaceKey": self.default_workspace,
            "workspaces": [
                binding.workspace_summary() for binding in self.workspaces.values()
            ],
        }

    def config_js(self, *, api_base_url: str = "/api") -> str:
        payload = {
            "apiBaseUrl": api_base_url,
            "streamTransport": "sse",
            "appName": self.name,
            "studioApiVersion": STUDIO_API_VERSION,
            "defaultWorkspaceKey": self.default_workspace,
        }

        return f"window.__FLOWAI__ = {json.dumps(payload, sort_keys=True)};\n"


def define_workspace_runtime(
    *,
    runtime_spec: RuntimeSpec | Mapping[str, Any],
    runtime_factory: RuntimeFactory | None = None,
    runtime: Any | None = None,
    workspace_key: str | None = None,
    display_name: str | None = None,
    description: str | None = None,
    resource_id: str | None = None,
    metadata: Mapping[str, Any] | None = None,
    capabilities: list[str] | tuple[str, ...] | None = None,
    data_environment: Mapping[str, Any] | None = None,
    target_database_url: str | None = None,
) -> WorkspaceRuntimeBinding:
    """Define a workspace runtime binding for local Studio.

    Args:
        runtime_spec: ``RuntimeSpec`` or mapping validated as one.
        runtime_factory: Zero-argument factory constructing the native
            runtime lazily on first use. Mutually exclusive with
            ``runtime``. Defaults to ``create_runtime(spec,
            data_environment=...)``.
        runtime: Pre-built native runtime handle to reuse.
        workspace_key: Stable workspace key; usually assigned by
            ``define_app(...)`` from the workspaces mapping key.
        display_name: Human-readable workspace name; derived from the
            workspace key when omitted.
        description: Optional workspace description shown in Studio.
        resource_id: Tenant resource id override; defaults to the spec
            tenant's ``resource_id``.
        metadata: Free-form workspace metadata.
        capabilities: Capability ids advertised to Studio. Defaults to the
            local base capabilities, plus the data capabilities when a data
            environment is attached.
        data_environment: Rust data dependencies for built-in toolkits;
            validated against the workspace tenant.
        target_database_url: Shorthand for
            ``data_environment["target_database_url"]``.

    Returns:
        A ``WorkspaceRuntimeBinding`` for registration with
        ``define_app(...)``.

    Raises:
        ValueError: If both ``runtime_factory`` and ``runtime`` are
            supplied, or the data environment conflicts with the workspace
            tenant.
    """

    return WorkspaceRuntimeBinding(
        workspace_key=workspace_key,
        display_name=display_name,
        description=description,
        runtime_spec=runtime_spec,
        runtime_factory=runtime_factory,
        runtime=runtime,
        resource_id=resource_id,
        metadata=metadata,
        capabilities=capabilities,
        data_environment=data_environment,
        target_database_url=target_database_url,
    )


def define_app(
    *,
    name: str,
    workspaces: Mapping[str, WorkspaceRuntimeBinding] | None = None,
    default_workspace: str = "default",
    description: str | None = None,
    metadata: Mapping[str, Any] | None = None,
    runtime_spec: RuntimeSpec | Mapping[str, Any] | None = None,
    runtime_factory: RuntimeFactory | None = None,
    runtime: Any | None = None,
    resource_id: str | None = None,
    capabilities: list[str] | tuple[str, ...] | None = None,
    data_environment: Mapping[str, Any] | None = None,
    target_database_url: str | None = None,
) -> FlowAIApp:
    """Define a local Studio app registry.

    Passing `runtime_spec` is single-runtime sugar. It creates a `default`
    workspace binding.

    Args:
        name: App name; also used as the app id.
        workspaces: Mapping from workspace key to
            ``WorkspaceRuntimeBinding``. Mutually exclusive with
            ``runtime_spec``.
        default_workspace: Key of the workspace served by default; must be
            registered.
        description: Optional app description.
        metadata: Free-form app metadata.
        runtime_spec: Single-runtime sugar; creates one ``default``
            workspace binding from this spec.
        runtime_factory: Zero-argument runtime factory for the sugar
            binding.
        runtime: Pre-built native runtime for the sugar binding.
        resource_id: Tenant resource id override for the sugar binding.
        capabilities: Capability ids for the sugar binding.
        data_environment: Data environment for the sugar binding.
        target_database_url: Target database URL shorthand for the sugar
            binding.

    Returns:
        A ``FlowAIApp`` servable with ``flowai-harness dev`` / ``serve``.

    Raises:
        ValueError: If both ``workspaces`` and ``runtime_spec`` are given,
            neither is given, sugar-only options are combined with
            ``workspaces``, ``default_workspace`` is not registered, or a
            workspace key is empty or duplicated.
        TypeError: If a workspaces value is not a
            ``WorkspaceRuntimeBinding``.
    """

    if workspaces is not None and runtime_spec is not None:
        raise ValueError("define_app accepts either workspaces or runtime_spec, not both")
    if workspaces is None:
        if runtime_spec is None:
            raise ValueError("define_app requires workspaces or runtime_spec")
        workspaces = {
            "default": define_workspace_runtime(
                display_name="Default",
                runtime_spec=runtime_spec,
                runtime_factory=runtime_factory,
                runtime=runtime,
                resource_id=resource_id,
                capabilities=capabilities,
                data_environment=data_environment,
                target_database_url=target_database_url,
            )
        }
        default_workspace = "default"
    elif (
        runtime_factory is not None
        or runtime is not None
        or resource_id is not None
        or data_environment is not None
        or target_database_url is not None
    ):
        raise ValueError(
            "runtime_factory, runtime, resource_id, data_environment, and target_database_url "
            "are only valid with runtime_spec sugar"
        )

    return FlowAIApp(
        name=name,
        workspaces=workspaces,
        default_workspace=default_workspace,
        description=description,
        metadata=metadata,
    )


def _agent_summary(agent: Any, *, entrypoint: bool) -> dict[str, Any]:
    return {
        "agentId": agent.name,
        "name": agent.name,
        "role": agent.role,
        "model": agent.model.id,
        "stateful": agent.stateful,
        "entrypoint": entrypoint,
        "toolkits": list(agent.toolkits),
        "tools": [tool.binding_id or tool.name for tool in agent.tools],
        "routes": list(agent.routes),
    }


def _eval_capabilities(agents: list[Any]) -> list[dict[str, Any]]:
    modes: list[dict[str, Any]] = []

    def first_agent_for_role(role: str) -> Any | None:
        return next((agent for agent in agents if agent.role == role), None)

    role_modes = [
        (
            "coordinator",
            "sequential",
            "Sequential",
            "Evaluate the coordinator-led workflow.",
        ),
        ("planner", "planner", "Planner", "Evaluate planned actions."),
        ("executor", "executor", "Executor", "Evaluate executed actions."),
    ]
    for role, mode, label, description in role_modes:
        agent = first_agent_for_role(role)
        if agent is None:
            continue
        modes.append(
            {
                "mode": mode,
                "label": label,
                "description": description,
                "agentId": agent.name,
                "role": role,
            }
        )

    for agent in agents:
        if agent.role != "specialist":
            continue
        modes.append(
            {
                "mode": "specialist",
                "label": _agent_label(agent.name),
                "description": f"Evaluate the {agent.name} specialist directly.",
                "agentId": agent.name,
                "role": "specialist",
                "targetAgentId": agent.name,
            }
        )

    return modes


def _agent_label(name: str) -> str:
    return " ".join(part for part in name.replace("-", " ").replace("_", " ").split()).title()


def _is_entrypoint(agent: Any) -> bool:
    return agent.role == "coordinator"


def _provider_summary(name: str, config: Any) -> dict[str, Any]:
    credential = {"kind": "missing"}
    configured = False
    if isinstance(config, Mapping):
        if config.get("apiKeyEnv") or config.get("api_key_env"):
            credential = {"kind": "env", "ref": str(config.get("apiKeyEnv") or config.get("api_key_env"))}
            configured = True
        elif config.get("secretRef") or config.get("secret_ref"):
            credential = {
                "kind": "secretRef",
                "ref": str(config.get("secretRef") or config.get("secret_ref")),
            }
            configured = True
        elif config.get("serverManaged") or config.get("server_managed"):
            credential = {"kind": "serverManaged"}
            configured = True
        elif config.get("apiKey") or config.get("api_key"):
            credential = {"kind": "serverManaged"}
            configured = True
    return {"name": name, "configured": configured, "credential": credential}


def _required_non_empty(name: str, value: str) -> str:
    if not isinstance(value, str) or value == "":
        raise ValueError(f"{name} must be a non-empty string")
    return value


def _optional_non_empty(name: str, value: str | None) -> str | None:
    if value is None:
        return None
    return _required_non_empty(name, value)


def _default_display_name(workspace_key: str | None) -> str:
    if not workspace_key:
        return "Default"
    return workspace_key.replace("_", " ").replace("-", " ").title()
