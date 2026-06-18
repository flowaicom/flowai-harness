from __future__ import annotations

import argparse
import os
import re
import sys
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from inventory_scenario.support.dataset_artifacts.build_sqlite import (
    INVENTORY_TABLE_ORDER,
    TABLE_COLUMNS,
    build_artifact_bundle,
)


DEFAULT_CONNECTION_URL_ENV = "INVENTORY_SCENARIO_NEON_URL"
DEFAULT_SOURCE_SCHEMA = "public"
_IDENTIFIER_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class PostgresInventorySource:
    def __init__(self, connection: Any, *, source_schema: str):
        self.connection = connection
        self.source_schema_name = source_schema
        self.source_schema = _quote_identifier(source_schema)
        self._columns_cache: dict[str, set[str]] = {}

    @classmethod
    def connect(cls, connection_url: str, *, source_schema: str):
        try:
            import psycopg
        except ImportError as exc:  # pragma: no cover - exercised by maintainers
            raise RuntimeError(
                "the maintainer Neon flow requires `psycopg[binary]`; install the example with maintainer dependencies"
            ) from exc

        connection = psycopg.connect(connection_url)
        return cls(connection, source_schema=source_schema)

    def close(self) -> None:
        self.connection.close()

    def count_rows(self, table_name: str) -> int:
        _require_inventory_table(table_name)
        with self.connection.cursor() as cursor:
            cursor.execute(
                f"SELECT count(*) FROM {self.source_schema}.{_quote_identifier(table_name)}"
            )
            return int(cursor.fetchone()[0])

    def iter_rows(
        self,
        table_name: str,
        columns: Sequence[str],
        order_by: Sequence[str],
    ) -> Iterable[dict[str, Any]]:
        _require_inventory_table(table_name)
        available_columns = self.table_columns(table_name)
        select_expressions = source_select_expressions(
            table_name,
            columns,
            available_columns,
        )
        for column in order_by:
            if column not in available_columns:
                raise ValueError(
                    f"source table {table_name} is missing required ordering column {column}; "
                    f"available columns: {', '.join(sorted(available_columns))}"
                )
        quoted_columns = ", ".join(select_expressions)
        quoted_order = ", ".join(_quote_identifier(column) for column in order_by)
        sql = (
            f"SELECT {quoted_columns} "
            f"FROM {self.source_schema}.{_quote_identifier(table_name)} "
            f"ORDER BY {quoted_order}"
        )
        try:
            from psycopg.rows import dict_row
        except ImportError as exc:  # pragma: no cover - guarded by connect
            raise RuntimeError("psycopg row helpers are unavailable") from exc

        cursor_name = f"inventory_{table_name}"
        with self.connection.cursor(name=cursor_name, row_factory=dict_row) as cursor:
            cursor.itersize = 10_000
            cursor.execute(sql)
            for row in cursor:
                yield dict(row)

    def table_columns(self, table_name: str) -> set[str]:
        _require_inventory_table(table_name)
        if table_name not in self._columns_cache:
            with self.connection.cursor() as cursor:
                cursor.execute(
                    """
                    SELECT column_name
                    FROM information_schema.columns
                    WHERE table_schema = %s AND table_name = %s
                    ORDER BY ordinal_position
                    """,
                    (self.source_schema_name, table_name),
                )
                columns = {str(row[0]) for row in cursor.fetchall()}
            if not columns:
                raise ValueError(
                    f"source table {self.source_schema_name}.{table_name} has no visible columns"
                )
            self._columns_cache[table_name] = columns
        return self._columns_cache[table_name]


def validate_unpooled_neon_url(connection_url: str) -> str:
    parsed = urlparse(connection_url)
    if parsed.scheme not in {"postgres", "postgresql"}:
        raise ValueError("Neon source URL must use postgres:// or postgresql://")
    if not parsed.hostname:
        raise ValueError("Neon source URL must include a host")
    if "-pooler" in parsed.hostname:
        raise ValueError(
            "use an unpooled Neon maintainer connection string; pooler hosts are not suitable for artifact generation"
        )
    return connection_url


def source_select_expressions(
    table_name: str,
    columns: Sequence[str],
    available_columns: set[str],
) -> list[str]:
    expressions = []
    for column in columns:
        if column in available_columns:
            expressions.append(_quote_identifier(column))
            continue
        raise ValueError(
            f"source table {table_name} is missing required column {column}; "
            f"available columns: {', '.join(sorted(available_columns))}"
        )
    return expressions


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Maintainer-only Neon to SQLite artifact generation flow.",
    )
    parser.add_argument("--dataset-version", required=True)
    parser.add_argument("--source-schema", default=DEFAULT_SOURCE_SCHEMA)
    parser.add_argument("--connection-url-env", default=DEFAULT_CONNECTION_URL_ENV)
    parser.add_argument("--output-dir", type=Path, default=Path("dist/inventory_scenario"))
    parser.add_argument("--artifact-base-url")
    parser.add_argument("--public-manifest", type=Path)
    parser.add_argument(
        "--yes",
        action="store_true",
        help="Confirm this private-source export should run.",
    )
    args = parser.parse_args(argv)

    if not args.yes:
        print(
            "Refusing to run without --yes because this reads the private Neon source.",
            file=sys.stderr,
        )
        return 2
    _load_env_file(Path(".env"))
    connection_url = os.environ.get(args.connection_url_env)
    if not connection_url:
        print(
            f"{args.connection_url_env} is required for the maintainer Neon export.",
            file=sys.stderr,
        )
        return 2

    try:
        validate_unpooled_neon_url(connection_url)
        artifact_base_url = args.artifact_base_url or _default_artifact_base_url(
            args.dataset_version
        )
        source = PostgresInventorySource.connect(
            connection_url,
            source_schema=args.source_schema,
        )
        try:
            bundle = build_artifact_bundle(
                source,
                output_dir=args.output_dir,
                dataset_version=args.dataset_version,
                artifact_base_url=artifact_base_url,
                public_manifest_path=args.public_manifest,
            )
        finally:
            source.close()
    except Exception as exc:
        print(str(exc), file=sys.stderr)
        return 2

    print(
        "\n".join(
            [
                f"target_sqlite={bundle.target_sqlite}",
                f"target_sqlite_zst={bundle.target_sqlite_zst}",
                f"manifest={bundle.manifest_path}",
                f"sha256sums={bundle.sha256sums_path}",
                f"target_sqlite_zst_sha256={bundle.target_sqlite_zst_sha256}",
            ]
        )
    )
    return 0


def _default_artifact_base_url(dataset_version: str) -> str:
    return (
        "https://flowai-public-data.hel1.your-objectstorage.com/"
        f"inventory-scenario/{dataset_version}"
    )


def _load_env_file(path: Path) -> None:
    if not path.exists():
        return
    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if not key or key in os.environ:
            continue
        os.environ[key] = value.strip().strip('"').strip("'")


def _quote_identifier(value: str) -> str:
    if not _IDENTIFIER_RE.match(value):
        raise ValueError(f"invalid SQL identifier: {value!r}")
    return f'"{value}"'


def _require_inventory_table(table_name: str) -> None:
    if table_name not in INVENTORY_TABLE_ORDER or table_name not in TABLE_COLUMNS:
        raise ValueError(f"unsupported inventory source table: {table_name}")


if __name__ == "__main__":
    raise SystemExit(main())
