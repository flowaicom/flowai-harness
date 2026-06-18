from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from tests.e2e.config import E2EConfig
from tests.e2e.neon import NeonClient
from tests.e2e.scenario import load_scenario
from tests.e2e.target_fixture import validate_target_fixture


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Create or refresh the Neon e2e-base fixture databases.",
    )
    parser.add_argument("--project-id", default=os.environ.get("FLOWAI_E2E_NEON_PROJECT_ID"))
    parser.add_argument(
        "--branch",
        default=os.environ.get("FLOWAI_E2E_NEON_PARENT_BRANCH", "e2e-base"),
    )
    parser.add_argument("--role", default=os.environ.get("FLOWAI_E2E_NEON_ROLE", "e2e_owner"))
    parser.add_argument("--scenario", default="retail_revenue")
    parser.add_argument(
        "--target-database",
        default=os.environ.get("FLOWAI_E2E_TARGET_DATABASE", "flowai_e2e_target"),
    )
    parser.add_argument(
        "--catalog-database",
        default=os.environ.get("FLOWAI_E2E_CATALOG_DATABASE", "flowai_e2e_catalog"),
    )
    parser.add_argument(
        "--kv-database",
        default=os.environ.get("FLOWAI_E2E_KV_DATABASE", "flowai_e2e_kv"),
    )
    args = parser.parse_args(argv)
    project_id = _required_non_blank(
        args.project_id,
        "FLOWAI_E2E_NEON_PROJECT_ID is required; set it or pass --project-id <neon-project-id>",
    )

    try:
        import psycopg
    except ImportError as error:
        raise SystemExit(
            "psycopg[binary] is required; install with `uv sync --extra dev`"
        ) from error

    config = E2EConfig(
        neon_project_id=project_id,
        neon_role=args.role,
        parent_branch=args.branch,
        target_database=args.target_database,
        catalog_database=args.catalog_database,
        kv_database=args.kv_database,
    )
    client = NeonClient(project_id=config.neon_project_id)
    scenario = load_scenario(args.scenario)

    admin_url = client.connection_string(
        branch=args.branch,
        database="neondb",
        role=args.role,
    )
    for database in [config.target_database, config.catalog_database, config.kv_database]:
        _ensure_database(psycopg, admin_url, database)

    target_url = client.connection_string(
        branch=args.branch,
        database=config.target_database,
        role=args.role,
    )
    _execute_sql_file(psycopg, target_url, scenario.schema_sql)
    _execute_sql_file(psycopg, target_url, scenario.seed_sql)
    snapshot = validate_target_fixture(psycopg, target_url, scenario)

    print(
        f"refreshed {args.scenario} fixtures on Neon branch {args.branch}: "
        f"{len(snapshot.tables)} tables, {sum(snapshot.row_counts.values())} rows"
    )
    return 0


def _required_non_blank(value: str | None, message: str) -> str:
    if value is None or not value.strip():
        raise SystemExit(message)
    return value.strip()


def _ensure_database(psycopg, admin_url: str, database: str) -> None:
    with psycopg.connect(admin_url, autocommit=True) as conn:
        exists = conn.execute(
            "SELECT 1 FROM pg_database WHERE datname = %s",
            (database,),
        ).fetchone()
        if exists is None:
            conn.execute(f'CREATE DATABASE "{database}"')


def _execute_sql_file(psycopg, database_url: str, path: Path) -> None:
    with psycopg.connect(database_url, autocommit=True) as conn:
        conn.execute(path.read_text())


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
