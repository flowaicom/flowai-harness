from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sqlite3
import subprocess
import sys
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from datetime import date, datetime
from decimal import Decimal
from pathlib import Path
from typing import Any, Protocol
from uuid import UUID


INVENTORY_TABLE_ORDER = (
    "dim_companies",
    "dim_brands",
    "dim_segments",
    "dim_subsegments",
    "dim_sales_channels",
    "dim_time_periods",
    "dim_coordinates",
    "dim_products",
    "dim_inventory",
    "fact_scenario",
)

TABLE_COLUMNS: dict[str, tuple[str, ...]] = {
    "dim_companies": (
        "company_id",
        "company_code",
        "company_name",
        "headquarters",
        "created_at",
    ),
    "dim_brands": (
        "brand_id",
        "company_id",
        "brand_code",
        "brand_name",
        "created_at",
    ),
    "dim_segments": (
        "segment_id",
        "segment_code",
        "segment_name",
        "created_at",
    ),
    "dim_subsegments": (
        "subsegment_id",
        "segment_id",
        "subsegment_code",
        "subsegment_name",
        "created_at",
    ),
    "dim_sales_channels": (
        "channel_id",
        "channel_code",
        "channel_name",
        "channel_type",
        "created_at",
    ),
    "dim_time_periods": (
        "time_period_id",
        "start_date",
        "end_date",
        "period_label",
        "year",
        "quarter",
        "month",
        "week_of_year",
        "created_at",
    ),
    "dim_coordinates": (
        "coordinate_id",
        "channel_id",
        "time_period_id",
        "created_at",
    ),
    "dim_products": (
        "product_id",
        "sku",
        "company_id",
        "brand_id",
        "segment_id",
        "subsegment_id",
        "product_name",
        "display_name",
        "industry",
        "primary_category",
        "brand_tier",
        "pack_config",
        "container_type",
        "target_age_group",
        "flavor_category",
        "is_organic",
        "is_gluten_free",
        "is_vegan",
        "is_vegetarian",
        "is_sugar_free",
        "is_lactose_free",
        "is_limited_edition",
        "size_value",
        "size_unit",
        "pack_size",
        "nutriscore",
        "created_at",
    ),
    "dim_inventory": (
        "inventory_id",
        "product_id",
        "channel_id",
        "current_stock_units",
        "avg_daily_velocity",
        "avg_weekly_velocity",
        "warehouse_code",
        "last_replenishment_date",
        "lead_time_days",
        "snapshot_date",
        "created_at",
    ),
    "fact_scenario": (
        "fact_id",
        "product_id",
        "coordinate_id",
        "base_price",
        "base_units",
        "availability",
        "market_size",
        "cost",
        "margin_percent",
        "raw_product_data",
        "created_at",
    ),
}

ORDER_BY: dict[str, tuple[str, ...]] = {
    table: (TABLE_COLUMNS[table][0],) for table in INVENTORY_TABLE_ORDER
}

INTEGER_COLUMNS = {
    ("dim_time_periods", "year"),
    ("dim_time_periods", "month"),
    ("dim_time_periods", "week_of_year"),
    ("dim_products", "is_organic"),
    ("dim_products", "is_gluten_free"),
    ("dim_products", "is_vegan"),
    ("dim_products", "is_vegetarian"),
    ("dim_products", "is_sugar_free"),
    ("dim_products", "is_lactose_free"),
    ("dim_products", "is_limited_edition"),
    ("dim_products", "pack_size"),
    ("dim_inventory", "current_stock_units"),
    ("dim_inventory", "lead_time_days"),
}
REAL_COLUMNS = {
    ("dim_products", "size_value"),
    ("dim_inventory", "avg_daily_velocity"),
    ("dim_inventory", "avg_weekly_velocity"),
    ("fact_scenario", "base_price"),
    ("fact_scenario", "base_units"),
    ("fact_scenario", "availability"),
    ("fact_scenario", "market_size"),
    ("fact_scenario", "cost"),
    ("fact_scenario", "margin_percent"),
}
JSON_COLUMNS = {("fact_scenario", "raw_product_data")}

SCHEMA_SQL = """
PRAGMA foreign_keys = ON;

CREATE TABLE dim_companies (
    company_id TEXT PRIMARY KEY,
    company_code TEXT NOT NULL,
    company_name TEXT NOT NULL,
    headquarters TEXT,
    created_at TEXT
);

CREATE TABLE dim_brands (
    brand_id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES dim_companies(company_id),
    brand_code TEXT NOT NULL,
    brand_name TEXT NOT NULL,
    created_at TEXT
);

CREATE TABLE dim_segments (
    segment_id TEXT PRIMARY KEY,
    segment_code TEXT NOT NULL,
    segment_name TEXT NOT NULL,
    created_at TEXT
);

CREATE TABLE dim_subsegments (
    subsegment_id TEXT PRIMARY KEY,
    segment_id TEXT NOT NULL REFERENCES dim_segments(segment_id),
    subsegment_code TEXT NOT NULL,
    subsegment_name TEXT NOT NULL,
    created_at TEXT
);

CREATE TABLE dim_sales_channels (
    channel_id TEXT PRIMARY KEY,
    channel_code TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    created_at TEXT
);

CREATE TABLE dim_time_periods (
    time_period_id TEXT PRIMARY KEY,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    period_label TEXT NOT NULL,
    year INTEGER NOT NULL,
    quarter TEXT NOT NULL,
    month INTEGER NOT NULL,
    week_of_year INTEGER NOT NULL,
    created_at TEXT
);

CREATE TABLE dim_coordinates (
    coordinate_id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES dim_sales_channels(channel_id),
    time_period_id TEXT NOT NULL REFERENCES dim_time_periods(time_period_id),
    created_at TEXT
);

CREATE TABLE dim_products (
    product_id TEXT PRIMARY KEY,
    sku TEXT NOT NULL,
    company_id TEXT NOT NULL REFERENCES dim_companies(company_id),
    brand_id TEXT REFERENCES dim_brands(brand_id),
    segment_id TEXT NOT NULL REFERENCES dim_segments(segment_id),
    subsegment_id TEXT NOT NULL REFERENCES dim_subsegments(subsegment_id),
    product_name TEXT NOT NULL,
    display_name TEXT,
    industry TEXT NOT NULL,
    primary_category TEXT,
    brand_tier TEXT,
    pack_config TEXT,
    container_type TEXT,
    target_age_group TEXT,
    flavor_category TEXT,
    is_organic INTEGER,
    is_gluten_free INTEGER,
    is_vegan INTEGER,
    is_vegetarian INTEGER,
    is_sugar_free INTEGER,
    is_lactose_free INTEGER,
    is_limited_edition INTEGER,
    size_value REAL,
    size_unit TEXT,
    pack_size INTEGER,
    nutriscore TEXT,
    created_at TEXT
);

CREATE TABLE dim_inventory (
    inventory_id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES dim_products(product_id),
    channel_id TEXT NOT NULL REFERENCES dim_sales_channels(channel_id),
    current_stock_units INTEGER NOT NULL,
    avg_daily_velocity REAL NOT NULL,
    avg_weekly_velocity REAL NOT NULL,
    warehouse_code TEXT NOT NULL,
    last_replenishment_date TEXT,
    lead_time_days INTEGER,
    snapshot_date TEXT NOT NULL,
    created_at TEXT
);

CREATE TABLE fact_scenario (
    fact_id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES dim_products(product_id),
    coordinate_id TEXT NOT NULL REFERENCES dim_coordinates(coordinate_id),
    base_price REAL NOT NULL,
    base_units REAL,
    availability REAL,
    market_size REAL,
    cost REAL,
    margin_percent REAL,
    raw_product_data TEXT,
    created_at TEXT
);

CREATE UNIQUE INDEX dim_companies_company_code_key
    ON dim_companies (company_code);
CREATE UNIQUE INDEX dim_brands_company_id_brand_code_key
    ON dim_brands (company_id, brand_code);
CREATE INDEX idx_brands_company
    ON dim_brands (company_id);
CREATE UNIQUE INDEX dim_segments_segment_code_key
    ON dim_segments (segment_code);
CREATE UNIQUE INDEX dim_subsegments_segment_id_subsegment_code_key
    ON dim_subsegments (segment_id, subsegment_code);
CREATE INDEX idx_subsegments_segment
    ON dim_subsegments (segment_id);
CREATE UNIQUE INDEX dim_sales_channels_channel_code_key
    ON dim_sales_channels (channel_code);
CREATE UNIQUE INDEX dim_time_periods_start_date_end_date_key
    ON dim_time_periods (start_date, end_date);
CREATE UNIQUE INDEX dim_coordinates_channel_id_time_period_id_key
    ON dim_coordinates (channel_id, time_period_id);
CREATE INDEX idx_coordinates_channel
    ON dim_coordinates (channel_id);
CREATE INDEX idx_coordinates_time
    ON dim_coordinates (time_period_id);
CREATE UNIQUE INDEX dim_products_sku_key
    ON dim_products (sku);
CREATE INDEX idx_products_brand
    ON dim_products (brand_id);
CREATE INDEX idx_products_brand_tier
    ON dim_products (brand_tier);
CREATE INDEX idx_products_company
    ON dim_products (company_id);
CREATE INDEX idx_products_segment
    ON dim_products (segment_id);
CREATE INDEX idx_products_subsegment
    ON dim_products (subsegment_id);
CREATE UNIQUE INDEX dim_inventory_product_id_channel_id_key
    ON dim_inventory (product_id, channel_id);
CREATE INDEX idx_inventory_channel
    ON dim_inventory (channel_id);
CREATE INDEX idx_inventory_product
    ON dim_inventory (product_id);
CREATE INDEX idx_inventory_warehouse
    ON dim_inventory (warehouse_code);
CREATE UNIQUE INDEX fact_scenario_product_id_coordinate_id_key
    ON fact_scenario (product_id, coordinate_id);
CREATE INDEX idx_fact_availability
    ON fact_scenario (availability);
CREATE INDEX idx_fact_coordinate
    ON fact_scenario (coordinate_id);
CREATE INDEX idx_fact_price
    ON fact_scenario (base_price);
CREATE INDEX idx_fact_product
    ON fact_scenario (product_id);

CREATE VIEW v_scenario_denormalized AS
SELECT
    p.product_id,
    p.sku,
    p.product_name,
    p.display_name,
    c.company_code,
    c.company_name,
    c.headquarters,
    b.brand_code,
    b.brand_name,
    p.industry,
    p.primary_category,
    seg.segment_code,
    seg.segment_name,
    sub.subsegment_code,
    sub.subsegment_name,
    p.brand_tier,
    p.pack_config,
    p.container_type,
    p.target_age_group,
    p.flavor_category,
    p.is_organic,
    p.is_gluten_free,
    p.is_vegan,
    p.is_vegetarian,
    p.is_sugar_free,
    p.is_lactose_free,
    p.is_limited_edition,
    p.size_value,
    p.size_unit,
    p.pack_size,
    p.nutriscore,
    ch.channel_code,
    ch.channel_name,
    ch.channel_type,
    tp.period_label,
    tp.start_date,
    tp.end_date,
    tp.year,
    tp.quarter,
    tp.month,
    tp.week_of_year,
    f.base_price,
    f.base_units,
    f.availability,
    f.market_size,
    f.cost,
    f.margin_percent,
    f.base_price * f.base_units AS revenue,
    f.base_units * f.cost AS total_cost,
    (f.base_price - f.cost) * f.base_units AS gross_profit,
    f.raw_product_data
FROM fact_scenario f
JOIN dim_products p ON f.product_id = p.product_id
JOIN dim_coordinates coord ON f.coordinate_id = coord.coordinate_id
JOIN dim_companies c ON p.company_id = c.company_id
JOIN dim_segments seg ON p.segment_id = seg.segment_id
JOIN dim_subsegments sub ON p.subsegment_id = sub.subsegment_id
JOIN dim_sales_channels ch ON coord.channel_id = ch.channel_id
JOIN dim_time_periods tp ON coord.time_period_id = tp.time_period_id
LEFT JOIN dim_brands b ON p.brand_id = b.brand_id;
"""


class InventorySource(Protocol):
    def count_rows(self, table_name: str) -> int: ...

    def iter_rows(
        self,
        table_name: str,
        columns: Sequence[str],
        order_by: Sequence[str],
    ) -> Iterable[dict[str, Any]]: ...


@dataclass(frozen=True)
class BuildResult:
    target_db: Path
    row_counts: dict[str, int]


@dataclass(frozen=True)
class ArtifactBundleResult:
    target_sqlite: Path
    target_sqlite_zst: Path
    manifest_path: Path
    sha256sums_path: Path
    manifest: dict[str, Any]
    target_sqlite_zst_sha256: str


Compressor = Callable[[Path, Path], None]


def build_sqlite_target(source: InventorySource, target_db: Path) -> BuildResult:
    target_db.parent.mkdir(parents=True, exist_ok=True)
    if target_db.exists():
        target_db.unlink()

    row_counts: dict[str, int] = {}
    with sqlite3.connect(target_db) as conn:
        conn.execute("PRAGMA foreign_keys = ON")
        conn.executescript(SCHEMA_SQL)
        for table in INVENTORY_TABLE_ORDER:
            expected = source.count_rows(table)
            inserted = _copy_table(conn, source, table)
            if inserted != expected:
                raise ValueError(
                    f"{table} row count mismatch while copying: expected {expected}, inserted {inserted}"
                )
            row_counts[table] = inserted
        _validate_sqlite(conn, row_counts)

    return BuildResult(target_db=target_db, row_counts=row_counts)


def build_artifact_bundle(
    source: InventorySource,
    *,
    output_dir: Path,
    dataset_version: str,
    artifact_base_url: str,
    public_manifest_path: Path | None = None,
    compressor: Compressor | None = None,
) -> ArtifactBundleResult:
    output_dir.mkdir(parents=True, exist_ok=True)
    prefix = f"inventory-scenario-v{dataset_version}"
    target_sqlite = output_dir / f"{prefix}.target.sqlite"
    target_sqlite_zst = output_dir / f"{prefix}.target.sqlite.zst"
    manifest_path = output_dir / f"{prefix}.manifest.json"
    sha256sums_path = output_dir / f"{prefix}.SHA256SUMS"

    build = build_sqlite_target(source, target_sqlite)
    (compressor or compress_zstd)(target_sqlite, target_sqlite_zst)
    target_sha = hash_file(target_sqlite_zst)
    artifact_url = f"{artifact_base_url.rstrip('/')}/{target_sqlite_zst.name}"
    manifest = {
        "dataset_version": dataset_version,
        "artifacts": {
            "target_sqlite_zst": {
                "url": artifact_url,
                "sha256": target_sha,
                "uncompressed_bytes": target_sqlite.stat().st_size,
            }
        },
        "tables": {
            table: {"rows": rows}
            for table, rows in build.row_counts.items()
        },
    }
    _write_json(manifest_path, manifest)
    manifest_sha = hash_file(manifest_path)
    sha256sums_path.write_text(
        "\n".join(
            [
                f"{target_sha}  {target_sqlite_zst.name}",
                f"{manifest_sha}  {manifest_path.name}",
            ]
        )
        + "\n"
    )

    if public_manifest_path is not None:
        _write_json(public_manifest_path, manifest)

    return ArtifactBundleResult(
        target_sqlite=target_sqlite,
        target_sqlite_zst=target_sqlite_zst,
        manifest_path=manifest_path,
        sha256sums_path=sha256sums_path,
        manifest=manifest,
        target_sqlite_zst_sha256=target_sha,
    )


def compress_zstd(source_path: Path, target_path: Path) -> None:
    target_path.parent.mkdir(parents=True, exist_ok=True)
    if target_path.exists():
        target_path.unlink()
    try:
        import zstandard as zstd  # type: ignore[import-not-found]
    except ImportError:
        zstd = None

    if zstd is not None:
        with source_path.open("rb") as source, target_path.open("wb") as target:
            zstd.ZstdCompressor(level=19).copy_stream(source, target)
        return

    zstd_bin = shutil.which("zstd")
    if zstd_bin is None:
        raise RuntimeError(
            "building the artifact requires either the `zstandard` Python package or the `zstd` CLI"
        )
    subprocess.run(
        [zstd_bin, "-19", "-f", str(source_path), "-o", str(target_path)],
        check=True,
    )


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Build inventory scenario SQLite artifacts from an already configured source module.",
    )
    parser.add_argument("--help-only", action="store_true")
    args = parser.parse_args(argv)
    if args.help_only:
        parser.print_help()
        return 0
    parser.error("use `uv run inventory-scenario dump-neon` for the maintainer Neon flow")
    return 2


def _copy_table(
    conn: sqlite3.Connection,
    source: InventorySource,
    table: str,
    *,
    batch_size: int = 1000,
) -> int:
    columns = TABLE_COLUMNS[table]
    placeholders = ", ".join("?" for _ in columns)
    sql = f"INSERT INTO {table} ({', '.join(columns)}) VALUES ({placeholders})"
    batch = []
    inserted = 0
    for row in source.iter_rows(table, columns, ORDER_BY[table]):
        batch.append(tuple(_convert_value(table, column, row[column]) for column in columns))
        if len(batch) >= batch_size:
            conn.executemany(sql, batch)
            inserted += len(batch)
            batch.clear()
    if batch:
        conn.executemany(sql, batch)
        inserted += len(batch)
    return inserted


def _convert_value(table: str, column: str, value: Any) -> Any:
    if value is None:
        return None
    column_key = (table, column)
    if column_key in JSON_COLUMNS:
        if isinstance(value, str):
            return value
        return json.dumps(value, sort_keys=True, separators=(",", ":"))
    if column_key in INTEGER_COLUMNS:
        return int(value)
    if column_key in REAL_COLUMNS:
        return float(value)
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, UUID):
        return str(value)
    if isinstance(value, datetime):
        return value.isoformat()
    if isinstance(value, date):
        return value.isoformat()
    if isinstance(value, Decimal):
        return float(value)
    if isinstance(value, (dict, list)):
        return json.dumps(value, sort_keys=True, separators=(",", ":"))
    return value


def _validate_sqlite(
    conn: sqlite3.Connection,
    row_counts: dict[str, int],
) -> None:
    integrity = conn.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise ValueError(f"SQLite integrity_check failed: {integrity}")
    foreign_key_errors = conn.execute("PRAGMA foreign_key_check").fetchall()
    if foreign_key_errors:
        raise ValueError(f"SQLite foreign_key_check failed: {foreign_key_errors}")
    for table, expected in row_counts.items():
        actual = conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
        if actual != expected:
            raise ValueError(
                f"{table} row count mismatch after validation: expected {expected}, got {actual}"
            )
    conn.execute("SELECT count(*) FROM v_scenario_denormalized").fetchone()


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
