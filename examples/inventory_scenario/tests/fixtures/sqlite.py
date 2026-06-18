from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Any


TABLES = (
    "dim_companies",
    "dim_brands",
    "dim_segments",
    "dim_subsegments",
    "dim_coordinates",
    "dim_sales_channels",
    "dim_time_periods",
    "dim_products",
    "dim_inventory",
    "fact_scenario",
)


SCHEMA_SQL = """
PRAGMA foreign_keys = ON;

CREATE TABLE dim_companies (
    company_id TEXT PRIMARY KEY,
    company_name TEXT NOT NULL
);

CREATE TABLE dim_brands (
    brand_id TEXT PRIMARY KEY,
    brand_name TEXT NOT NULL,
    company_id TEXT NOT NULL REFERENCES dim_companies(company_id)
);

CREATE TABLE dim_segments (
    segment_id TEXT PRIMARY KEY,
    segment_name TEXT NOT NULL
);

CREATE TABLE dim_subsegments (
    subsegment_id TEXT PRIMARY KEY,
    segment_id TEXT NOT NULL REFERENCES dim_segments(segment_id),
    subsegment_name TEXT NOT NULL
);

CREATE TABLE dim_coordinates (
    coordinate_id TEXT PRIMARY KEY,
    region TEXT NOT NULL,
    country TEXT NOT NULL,
    market TEXT NOT NULL
);

CREATE TABLE dim_sales_channels (
    channel_id TEXT PRIMARY KEY,
    channel_name TEXT NOT NULL
);

CREATE TABLE dim_time_periods (
    period_id TEXT PRIMARY KEY,
    period_label TEXT NOT NULL,
    starts_on TEXT NOT NULL,
    ends_on TEXT NOT NULL
);

CREATE TABLE dim_products (
    product_id TEXT PRIMARY KEY,
    product_name TEXT NOT NULL,
    brand_id TEXT NOT NULL REFERENCES dim_brands(brand_id),
    segment_id TEXT NOT NULL REFERENCES dim_segments(segment_id),
    subsegment_id TEXT NOT NULL REFERENCES dim_subsegments(subsegment_id),
    unit_cost REAL NOT NULL,
    list_price REAL NOT NULL,
    attributes_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE dim_inventory (
    inventory_id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES dim_products(product_id),
    coordinate_id TEXT NOT NULL REFERENCES dim_coordinates(coordinate_id),
    channel_id TEXT NOT NULL REFERENCES dim_sales_channels(channel_id),
    on_hand INTEGER NOT NULL,
    safety_stock INTEGER NOT NULL,
    reorder_point INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE fact_scenario (
    scenario_id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES dim_products(product_id),
    coordinate_id TEXT NOT NULL REFERENCES dim_coordinates(coordinate_id),
    channel_id TEXT NOT NULL REFERENCES dim_sales_channels(channel_id),
    period_id TEXT NOT NULL REFERENCES dim_time_periods(period_id),
    scenario_name TEXT NOT NULL,
    projected_units REAL NOT NULL,
    projected_revenue REAL NOT NULL,
    gross_margin_pct REAL NOT NULL
);

CREATE VIEW v_scenario_denormalized AS
SELECT
    fs.scenario_id,
    fs.scenario_name,
    fs.projected_units,
    fs.projected_revenue,
    fs.gross_margin_pct,
    p.product_id,
    p.product_name,
    p.list_price,
    b.brand_name,
    c.company_name,
    s.segment_name,
    ss.subsegment_name,
    co.region,
    co.country,
    co.market,
    ch.channel_name,
    tp.period_label,
    inv.on_hand,
    inv.safety_stock,
    inv.reorder_point
FROM fact_scenario fs
JOIN dim_products p ON p.product_id = fs.product_id
JOIN dim_brands b ON b.brand_id = p.brand_id
JOIN dim_companies c ON c.company_id = b.company_id
JOIN dim_segments s ON s.segment_id = p.segment_id
JOIN dim_subsegments ss ON ss.subsegment_id = p.subsegment_id
JOIN dim_coordinates co ON co.coordinate_id = fs.coordinate_id
JOIN dim_sales_channels ch ON ch.channel_id = fs.channel_id
JOIN dim_time_periods tp ON tp.period_id = fs.period_id
LEFT JOIN dim_inventory inv
    ON inv.product_id = fs.product_id
    AND inv.coordinate_id = fs.coordinate_id
    AND inv.channel_id = fs.channel_id;
"""


def create_tiny_target_db(path: Path) -> dict[str, Any]:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        path.unlink()
    with sqlite3.connect(path) as conn:
        conn.executescript(SCHEMA_SQL)
        _insert(conn, "dim_companies", [{"company_id": "COMP-001", "company_name": "Acme Outdoors"}])
        _insert(
            conn,
            "dim_brands",
            [{"brand_id": "BRAND-001", "brand_name": "Northwind", "company_id": "COMP-001"}],
        )
        _insert(conn, "dim_segments", [{"segment_id": "SEG-TRAIL", "segment_name": "Trail"}])
        _insert(
            conn,
            "dim_subsegments",
            [{"subsegment_id": "SUB-SNACKS", "segment_id": "SEG-TRAIL", "subsegment_name": "Snacks"}],
        )
        _insert(
            conn,
            "dim_coordinates",
            [{"coordinate_id": "LOC-WEST", "region": "West", "country": "US", "market": "Portland"}],
        )
        _insert(conn, "dim_sales_channels", [{"channel_id": "CH-ONLINE", "channel_name": "Online"}])
        _insert(
            conn,
            "dim_time_periods",
            [{"period_id": "2026-06", "period_label": "June 2026", "starts_on": "2026-06-01", "ends_on": "2026-06-30"}],
        )
        _insert(
            conn,
            "dim_products",
            [
                {
                    "product_id": "SKU-001",
                    "product_name": "Trail Mix",
                    "brand_id": "BRAND-001",
                    "segment_id": "SEG-TRAIL",
                    "subsegment_id": "SUB-SNACKS",
                    "unit_cost": 2.4,
                    "list_price": 5.99,
                    "attributes_json": {"pack_size": "12oz"},
                },
                {
                    "product_id": "SKU-002",
                    "product_name": "Hydration Tablets",
                    "brand_id": "BRAND-001",
                    "segment_id": "SEG-TRAIL",
                    "subsegment_id": "SUB-SNACKS",
                    "unit_cost": 1.1,
                    "list_price": 3.99,
                    "attributes_json": {"pack_size": "10ct"},
                },
                {
                    "product_id": "SKU-003",
                    "product_name": "Camp Lantern",
                    "brand_id": "BRAND-001",
                    "segment_id": "SEG-TRAIL",
                    "subsegment_id": "SUB-SNACKS",
                    "unit_cost": 12.0,
                    "list_price": 29.99,
                    "attributes_json": {"battery": "AA"},
                },
            ],
        )
        _insert(
            conn,
            "dim_inventory",
            [
                {
                    "inventory_id": "INV-001",
                    "product_id": "SKU-001",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "on_hand": 120,
                    "safety_stock": 80,
                    "reorder_point": 100,
                    "updated_at": "2026-06-01T00:00:00Z",
                },
                {
                    "inventory_id": "INV-002",
                    "product_id": "SKU-002",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "on_hand": 240,
                    "safety_stock": 140,
                    "reorder_point": 180,
                    "updated_at": "2026-06-01T00:00:00Z",
                },
                {
                    "inventory_id": "INV-003",
                    "product_id": "SKU-003",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "on_hand": 18,
                    "safety_stock": 25,
                    "reorder_point": 30,
                    "updated_at": "2026-06-01T00:00:00Z",
                },
            ],
        )
        _insert(
            conn,
            "fact_scenario",
            [
                {
                    "scenario_id": "SCN-001",
                    "product_id": "SKU-001",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "period_id": "2026-06",
                    "scenario_name": "baseline",
                    "projected_units": 125.0,
                    "projected_revenue": 748.75,
                    "gross_margin_pct": 0.5993,
                },
                {
                    "scenario_id": "SCN-002",
                    "product_id": "SKU-002",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "period_id": "2026-06",
                    "scenario_name": "baseline",
                    "projected_units": 260.0,
                    "projected_revenue": 1037.4,
                    "gross_margin_pct": 0.7243,
                },
                {
                    "scenario_id": "SCN-003",
                    "product_id": "SKU-003",
                    "coordinate_id": "LOC-WEST",
                    "channel_id": "CH-ONLINE",
                    "period_id": "2026-06",
                    "scenario_name": "promotion_holdback",
                    "projected_units": 42.0,
                    "projected_revenue": 1259.58,
                    "gross_margin_pct": 0.5999,
                },
            ],
        )

    return {
        "dataset_version": "test-2026-06",
        "tables": _counts(path),
    }


def _insert(conn: sqlite3.Connection, table: str, rows: list[dict[str, Any]]) -> None:
    columns = list(rows[0].keys())
    conn.executemany(
        f"INSERT INTO {table} ({', '.join(columns)}) VALUES ({', '.join('?' for _ in columns)})",
        [
            tuple(json.dumps(row[column], sort_keys=True) if isinstance(row[column], dict) else row[column] for column in columns)
            for row in rows
        ],
    )


def _counts(path: Path) -> dict[str, dict[str, int]]:
    with sqlite3.connect(path) as conn:
        return {
            table: {"rows": int(conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0])}
            for table in TABLES
        }
