from __future__ import annotations

import os
from typing import Any

from flowai_harness import (
    TestingConfig,
    create_runtime,
    define_coordinator,
    define_executor,
    define_planner,
    define_runtime,
    define_specialist,
    define_tenant,
)

from inventory_scenario.action_dispatcher import build_action_dispatcher
from inventory_scenario.plans import (
    ProductSet,
    inventory_scenario_plan,
)
from inventory_scenario.product_sets import (
    PLANNER_TOOLS,
    resolve_product_set_tool_for_data_environment,
)
from inventory_scenario.prompts import (
    COORDINATOR_PROMPT,
    EXECUTOR_PROMPT,
    PLANNER_PROMPT,
    SPECIALIST_PROMPT,
)
from inventory_scenario.support.data_environment import TENANT_ID
from inventory_scenario.support.mock_platform.client import default_platform_client


ANTHROPIC_API_KEY_ENV = "ANTHROPIC_API_KEY"
DEFAULT_RUNTIME_INTERPRETER = "anthropic"


def build_runtime_spec(*, data_environment: dict[str, Any] | None = None):
    planner_tools = (
        [resolve_product_set_tool_for_data_environment(data_environment)]
        if data_environment is not None
        else PLANNER_TOOLS
    )
    coordinator = define_coordinator(
        name="coordinator",
        model="claude-opus-4-8",
        routes=["planner", "executor", "explorer"],
        approval={"plans": "always", "tools": "never"},
        prompt=COORDINATOR_PROMPT,
    )
    planner = define_planner(
        name="planner",
        model="claude-opus-4-8",
        plan=inventory_scenario_plan,
        tools=planner_tools,
        toolkits=["catalog"],
        prompt=PLANNER_PROMPT,
        max_turns=50,
    )
    executor = define_executor(
        name="executor",
        model="claude-sonnet-4-6",
        plan=inventory_scenario_plan,
        prompt=EXECUTOR_PROMPT,
    )

    explorer = define_specialist(
        name="explorer",
        model="claude-sonnet-4-6",
        toolkits=["catalog"],
        prompt=SPECIALIST_PROMPT,
    )

    return define_runtime(
        tenant=define_tenant(TENANT_ID, "v2026-06"),
        agents=[coordinator, planner, executor, explorer],
        references=[ProductSet],
        plans=[inventory_scenario_plan],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    )


def build_runtime(
    *,
    data_environment: dict[str, Any],
    services: dict[str, Any] | None = None,
    interpreter: str | None = None,
    testing: TestingConfig | None = None,
):
    if services is None:
        platform = default_platform_client(data_environment)
        runtime_services = {"platform": platform}
    else:
        runtime_services = dict(services)
        if "platform" not in runtime_services:
            runtime_services["platform"] = default_platform_client(data_environment)
        platform = runtime_services["platform"]
    kwargs: dict[str, Any] = {
        "data_environment": data_environment,
        "services": runtime_services,
        "action_dispatcher": build_action_dispatcher(platform),
    }
    if testing is None:
        selected_interpreter = interpreter or DEFAULT_RUNTIME_INTERPRETER
        if selected_interpreter == DEFAULT_RUNTIME_INTERPRETER:
            _require_anthropic_api_key()
        kwargs["interpreter"] = selected_interpreter
    elif interpreter is not None:
        kwargs["interpreter"] = interpreter
    if testing is not None:
        kwargs["testing"] = testing
    return create_runtime(build_runtime_spec(data_environment=data_environment), **kwargs)


def _require_anthropic_api_key() -> None:
    if os.environ.get(ANTHROPIC_API_KEY_ENV):
        return
    raise ValueError(
        f"{ANTHROPIC_API_KEY_ENV} is required to build the live inventory scenario runtime"
    )
