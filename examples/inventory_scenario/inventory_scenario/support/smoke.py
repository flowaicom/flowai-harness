from __future__ import annotations

import argparse
import asyncio
import sqlite3
from pathlib import Path

from flowai_harness import TestingConfig

from inventory_scenario.runtime import build_runtime
from inventory_scenario.support.data_environment import build_data_environment
from inventory_scenario.support.seed import SeedOptions, SeedResult, run_seed


async def run_smoke_check() -> tuple[SeedResult, list[dict]]:
    result = run_seed(SeedOptions())
    _verify_local_state(result)
    runtime = build_runtime(
        data_environment=build_data_environment(result.data_root),
        testing=TestingConfig(mock_response="inventory scenario smoke ok"),
    )
    events = []
    async for event in runtime.query(
        "Summarize the local inventory scenario.",
        thread_id="inventory-scenario-smoke",
    ):
        events.append(event)
    return result, events


async def collect_smoke_events():
    _result, events = await run_smoke_check()
    return events


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run deterministic local inventory scenario smoke verification.",
    )
    parser.parse_args(argv)

    result, events = asyncio.run(run_smoke_check())
    print(smoke_success_line(data_root=result.data_root, event_count=len(events)))
    return 0


def smoke_success_line(*, data_root: Path, event_count: int) -> str:
    return (
        "inventory scenario smoke ok: "
        f"target DB={data_root / 'target.db'}; "
        "catalog query=ok; "
        "mock platform=ok; "
        f"scripted runtime events={event_count}"
    )


def _verify_local_state(result: SeedResult) -> None:
    _require_count(result.target_db, "dim_products")
    _require_count(
        result.data_root / "catalog.db",
        "catalog_entries",
        missing_hint=(
            "Run the post-seed Flow AI data ops first: profile the database, "
            "ingest knowledge, and rebuild the catalog index."
        ),
    )
    _require_count(result.platform_db, "products")


def _require_count(db_path: Path, table: str, *, missing_hint: str | None = None) -> None:
    if not db_path.exists():
        hint = f" {missing_hint}" if missing_hint else ""
        raise RuntimeError(f"smoke check failed: {db_path} does not exist.{hint}")
    with sqlite3.connect(db_path) as conn:
        count = int(conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0])
    if count <= 0:
        raise RuntimeError(f"smoke check failed: {db_path} table {table} is empty")


if __name__ == "__main__":
    raise SystemExit(main())
