from __future__ import annotations

import re
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any

from tests.e2e.scenario import ExpectedRelationship, Scenario


@dataclass(frozen=True)
class TargetFixtureSnapshot:
    tables: set[str]
    row_counts: Mapping[str, int]
    columns: Mapping[str, Sequence[str]]
    relationships: set[ExpectedRelationship]


def validate_target_fixture(
    psycopg,
    database_url: str,
    scenario: Scenario,
    *,
    progress: Any | None = None,
) -> TargetFixtureSnapshot:
    snapshot = read_target_fixture_snapshot(
        psycopg,
        database_url,
        scenario,
        progress=progress,
    )
    errors = validate_target_fixture_snapshot(scenario, snapshot)
    if errors:
        rendered = "\n".join(f"- {error}" for error in errors)
        raise RuntimeError(
            "Neon e2e target fixture is not ready. "
            "Refresh the parent branch before running e2e tests:\n"
            "  uv run --extra dev python -m tests.e2e.bootstrap_neon_base "
            "--project-id \"$FLOWAI_E2E_NEON_PROJECT_ID\" "
            "--branch \"${FLOWAI_E2E_NEON_PARENT_BRANCH:-e2e-base}\"\n"
            f"Validation errors:\n{rendered}"
        )
    return snapshot


def read_target_fixture_snapshot(
    psycopg,
    database_url: str,
    scenario: Scenario,
    *,
    progress: Any | None = None,
    retry_delays: Sequence[float] = (1.0, 3.0, 5.0, 10.0),
) -> TargetFixtureSnapshot:
    attempts = len(retry_delays) + 1
    for attempt in range(1, attempts + 1):
        try:
            return _read_target_fixture_snapshot_once(
                psycopg,
                database_url,
                scenario,
                progress=progress,
            )
        except Exception as error:
            if not _is_retryable_neon_error(error) or attempt == attempts:
                raise
            delay = retry_delays[attempt - 1]
            _log(
                progress,
                "target validation: retryable Neon connection error "
                f"on attempt {attempt}/{attempts}; retrying in {delay:g}s: {error}",
            )
            time.sleep(delay)
    raise AssertionError("unreachable")


def _read_target_fixture_snapshot_once(
    psycopg,
    database_url: str,
    scenario: Scenario,
    *,
    progress: Any | None = None,
) -> TargetFixtureSnapshot:
    schema = scenario.target_schema
    _log(progress, "target validation: connect to target database")
    with psycopg.connect(
        database_url,
        autocommit=True,
        connect_timeout=15,
        application_name="flowai-e2e-fixture-validation",
    ) as conn:
        _log(progress, "target validation: set database timeouts")
        conn.execute("SELECT set_config('statement_timeout', '15000', false)")
        conn.execute("SELECT set_config('lock_timeout', '5000', false)")

        _log(progress, f"target validation: list tables in schema {schema}")
        tables = {
            row[0]
            for row in conn.execute(
                """
                SELECT table_name
                FROM information_schema.tables
                WHERE table_schema = %s
                  AND table_type = 'BASE TABLE'
                """,
                (schema,),
            ).fetchall()
        }

        _log(progress, f"target validation: list columns in schema {schema}")
        columns: dict[str, list[str]] = {}
        for table_name, column_name in conn.execute(
            """
            SELECT table_name, column_name
            FROM information_schema.columns
            WHERE table_schema = %s
            ORDER BY table_name, ordinal_position
            """,
            (schema,),
        ).fetchall():
            columns.setdefault(table_name, []).append(column_name)

        _log(progress, f"target validation: list relationships in schema {schema}")
        relationships = {
            (from_table, from_column, to_table, to_column)
            for from_table, from_column, to_table, to_column in conn.execute(
                """
                SELECT
                    tc.table_name AS from_table,
                    kcu.column_name AS from_column,
                    ccu.table_name AS to_table,
                    ccu.column_name AS to_column
                FROM information_schema.table_constraints tc
                JOIN information_schema.key_column_usage kcu
                  ON tc.constraint_name = kcu.constraint_name
                 AND tc.constraint_schema = kcu.constraint_schema
                JOIN information_schema.constraint_column_usage ccu
                  ON ccu.constraint_name = tc.constraint_name
                 AND ccu.constraint_schema = tc.constraint_schema
                WHERE tc.table_schema = %s
                  AND tc.constraint_type = 'FOREIGN KEY'
                """,
                (schema,),
            ).fetchall()
        }

        count_tables = [table for table in scenario.profile_tables if table in tables]
        _log(
            progress,
            f"target validation: count rows for {len(count_tables)} expected tables",
        )
        row_counts = {
            table: int(count)
            for table, count in conn.execute(
                _row_count_sql(schema, count_tables)
            ).fetchall()
        }

    return TargetFixtureSnapshot(
        tables=tables,
        row_counts=row_counts,
        columns=columns,
        relationships=relationships,
    )


def validate_target_fixture_snapshot(
    scenario: Scenario,
    snapshot: TargetFixtureSnapshot,
) -> list[str]:
    errors: list[str] = []
    expected_tables = set(scenario.profile_tables)
    missing_tables = sorted(expected_tables - snapshot.tables)
    if missing_tables:
        errors.append(f"missing tables in schema {scenario.target_schema}: {missing_tables}")

    for table, expected_count in scenario.expected_row_counts.items():
        found_count = int(snapshot.row_counts.get(table, 0))
        if found_count != expected_count:
            errors.append(f"{table} expected {expected_count} rows, found {found_count}")

    for table, expected_columns in scenario.expected_columns.items():
        found_columns = list(snapshot.columns.get(table, []))
        if found_columns != expected_columns:
            errors.append(
                f"{table} columns mismatch: expected {expected_columns}, found {found_columns}"
            )

    expected_relationships = set(scenario.expected_relationships)
    missing_relationships = sorted(expected_relationships - snapshot.relationships)
    if missing_relationships:
        errors.append(f"missing foreign-key relationships: {missing_relationships}")

    return errors


def _quote_identifier(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", value):
        raise ValueError(f"invalid SQL identifier: {value!r}")
    return f'"{value}"'


def _row_count_sql(schema: str, tables: Sequence[str]) -> str:
    if not tables:
        return "SELECT NULL::text AS table_name, 0::bigint AS row_count WHERE false"
    return "\nUNION ALL\n".join(
        "SELECT "
        f"{_quote_literal(table)} AS table_name, "
        f"COUNT(*)::bigint AS row_count FROM {_quote_identifier(schema)}.{_quote_identifier(table)}"
        for table in tables
    )


def _quote_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def _log(progress: Any | None, message: str) -> None:
    if progress is not None:
        progress.log(message)


def _is_retryable_neon_error(error: Exception) -> bool:
    message = str(error).lower()
    return (
        "neon:retryable" in message
        or "failed to acquire permit" in message
        or "too many database connection attempts" in message
    )
