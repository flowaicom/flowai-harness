from __future__ import annotations

import asyncio
import re
import sqlite3
from pathlib import Path
from typing import Any

from pydantic import BaseModel, ConfigDict, Field
from pydantic.alias_generators import to_camel
from flowai_harness import define_tool

from inventory_scenario.plans import ProductSet, ProductSetPayload


SQL_PARAM = str | int | float | bool | None

_FORBIDDEN_SQL = re.compile(
    r"\b(insert|update|delete|drop|alter|create|replace|pragma|attach|detach|vacuum)\b",
    re.IGNORECASE,
)


class ToolModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        extra="forbid",
        populate_by_name=True,
    )


class ResolveProductSetInput(ToolModel):
    sql: str = Field(min_length=1)
    params: list[SQL_PARAM] = Field(default_factory=list)
    reason: str = Field(min_length=1)
    selection_summary: str | None = None


class _SqliteProductSetQueryResolver:
    def __init__(self, data_environment: dict[str, Any]):
        self.target_db = _target_db_from_data_environment(data_environment)

    async def resolve(self, value: ResolveProductSetInput) -> ProductSetPayload:
        return await asyncio.to_thread(self._resolve_sync, value)

    def _resolve_sync(self, value: ResolveProductSetInput) -> ProductSetPayload:
        sql = _read_only_select_sql(value.sql)
        with sqlite3.connect(self.target_db) as conn:
            conn.row_factory = sqlite3.Row
            cursor = conn.execute(sql, value.params)
            rows = cursor.fetchall()
            columns = [description[0] for description in cursor.description or []]
        if "product_id" not in columns:
            raise ValueError("resolveProductSet SQL must return a product_id column")
        sample = [_json_row(row) for row in rows[:3]]
        product_ids = list(dict.fromkeys(str(row["product_id"]) for row in rows))
        return ProductSetPayload(
            product_ids=product_ids,
            sql=sql,
            params=value.params,
            reason=value.reason,
            selection_summary=value.selection_summary,
            sample=sample,
        )


def _target_db_from_data_environment(data_environment: dict[str, Any]) -> Path:
    target_database = data_environment.get("target_database") or data_environment.get(
        "targetDatabase"
    )
    if not isinstance(target_database, dict):
        raise ValueError("data_environment target_database is required")
    url = target_database.get("url")
    if not isinstance(url, str) or not url.startswith("sqlite:"):
        raise ValueError("inventory scenario resolver requires a sqlite target_database url")
    return Path(url.removeprefix("sqlite:"))


def _read_only_select_sql(sql: str) -> str:
    stripped = sql.strip()
    if stripped.endswith(";"):
        stripped = stripped[:-1].strip()
    if ";" in stripped:
        raise ValueError("resolveProductSet SQL must be a single read-only statement")
    first_token = stripped.split(None, 1)[0].lower() if stripped else ""
    if first_token not in {"select", "with"}:
        raise ValueError("resolveProductSet SQL must be a read-only SELECT or WITH query")
    if _FORBIDDEN_SQL.search(stripped):
        raise ValueError("resolveProductSet SQL must be read-only")
    return stripped


def _json_row(row: sqlite3.Row) -> dict[str, Any]:
    return {key: row[key] for key in row.keys()}


async def _resolve_product_set(
    args: dict[str, Any],
    ctx,
    *,
    resolver: _SqliteProductSetQueryResolver,
) -> dict[str, Any]:
    value = ResolveProductSetInput.model_validate(args)
    payload = await resolver.resolve(value)
    ref = await ctx.references.create(ProductSet, payload)
    return {
        "reference": {"kind": ref["kind"], "id": ref["id"]},
        "glimpse": ref["glimpse"],
    }


def _build_resolve_product_set_tool(
    resolver: _SqliteProductSetQueryResolver | None,
):
    @define_tool(
        name="resolveProductSet",
        description=(
            "Run a read-only SQL product selection, store all product ids as an "
            "InventoryProductSet reference, and return only a handle plus glimpse."
        ),
        input_schema=ResolveProductSetInput,
        approval="never",
    )
    async def tool(args: dict[str, Any], ctx):
        if resolver is None:
            raise ValueError(
                "resolveProductSet must be bound with a data_environment before use"
            )
        return await _resolve_product_set(args, ctx, resolver=resolver)

    return tool


def resolve_product_set_tool_for_data_environment(data_environment: dict[str, Any]):
    return _build_resolve_product_set_tool(
        _SqliteProductSetQueryResolver(data_environment)
    )


resolve_product_set_tool = _build_resolve_product_set_tool(None)
PLANNER_TOOLS = [resolve_product_set_tool]
EXECUTOR_TOOLS: list[Any] = []
