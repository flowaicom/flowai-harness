from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

from inventory_scenario.support.mock_platform.api import create_app
from inventory_scenario.support.mock_platform.store import seed_platform_db
from fixtures.sqlite import create_tiny_target_db


def _client(tmp_path: Path) -> TestClient:
    target_db = tmp_path / "target.db"
    platform_db = tmp_path / "platform.db"
    create_tiny_target_db(target_db)
    seed_platform_db(target_db, platform_db)
    return TestClient(create_app(platform_db))


def test_platform_api_exposes_health_products_and_summary(tmp_path: Path):
    client = _client(tmp_path)

    assert client.get("/health").json() == {"ok": True}
    assert client.get("/state/summary").json()["products"] == 3
    products = client.get("/products", params={"limit": 2}).json()

    assert [product["product_id"] for product in products["products"]] == [
        "SKU-001",
        "SKU-002",
    ]


def test_platform_api_applies_idempotent_replenishment_actions(tmp_path: Path):
    client = _client(tmp_path)
    payload = {
        "product_ids": ["SKU-001", "SKU-002", "SKU-003"],
        "quantity": 25,
        "reason": "baseline stockout risk",
        "priority": "high",
        "idempotency_key": "replenishment:test-1",
    }

    first = client.post("/actions/replenishment", json=payload).json()
    second = client.post("/actions/replenishment", json=payload).json()

    assert first["action_id"] == second["action_id"]
    assert first["created"] is True
    assert second["created"] is False
    assert first["product_count"] == 3
    assert client.get("/actions").json()["total"] == 1

    preview = client.post(
        "/inventory/preview",
        json={"product_ids": ["SKU-001"], "quantity_delta": 25},
    ).json()

    assert preview["items"][0]["before_on_hand"] == 145
    assert preview["items"][0]["after_on_hand"] == 170
