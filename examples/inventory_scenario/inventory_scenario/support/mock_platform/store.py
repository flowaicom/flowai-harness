from __future__ import annotations

import json
import sqlite3
import uuid
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from inventory_scenario.support.mock_platform.schemas import (
    InventoryPreviewRequest,
    PromotionHoldbackRequest,
    ReplenishmentRequest,
    SafetyStockRequest,
)


PLATFORM_SCHEMA_SQL = """
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS products (
    product_id TEXT PRIMARY KEY,
    product_name TEXT NOT NULL,
    brand_name TEXT NOT NULL,
    segment_name TEXT NOT NULL,
    region TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    on_hand INTEGER NOT NULL,
    safety_stock INTEGER NOT NULL,
    reorder_point INTEGER NOT NULL,
    holdback_units INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS actions (
    action_id TEXT PRIMARY KEY,
    action_type TEXT NOT NULL,
    idempotency_key TEXT UNIQUE,
    priority TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL,
    product_count INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS action_products (
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    product_id TEXT NOT NULL,
    before_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    PRIMARY KEY (action_id, product_id)
);
"""


def seed_platform_db(target_db: Path, platform_db: Path) -> None:
    platform_db.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(platform_db) as conn:
        conn.executescript(PLATFORM_SCHEMA_SQL)
        target = sqlite3.connect(target_db)
        target.row_factory = sqlite3.Row
        try:
            rows = _platform_seed_rows(target)
        finally:
            target.close()

        now = _now()
        conn.executemany(
            """
            INSERT INTO products (
                product_id,
                product_name,
                brand_name,
                segment_name,
                region,
                channel_name,
                on_hand,
                safety_stock,
                reorder_point,
                holdback_units,
                updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
            ON CONFLICT(product_id) DO UPDATE SET
                product_name = excluded.product_name,
                brand_name = excluded.brand_name,
                segment_name = excluded.segment_name,
                region = excluded.region,
                channel_name = excluded.channel_name,
                on_hand = excluded.on_hand,
                safety_stock = excluded.safety_stock,
                reorder_point = excluded.reorder_point,
                updated_at = excluded.updated_at
            """,
            [
                (
                    row["product_id"],
                    row["product_name"],
                    row["brand_name"],
                    row["segment_name"],
                    row["region"],
                    row["channel_name"],
                    row["on_hand"],
                    row["safety_stock"],
                    row["reorder_point"],
                    now,
                )
                for row in rows
            ],
        )


def _platform_seed_rows(target: sqlite3.Connection) -> list[sqlite3.Row]:
    inventory_columns = _table_columns(target, "dim_inventory")
    if "current_stock_units" in inventory_columns:
        return target.execute(
            """
            WITH ranked_inventory AS (
                SELECT
                    p.product_id,
                    p.product_name,
                    COALESCE(b.brand_name, 'Unknown') AS brand_name,
                    seg.segment_name,
                    inv.warehouse_code AS region,
                    ch.channel_name,
                    inv.current_stock_units AS on_hand,
                    CAST(ROUND(inv.avg_daily_velocity * COALESCE(inv.lead_time_days, 14)) AS INTEGER)
                        AS safety_stock,
                    CAST(ROUND(inv.avg_daily_velocity * (COALESCE(inv.lead_time_days, 14) + 7)) AS INTEGER)
                        AS reorder_point,
                    ROW_NUMBER() OVER (
                        PARTITION BY p.product_id
                        ORDER BY inv.current_stock_units ASC, ch.channel_code ASC
                    ) AS product_rank
                FROM dim_inventory inv
                JOIN dim_products p ON p.product_id = inv.product_id
                LEFT JOIN dim_brands b ON b.brand_id = p.brand_id
                JOIN dim_segments seg ON seg.segment_id = p.segment_id
                JOIN dim_sales_channels ch ON ch.channel_id = inv.channel_id
            )
            SELECT
                product_id,
                product_name,
                brand_name,
                segment_name,
                region,
                channel_name,
                on_hand,
                safety_stock,
                reorder_point
            FROM ranked_inventory
            WHERE product_rank = 1
            ORDER BY product_id
            """
        ).fetchall()

    return target.execute(
        """
        SELECT
            product_id,
            product_name,
            brand_name,
            segment_name,
            region,
            channel_name,
            on_hand,
            safety_stock,
            reorder_point
        FROM v_scenario_denormalized
        GROUP BY product_id
        ORDER BY product_id
        """
    ).fetchall()


def _table_columns(conn: sqlite3.Connection, table_name: str) -> set[str]:
    return {
        str(row["name"])
        for row in conn.execute(f"PRAGMA table_info('{table_name}')").fetchall()
    }


class PlatformStore:
    def __init__(self, db_path: Path):
        self.db_path = db_path
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        with self._connect() as conn:
            conn.executescript(PLATFORM_SCHEMA_SQL)

    def summary(self) -> dict[str, Any]:
        with self._connect() as conn:
            products = conn.execute("SELECT count(*) FROM products").fetchone()[0]
            actions = conn.execute("SELECT count(*) FROM actions").fetchone()[0]
            low_stock = conn.execute(
                "SELECT count(*) FROM products WHERE on_hand < reorder_point"
            ).fetchone()[0]
        return {
            "products": int(products),
            "actions": int(actions),
            "low_stock_products": int(low_stock),
        }

    def list_products(self, *, limit: int = 50, offset: int = 0) -> dict[str, Any]:
        with self._connect() as conn:
            rows = conn.execute(
                """
                SELECT * FROM products
                ORDER BY product_id
                LIMIT ? OFFSET ?
                """,
                (limit, offset),
            ).fetchall()
            total = conn.execute("SELECT count(*) FROM products").fetchone()[0]
        return {
            "products": [_product_row(row) for row in rows],
            "total": int(total),
        }

    def resolve_products(self, product_ids: list[str]) -> dict[str, Any]:
        rows = self._products_by_id(product_ids)
        found_ids = {row["product_id"] for row in rows}
        return {
            "products": [_product_row(row) for row in rows],
            "missing": [product_id for product_id in product_ids if product_id not in found_ids],
        }

    def preview(self, request: InventoryPreviewRequest) -> dict[str, Any]:
        rows = self._products_by_id(request.product_ids)
        items = []
        for row in rows:
            after_on_hand = row["on_hand"] + request.quantity_delta
            after_safety_stock = (
                request.safety_stock
                if request.safety_stock is not None
                else row["safety_stock"]
            )
            items.append(
                {
                    "product_id": row["product_id"],
                    "before_on_hand": row["on_hand"],
                    "after_on_hand": after_on_hand,
                    "before_safety_stock": row["safety_stock"],
                    "after_safety_stock": after_safety_stock,
                    "before_holdback_units": row["holdback_units"],
                    "after_holdback_units": row["holdback_units"]
                    + request.holdback_delta,
                }
            )
        return {"items": items, "missing": _missing_ids(request.product_ids, rows)}

    def apply_replenishment(self, request: ReplenishmentRequest) -> dict[str, Any]:
        return self._apply_action(
            action_type="replenishment",
            request=request.model_dump(mode="json"),
            product_ids=request.product_ids,
            priority=request.priority,
            reason=request.reason,
            idempotency_key=request.idempotency_key,
            mutator=lambda row: {
                **row,
                "on_hand": row["on_hand"] + request.quantity,
            },
        )

    def apply_safety_stock(self, request: SafetyStockRequest) -> dict[str, Any]:
        return self._apply_action(
            action_type="safety_stock",
            request=request.model_dump(mode="json"),
            product_ids=request.product_ids,
            priority=request.priority,
            reason=request.reason,
            idempotency_key=request.idempotency_key,
            mutator=lambda row: {
                **row,
                "safety_stock": request.safety_stock,
            },
        )

    def apply_holdback(self, request: PromotionHoldbackRequest) -> dict[str, Any]:
        return self._apply_action(
            action_type="promotion_holdback",
            request=request.model_dump(mode="json"),
            product_ids=request.product_ids,
            priority=request.priority,
            reason=request.reason,
            idempotency_key=request.idempotency_key,
            mutator=lambda row: {
                **row,
                "holdback_units": request.holdback_units,
            },
        )

    def list_actions(self) -> dict[str, Any]:
        with self._connect() as conn:
            rows = conn.execute(
                "SELECT * FROM actions ORDER BY created_at DESC, action_id DESC"
            ).fetchall()
        return {
            "actions": [_action_row(row) for row in rows],
            "total": len(rows),
        }

    def get_action(self, action_id: str) -> dict[str, Any] | None:
        with self._connect() as conn:
            row = conn.execute(
                "SELECT * FROM actions WHERE action_id = ?",
                (action_id,),
            ).fetchone()
            if row is None:
                return None
            products = conn.execute(
                """
                SELECT product_id, before_json, after_json
                FROM action_products
                WHERE action_id = ?
                ORDER BY product_id
                """,
                (action_id,),
            ).fetchall()
        action = _action_row(row)
        action["products"] = [
            {
                "product_id": product["product_id"],
                "before": json.loads(product["before_json"]),
                "after": json.loads(product["after_json"]),
            }
            for product in products
        ]
        return action

    def _apply_action(
        self,
        *,
        action_type: str,
        request: dict[str, Any],
        product_ids: list[str],
        priority: str,
        reason: str,
        idempotency_key: str | None,
        mutator,
    ) -> dict[str, Any]:
        with self._connect() as conn:
            if idempotency_key is not None:
                existing = conn.execute(
                    "SELECT * FROM actions WHERE idempotency_key = ?",
                    (idempotency_key,),
                ).fetchone()
                if existing is not None:
                    response = _action_response(existing)
                    response["created"] = False
                    return response

            rows = self._products_by_id(product_ids, conn=conn)
            missing = _missing_ids(product_ids, rows)
            if missing:
                raise ValueError(f"unknown product ids: {', '.join(missing)}")

            action_id = f"act_{uuid.uuid4().hex}"
            created_at = _now()
            before_after = []
            for row in rows:
                before = _product_row(row)
                after = mutator(before)
                before_after.append((before, after))
                conn.execute(
                    """
                    UPDATE products
                    SET on_hand = ?,
                        safety_stock = ?,
                        holdback_units = ?,
                        updated_at = ?
                    WHERE product_id = ?
                    """,
                    (
                        after["on_hand"],
                        after["safety_stock"],
                        after["holdback_units"],
                        created_at,
                        row["product_id"],
                    ),
                )

            conn.execute(
                """
                INSERT INTO actions (
                    action_id,
                    action_type,
                    idempotency_key,
                    priority,
                    reason,
                    status,
                    product_count,
                    payload_json,
                    created_at
                )
                VALUES (?, ?, ?, ?, ?, 'applied', ?, ?, ?)
                """,
                (
                    action_id,
                    action_type,
                    idempotency_key,
                    priority,
                    reason,
                    len(rows),
                    json.dumps(request, sort_keys=True),
                    created_at,
                ),
            )
            conn.executemany(
                """
                INSERT INTO action_products (
                    action_id,
                    product_id,
                    before_json,
                    after_json
                )
                VALUES (?, ?, ?, ?)
                """,
                [
                    (
                        action_id,
                        before["product_id"],
                        json.dumps(before, sort_keys=True),
                        json.dumps(after, sort_keys=True),
                    )
                    for before, after in before_after
                ],
            )
            row = conn.execute(
                "SELECT * FROM actions WHERE action_id = ?",
                (action_id,),
            ).fetchone()
        response = _action_response(row)
        response["created"] = True
        return response

    def _products_by_id(
        self,
        product_ids: list[str],
        *,
        conn: sqlite3.Connection | None = None,
    ) -> list[sqlite3.Row]:
        owns_conn = conn is None
        active_conn = conn or self._connect()
        try:
            placeholders = ", ".join("?" for _ in product_ids)
            return active_conn.execute(
                f"""
                SELECT * FROM products
                WHERE product_id IN ({placeholders})
                ORDER BY product_id
                """,
                product_ids,
            ).fetchall()
        finally:
            if owns_conn:
                active_conn.close()

    def _connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.db_path)
        conn.row_factory = sqlite3.Row
        return conn


def _product_row(row: sqlite3.Row | dict[str, Any]) -> dict[str, Any]:
    return {
        "product_id": row["product_id"],
        "product_name": row["product_name"],
        "brand_name": row["brand_name"],
        "segment_name": row["segment_name"],
        "region": row["region"],
        "channel_name": row["channel_name"],
        "on_hand": int(row["on_hand"]),
        "safety_stock": int(row["safety_stock"]),
        "reorder_point": int(row["reorder_point"]),
        "holdback_units": int(row["holdback_units"]),
    }


def _action_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "action_id": row["action_id"],
        "action_type": row["action_type"],
        "idempotency_key": row["idempotency_key"],
        "priority": row["priority"],
        "reason": row["reason"],
        "status": row["status"],
        "product_count": int(row["product_count"]),
        "payload": json.loads(row["payload_json"]),
        "created_at": row["created_at"],
    }


def _action_response(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "action_id": row["action_id"],
        "action_type": row["action_type"],
        "created": True,
        "product_count": int(row["product_count"]),
        "idempotency_key": row["idempotency_key"],
        "status": row["status"],
    }


def _missing_ids(product_ids: list[str], rows: list[sqlite3.Row]) -> list[str]:
    found = {row["product_id"] for row in rows}
    return [product_id for product_id in product_ids if product_id not in found]


def _now() -> str:
    return datetime.now(UTC).isoformat()
