import pytest
from pydantic import BaseModel, ValidationError

from flowai_harness import (
    ToolSpec,
    define_plan,
    define_reference,
    define_tenant,
    define_tool,
)


class SelectionPayload(BaseModel):
    ids: list[str]
    label: str


class ScenarioPlanPayload(BaseModel):
    scope_ref: str
    actions: list[dict]
    rationale: str


def test_define_tenant_accepts_happy_path_and_emits_camel_case():
    tenant = define_tenant(
        resource_id="acme-corp",
        version="2026-05-15",
    )

    assert tenant.model_dump(by_alias=True, mode="json") == {
        "resourceId": "acme-corp",
        "version": "2026-05-15",
    }


def test_define_tenant_accepts_mapping_payload_and_kwargs_override():
    tenant = define_tenant({"resourceId": "old", "version": "v1"}, resource_id="acme")

    assert tenant.resource_id == "acme"
    assert tenant.version == "v1"


def test_define_tenant_rejects_empty_identity():
    with pytest.raises(ValidationError) as exc_info:
        define_tenant(resource_id="", version="v1")

    message = str(exc_info.value)
    assert "resource_id" in message or "resourceId" in message


def test_define_plan_accepts_pydantic_schema():
    plan = define_plan(
        name="ScenarioPlan",
        schema=ScenarioPlanPayload,
        display_aliases={"draft": "pending_approval"},
    )

    dumped = plan.model_dump(by_alias=True, mode="json")
    assert dumped["name"] == "ScenarioPlan"
    assert dumped["schema"]["type"] == "object"
    assert dumped["displayAliases"] == [{"status": "draft", "alias": "pending_approval"}]


def test_define_plan_rejects_unknown_display_status():
    with pytest.raises(ValidationError) as exc_info:
        define_plan(
            name="ScenarioPlan",
            schema={"type": "object"},
            display_aliases={"pending_approval": "Pending approval"},
        )

    assert "pending_approval" in str(exc_info.value)


def test_define_reference_accepts_pydantic_schema_and_python_only_glimpse():
    reference = define_reference(
        name="Selection",
        schema=SelectionPayload,
        ttl_ms=3600000,
        glimpse=lambda value: {"selectionCount": len(value.ids), "label": value.label},
    )

    dumped = reference.model_dump(by_alias=True, mode="json")
    assert dumped["name"] == "Selection"
    assert dumped["schema"]["type"] == "object"
    assert dumped["ttlMs"] == 3600000
    assert "glimpse" not in dumped
    assert reference.glimpse is not None
    assert reference.glimpse(SelectionPayload(ids=["a"], label="Control")) == {
        "selectionCount": 1,
        "label": "Control",
    }


def test_define_reference_rejects_negative_ttl():
    with pytest.raises(ValidationError) as exc_info:
        define_reference(name="ProductSet", schema={"type": "object"}, ttl_ms=-1)

    assert "ttl_ms" in str(exc_info.value)


def test_define_tool_accepts_direct_and_decorator_forms():
    direct = define_tool(
        name="search_products",
        description="Search products",
        input_schema={"query": str, "limit": int},
        approval="never",
    )

    assert isinstance(direct, ToolSpec)
    assert direct.model_dump(by_alias=True, mode="json") == {
        "name": "search_products",
        "description": "Search products",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer"},
            },
            "required": ["query", "limit"],
        },
        "approval": {"kind": "never"},
        "outputSchema": None,
        "bindingId": None,
    }

    @define_tool(name="create_scenario", input_schema={"plan_ref": str}, approval="always")
    async def create_scenario(args, ctx):
        return {"ok": True}

    assert isinstance(create_scenario, ToolSpec)
    assert create_scenario.handler is not None
    assert create_scenario.approval == {"kind": "always"}

    rebound = create_scenario.bind(create_scenario.handler)
    assert rebound.handler is create_scenario.handler


def test_define_tool_rejects_unknown_approval_string():
    with pytest.raises(ValidationError) as exc_info:
        define_tool(name="search_products", input_schema={"type": "object"}, approval="sometimes")

    assert "approval" in str(exc_info.value)
