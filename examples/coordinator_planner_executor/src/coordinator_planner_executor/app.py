from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

from flowai_harness import (
    create_runtime,
    define_app,
    define_coordinator,
    define_executor,
    define_plan,
    define_planner,
    define_runtime,
    define_specialist,
    define_tenant,
    define_workspace_runtime,
)

_DATA_ROOT = Path(".flowai/coordinator-planner-executor")
EXAMPLE_APP_NAME = "coordinator-planner-executor"
PRICE_CHANGE_PLAN_ID = "plan-eval-price-change"

CATALOG_ACTION_PLAN = define_plan(
    "CatalogActionPlan",
    {
        "type": "object",
        "additionalProperties": False,
        "required": ["rationale", "actions"],
        "properties": {
            "rationale": {"type": "string"},
            "actions": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["kind", "productId", "newPrice", "reason"],
                    "properties": {
                        "kind": {
                            "const": "price_change",
                            "description": "Only executable catalog price-change actions are valid.",
                        },
                        "productId": {
                            "type": "string",
                            "description": "Canonical product ID from the request, such as p-001.",
                        },
                        "newPrice": {
                            "type": "number",
                            "description": "Concrete target price. Must never be null.",
                        },
                        "reason": {"type": "string"},
                    },
                },
            },
        },
    },
)

PRICE_CHANGE_ACTION = {
    "type": "price_change",
    "payload": {
        "productId": "p-001",
        "newPrice": 6.49,
        "reason": "Wholesale demand is strong",
    },
}

PRICE_CHANGE_EXPECTED_ACTION = {
    "type": "price_change",
    "payload": {
        "productId": "p-001",
        "newPrice": 6.49,
    },
}

PRICE_CHANGE_PLAN_BODY = {
    "rationale": "Sparkling Water 12pk has strong wholesale demand.",
    "actions": [
        {
            "kind": "price_change",
            "productId": "p-001",
            "newPrice": 6.49,
            "reason": "Wholesale demand is strong",
        }
    ],
}

PRICE_CHANGE_STORE_PLAN_ARGS = {
    "specName": "CatalogActionPlan",
    "planId": PRICE_CHANGE_PLAN_ID,
    "body": PRICE_CHANGE_PLAN_BODY,
}

PRICE_CHANGE_EXECUTE_PLAN_ARGS = {"planId": PRICE_CHANGE_PLAN_ID}

PLANNER_EVAL_TEST_CASE = {
    "id": "tc-planner-price-change",
    "input": (
        "We need a catalog action plan for product p-001, Sparkling Water "
        "12pk. Lower the price to $6.49 because wholesale demand is strong."
    ),
    "expectedTrajectory": ["storePlan"],
    "trajectoryMode": "subsequence",
    "structuredGroundTruth": {
        "kind": "structured",
        "payload": {
            "kind": "flat",
            "payloadMatch": "subset",
            "plannedActions": [PRICE_CHANGE_EXPECTED_ACTION],
        },
    },
    "tags": ["eval-role:planner", "example"],
}

EXECUTOR_EVAL_TEST_CASE = {
    "id": "tc-executor-price-change",
    "input": (
        "Create the catalog action plan for product p-001, Sparkling Water "
        "12pk, to lower the price to $6.49 because wholesale demand is "
        "strong, then apply that plan."
    ),
    "expectedTrajectory": ["storePlan", "executePlan"],
    "trajectoryMode": "subsequence",
    "structuredGroundTruth": {
        "kind": "structured",
        "payload": {
            "kind": "flat",
            "payloadMatch": "subset",
            "executedActions": [PRICE_CHANGE_EXPECTED_ACTION],
        },
    },
    "tags": ["eval-role:executor", "example"],
}


def _runtime_spec(resource_id: str):
    analyst = define_specialist(
        name="data_analyst",
        model="claude-haiku-4-5",
        prompt=(
            "You inspect customer data questions. In this local Studio example, "
            "answer with concise analysis grounded in the connected demo data. "
            "Use the read-only execute_query tool for questions about the demo "
            "database. The SQLite schema is products(product_id, product_name, "
            "category, base_price) and orders(order_id, product_id, "
            "customer_segment, units, revenue, ordered_at). Join orders.product_id "
            "to products.product_id; revenue totals come from orders.revenue."
        ),
        stateful=True,
        toolkits=["catalog"],
    )
    planner = define_planner(
        name="planner",
        model="claude-sonnet-4-6",
        plan=CATALOG_ACTION_PLAN,
        prompt=(
            "You create typed catalog action plans for Studio eval testing. "
            "Use storePlan with specName CatalogActionPlan whenever a prompt asks "
            "you to create or store a plan. storePlan requires arguments shaped "
            "as {\"specName\":\"CatalogActionPlan\",\"planId\":\"...\",\"body\":"
            "{\"rationale\":\"...\",\"actions\":[...]}}. The rationale field "
            "belongs on body, not inside action items. CatalogActionPlan action "
            "items are flat objects with only kind, productId, newPrice, and "
            "reason fields. The only valid action kind is the literal string "
            "\"price_change\". The newPrice field must be the concrete numeric "
            "target price from the user request, never null. Do not include "
            "review, lookup, validation, current-price, no-op, or other "
            "workflow steps as actions; kinds such as REVIEW_CURRENT_PRICE are "
            "invalid because stored plans contain only executable catalog "
            "changes. For a price-change request, store a body like "
            "{\"rationale\":\"...\",\"actions\":[{\"kind\":\"price_change\","
            "\"productId\":\"p-001\",\"newPrice\":6.49,\"reason\":\"...\"}]}. "
            "Use canonical product IDs from the user request; do not invent "
            "slug IDs from product names."
        ),
        approval={"plans": "never", "tools": "never"},
    )
    executor = define_executor(
        name="executor",
        model="claude-sonnet-4-6",
        plan=CATALOG_ACTION_PLAN,
        prompt=(
            "You execute CatalogActionPlan plans for Studio eval testing. "
            "Use getPlan to inspect existing plans and executePlan to apply "
            "them. Never create, store, or revise plans; if the requested "
            "change does not already have a planner-created plan, ask for the "
            "planner to create one before execution."
        ),
        approval={"plans": "never", "tools": "never"},
    )
    coordinator = define_coordinator(
        name="coordinator",
        model="claude-sonnet-4-6",
        prompt=(
            "You coordinate a small data-agent system for local Studio testing. "
            "Route data, database, product, order, and revenue questions to "
            "data_analyst. Route catalog changes that need a new plan to "
            "planner first, then route execution of an existing stored plan to "
            "executor."
        ),
        routes=["data_analyst", "planner", "executor"],
    )
    return define_runtime(
        tenant=define_tenant(resource_id, "0.1"),
        agents=[coordinator, planner, executor, analyst],
        toolkits=[
            {"id": "catalog", "config": {"tools": ["execute_query"]}},
        ],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    )


def _sqlite_url(path: Path) -> str:
    return f"sqlite:{path}"


def _data_environment(resource_id: str) -> dict:
    root = _DATA_ROOT / resource_id
    root.mkdir(parents=True, exist_ok=True)
    target_path = root / "target.db"
    _seed_target_database(target_path)
    return {
        "target_database": {
            "kind": "sqlite",
            "url": _sqlite_url(target_path),
        },
        "catalog": {
            "kind": "sqlite",
            "url": _sqlite_url(root / "catalog.db"),
            "ensure_schema": True,
        },
        "kv": {
            "kind": "sqlite",
            "url": _sqlite_url(root / "kv.db"),
            "ensure_schema": True,
        },
        # The catalog toolkit requires a catalog-search backend. Point it at a
        # local Tantivy index and rebuild on start so search works out of the box.
        "catalog_search": {
            "index_path": str(root / "catalog-index"),
            "rebuild_on_start": True,
        },
    }


def _seed_target_database(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(path) as conn:
        conn.executescript(
            """
            create table if not exists products (
                product_id text primary key,
                product_name text not null,
                category text not null,
                base_price real not null
            );

            create table if not exists orders (
                order_id text primary key,
                product_id text not null references products(product_id),
                customer_segment text not null,
                units integer not null,
                revenue real not null,
                ordered_at text not null
            );

            delete from orders;
            delete from products;

            insert into products(product_id, product_name, category, base_price) values
                ('p-001', 'Sparkling Water 12pk', 'beverages', 6.99),
                ('p-002', 'Protein Bar Chocolate', 'snacks', 2.49),
                ('p-003', 'Organic Pasta 500g', 'pantry', 3.79);

            insert into orders(order_id, product_id, customer_segment, units, revenue, ordered_at) values
                ('o-1001', 'p-001', 'retail', 12, 83.88, '2026-05-01'),
                ('o-1002', 'p-002', 'retail', 35, 87.15, '2026-05-02'),
                ('o-1003', 'p-003', 'wholesale', 50, 189.50, '2026-05-03'),
                ('o-1004', 'p-001', 'wholesale', 40, 279.60, '2026-05-04');
            """
        )


def _dispatch_catalog_actions(actions: Any, _ctx: Any) -> dict[str, Any]:
    count = len(actions) if hasattr(actions, "__len__") else 0
    return {
        "entitiesAffected": count,
        "summary": f"mock-dispatched {count} catalog action(s)",
        "details": {
            "dispatcher": EXAMPLE_APP_NAME,
            "mode": "mock",
        },
    }


def create_example_runtime(resource_id: str = "acme", *, interpreter: str = "anthropic"):
    spec = _runtime_spec(resource_id)
    return create_runtime(
        spec,
        data_environment=_data_environment(resource_id),
        action_dispatcher=_dispatch_catalog_actions,
        interpreter=interpreter,
    )


def _workspace(resource_id: str, display_name: str):
    spec = _runtime_spec(resource_id)
    data_environment = _data_environment(resource_id)
    return define_workspace_runtime(
        workspace_key=resource_id,
        display_name=display_name,
        description="Anthropic-backed local Studio sample workspace with demo data.",
        runtime_spec=spec,
        runtime_factory=lambda: create_example_runtime(resource_id, interpreter="anthropic"),
        data_environment=data_environment,
        metadata={"example": True},
    )


app = define_app(
    name=EXAMPLE_APP_NAME,
    description="Coordinator-planner-executor Flow AI Harness example app.",
    default_workspace="acme",
    workspaces={
        "acme": _workspace(
            "acme",
            "ACME Demo",
        ),
        "globex": _workspace(
            "globex",
            "Globex Demo",
        ),
    },
)
