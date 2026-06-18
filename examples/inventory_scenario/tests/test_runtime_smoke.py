from __future__ import annotations

import asyncio
import json
import sqlite3
from pathlib import Path

import pytest
from pydantic import ValidationError

from flowai_harness import FlowAIApp
from flowai_harness.studio.import_resolver import resolve_app_reference
from inventory_scenario.plans import (
    HoldInventoryAction,
    InventoryScenarioPlan,
    ProductSet,
    ProductSetPayload,
    ReorderProductsAction,
    inventory_scenario_plan,
)
from fixtures.sqlite import create_tiny_target_db


async def _collect(stream):
    events = []
    async for event in stream:
        events.append(event)
    return events


class RecordingPlatform:
    def __init__(self):
        self.calls = []
        self.replenishment_calls = []
        self.holdback_calls = []

    async def replenishment(self, payload):
        self.calls.append(payload)
        self.replenishment_calls.append(payload)
        return {
            "action_id": f"action-{len(self.calls)}",
            "created": True,
            "product_count": len(payload["product_ids"]),
        }

    async def holdback(self, payload):
        self.calls.append(payload)
        self.holdback_calls.append(payload)
        return {
            "action_id": f"holdback-{len(self.holdback_calls)}",
            "created": True,
            "product_count": len(payload["product_ids"]),
        }


class RecordingReferences:
    def __init__(self, payloads):
        self.payloads = payloads
        self.resolved = []

    async def resolve(self, handle):
        self.resolved.append(handle)
        return self.payloads[(handle["kind"], handle["id"])]


class ProductSetPlatform(RecordingPlatform):
    async def list_products(self, *, limit: int = 5000, offset: int = 0):
        products = [
            {
                "product_id": "SKU-001",
                "product_name": "Winter Jacket",
                "brand_name": "Apex",
                "segment_name": "Outerwear",
                "region": "WH-WEST",
                "channel_name": "Online",
                "on_hand": 2,
                "safety_stock": 15,
                "reorder_point": 25,
                "holdback_units": 0,
            },
            {
                "product_id": "SKU-002",
                "product_name": "Snow Boot",
                "brand_name": "Apex",
                "segment_name": "Footwear",
                "region": "WH-WEST",
                "channel_name": "Online",
                "on_hand": 18,
                "safety_stock": 10,
                "reorder_point": 20,
                "holdback_units": 0,
            },
            {
                "product_id": "SKU-003",
                "product_name": "Running Tee",
                "brand_name": "Apex",
                "segment_name": "Basics",
                "region": "WH-EAST",
                "channel_name": "Retail",
                "on_hand": 80,
                "safety_stock": 12,
                "reorder_point": 18,
                "holdback_units": 0,
            },
        ]
        return {"products": products[offset : offset + limit], "total": len(products)}


class CreatingReferences:
    def __init__(self):
        self.created = []

    async def create(self, reference, payload):
        self.created.append((reference, payload))
        return {
            "kind": reference.name,
            "id": "ref-low-stock-online",
            "glimpse": reference.glimpse(payload),
        }


def test_runtime_spec_contains_expected_agent_architecture(tmp_path: Path):
    from inventory_scenario.runtime import build_runtime_spec

    spec = build_runtime_spec()
    wire = spec.model_dump(by_alias=True, mode="json")

    assert [agent["name"] for agent in wire["agents"]] == [
        "coordinator",
        "planner",
        "executor",
        "explorer",
    ]
    assert wire["agents"][0]["routes"] == ["planner", "executor", "explorer"]
    assert "catalog" in wire["agents"][1]["toolkits"]
    assert "catalog" in wire["agents"][3]["toolkits"]
    assert wire["references"][0]["name"] == "InventoryProductSet"
    assert wire["plans"][0]["name"] == "InventoryScenarioPlan"
    for agent in spec.agents:
        assert "Use concise, customer-facing language." in agent.system_prompt
    assert "wait for explicit approval before execution" in spec.agents[0].system_prompt
    assert "Route read-only data exploration" in spec.agents[0].system_prompt
    assert "resolveProductSet" in spec.agents[1].system_prompt
    assert "references" in spec.agents[1].system_prompt
    assert "product_set_ref" not in spec.agents[1].system_prompt
    assert "storePlan" in spec.agents[1].system_prompt
    assert "executePlan" in spec.agents[2].system_prompt


def test_planner_prompt_teaches_sql_backed_product_set_resolution():
    from inventory_scenario.runtime import build_runtime_spec

    spec = build_runtime_spec()
    planner = next(agent for agent in spec.agents if agent.name == "planner")

    assert "Use catalog tools and exploratory read-only SQL" in planner.system_prompt
    assert "resolveProductSet" in planner.system_prompt
    assert "SQL is the authoritative product selection" in planner.system_prompt
    assert "filters are audit metadata only" in planner.system_prompt
    assert "references" in planner.system_prompt
    assert "productIds" not in planner.system_prompt
    assert "productSetRef" not in planner.system_prompt
    assert "productGlimpse" not in planner.system_prompt


def test_executor_prompt_uses_execute_plan_boundary():
    from inventory_scenario.runtime import build_runtime_spec

    spec = build_runtime_spec()
    executor = next(agent for agent in spec.agents if agent.name == "executor")

    assert "Call executePlan with the approved plan id" in executor.system_prompt
    assert "Do not resolve product ids manually" in executor.system_prompt
    assert "hydrated references" in executor.system_prompt


def test_planner_prompt_requires_brief_tool_preamble():
    from inventory_scenario.runtime import build_runtime_spec

    spec = build_runtime_spec()
    planner = next(agent for agent in spec.agents if agent.name == "planner")

    assert "Before calling catalog or planner tools" in planner.system_prompt
    assert "brief customer-facing preamble" in planner.system_prompt
    assert "do not reveal hidden chain-of-thought" in planner.system_prompt


def test_planner_output_format_matches_linked_inventory_plan_contract():
    from inventory_scenario.prompts import PLAN_OUTPUT_FORMAT
    from inventory_scenario.runtime import build_runtime_spec

    assert PLAN_OUTPUT_FORMAT["tool"] == "storePlan"
    assert PLAN_OUTPUT_FORMAT["args"]["specName"] == inventory_scenario_plan.name

    body = PLAN_OUTPUT_FORMAT["args"]["body"]
    assert InventoryScenarioPlan.model_validate(body).model_dump(mode="json") == body
    assert body["actions"][0]["references"] == [
        {"kind": "InventoryProductSet", "id": "reference-id-from-resolveProductSet"}
    ]

    spec = build_runtime_spec()
    planner = next(agent for agent in spec.agents if agent.name == "planner")
    assert '"tool": "storePlan"' in planner.system_prompt
    assert '"args": {' in planner.system_prompt


def test_inventory_actions_use_harness_reserved_references():
    action = ReorderProductsAction.model_validate(
        {
            "kind": "reorder_products",
            "name": "Reorder online low-stock products",
            "quantity": 25,
            "reason": "Online inventory is below reorder point.",
            "references": [
                {"kind": "InventoryProductSet", "id": "ref-low-stock-online"}
            ],
        }
    )

    assert action.model_dump(by_alias=True, mode="json") == {
        "kind": "reorder_products",
        "name": "Reorder online low-stock products",
        "quantity": 25,
        "reason": "Online inventory is below reorder point.",
        "references": [
            {"kind": "InventoryProductSet", "id": "ref-low-stock-online"}
        ],
    }


def test_inventory_plan_supports_two_compact_action_kinds():
    plan = InventoryScenarioPlan.model_validate(
        {
            "objective": "Protect availability for online low-stock products.",
            "actions": [
                {
                    "kind": "reorder_products",
                    "name": "Reorder low-stock SKUs",
                    "quantity": 25,
                    "reason": "Products are below reorder point.",
                    "references": [
                        {"kind": "InventoryProductSet", "id": "ref-low-stock"}
                    ],
                },
                {
                    "kind": "hold_inventory",
                    "name": "Hold promotion inventory",
                    "holdbackUnits": 10,
                    "reason": "Reserve units for fulfillment risk.",
                    "references": [
                        {"kind": "InventoryProductSet", "id": "ref-promo-risk"}
                    ],
                },
            ],
            "assumptions": ["Use current target DB snapshot."],
        }
    )

    dumped = plan.model_dump(by_alias=True, mode="json")

    assert [action["kind"] for action in dumped["actions"]] == [
        "reorder_products",
        "hold_inventory",
    ]
    assert dumped["actions"][0]["references"] == [
        {"kind": "InventoryProductSet", "id": "ref-low-stock"}
    ]
    assert "productIds" not in dumped["actions"][0]
    assert "productSetRef" not in dumped["actions"][0]
    assert "productGlimpse" not in dumped["actions"][0]


def test_inventory_action_rejects_inline_product_ids():
    with pytest.raises(ValidationError, match="Extra inputs are not permitted"):
        ReorderProductsAction.model_validate(
            {
                "kind": "reorder_products",
                "name": "Bad inline action",
                "quantity": 10,
                "reason": "This action inlines product ids.",
                "references": [
                    {"kind": "InventoryProductSet", "id": "ref-low-stock-online"}
                ],
                "productIds": ["SKU-001", "SKU-002"],
                "productSetRef": {
                    "kind": "InventoryProductSet",
                    "id": "ref-low-stock-online",
                },
                "productGlimpse": {
                    "productCount": 2,
                    "previewProductIds": ["SKU-001", "SKU-002"],
                },
            }
        )


def test_product_set_reference_glimpse_keeps_query_results_compact():
    payload = ProductSetPayload(
        product_ids=[f"SKU-{index:04d}" for index in range(25)],
        sql=(
            "SELECT product_id, product_name, on_hand, reorder_point "
            "FROM v_scenario_denormalized WHERE on_hand < reorder_point"
        ),
        params=[],
        reason="Find products below reorder point.",
        selection_summary="Low-stock products.",
        sample=[
            {
                "product_id": "SKU-0000",
                "product_name": "Trail Mix",
                "on_hand": 2,
                "reorder_point": 25,
            },
            {
                "product_id": "SKU-0001",
                "product_name": "Hydration Tablets",
                "on_hand": 4,
                "reorder_point": 20,
            },
        ],
    )

    assert ProductSet.glimpse is not None
    assert ProductSet.glimpse(payload) == {
        "productCount": 25,
        "previewProductIds": ["SKU-0000", "SKU-0001", "SKU-0002"],
        "selectionSummary": "Low-stock products.",
        "sample": [
            {
                "product_id": "SKU-0000",
                "product_name": "Trail Mix",
                "on_hand": 2,
                "reorder_point": 25,
            },
            {
                "product_id": "SKU-0001",
                "product_name": "Hydration Tablets",
                "on_hand": 4,
                "reorder_point": 20,
            },
        ],
    }


def test_product_set_payload_model_dump_defaults_to_runtime_aliases():
    payload = ProductSetPayload(
        product_ids=["SKU-001", "SKU-002"],
        sql="SELECT product_id FROM dim_products",
        params=[],
        reason="Alias serialization for runtime references.",
        selection_summary="Two products.",
    )

    dumped = payload.model_dump()

    assert dumped["productIds"] == ["SKU-001", "SKU-002"]
    assert dumped["selectionSummary"] == "Two products."
    assert "product_ids" not in dumped
    assert payload.model_dump(by_alias=False)["product_ids"] == ["SKU-001", "SKU-002"]


def test_product_set_reference_payload_can_be_created_by_runtime(tmp_path: Path):
    from inventory_scenario.runtime import build_runtime
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, data_root / "platform.db")
    runtime = build_runtime(
        data_environment=build_data_environment(data_root),
        interpreter="scripted",
        services={"platform": ProductSetPlatform()},
    )

    async def flow():
        payload = ProductSetPayload(
            product_ids=["SKU-001", "SKU-002"],
            sql="SELECT product_id FROM dim_products ORDER BY product_id LIMIT 2",
            params=[],
            reason="Create a compact product set for runtime reference testing.",
        )
        ref = await runtime.create_reference(ProductSet, payload)
        resolved = await runtime.resolve_reference(ref)
        return ref, resolved

    ref, resolved = asyncio.run(flow())

    assert ref["kind"] == "InventoryProductSet"
    assert ref["glimpse"]["productCount"] == 2
    assert resolved["productIds"] == ["SKU-001", "SKU-002"]


def test_execute_plan_hydrates_product_set_reference_before_dispatch(tmp_path: Path):
    from inventory_scenario.runtime import build_runtime
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, data_root / "platform.db")
    data_environment = build_data_environment(data_root)
    platform = RecordingPlatform()
    runtime = build_runtime(
        data_environment=data_environment,
        interpreter="scripted",
        services={"platform": platform},
    )

    async def flow():
        ref = await runtime.create_reference(
            ProductSet,
            ProductSetPayload(
                product_ids=["SKU-001", "SKU-002", "SKU-003"],
                sql="SELECT product_id FROM dim_products ORDER BY product_id",
                params=[],
                reason="test reference hydration",
                selection_summary="All tiny fixture products.",
            ),
        )
        plan_id = "inventory-plan-hydration"
        plan_action = {
            "kind": "reorder_products",
            "name": "Reorder all products",
            "quantity": 7,
            "reason": "Scripted hydration test.",
            "references": [{"kind": ref["kind"], "id": ref["id"]}],
        }
        assert "references" in plan_action
        assert "productIds" not in plan_action
        assert "product_ids" not in plan_action
        assert "productSetRef" not in plan_action
        planner_prompt = json.dumps(
            {
                "tool": "storePlan",
                "args": {
                    "specName": "InventoryScenarioPlan",
                    "planId": plan_id,
                    "body": {
                        "objective": "Reorder all tiny fixture products.",
                        "actions": [
                            plan_action,
                        ],
                        "assumptions": ["Scripted test."],
                    },
                },
            }
        )
        executor_prompt = json.dumps(
            {"tool": "executePlan", "args": {"planId": plan_id}}
        )
        events = []
        approval_required_seen = False
        async for event in runtime.query(
            json.dumps(
                {
                    "script": [
                        {
                            "tool": "call_agent",
                            "args": {"agent": "planner", "prompt": planner_prompt},
                        },
                        {
                            "tool": "call_agent",
                            "args": {"agent": "executor", "prompt": executor_prompt},
                        },
                    ]
                }
            ),
            thread_id="thread-inventory-hydration",
        ):
            events.append(event)
            if event["type"] == "approval-required":
                approval_required_seen = True
                assert platform.replenishment_calls == []
                assert platform.calls == []
                await runtime.respond_to_approval(event["data"]["id"], "approve")
        assert approval_required_seen
        return events

    events = asyncio.run(flow())

    execute_result = next(
        event["result"]
        for event in events
        if event["type"] == "tool-invocation"
        and event["toolName"] == "executePlan"
        and event["state"] == "result"
    )
    assert execute_result["entitiesAffected"] == 1
    assert len(execute_result["details"]["actions"]) == 1
    assert [call["product_ids"] for call in platform.replenishment_calls] == [
        ["SKU-001", "SKU-002", "SKU-003"]
    ]
    assert platform.replenishment_calls[0]["quantity"] == 7


def test_local_platform_client_accepts_dispatcher_payloads_without_priority(tmp_path: Path):
    from inventory_scenario.support.mock_platform.store import seed_platform_db
    from inventory_scenario.support.mock_platform.client import LocalPlatformClient

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    platform_db = data_root / "platform.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, platform_db)
    client = LocalPlatformClient(platform_db)

    replenishment = asyncio.run(
        client.replenishment(
            {
                "product_ids": ["SKU-001", "SKU-002"],
                "quantity": 25,
                "reason": "Dispatcher-style reorder.",
            }
        )
    )
    holdback = asyncio.run(
        client.holdback(
            {
                "product_ids": ["SKU-001", "SKU-002"],
                "holdback_units": 5,
                "reason": "Dispatcher-style holdback.",
            }
        )
    )

    assert replenishment["created"] is True
    assert replenishment["product_count"] == 2
    assert holdback["created"] is True
    assert holdback["product_count"] == 2


def test_inventory_action_dispatcher_uses_hydrated_product_set_references():
    from inventory_scenario.action_dispatcher import build_action_dispatcher

    platform = RecordingPlatform()
    dispatcher = build_action_dispatcher(platform)
    actions = [
        {
            "kind": "reorder_products",
            "payload": {
                "name": "Reorder low stock",
                "quantity": 25,
                "reason": "Below reorder point.",
            },
            "references": [{"kind": "InventoryProductSet", "id": "ref-products"}],
        },
        {
            "kind": "hold_inventory",
            "payload": {
                "name": "Hold campaign inventory",
                "holdbackUnits": 5,
                "reason": "Reserve units.",
            },
            "references": [{"kind": "InventoryProductSet", "id": "ref-products"}],
        },
    ]
    ctx = {
        "resolved_refs": {
            "InventoryProductSet": {
                "ref-products": {
                    "productIds": ["SKU-001", "SKU-002"],
                    "sql": "SELECT product_id FROM dim_products",
                    "params": [],
                    "reason": "test",
                    "sample": [],
                }
            }
        }
    }

    result = asyncio.run(dispatcher(actions, ctx))

    assert platform.replenishment_calls == [
        {
            "product_ids": ["SKU-001", "SKU-002"],
            "quantity": 25,
            "reason": "Below reorder point.",
        }
    ]
    assert platform.holdback_calls == [
        {
            "product_ids": ["SKU-001", "SKU-002"],
            "holdback_units": 5,
            "reason": "Reserve units.",
        }
    ]
    assert result["entitiesAffected"] == 2
    assert result["summary"] == "Applied 2 inventory action(s)."
    assert result["details"]["actions"][0]["productCount"] == 2


def test_inventory_action_dispatcher_reports_missing_product_reference_id():
    from inventory_scenario.action_dispatcher import build_action_dispatcher

    dispatcher = build_action_dispatcher(RecordingPlatform())
    actions = [
        {
            "kind": "reorder_products",
            "payload": {
                "name": "Missing reference id",
                "quantity": 25,
                "reason": "Exercise validation.",
            },
            "references": [{"kind": "InventoryProductSet"}],
        }
    ]

    with pytest.raises(ValueError, match="InventoryProductSet reference id"):
        asyncio.run(dispatcher(actions, {"resolved_refs": {"InventoryProductSet": {}}}))


def test_runtime_spec_uses_execute_plan_instead_of_executor_mutation_tools():
    from inventory_scenario.runtime import build_runtime_spec

    spec = build_runtime_spec()
    executor = next(agent for agent in spec.agents if agent.name == "executor")
    planner = next(agent for agent in spec.agents if agent.name == "planner")

    assert "resolveProductSet" in planner.system_prompt
    assert "executePlan" in executor.system_prompt
    assert executor.tools == ()
    assert "execute_replenishment_action" not in executor.system_prompt
    assert "execute_promotion_holdback" not in executor.system_prompt


def test_build_runtime_uses_native_references_without_resolver_service(tmp_path: Path):
    from inventory_scenario.runtime import build_runtime
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, data_root / "platform.db")
    data_environment = build_data_environment(data_root)

    runtime = build_runtime(
        data_environment=data_environment,
        interpreter="scripted",
        services={},
    )

    assert runtime is not None


def test_build_runtime_uses_supplied_platform_without_constructing_default(
    tmp_path: Path,
    monkeypatch,
):
    from inventory_scenario import runtime
    from inventory_scenario.support.data_environment import build_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    data_environment = build_data_environment(data_root)

    def fail_default_platform(_data_environment):
        raise AssertionError("default platform should not be constructed")

    monkeypatch.setattr(runtime, "default_platform_client", fail_default_platform)

    app_runtime = runtime.build_runtime(
        data_environment=data_environment,
        interpreter="scripted",
        services={"platform": RecordingPlatform()},
    )

    assert app_runtime is not None


def test_resolve_product_set_uses_sql_and_returns_reference_glimpse(tmp_path: Path):
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.product_sets import resolve_product_set_tool_for_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    data_environment = build_data_environment(data_root)
    resolve_product_set_tool = resolve_product_set_tool_for_data_environment(
        data_environment
    )
    references = CreatingReferences()
    ctx = type("ToolContext", (), {"references": references})()

    result = asyncio.run(
        resolve_product_set_tool.handler(
            {
                "sql": (
                    "SELECT p.product_id, p.product_name, inv.on_hand, inv.reorder_point "
                    "FROM dim_inventory inv "
                    "JOIN dim_products p ON p.product_id = inv.product_id "
                    "WHERE inv.on_hand < ? "
                    "ORDER BY p.product_id"
                ),
                "params": [30],
                "reason": "Resolve products below reorder point.",
                "selectionSummary": "Low-stock products.",
            },
            ctx,
        )
    )

    reference, payload = references.created[0]
    assert reference is ProductSet
    assert payload == ProductSetPayload(
        product_ids=["SKU-003"],
        sql=(
            "SELECT p.product_id, p.product_name, inv.on_hand, inv.reorder_point "
            "FROM dim_inventory inv "
            "JOIN dim_products p ON p.product_id = inv.product_id "
            "WHERE inv.on_hand < ? "
            "ORDER BY p.product_id"
        ),
        params=[30],
        reason="Resolve products below reorder point.",
        selection_summary="Low-stock products.",
        sample=[
            {
                "product_id": "SKU-003",
                "product_name": "Camp Lantern",
                "on_hand": 18,
                "reorder_point": 30,
            }
        ],
    )
    assert set(result) == {"reference", "glimpse"}
    assert "product_ids" not in result
    assert "productIds" not in result
    assert result == {
        "reference": {
            "kind": "InventoryProductSet",
            "id": "ref-low-stock-online",
        },
        "glimpse": {
            "productCount": 1,
            "previewProductIds": ["SKU-003"],
            "selectionSummary": "Low-stock products.",
            "sample": [
                {
                    "product_id": "SKU-003",
                    "product_name": "Camp Lantern",
                    "on_hand": 18,
                    "reorder_point": 30,
                }
            ],
        },
    }


def test_resolve_product_set_stores_all_query_rows_without_tool_limit(tmp_path: Path):
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.product_sets import resolve_product_set_tool_for_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    with sqlite3.connect(target_db) as conn:
        conn.execute("CREATE TABLE dim_products (product_id TEXT PRIMARY KEY)")
        conn.executemany(
            "INSERT INTO dim_products (product_id) VALUES (?)",
            [(f"SKU-{index:05d}",) for index in range(5001)],
        )
    resolve_product_set_tool = resolve_product_set_tool_for_data_environment(
        build_data_environment(data_root)
    )
    references = CreatingReferences()
    ctx = type("ToolContext", (), {"references": references})()

    result = asyncio.run(
        resolve_product_set_tool.handler(
            {
                "sql": "SELECT product_id FROM dim_products ORDER BY product_id",
                "reason": "Resolve the complete large product set.",
                "selectionSummary": "All synthetic products.",
            },
            ctx,
        )
    )

    _, payload = references.created[0]
    assert len(payload.product_ids) == 5001
    assert payload.product_ids[0] == "SKU-00000"
    assert payload.product_ids[-1] == "SKU-05000"
    assert result["glimpse"]["productCount"] == 5001
    assert result["glimpse"]["previewProductIds"] == [
        "SKU-00000",
        "SKU-00001",
        "SKU-00002",
    ]


def test_resolve_product_set_rejects_query_without_product_id(tmp_path: Path):
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.product_sets import resolve_product_set_tool_for_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    resolve_product_set_tool = resolve_product_set_tool_for_data_environment(
        build_data_environment(data_root)
    )
    ctx = type("ToolContext", (), {"references": CreatingReferences()})()

    with pytest.raises(ValueError, match="product_id"):
        asyncio.run(
            resolve_product_set_tool.handler(
                {
                    "sql": "SELECT product_name FROM dim_products ORDER BY product_name",
                    "reason": "This query omits product_id.",
                },
                ctx,
            )
        )


def test_resolve_product_set_rejects_empty_query_without_product_id(tmp_path: Path):
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.product_sets import resolve_product_set_tool_for_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    resolve_product_set_tool = resolve_product_set_tool_for_data_environment(
        build_data_environment(data_root)
    )
    ctx = type("ToolContext", (), {"references": CreatingReferences()})()

    with pytest.raises(ValueError, match="product_id"):
        asyncio.run(
            resolve_product_set_tool.handler(
                {
                    "sql": "SELECT product_name FROM dim_products WHERE 1 = 0",
                    "reason": "This empty query still omits product_id.",
                },
                ctx,
            )
        )


def test_resolve_product_set_rejects_mutation_sql(tmp_path: Path):
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.product_sets import resolve_product_set_tool_for_data_environment

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    resolve_product_set_tool = resolve_product_set_tool_for_data_environment(
        build_data_environment(data_root)
    )
    ctx = type("ToolContext", (), {"references": CreatingReferences()})()

    with pytest.raises(ValueError, match="read-only"):
        asyncio.run(
            resolve_product_set_tool.handler(
                {
                    "sql": "DELETE FROM dim_products",
                    "reason": "Mutation should not be accepted.",
                },
                ctx,
            )
        )


def test_studio_app_import_target_resolves_to_flowai_app():
    app = resolve_app_reference("inventory_scenario.app:runtime")

    assert isinstance(app, FlowAIApp)
    assert app.name == "inventory-scenario"
    assert app.default_workspace == "default"
    assert app.default_binding().data_environment["catalogSearch"]["writeThrough"] is False


def test_explorer_runtime_can_execute_query_against_local_sqlite(tmp_path: Path):
    from inventory_scenario.runtime import build_runtime
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, data_root / "platform.db")
    data_environment = build_data_environment(data_root)

    runtime = build_runtime(
        data_environment=data_environment,
        interpreter="scripted",
        services={"platform": RecordingPlatform()},
    )

    prompt = json.dumps(
        {
            "tool": "execute_query",
            "args": {
                "sql": "SELECT count(*) AS product_count FROM dim_products",
                "limit": 5,
            },
        }
    )
    events = asyncio.run(
        _collect(runtime.run_specialist("explorer", prompt, thread_id="thread-1"))
    )

    result = next(
        event["result"]
        for event in events
        if event["type"] == "tool-invocation"
        and event["toolName"] == "execute_query"
        and event["state"] == "result"
    )
    assert result["rows"] == [{"product_count": 3}]


def test_build_runtime_defaults_to_anthropic_and_requires_api_key(
    tmp_path: Path,
    monkeypatch,
):
    from inventory_scenario.runtime import build_runtime
    from inventory_scenario.support.data_environment import build_data_environment
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    data_root = tmp_path / "data"
    data_root.mkdir()
    target_db = data_root / "target.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, data_root / "platform.db")

    with pytest.raises(ValueError, match="ANTHROPIC_API_KEY"):
        build_runtime(
            data_environment=build_data_environment(data_root),
            services={"platform": RecordingPlatform()},
        )


def test_data_environment_cli_writes_post_seed_descriptor(tmp_path: Path):
    from inventory_scenario.support.data_environment import main as data_environment_main

    data_root = tmp_path / "inventory-data"
    out_path = tmp_path / "data-environment.json"

    assert data_environment_main(["--data-root", str(data_root), "--out", str(out_path)]) == 0

    payload = json.loads(out_path.read_text())
    assert payload["tenant_id"] == "inventory_scenario"
    assert payload["workspace_id"] == "default"
    assert payload["target_database"]["url"] == f"sqlite:{data_root / 'target.db'}"
    assert payload["catalog"]["url"] == f"sqlite:{data_root / 'catalog.db'}"
    assert payload["kv"]["url"] == f"sqlite:{data_root / 'kv.db'}"
    assert payload["catalog_search"]["index_path"] == str(data_root / "catalog-index")
    assert payload["catalog_search"]["rebuild_on_start"] is False
    assert payload["catalog_search"]["write_through"] is False


def test_smoke_success_line_names_verified_checks(tmp_path: Path):
    from inventory_scenario.support.smoke import smoke_success_line

    line = smoke_success_line(data_root=tmp_path / "data", event_count=4)

    assert "inventory scenario smoke ok" in line
    assert "target DB" in line
    assert "catalog query" in line
    assert "mock platform" in line
    assert "scripted runtime" in line


def test_cli_dispatches_smoke_subcommand_with_argv(monkeypatch, tmp_path: Path):
    from types import SimpleNamespace

    from inventory_scenario import cli
    from inventory_scenario.support import smoke

    async def fake_smoke_check():
        return SimpleNamespace(data_root=tmp_path / "data"), [{"type": "done"}]

    monkeypatch.setattr(smoke, "run_smoke_check", fake_smoke_check)

    assert cli.main(["smoke"]) == 0
