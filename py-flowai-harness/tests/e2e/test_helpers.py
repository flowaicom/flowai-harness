import json
import subprocess
from datetime import datetime, timezone
from decimal import Decimal

import pytest

from tests.e2e.bootstrap_neon_base import main as bootstrap_main
from tests.e2e.conftest import _e2e_connection_budget
from tests.e2e.config import E2EConfig, branch_name_for
from tests.e2e.data_environment import build_data_environment, write_data_environment
from tests.e2e.neon import NeonBranch, NeonClient
from tests.e2e.progress import Progress
from tests.e2e.scenario import load_scenario
from tests.e2e.target_fixture import (
    TargetFixtureSnapshot,
    read_target_fixture_snapshot,
    validate_target_fixture_snapshot,
)
from tests.e2e.test_retail_revenue_neon import (
    _assert_live_metric_trajectory,
    _assert_profile_estimate_matches_scenario,
    _assert_profile_events_match_scenario,
    _assert_store_region_path,
    _decimal_values,
    _profile_scope_args,
)


def test_loads_retail_revenue_scenario_manifest():
    scenario = load_scenario("retail_revenue")

    assert scenario.name == "retail_revenue"
    assert scenario.target_schema == "retail"
    assert scenario.database_id == "retail_warehouse"
    assert scenario.profile_tables == [
        "regions",
        "stores",
        "customers",
        "products",
        "campaigns",
        "orders",
        "order_items",
        "refunds",
    ]
    assert scenario.document_paths == [
        "documents/revenue_policy.md",
        "documents/refund_policy.md",
        "documents/regional_reporting.md",
        "documents/campaign_attribution.md",
    ]
    assert scenario.expected_row_counts == {
        "regions": 3,
        "stores": 3,
        "customers": 4,
        "products": 3,
        "campaigns": 2,
        "orders": 5,
        "order_items": 7,
        "refunds": 2,
    }
    assert (
        "orders",
        "store_id",
        "stores",
        "id",
    ) in scenario.expected_relationships

    metric = scenario.question("q1_net_revenue_by_region")
    assert metric.kind == "metric"
    assert metric.expected_rows == {
        "APAC": "150.00",
        "EMEA": "240.00",
        "NAM": "260.00",
    }
    assert metric.expected_total == "650.00"


def test_branch_name_is_stable_bounded_and_neon_safe():
    created_at = datetime(2026, 6, 1, 12, 34, tzinfo=timezone.utc)

    name = branch_name_for(
        created_at=created_at,
        git_sha="67846a6abcdef",
        scenario="Retail Revenue/Smoke",
        suffix="A_B.c",
    )

    assert name == "e2e-20260601-67846a6-retail-revenue-smoke-a-b-c"
    assert len(name) <= 63


def test_data_environment_uses_url_envs_and_scoped_index_path(tmp_path):
    config = E2EConfig(
        neon_project_id="project-123",
        neon_role="e2e_owner",
        target_database="flowai_e2e_target",
        catalog_database="flowai_e2e_catalog",
        kv_database="flowai_e2e_kv",
        target_schema="retail",
        run_root=tmp_path,
    )

    payload = build_data_environment(config, run_id="run-123")
    path = write_data_environment(payload, tmp_path / "data-environment.json")

    assert payload == {
        "tenantId": "flowai-e2e",
        "workspaceId": "retail_revenue",
        "targetDatabase": {
            "kind": "postgres",
            "urlEnv": "FLOWAI_E2E_TARGET_DATABASE_URL",
            "schema": "retail",
        },
        "catalog": {
            "kind": "postgres",
            "urlEnv": "FLOWAI_E2E_CATALOG_DATABASE_URL",
            "ensureSchema": True,
        },
        "kv": {
            "kind": "postgres",
            "urlEnv": "FLOWAI_E2E_KV_DATABASE_URL",
            "table": "flowai_e2e_kv",
            "ensureSchema": True,
        },
        "catalogSearch": {
            "indexPath": str(tmp_path / "run-123" / "catalog-index"),
            "rebuildOnStart": True,
            "writeThrough": True,
        },
    }
    assert json.loads(path.read_text()) == payload


def test_neon_client_creates_branch_with_parent_and_expiration():
    calls = []

    def runner(args, **kwargs):
        calls.append((args, kwargs))
        return subprocess.CompletedProcess(
            args,
            0,
            stdout=json.dumps(
                {
                    "branch": {
                        "id": "br-test",
                        "name": "e2e-20260601-67846a6-retail-a1b2",
                    }
                }
            ),
            stderr="",
        )

    client = NeonClient(project_id="project-123", runner=runner)
    branch = client.create_branch(
        name="e2e-20260601-67846a6-retail-a1b2",
        parent="e2e-base",
        expires_at="2026-06-02T12:34:00Z",
    )

    assert branch == NeonBranch(
        id="br-test",
        name="e2e-20260601-67846a6-retail-a1b2",
    )
    args, kwargs = calls[0]
    assert args == [
        "neonctl",
        "branches",
        "create",
        "--project-id",
        "project-123",
        "--name",
        "e2e-20260601-67846a6-retail-a1b2",
        "--parent",
        "e2e-base",
        "--expires-at",
        "2026-06-02T12:34:00Z",
        "-o",
        "json",
    ]
    assert kwargs["check"] is False
    assert kwargs["capture_output"] is True
    assert kwargs["text"] is True


def test_neon_client_resolves_direct_connection_string():
    calls = []

    def runner(args, **kwargs):
        calls.append((args, kwargs))
        return subprocess.CompletedProcess(
            args,
            0,
            stdout="postgresql://user:secret@ep-test.neon.tech/flowai_e2e_target?sslmode=require\n",
            stderr="",
        )

    client = NeonClient(project_id="project-123", runner=runner)
    url = client.connection_string(
        branch="e2e-branch",
        database="flowai_e2e_target",
        role="e2e_owner",
    )

    assert url.startswith("postgresql://")
    args, _kwargs = calls[0]
    assert args == [
        "neonctl",
        "connection-string",
        "e2e-branch",
        "--project-id",
        "project-123",
        "--database-name",
        "flowai_e2e_target",
        "--role-name",
        "e2e_owner",
        "--ssl",
        "require",
    ]


def test_bootstrap_rejects_blank_project_id_before_calling_neonctl(monkeypatch):
    calls = []

    def fake_run(*args, **kwargs):
        calls.append((args, kwargs))
        raise AssertionError("bootstrap should reject blank project id before neonctl")

    monkeypatch.setattr(subprocess, "run", fake_run)

    with pytest.raises(SystemExit) as exc:
        bootstrap_main(["--project-id", "", "--branch", "e2e-base"])

    assert "FLOWAI_E2E_NEON_PROJECT_ID" in str(exc.value)
    assert calls == []


def test_neon_client_rejects_blank_project_id():
    with pytest.raises(ValueError, match="project_id"):
        NeonClient(project_id="  ")


def test_neon_client_failure_includes_stderr_without_raw_called_process_error():
    def runner(args, **kwargs):
        return subprocess.CompletedProcess(
            args,
            1,
            stdout="",
            stderr="branch e2e-base was not found",
        )

    client = NeonClient(project_id="project-123", runner=runner)

    with pytest.raises(RuntimeError) as exc:
        client.connection_string(
            branch="e2e-base",
            database="neondb",
            role="e2e_owner",
        )

    message = str(exc.value)
    assert "neonctl command failed" in message
    assert "branch e2e-base was not found" in message
    assert "--project-id project-123" in message


def test_progress_writes_to_configured_terminal_path(tmp_path):
    tty_path = tmp_path / "progress.log"
    progress = Progress(tty_path=tty_path)

    progress.log("creating branch")

    assert "creating branch" in tty_path.read_text()
    assert "flowai-e2e" in tty_path.read_text()


def test_live_assertion_helpers_accept_semantic_equivalent_output():
    text = (
        "Use orders, order_items, refunds, stores, and regions. Join orders.store_id "
        "to stores and then stores.region_id to regions. EMEA net revenue is $240."
    )

    _assert_store_region_path(text.lower())
    assert Decimal("240.00") in _decimal_values(text)


def test_live_metric_trajectory_requires_catalog_before_sql():
    with pytest.raises(AssertionError, match="catalog tool"):
        _assert_live_metric_trajectory(
            [{"type": "tool-invocation", "toolName": "execute_query"}]
        )

    _assert_live_metric_trajectory(
        [
            {"type": "tool-invocation", "toolName": "search_catalog"},
            {"type": "tool-invocation", "toolName": "execute_query"},
        ]
    )


def test_profile_estimate_assertion_rejects_empty_target_shape():
    scenario = load_scenario("retail_revenue")

    with pytest.raises(AssertionError, match="tableCount"):
        _assert_profile_estimate_matches_scenario(
            scenario,
            {
                "tableCount": 0,
                "columnCount": 0,
                "estimatedInputTokens": 10,
            },
        )


def test_profile_event_assertion_requires_each_expected_table():
    scenario = load_scenario("retail_revenue")

    with pytest.raises(AssertionError, match="missing completed tables"):
        _assert_profile_events_match_scenario(
            scenario,
            [
                {
                    "type": "completed",
                    "summary": {
                        "tablesDiscovered": 0,
                        "columnsProfiled": 0,
                        "relationshipsFound": 0,
                    },
                }
            ],
        )


def test_profile_scope_profiles_full_schema_without_table_filters():
    scenario = load_scenario("retail_revenue")

    args = _profile_scope_args(scenario)

    assert args == [
        "--database-id",
        "retail_warehouse",
        "--schema",
        "retail",
    ]
    assert "--table" not in args


def test_e2e_connection_budget_is_small_and_explicit():
    budget = _e2e_connection_budget()

    assert budget["python_validator_connection_attempts_per_try"] == 1
    assert budget["neon_connection_string_cli_calls"] == 3
    assert budget["rust_profile_table_concurrency"] == 4
    assert budget["sqlx_default_max_pool_connections_per_postgres_handle"] == 10


def test_target_fixture_snapshot_detects_empty_target_database():
    scenario = load_scenario("retail_revenue")

    errors = validate_target_fixture_snapshot(
        scenario,
        TargetFixtureSnapshot(
            tables=set(),
            row_counts={},
            columns={},
            relationships=set(),
        ),
    )

    assert any("missing tables" in error for error in errors)
    assert any("orders expected 5 rows, found 0" in error for error in errors)


def test_target_fixture_snapshot_accepts_retail_seed_contract():
    scenario = load_scenario("retail_revenue")

    errors = validate_target_fixture_snapshot(
        scenario,
        TargetFixtureSnapshot(
            tables=set(scenario.profile_tables),
            row_counts=scenario.expected_row_counts,
            columns=scenario.expected_columns,
            relationships=set(scenario.expected_relationships),
        ),
    )

    assert errors == []


def test_target_fixture_reader_sets_db_timeouts_and_logs_progress(tmp_path):
    scenario = load_scenario("retail_revenue")
    psycopg = _FakePsycopg(scenario)
    progress = Progress(tty_path=tmp_path / "progress.log")

    snapshot = read_target_fixture_snapshot(
        psycopg,
        "postgresql://example/flowai_e2e_target",
        scenario,
        progress=progress,
    )

    assert snapshot.row_counts["orders"] == 5
    assert psycopg.connect_kwargs["autocommit"] is True
    assert psycopg.connect_kwargs["connect_timeout"] == 15
    assert psycopg.connect_kwargs["application_name"] == "flowai-e2e-fixture-validation"
    assert any("set_config('statement_timeout'" in statement for statement in psycopg.statements)
    log = (tmp_path / "progress.log").read_text()
    assert "target validation: connect to target database" in log
    assert "target validation: count rows for 8 expected tables" in log
    count_statements = [
        statement for statement in psycopg.statements if "COUNT(*)" in statement
    ]
    assert len(count_statements) == 1


def test_target_fixture_reader_retries_neon_retryable_connection_error(tmp_path):
    scenario = load_scenario("retail_revenue")
    psycopg = _FakePsycopg(scenario, fail_connect_attempts=1)
    progress = Progress(tty_path=tmp_path / "progress.log")

    snapshot = read_target_fixture_snapshot(
        psycopg,
        "postgresql://example/flowai_e2e_target",
        scenario,
        progress=progress,
        retry_delays=(0,),
    )

    assert psycopg.connect_calls == 2
    assert snapshot.row_counts["orders"] == 5
    assert "retryable Neon connection error" in (tmp_path / "progress.log").read_text()


class _FakePsycopg:
    def __init__(self, scenario, fail_connect_attempts=0):
        self.scenario = scenario
        self.fail_connect_attempts = fail_connect_attempts
        self.connect_calls = 0
        self.connect_kwargs = {}
        self.statements = []

    def connect(self, database_url, **kwargs):
        self.connect_calls += 1
        if self.connect_calls <= self.fail_connect_attempts:
            raise RuntimeError(
                "Failed to acquire permit to connect to the database. "
                "Too many database connection attempts are currently ongoing. "
                '"neon:retryable":true'
            )
        self.database_url = database_url
        self.connect_kwargs = kwargs
        return _FakeConnection(self)


class _FakeConnection:
    def __init__(self, psycopg):
        self.psycopg = psycopg

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return None

    def execute(self, statement, params=None):
        self.psycopg.statements.append(statement)
        scenario = self.psycopg.scenario
        if "information_schema.tables" in statement:
            return _FakeResult([(table,) for table in scenario.profile_tables])
        if "information_schema.columns" in statement:
            return _FakeResult(
                [
                    (table, column)
                    for table, columns in scenario.expected_columns.items()
                    for column in columns
                ]
            )
        if "information_schema.table_constraints" in statement:
            return _FakeResult(list(scenario.expected_relationships))
        if "COUNT(*)" in statement:
            return _FakeResult(
                [
                    (table, scenario.expected_row_counts[table])
                    for table in scenario.profile_tables
                ]
            )
        return _FakeResult([])


class _FakeResult:
    def __init__(self, rows):
        self.rows = rows

    def fetchall(self):
        return self.rows

    def fetchone(self):
        return self.rows[0]
