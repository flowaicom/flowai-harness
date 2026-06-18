from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from flowai_harness.plans import PlanSpec
from flowai_harness.prompts import LayeredPrompt
from flowai_harness.runtime import ApprovalPolicyPatch, ModelSpec, AgentSpec
from flowai_harness.tools import ToolSpec

ModelInput = str | ModelSpec | Mapping[str, Any]


def define_coordinator(
    name: str,
    *,
    model: ModelInput,
    prompt: str | LayeredPrompt,
    routes: Iterable[str] = (),
    tools: Iterable[ToolSpec] = (),
    stateful: bool | None = None,
    max_turns: int | None = None,
    approval: ApprovalPolicyPatch | Mapping[str, Any] | None = None,
    tool_approvals: Mapping[str, Any] | None = None,
    toolkits: Iterable[str] | None = None,
) -> AgentSpec:
    """Define a coordinator agent spec.

    The coordinator is the user-facing entrypoint agent. It owns the
    conversation thread and hands work off to the agents named in ``routes``.

    Args:
        name: Unique agent name within the runtime spec.
        model: Model selection: a model id string, a ``ModelSpec``, or a
            mapping with ``id`` and optional ``provider``.
        prompt: System prompt as a string or a ``LayeredPrompt``. A
            ``LayeredPrompt`` also contributes its deterministic prompt cache
            key.
        routes: Names of agents this coordinator can hand off to. At least
            one route is required; every route must name an agent registered
            in the same runtime spec.
        tools: ``ToolSpec`` values bound directly to this agent.
        stateful: Whether the agent keeps thread state across turns. Defaults
            to ``True`` for coordinators when ``None``.
        max_turns: Maximum orchestration turns for this agent, or ``None``
            for the runtime default.
        approval: Agent-level approval override for the ``plans`` and
            ``tools`` channels as an ``ApprovalPolicyPatch`` or mapping. The
            ``"default"`` sentinel for a channel means "do not override the
            runtime default for this channel".
        tool_approvals: Per-tool approval rules keyed by tool name. Each rule
            is ``"never"``, ``"always"``, or ``{"kind": "dynamic", "value": id}``.
        toolkits: Additional built-in toolkit ids to attach. Role-default
            route hand-off tools are documented in the prompt automatically;
            this field only records explicit extra toolkit ids.

    Returns:
        A validated ``AgentSpec`` with ``role="coordinator"``.

    Raises:
        ValueError: If ``routes`` is empty, or a ``tool_approvals`` key or
            approval rule is invalid.
        TypeError: If ``prompt`` is neither a string nor a ``LayeredPrompt``.
    """

    route_list = list(routes)
    if not route_list:
        raise ValueError("define_coordinator requires at least one route")
    text, cache_key = _prompt_text_and_cache_key(prompt)
    toolkit_list = _merge_toolkits([], toolkits)
    return AgentSpec(
        name=name,
        role="coordinator",
        stateful=_stateful_default("coordinator", stateful),
        model=model,
        system_prompt=text,
        routes=route_list,
        toolkits=toolkit_list,
        max_turns=max_turns,
        tools=tuple(tools),
        approval_policies=_approval_policies(approval),
        tool_approval_policies=_tool_approval_policies(tool_approvals),
        prompt_cache_key=cache_key,
    )


def define_planner(
    name: str,
    *,
    model: ModelInput,
    prompt: str | LayeredPrompt,
    plan: PlanSpec | Mapping[str, Any],
    tools: Iterable[ToolSpec] = (),
    stateful: bool | None = None,
    max_turns: int | None = None,
    approval: ApprovalPolicyPatch | Mapping[str, Any] | None = None,
    tool_approvals: Mapping[str, Any] | None = None,
    toolkits: Iterable[str] | None = None,
) -> AgentSpec:
    """Define a planner agent spec.

    A planner authors plan instances for the supplied plan schema. The plan
    is auto-attached to the runtime spec when not declared explicitly.

    Args:
        name: Unique agent name within the runtime spec.
        model: Model selection: a model id string, a ``ModelSpec``, or a
            mapping with ``id`` and optional ``provider``.
        prompt: System prompt as a string or a ``LayeredPrompt``.
        plan: ``PlanSpec`` (or mapping validated as one) describing the
            action schema this planner authors.
        tools: ``ToolSpec`` values bound directly to this agent.
        stateful: Whether the agent keeps thread state across turns. Defaults
            to ``True`` for planners when ``None``.
        max_turns: Maximum orchestration turns for this agent, or ``None``
            for the runtime default.
        approval: Agent-level approval override for the ``plans`` and
            ``tools`` channels as an ``ApprovalPolicyPatch`` or mapping.
        tool_approvals: Per-tool approval rules keyed by tool name.
        toolkits: Additional built-in toolkit ids to attach. Planner
            ``storePlan`` / ``getPlan`` prompt tools are added automatically
            by role; this field only records explicit extra toolkit ids.

    Returns:
        A validated ``AgentSpec`` with ``role="planner"``.

    Raises:
        ValueError: If a ``tool_approvals`` key or approval rule is invalid.
        TypeError: If ``prompt`` is neither a string nor a ``LayeredPrompt``.
    """

    text, cache_key = _prompt_text_and_cache_key(prompt)
    plan_spec = _plan_spec(plan)
    return AgentSpec(
        name=name,
        role="planner",
        stateful=_stateful_default("planner", stateful),
        model=model,
        system_prompt=text,
        toolkits=_merge_toolkits([], toolkits),
        max_turns=max_turns,
        plan=plan_spec,
        tools=tuple(tools),
        approval_policies=_approval_policies(approval),
        tool_approval_policies=_tool_approval_policies(tool_approvals),
        prompt_cache_key=cache_key,
    )


def define_executor(
    name: str,
    *,
    model: ModelInput,
    prompt: str | LayeredPrompt,
    plan: PlanSpec | Mapping[str, Any],
    tools: Iterable[ToolSpec] = (),
    stateful: bool | None = None,
    max_turns: int | None = None,
    approval: ApprovalPolicyPatch | Mapping[str, Any] | None = None,
    tool_approvals: Mapping[str, Any] | None = None,
    toolkits: Iterable[str] | None = None,
) -> AgentSpec:
    """Define an executor agent spec.

    An executor carries out approved plan instances of the supplied plan
    schema, emitting business actions for host dispatch.

    Args:
        name: Unique agent name within the runtime spec.
        model: Model selection: a model id string, a ``ModelSpec``, or a
            mapping with ``id`` and optional ``provider``.
        prompt: System prompt as a string or a ``LayeredPrompt``.
        plan: ``PlanSpec`` (or mapping validated as one) describing the
            action schema this executor executes.
        tools: ``ToolSpec`` values bound directly to this agent.
        stateful: Whether the agent keeps thread state across turns. Defaults
            to ``False`` for executors when ``None``.
        max_turns: Maximum orchestration turns for this agent, or ``None``
            for the runtime default.
        approval: Agent-level approval override for the ``plans`` and
            ``tools`` channels as an ``ApprovalPolicyPatch`` or mapping.
        tool_approvals: Per-tool approval rules keyed by tool name.
        toolkits: Additional built-in toolkit ids to attach. Executor
            ``getPlan`` / ``executePlan`` and reference prompt tools are added
            automatically by role; this field only records explicit extra
            toolkit ids.

    Returns:
        A validated ``AgentSpec`` with ``role="executor"``.

    Raises:
        ValueError: If a ``tool_approvals`` key or approval rule is invalid.
        TypeError: If ``prompt`` is neither a string nor a ``LayeredPrompt``.
    """

    text, cache_key = _prompt_text_and_cache_key(prompt)
    return AgentSpec(
        name=name,
        role="executor",
        stateful=_stateful_default("executor", stateful),
        model=model,
        system_prompt=text,
        toolkits=_merge_toolkits([], toolkits),
        max_turns=max_turns,
        plan=_plan_spec(plan),
        tools=tuple(tools),
        approval_policies=_approval_policies(approval),
        tool_approval_policies=_tool_approval_policies(tool_approvals),
        prompt_cache_key=cache_key,
    )


def define_specialist(
    name: str,
    *,
    model: ModelInput,
    prompt: str | LayeredPrompt,
    tools: Iterable[ToolSpec] = (),
    stateful: bool | None = None,
    max_turns: int | None = None,
    approval: ApprovalPolicyPatch | Mapping[str, Any] | None = None,
    tool_approvals: Mapping[str, Any] | None = None,
    toolkits: Iterable[str] | None = None,
) -> AgentSpec:
    """Define a specialist agent spec.

    A specialist is a focused single-purpose agent without routes or a plan.
    It can be routed to by a coordinator or dispatched directly via
    ``runtime.run_specialist(...)``.

    Args:
        name: Unique agent name within the runtime spec.
        model: Model selection: a model id string, a ``ModelSpec``, or a
            mapping with ``id`` and optional ``provider``.
        prompt: System prompt as a string or a ``LayeredPrompt``.
        tools: ``ToolSpec`` values bound directly to this agent.
        stateful: Whether the agent keeps thread state across turns. Defaults
            to ``False`` for specialists when ``None``.
        max_turns: Maximum orchestration turns for this agent, or ``None``
            for the runtime default.
        approval: Agent-level approval override for the ``plans`` and
            ``tools`` channels as an ``ApprovalPolicyPatch`` or mapping.
        tool_approvals: Per-tool approval rules keyed by tool name.
        toolkits: Built-in toolkit ids to attach. Defaults to none.

    Returns:
        A validated ``AgentSpec`` with ``role="specialist"``.

    Raises:
        ValueError: If a ``tool_approvals`` key or approval rule is invalid.
        TypeError: If ``prompt`` is neither a string nor a ``LayeredPrompt``.
    """

    text, cache_key = _prompt_text_and_cache_key(prompt)
    return AgentSpec(
        name=name,
        role="specialist",
        stateful=_stateful_default("specialist", stateful),
        model=model,
        system_prompt=text,
        toolkits=list(toolkits or []),
        max_turns=max_turns,
        tools=tuple(tools),
        approval_policies=_approval_policies(approval),
        tool_approval_policies=_tool_approval_policies(tool_approvals),
        prompt_cache_key=cache_key,
    )


def _prompt_text_and_cache_key(prompt: str | LayeredPrompt) -> tuple[str, str | None]:
    if isinstance(prompt, LayeredPrompt):
        return prompt.text, prompt.cache_key
    if isinstance(prompt, str):
        return prompt, None
    raise TypeError("prompt must be a string or LayeredPrompt")


def _merge_toolkits(required: Iterable[str], extra: Iterable[str] | None) -> list[str]:
    result: list[str] = []
    seen: set[str] = set()
    for toolkit in [*required, *(extra or [])]:
        if toolkit not in seen:
            result.append(toolkit)
            seen.add(toolkit)
    return result


def _plan_spec(plan: PlanSpec | Mapping[str, Any]) -> PlanSpec:
    if isinstance(plan, PlanSpec):
        return plan
    return PlanSpec.model_validate(plan)


def _stateful_default(role: str, stateful: bool | None) -> bool:
    if stateful is not None:
        return stateful
    return role in {"coordinator", "planner"}


def _approval_policies(
    approval: ApprovalPolicyPatch | Mapping[str, Any] | None,
) -> ApprovalPolicyPatch | None:
    """Normalize coordinator approval.

    The `"default"` sentinel means "do not override the runtime default for
    this channel"; e.g. `{"plans": "always", "tools": "default"}` only
    contributes the plan approval floor.
    """

    if approval is None:
        return None
    if isinstance(approval, ApprovalPolicyPatch):
        return approval
    data = dict(approval)
    if data.get("plans") == "default":
        data.pop("plans")
    if data.get("tools") == "default":
        data.pop("tools")
    return ApprovalPolicyPatch.model_validate(data)


def _tool_approval_policies(
    approvals: Mapping[str, Any] | None,
) -> dict[str, dict[str, Any]]:
    if approvals is None:
        return {}
    from flowai_harness.runtime import _approval_rule_to_wire

    result: dict[str, dict[str, Any]] = {}
    for name, approval in approvals.items():
        if not isinstance(name, str) or name == "":
            raise ValueError("tool_approvals keys must be non-empty tool names")
        result[name] = _approval_rule_to_wire(approval)
    return result
