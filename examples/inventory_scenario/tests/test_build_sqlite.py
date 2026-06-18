from __future__ import annotations

import json
import sqlite3
from datetime import date, datetime, timezone
from decimal import Decimal
from pathlib import Path
from uuid import UUID

import pytest


class InMemorySource:
    def __init__(self, rows_by_table):
        self.rows_by_table = rows_by_table
        self.requests = []

    def count_rows(self, table_name: str) -> int:
        return len(self.rows_by_table[table_name])

    def iter_rows(self, table_name: str, columns, order_by):
        self.requests.append((table_name, tuple(order_by)))
        rows = list(self.rows_by_table[table_name])
        rows.sort(key=lambda row: tuple(str(row[column]) for column in order_by))
        for row in rows:
            yield {column: row[column] for column in columns}


def test_builds_sqlite_target_from_source_rows_with_constraints_and_view(
    tmp_path: Path,
):
    from inventory_scenario.support.dataset_artifacts.build_sqlite import (
        INVENTORY_TABLE_ORDER,
        build_sqlite_target,
    )

    source = InMemorySource(_source_rows())
    target_db = tmp_path / "target.db"

    result = build_sqlite_target(source, target_db)

    assert result.row_counts["dim_products"] == 2
    assert result.row_counts["fact_scenario"] == 2
    assert source.requests[0] == ("dim_companies", ("company_id",))
    assert [request[0] for request in source.requests] == list(INVENTORY_TABLE_ORDER)

    with sqlite3.connect(target_db) as conn:
        conn.row_factory = sqlite3.Row
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
        time_columns = {
            row["name"]
            for row in conn.execute("PRAGMA table_info('dim_time_periods')").fetchall()
        }
        assert "time_period_id" in time_columns
        assert "period_id" not in time_columns

        product = conn.execute(
            """
            SELECT product_id, sku, brand_tier, is_vegetarian
            FROM dim_products
            WHERE product_id = '00000000-0000-0000-0000-000000000002'
            """
        ).fetchone()
        assert product["sku"] == "HYD-TAB-6"
        assert product["brand_tier"] == "value"
        assert product["is_vegetarian"] == 1

        fact = conn.execute(
            """
            SELECT base_price, raw_product_data
            FROM fact_scenario
            WHERE fact_id = '80000000-0000-0000-0000-000000000002'
            """
        ).fetchone()
        assert fact["base_price"] == 4.99
        assert json.loads(fact["raw_product_data"]) == {
            "attributes": {"pack_size": "6ct"},
            "sku": "HYD-TAB-6",
        }

        view_row = conn.execute(
            """
            SELECT channel_code, period_label, base_price, revenue, raw_product_data
            FROM v_scenario_denormalized
            WHERE sku = 'HYD-TAB-6'
            """
        ).fetchone()
        assert view_row["channel_code"] == "online"
        assert view_row["period_label"] == "W23-2026"
        assert view_row["base_price"] == 4.99
        assert view_row["revenue"] == 199.60000000000002
        assert json.loads(view_row["raw_product_data"])["sku"] == "HYD-TAB-6"

        view_count = conn.execute("SELECT count(*) FROM v_scenario_denormalized").fetchone()[0]
        assert view_count == 2
        indexes = {
            row[1]
            for row in conn.execute("PRAGMA index_list('fact_scenario')").fetchall()
        }
        assert "idx_fact_product" in indexes


def test_artifact_bundle_writes_manifest_checksums_and_public_manifest(
    tmp_path: Path,
):
    from inventory_scenario.support.dataset_artifacts.build_sqlite import build_artifact_bundle

    source = InMemorySource(_source_rows())
    output_dir = tmp_path / "dist"
    public_manifest_path = tmp_path / "manifest.example.json"

    def fake_compressor(source_path: Path, target_path: Path) -> None:
        target_path.write_bytes(b"compressed sqlite bytes")

    bundle = build_artifact_bundle(
        source,
        output_dir=output_dir,
        dataset_version="2026-06",
        artifact_base_url="https://example.invalid/releases/inventory-scenario-v2026-06",
        public_manifest_path=public_manifest_path,
        compressor=fake_compressor,
    )

    assert bundle.target_sqlite.name == "inventory-scenario-v2026-06.target.sqlite"
    assert bundle.target_sqlite_zst.name == "inventory-scenario-v2026-06.target.sqlite.zst"
    assert bundle.manifest_path.name == "inventory-scenario-v2026-06.manifest.json"
    assert bundle.sha256sums_path.name == "inventory-scenario-v2026-06.SHA256SUMS"
    assert bundle.manifest["tables"]["dim_products"] == {"rows": 2}
    artifact = bundle.manifest["artifacts"]["target_sqlite_zst"]
    assert artifact["url"].endswith("/inventory-scenario-v2026-06.target.sqlite.zst")
    assert artifact["sha256"] == bundle.target_sqlite_zst_sha256
    assert artifact["uncompressed_bytes"] == bundle.target_sqlite.stat().st_size

    public_manifest = json.loads(public_manifest_path.read_text())
    assert public_manifest == bundle.manifest
    assert "source" not in public_manifest

    sums = bundle.sha256sums_path.read_text().splitlines()
    assert any(line.endswith("  inventory-scenario-v2026-06.target.sqlite.zst") for line in sums)
    assert any(line.endswith("  inventory-scenario-v2026-06.manifest.json") for line in sums)


def test_platform_seed_accepts_generated_neon_inventory_shape(tmp_path: Path):
    from inventory_scenario.support.dataset_artifacts.build_sqlite import build_sqlite_target
    from inventory_scenario.support.mock_platform.store import seed_platform_db

    target_db = tmp_path / "target.db"
    platform_db = tmp_path / "platform.db"
    build_sqlite_target(InMemorySource(_source_rows()), target_db)

    seed_platform_db(target_db, platform_db)

    with sqlite3.connect(platform_db) as conn:
        conn.row_factory = sqlite3.Row
        product = conn.execute(
            """
            SELECT region, channel_name, on_hand, safety_stock, reorder_point
            FROM products
            WHERE product_id = '00000000-0000-0000-0000-000000000002'
            """
        ).fetchone()

    assert product["region"] == "WH-WEST"
    assert product["channel_name"] == "Online"
    assert product["on_hand"] == 30
    assert product["safety_stock"] == 46
    assert product["reorder_point"] == 68


def test_validate_unpooled_neon_url_rejects_pooler_connection():
    from inventory_scenario.support.dataset_artifacts.dump_neon import validate_unpooled_neon_url

    with pytest.raises(ValueError, match="unpooled"):
        validate_unpooled_neon_url(
            "postgres://maintainer:secret@ep-test-pooler.us-east-2.aws.neon.tech/db"
        )

    assert (
        validate_unpooled_neon_url(
            "postgres://maintainer:secret@ep-test.us-east-2.aws.neon.tech/db"
        )
        == "postgres://maintainer:secret@ep-test.us-east-2.aws.neon.tech/db"
    )


def test_default_artifact_base_url_uses_public_object_storage():
    from inventory_scenario.support.dataset_artifacts.dump_neon import _default_artifact_base_url

    assert (
        _default_artifact_base_url("2026-06")
        == "https://flowai-public-data.hel1.your-objectstorage.com/inventory-scenario/2026-06"
    )


def test_table_column_contract_uses_inspected_neon_inventory_shape():
    from inventory_scenario.support.dataset_artifacts.build_sqlite import TABLE_COLUMNS

    assert TABLE_COLUMNS["dim_coordinates"] == (
        "coordinate_id",
        "channel_id",
        "time_period_id",
        "created_at",
    )
    assert TABLE_COLUMNS["dim_time_periods"][:4] == (
        "time_period_id",
        "start_date",
        "end_date",
        "period_label",
    )
    assert "period_id" not in TABLE_COLUMNS["dim_time_periods"]
    assert TABLE_COLUMNS["dim_inventory"][:4] == (
        "inventory_id",
        "product_id",
        "channel_id",
        "current_stock_units",
    )
    assert TABLE_COLUMNS["fact_scenario"][:4] == (
        "fact_id",
        "product_id",
        "coordinate_id",
        "base_price",
    )


def _source_rows():
    product_one = UUID("00000000-0000-0000-0000-000000000001")
    product_two = UUID("00000000-0000-0000-0000-000000000002")
    return {
        "dim_companies": [
            {
                "company_id": UUID("10000000-0000-0000-0000-000000000001"),
                "company_code": "flow-retail",
                "company_name": "Flow Retail",
                "headquarters": "USA",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_brands": [
            {
                "brand_id": UUID("20000000-0000-0000-0000-000000000001"),
                "company_id": UUID("10000000-0000-0000-0000-000000000001"),
                "brand_code": "northwind",
                "brand_name": "Northwind",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_segments": [
            {
                "segment_id": UUID("30000000-0000-0000-0000-000000000001"),
                "segment_code": "trail",
                "segment_name": "Trail",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_subsegments": [
            {
                "subsegment_id": UUID("40000000-0000-0000-0000-000000000001"),
                "segment_id": UUID("30000000-0000-0000-0000-000000000001"),
                "subsegment_code": "nutrition",
                "subsegment_name": "Nutrition",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_sales_channels": [
            {
                "channel_id": UUID("60000000-0000-0000-0000-000000000001"),
                "channel_code": "online",
                "channel_name": "Online",
                "channel_type": "digital",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_time_periods": [
            {
                "time_period_id": UUID("50000000-0000-0000-0000-000000000001"),
                "start_date": date(2026, 6, 1),
                "end_date": date(2026, 6, 7),
                "period_label": "W23-2026",
                "year": 2026,
                "quarter": "Q2",
                "month": 6,
                "week_of_year": 23,
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_coordinates": [
            {
                "coordinate_id": UUID("50000000-0000-0000-0000-000000000002"),
                "channel_id": UUID("60000000-0000-0000-0000-000000000001"),
                "time_period_id": UUID("50000000-0000-0000-0000-000000000001"),
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            }
        ],
        "dim_products": [
            {
                "product_id": product_two,
                "sku": "HYD-TAB-6",
                "company_id": UUID("10000000-0000-0000-0000-000000000001"),
                "brand_id": UUID("20000000-0000-0000-0000-000000000001"),
                "segment_id": UUID("30000000-0000-0000-0000-000000000001"),
                "subsegment_id": UUID("40000000-0000-0000-0000-000000000001"),
                "product_name": "Hydration Tablets",
                "display_name": "Northwind Hydration Tablets 6ct",
                "industry": "food",
                "primary_category": "supplements",
                "brand_tier": "value",
                "pack_config": "single",
                "container_type": "tube",
                "target_age_group": "adult",
                "flavor_category": "citrus",
                "is_organic": False,
                "is_gluten_free": True,
                "is_vegan": True,
                "is_vegetarian": True,
                "is_sugar_free": True,
                "is_lactose_free": True,
                "is_limited_edition": False,
                "size_value": Decimal("6"),
                "size_unit": "ct",
                "pack_size": 1,
                "nutriscore": "A",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
            {
                "product_id": product_one,
                "sku": "TRAIL-MIX-12",
                "company_id": UUID("10000000-0000-0000-0000-000000000001"),
                "brand_id": UUID("20000000-0000-0000-0000-000000000001"),
                "segment_id": UUID("30000000-0000-0000-0000-000000000001"),
                "subsegment_id": UUID("40000000-0000-0000-0000-000000000001"),
                "product_name": "Trail Mix",
                "display_name": "Northwind Trail Mix 12oz",
                "industry": "food",
                "primary_category": "snacks",
                "brand_tier": "standard",
                "pack_config": "single",
                "container_type": "bag",
                "target_age_group": "all_ages",
                "flavor_category": "savory",
                "is_organic": False,
                "is_gluten_free": False,
                "is_vegan": False,
                "is_vegetarian": True,
                "is_sugar_free": False,
                "is_lactose_free": True,
                "is_limited_edition": False,
                "size_value": Decimal("12"),
                "size_unit": "oz",
                "pack_size": 1,
                "nutriscore": "B",
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
        ],
        "dim_inventory": [
            {
                "inventory_id": UUID("70000000-0000-0000-0000-000000000001"),
                "product_id": product_one,
                "channel_id": UUID("60000000-0000-0000-0000-000000000001"),
                "current_stock_units": 120,
                "avg_daily_velocity": Decimal("7.5"),
                "avg_weekly_velocity": Decimal("52.5"),
                "warehouse_code": "WH-WEST",
                "last_replenishment_date": date(2026, 5, 28),
                "lead_time_days": 14,
                "snapshot_date": date(2026, 6, 1),
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
            {
                "inventory_id": UUID("70000000-0000-0000-0000-000000000002"),
                "product_id": product_two,
                "channel_id": UUID("60000000-0000-0000-0000-000000000001"),
                "current_stock_units": 30,
                "avg_daily_velocity": Decimal("3.25"),
                "avg_weekly_velocity": Decimal("22.75"),
                "warehouse_code": "WH-WEST",
                "last_replenishment_date": date(2026, 5, 29),
                "lead_time_days": 14,
                "snapshot_date": date(2026, 6, 1),
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
        ],
        "fact_scenario": [
            {
                "fact_id": UUID("80000000-0000-0000-0000-000000000001"),
                "product_id": product_one,
                "coordinate_id": UUID("50000000-0000-0000-0000-000000000002"),
                "base_price": Decimal("5.99"),
                "base_units": Decimal("125.5"),
                "availability": Decimal("0.96"),
                "market_size": Decimal("200000"),
                "cost": Decimal("2.40"),
                "margin_percent": Decimal("59.93"),
                "raw_product_data": {"sku": "TRAIL-MIX-12", "attributes": {"pack_size": "12oz"}},
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
            {
                "fact_id": UUID("80000000-0000-0000-0000-000000000002"),
                "product_id": product_two,
                "coordinate_id": UUID("50000000-0000-0000-0000-000000000002"),
                "base_price": Decimal("4.99"),
                "base_units": Decimal("40"),
                "availability": Decimal("0.92"),
                "market_size": Decimal("100000"),
                "cost": Decimal("1.25"),
                "margin_percent": Decimal("74.95"),
                "raw_product_data": {"sku": "HYD-TAB-6", "attributes": {"pack_size": "6ct"}},
                "created_at": datetime(2026, 6, 1, 12, tzinfo=timezone.utc),
            },
        ],
    }
