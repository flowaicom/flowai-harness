import json
from pathlib import Path
from typing import Optional

import pytest
from pydantic import BaseModel, Field

from flowai_harness import (
    AgentSpec,
    ApprovalPolicies,
    ModelSpec,
    RuntimeSpec,
    TenantIdentity,
    ToolkitSpec,
    create_runtime,
    normalize_schema,
    define_plan,
    define_reference,
    define_tenant,
    define_tool,
)


class ProductSetPayload(BaseModel):
    product_ids: list[str] = Field(min_length=1)


class TreeNode(BaseModel):
    name: str
    children: list["TreeNode"] = Field(default_factory=list)


def test_tenant_dump_matches_flowai_runtime_wire_shape():
    tenant = define_tenant(
        {
            "resourceId": "acme",
            "version": "v1",
        }
    )

    dumped = tenant.model_dump(by_alias=True, mode="json")
    assert dumped == {
        "resourceId": "acme",
        "version": "v1",
    }
    assert TenantIdentity.model_validate(dumped) == tenant


def test_schema_normalization_accepts_dict_pydantic_model_and_type_map():
    assert normalize_schema({"type": "object"}) == {"type": "object"}

    pydantic_schema = normalize_schema(ProductSetPayload)
    assert pydantic_schema["type"] == "object"
    assert "product_ids" in pydantic_schema["properties"]

    type_map_schema = normalize_schema({"query": str, "limit": int, "active": bool})
    assert type_map_schema == {
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "limit": {"type": "integer"},
            "active": {"type": "boolean"},
        },
        "required": ["query", "limit", "active"],
    }


def test_schema_normalization_treats_description_key_as_type_map_field():
    assert normalize_schema({"description": str, "limit": int}) == {
        "type": "object",
        "properties": {
            "description": {"type": "string"},
            "limit": {"type": "integer"},
        },
        "required": ["description", "limit"],
    }


def test_schema_normalization_handles_recursive_pydantic_models():
    schema = normalize_schema(TreeNode)

    assert schema["type"] == "object"
    assert "$defs" in schema
    assert schema["properties"]["children"]["items"] == {"$ref": "#/$defs/TreeNode"}


def test_schema_normalization_handles_typing_union_without_dynamic_import():
    schema = normalize_schema(Optional[int])

    assert schema == {"anyOf": [{"type": "integer"}, {"type": "null"}]}


def test_constructor_dumps_are_runtime_spec_compatible():
    tenant = define_tenant("acme", "v1")
    plan = define_plan("ScenarioPlan", {"type": "object"})
    reference = define_reference("ProductSet", ProductSetPayload, ttl_ms=100)
    tool = define_tool("search_products", {"query": str}, approval=lambda args, ctx: True)

    assert tenant.model_dump(by_alias=True, mode="json")["resourceId"] == "acme"
    assert plan.model_dump(by_alias=True, mode="json")["displayAliases"] == []
    assert reference.model_dump(by_alias=True, mode="json")["ttlMs"] == 100
    assert tool.model_dump(by_alias=True, mode="json")["approval"] == {
        "kind": "dynamic",
        "value": "search_products_approval",
    }


def test_runtime_spec_models_round_trip_flowai_runtime_fixture():
    fixture_path = (
        Path(__file__).parents[2]
        / "crates"
        / "flowai-runtime"
        / "tests"
        / "fixtures"
        / "runtime_spec.json"
    )
    fixture = json.loads(fixture_path.read_text())

    runtime = RuntimeSpec.model_validate(fixture)

    assert runtime.agents[0] == AgentSpec(
        name="coordinator",
        role="coordinator",
        model=ModelSpec(id="claude-sonnet-4-6"),
        system_prompt="You coordinate analytical work.",
        routes=["planner", "executor"],
    )
    assert runtime.toolkits[0] == ToolkitSpec(id="catalog")
    assert runtime.approval_policies == ApprovalPolicies()
    assert runtime.model_dump(by_alias=True, mode="json") == fixture


def test_runtime_spec_can_be_constructed_from_existing_specs():
    tenant = define_tenant("acme", "v1")
    plan = define_plan("ScenarioPlan", {"type": "object"})
    reference = define_reference("ProductSet", ProductSetPayload, ttl_ms=100)

    runtime = RuntimeSpec(
        tenant=tenant,
        agents=[
            AgentSpec(
                name="planner",
                role="planner",
                model="claude-sonnet-4-6",
                system_prompt="You produce typed plans.",
            )
        ],
        references=[reference],
        plans=[plan],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    )

    assert runtime.model_dump(by_alias=True, mode="json") == {
        "tenant": tenant.model_dump(by_alias=True, mode="json"),
        "agents": [
            {
                "name": "planner",
                "role": "planner",
                "stateful": True,
                "model": {"id": "claude-sonnet-4-6", "provider": None},
                "systemPrompt": "You produce typed plans.",
                "routes": [],
                "toolkits": [],
            }
        ],
        "references": [reference.model_dump(by_alias=True, mode="json")],
        "plans": [plan.model_dump(by_alias=True, mode="json")],
        "toolkits": [],
        "approvalPolicies": {
            "plans": {"kind": "always"},
            "tools": {"kind": "never"},
        },
        "storageFactories": {"kv": None, "plans": None, "memory": None},
        "providers": {"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    }


def test_create_runtime_rejects_unregistered_dynamic_approval_predicate():
    tenant = define_tenant("acme", "v1")
    tool = define_tool(
        "guarded_tool",
        {"value": str},
        approval={"kind": "dynamic", "value": "missing_predicate"},
    )
    specialist = AgentSpec(
        name="worker",
        role="specialist",
        model="claude-sonnet-4-6",
        system_prompt="Use tools when requested.",
        tools=(tool,),
    )
    runtime = RuntimeSpec(
        tenant=tenant,
        agents=[specialist],
        providers={"anthropic": {"apiKey": "unused"}},
    )

    with pytest.raises(ValueError, match="missing_predicate"):
        create_runtime(runtime, interpreter="scripted")
