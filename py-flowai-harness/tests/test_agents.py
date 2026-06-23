import pytest
from pydantic import ValidationError

from flowai_harness import (
    ApprovalOverrides,
    ApprovalPolicies,
    ApprovalPolicyPatch,
    AgentSpec,
    ModelSpec,
    RuntimeSpec,
    ToolkitSpec,
    define_coordinator,
    define_executor,
    define_plan,
    define_planner,
    define_runtime,
    define_specialist,
    define_tenant,
    define_tool,
    layered_prompt,
)


@pytest.fixture
def scenario_plan():
    return define_plan("ScenarioPlan", {"type": "object"})


@pytest.fixture
def prompt():
    domain_knowledge = {"segments": ["retail"]}
    return layered_prompt(
        identity="You coordinate scenario planning.",
        domain_knowledge=domain_knowledge,
    )


def test_role_constructors_require_model(scenario_plan, prompt):
    constructors = [
        (define_coordinator, {"prompt": prompt}),
        (define_planner, {"prompt": prompt, "plan": scenario_plan}),
        (define_executor, {"prompt": prompt, "plan": scenario_plan}),
        (define_specialist, {"prompt": prompt}),
    ]

    for constructor, kwargs in constructors:
        with pytest.raises(TypeError):
            constructor("agent", **kwargs)


def test_role_constructors_emit_agent_specs_and_keep_python_metadata_out_of_wire(
    scenario_plan,
    prompt,
):
    search_products = define_tool("search_products", {"query": str})

    coordinator = define_coordinator(
        "scenario_coordinator",
        model={"id": "claude-opus-4-7", "provider": "anthropic"},
        prompt=prompt,
        routes=["scenario_planner", "scenario_executor"],
        approval={"plans": "always", "tools": "never"},
    )
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
    )
    executor = define_executor(
        "scenario_executor",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        tools=[search_products],
    )
    specialist = define_specialist(
        "product_insights",
        model="claude-haiku-4-5",
        prompt="You answer product questions.",
        tools=[search_products],
    )

    assert coordinator.role == "coordinator"
    assert coordinator.routes == ["scenario_planner", "scenario_executor"]
    assert coordinator.toolkits == []
    assert coordinator.stateful is True
    assert coordinator.prompt_cache_key == prompt.cache_key
    assert coordinator.approval_policies == ApprovalPolicyPatch(
        plans="always",
        tools="never",
    )

    assert planner.role == "planner"
    assert planner.plan == scenario_plan
    assert planner.toolkits == []
    assert planner.stateful is True

    assert executor.role == "executor"
    assert executor.plan == scenario_plan
    assert executor.tools == (search_products,)
    assert executor.toolkits == []
    assert executor.stateful is False

    assert specialist.role == "specialist"
    assert specialist.tools == (search_products,)
    assert specialist.toolkits == []
    assert specialist.stateful is False

    dumped = executor.model_dump(by_alias=True, mode="json")
    assert dumped == {
        "name": "scenario_executor",
        "role": "executor",
        "stateful": False,
        "model": {"id": "claude-sonnet-4-6", "provider": None},
        "systemPrompt": prompt.text,
        "routes": [],
        "toolkits": [],
    }
    assert "plan" not in dumped
    assert "tools" not in dumped
    assert "promptCacheKey" not in dumped


def test_role_constructors_accept_per_agent_max_turns(scenario_plan, prompt):
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        max_turns=24,
    )

    assert planner.max_turns == 24
    assert planner.model_dump(by_alias=True, mode="json")["maxTurns"] == 24


def test_agent_spec_rejects_invalid_max_turns(prompt):
    with pytest.raises(ValidationError):
        AgentSpec(
            name="planner",
            role="planner",
            model="claude-sonnet-4-6",
            system_prompt=prompt.text,
            max_turns=0,
        )


def test_agent_spec_validates_raw_approval_policy_mapping(prompt):
    agent = AgentSpec.model_validate(
        {
            "name": "scenario_coordinator",
            "role": "coordinator",
            "model": "claude-sonnet-4-6",
            "systemPrompt": prompt.text,
            "routes": ["planner"],
            "approvalPolicies": {"plans": "always", "tools": "never"},
        }
    )

    assert agent.approval_policies == ApprovalPolicyPatch(plans="always", tools="never")
    assert agent.stateful is True


def test_define_coordinator_requires_at_least_one_route(prompt):
    with pytest.raises(ValueError, match="at least one route"):
        define_coordinator(
            "scenario_coordinator",
            model=ModelSpec(id="claude-sonnet-4-6"),
            prompt=prompt,
        )


def test_agent_spec_rejects_coordinator_without_routes(prompt):
    with pytest.raises(ValidationError, match="at least one route"):
        AgentSpec(
            name="scenario_coordinator",
            role="coordinator",
            model="claude-sonnet-4-6",
            system_prompt=prompt.text,
        )


def test_agent_spec_rejects_non_public_harness_role(prompt):
    with pytest.raises(ValidationError):
        AgentSpec(
            name="builder",
            role="test_case_builder",
            model="claude-sonnet-4-6",
            system_prompt=prompt.text,
        )


def test_runtime_spec_rejects_duplicate_agent_names(prompt):
    coordinator = define_coordinator(
        "agent",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["planner"],
    )
    planner = define_planner(
        "agent",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=define_plan("ScenarioPlan", {"type": "object"}),
    )

    with pytest.raises(ValidationError, match="duplicate agent name 'agent'"):
        RuntimeSpec(
            tenant=define_tenant("acme", "v1"),
            agents=[coordinator, planner],
        )


def test_runtime_spec_rejects_unknown_route_target(prompt):
    coordinator = define_coordinator(
        "scenario_coordinator",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["missing_planner"],
    )

    with pytest.raises(ValidationError, match="unknown route target 'missing_planner'"):
        RuntimeSpec(
            tenant=define_tenant("acme", "v1"),
            agents=[coordinator],
        )


def test_agent_spec_wire_round_trip_is_stable(prompt):
    agent = AgentSpec(
        name="scenario_coordinator",
        role="coordinator",
        model={"id": "claude-sonnet-4-6", "provider": "anthropic"},
        system_prompt=prompt.text,
        routes=["scenario_planner"],
        toolkits=["agents"],
    )

    dumped = agent.model_dump(by_alias=True, mode="json")

    assert (
        AgentSpec.model_validate(dumped).model_dump(by_alias=True, mode="json")
        == dumped
    )


def test_role_stateful_defaults_and_overrides(scenario_plan, prompt):
    coordinator = define_coordinator(
        "scenario_coordinator",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["scenario_planner"],
    )
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        stateful=False,
    )
    executor = define_executor(
        "scenario_executor",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        stateful=True,
    )
    specialist = define_specialist(
        "insights",
        model="claude-sonnet-4-6",
        prompt=prompt,
        stateful=True,
    )

    assert coordinator.stateful is True
    assert planner.stateful is False
    assert executor.stateful is True
    assert specialist.stateful is True


def test_coordinator_and_planner_accept_custom_tools(scenario_plan, prompt):
    classify_intent = define_tool("classify_intent", {"prompt": str}, approval="never")
    search_products = define_tool("search_products", {"query": str}, approval="always")

    coordinator = define_coordinator(
        "scenario_coordinator",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["scenario_planner"],
        tools=[classify_intent],
        toolkits=[],
    )
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        tools=[search_products],
        toolkits=[],
    )

    runtime = define_runtime(define_tenant("acme", "v1"), agents=[coordinator, planner])

    assert runtime.tool_bindings == (classify_intent, search_products)
    assert "classify_intent" in runtime.agents[0].system_prompt
    assert "search_products" in runtime.agents[1].system_prompt
    assert runtime.approval_overrides.tools == {
        "scenario_coordinator": {"classify_intent": {"kind": "never"}},
        "scenario_planner": {"search_products": {"kind": "always"}},
    }


def test_role_default_tools_are_additive_to_explicit_toolkits(scenario_plan, prompt):
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        toolkits=["catalog"],
    )
    executor = define_executor(
        "scenario_executor",
        model="claude-haiku-4-5",
        prompt=prompt,
        plan=scenario_plan,
        toolkits=["catalog"],
    )

    runtime = define_runtime(
        define_tenant("acme", "v1"),
        agents=[planner, executor],
    )

    assert runtime.agents[0].toolkits == ["catalog"]
    assert runtime.agents[1].toolkits == ["catalog"]
    assert [toolkit.id for toolkit in runtime.toolkits] == ["catalog"]

    assert "storePlan" in runtime.agents[0].system_prompt
    assert "getPlan" in runtime.agents[0].system_prompt
    assert "executePlan" not in runtime.agents[0].system_prompt
    assert "search_catalog" in runtime.agents[0].system_prompt

    assert "storePlan" not in runtime.agents[1].system_prompt
    assert "getPlan" in runtime.agents[1].system_prompt
    assert "executePlan" in runtime.agents[1].system_prompt
    assert "resolveRef" in runtime.agents[1].system_prompt
    assert "search_catalog" in runtime.agents[1].system_prompt


def test_explicit_plans_toolkit_keeps_role_scoped_plan_tools(scenario_plan, prompt):
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
        toolkits=["plans"],
    )
    executor = define_executor(
        "scenario_executor",
        model="claude-haiku-4-5",
        prompt=prompt,
        plan=scenario_plan,
        toolkits=["plans"],
    )

    runtime = define_runtime(
        define_tenant("acme", "v1"),
        agents=[planner, executor],
    )

    assert "storePlan" in runtime.agents[0].system_prompt
    assert "getPlan" in runtime.agents[0].system_prompt
    assert "executePlan" not in runtime.agents[0].system_prompt

    assert "storePlan" not in runtime.agents[1].system_prompt
    assert "getPlan" in runtime.agents[1].system_prompt
    assert "executePlan" in runtime.agents[1].system_prompt


def test_define_runtime_derives_agent_toolkits_and_coordinator_approval(
    scenario_plan,
    prompt,
):
    tenant = define_tenant("acme", "v1")
    search_products = define_tool("search_products", {"query": str})
    coordinator = define_coordinator(
        "scenario_coordinator",
        model="claude-opus-4-7",
        prompt=prompt,
        routes=["scenario_planner", "scenario_executor"],
        approval={"plans": "always", "tools": "never"},
    )
    planner = define_planner(
        "scenario_planner",
        model="claude-sonnet-4-6",
        prompt=prompt,
        plan=scenario_plan,
    )
    executor = define_executor(
        "scenario_executor",
        model="claude-haiku-4-5",
        prompt=prompt,
        plan=scenario_plan,
        tools=[search_products],
        toolkits=["catalog"],
    )

    runtime = define_runtime(
        tenant,
        agents=[coordinator, planner, executor],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    )

    assert runtime.plans == [scenario_plan]
    assert [toolkit.id for toolkit in runtime.toolkits] == ["catalog"]
    assert runtime.tool_bindings == (search_products,)
    assert runtime.approval_policies == ApprovalPolicies(plans="always", tools="never")
    assert runtime.approval_overrides == ApprovalOverrides(
        tools={"scenario_executor": {"search_products": "always"}}
    )

    dumped = runtime.model_dump(by_alias=True, mode="json")
    assert "toolBindings" not in dumped
    assert dumped["agents"][1] == {
        "name": "scenario_planner",
        "role": "planner",
        "stateful": True,
        "model": {"id": "claude-sonnet-4-6", "provider": None},
        "systemPrompt": runtime.agents[1].system_prompt,
        "routes": [],
        "toolkits": [],
    }
    assert "storePlan" in runtime.agents[1].system_prompt
    assert "execute_query" in runtime.agents[2].system_prompt
    assert "search_products" in runtime.agents[2].system_prompt
    assert dumped["plans"] == [scenario_plan.model_dump(by_alias=True, mode="json")]
    assert dumped["toolkits"] == [
        {"id": "catalog", "config": None},
    ]
    assert dumped["approvalOverrides"] == {
        "agents": {},
        "tools": {
            "scenario_executor": {
                "search_products": {"kind": "never"},
            }
        },
    }


def test_define_runtime_derives_agent_and_tool_approval_overrides(prompt):
    tenant = define_tenant("acme", "v1")
    search_products = define_tool("search_products", {"query": str}, approval="always")
    specialist = define_specialist(
        "catalog_reader",
        model="claude-sonnet-4-6",
        prompt=prompt,
        tools=[search_products],
        approval={"tools": "always", "plans": "default"},
        tool_approvals={"execute_query": "never"},
    )

    runtime = define_runtime(tenant, agents=[specialist])

    assert runtime.approval_overrides.agents == {
        "catalog_reader": ApprovalPolicyPatch(tools="always")
    }
    assert runtime.approval_overrides.tools["catalog_reader"] == {
        "search_products": {"kind": "always"},
        "execute_query": {"kind": "never"},
    }
    dumped = runtime.model_dump(by_alias=True, mode="json")
    assert dumped["approvalOverrides"] == {
        "agents": {
            "catalog_reader": {
                "tools": {"kind": "always"},
            }
        },
        "tools": {
            "catalog_reader": {
                "search_products": {"kind": "always"},
                "execute_query": {"kind": "never"},
            }
        },
    }


def test_define_runtime_auto_renders_narrowed_catalog_toolkit(prompt):
    tenant = define_tenant("acme", "v1")
    specialist = define_specialist(
        "catalog_reader",
        model="claude-sonnet-4-6",
        prompt=prompt,
        toolkits=["catalog"],
    )

    runtime = define_runtime(
        tenant,
        agents=[specialist],
        toolkits=[ToolkitSpec(id="catalog", config={"tools": ["execute_query"]})],
    )

    rendered = runtime.agents[0].system_prompt
    assert "# Tools" in rendered
    assert "execute_query" in rendered
    assert "| search_catalog |" not in rendered
    assert runtime.agents[0].prompt_cache_key != prompt.cache_key


def test_define_runtime_auto_renders_catalog_toolkit(prompt):
    tenant = define_tenant("acme", "v1")
    specialist = define_specialist(
        "catalog_searcher",
        model="claude-sonnet-4-6",
        prompt=prompt,
        toolkits=["catalog"],
    )

    runtime = define_runtime(tenant, agents=[specialist])
    rendered = runtime.agents[0].system_prompt

    assert "search_catalog" in rendered
    assert "get_catalog_entities" in rendered
    assert "get_relation_paths_between" in rendered
    assert "execute_query" in rendered


def test_define_runtime_renders_agent_host_tools_when_prompt_does_not(prompt):
    tenant = define_tenant("acme", "v1")
    search_products = define_tool(
        "search_products",
        {"query": str},
        description="Search products by query.",
        approval="always",
    )
    specialist = define_specialist(
        "product_searcher",
        model="claude-sonnet-4-6",
        prompt=prompt,
        tools=[search_products],
    )

    runtime = define_runtime(tenant, agents=[specialist])
    rendered = runtime.agents[0].system_prompt

    assert "search_products" in rendered
    assert "Search products by query." in rendered
    assert "| search_products | Search products by query. | always |" in rendered


def test_define_runtime_dedupes_toolkit_tools_already_in_layered_prompt(prompt):
    tenant = define_tenant("acme", "v1")
    explicit_prompt = layered_prompt(
        identity="You read warehouse data.",
        tools=[
            {
                "name": "execute_query",
                "description": "Use only approved customer SQL patterns.",
                "approval": "always",
            }
        ],
    )
    specialist = define_specialist(
        "catalog_reader",
        model="claude-sonnet-4-6",
        prompt=explicit_prompt,
        toolkits=["catalog"],
    )

    runtime = define_runtime(
        tenant,
        agents=[specialist],
        toolkits=[ToolkitSpec(id="catalog", config={"tools": ["execute_query"]})],
    )
    rendered = runtime.agents[0].system_prompt

    assert rendered.count("execute_query") == 1
    assert "Use only approved customer SQL patterns." in rendered
    assert "Execute a validated read-only SQL query" not in rendered


def test_define_runtime_prompt_cache_changes_with_effective_toolkit_config(prompt):
    tenant = define_tenant("acme", "v1")
    specialist = define_specialist(
        "catalog_reader",
        model="claude-sonnet-4-6",
        prompt=prompt,
        toolkits=["catalog"],
    )

    execute_only = define_runtime(
        tenant,
        agents=[specialist],
        toolkits=[ToolkitSpec(id="catalog", config={"tools": ["execute_query"]})],
    )
    search_only = define_runtime(
        tenant,
        agents=[specialist],
        toolkits=[ToolkitSpec(id="catalog", config={"tools": ["search_catalog"]})],
    )

    assert execute_only.agents[0].prompt_cache_key != search_only.agents[0].prompt_cache_key


def test_define_runtime_rejects_multiple_coordinator_approval_sources(prompt):
    tenant = define_tenant("acme", "v1")
    first = define_coordinator(
        "first",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["planner"],
        approval={"plans": "always"},
    )
    second = define_coordinator(
        "second",
        model="claude-sonnet-4-6",
        prompt=prompt,
        routes=["executor"],
        approval={"tools": "always"},
    )

    with pytest.raises(ValidationError, match="approval_policies"):
        define_runtime(tenant, agents=[first, second])
