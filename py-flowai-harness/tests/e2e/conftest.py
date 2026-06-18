from __future__ import annotations

import os
import subprocess
import uuid
from dataclasses import dataclass, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Mapping, Sequence

import pytest

from tests.e2e.cli_runner import CliRun, run_flowai_harness
from tests.e2e.config import E2EConfig, branch_name_for
from tests.e2e.data_environment import build_data_environment, write_data_environment
from tests.e2e.neon import NeonBranch, NeonClient
from tests.e2e.progress import Progress
from tests.e2e.scenario import Scenario, load_scenario
from tests.e2e.target_fixture import validate_target_fixture


def pytest_configure(config):
    config.addinivalue_line("markers", "e2e: FlowAI harness end-to-end tests")
    config.addinivalue_line("markers", "neon: tests that create temporary Neon branches")
    config.addinivalue_line("markers", "live_llm: tests that call a live model provider")
    config.addinivalue_line("markers", "live_enrichment: tests that call LLM enrichment")
    if (
        os.environ.get("FLOWAI_E2E") == "1"
        and os.environ.get("FLOWAI_E2E_PROGRESS", "1") != "0"
    ):
        config.option.capture = "no"


def pytest_collection_modifyitems(config, items):
    skip_neon = pytest.mark.skip(
        reason="set FLOWAI_E2E=1 and FLOWAI_E2E_NEON_PROJECT_ID to run Neon e2e tests"
    )
    skip_live_llm = pytest.mark.skip(
        reason="set FLOWAI_E2E_LIVE_LLM=1 and ANTHROPIC_API_KEY to run live LLM e2e tests"
    )
    skip_live_enrichment = pytest.mark.skip(
        reason="set FLOWAI_E2E_LIVE_ENRICHMENT=1 and ANTHROPIC_API_KEY to run live enrichment"
    )
    neon_enabled = os.environ.get("FLOWAI_E2E") == "1" and bool(
        os.environ.get("FLOWAI_E2E_NEON_PROJECT_ID", "").strip()
    )
    live_llm_enabled = (
        neon_enabled
        and os.environ.get("FLOWAI_E2E_LIVE_LLM") == "1"
        and bool(os.environ.get("ANTHROPIC_API_KEY", "").strip())
    )
    live_enrichment_enabled = (
        neon_enabled
        and os.environ.get("FLOWAI_E2E_LIVE_ENRICHMENT") == "1"
        and bool(os.environ.get("ANTHROPIC_API_KEY", "").strip())
    )

    for item in items:
        if item.get_closest_marker("neon") and not neon_enabled:
            item.add_marker(skip_neon)
        if item.get_closest_marker("live_llm") and not live_llm_enabled:
            item.add_marker(skip_live_llm)
        if item.get_closest_marker("live_enrichment") and not live_enrichment_enabled:
            item.add_marker(skip_live_enrichment)


@dataclass
class NeonE2ERun:
    package_root: Path
    repo_root: Path
    config: E2EConfig
    scenario: Scenario
    branch: NeonBranch
    run_id: str
    data_environment: dict
    data_environment_path: Path
    env: Mapping[str, str]
    progress: Progress

    def run_cli(
        self,
        args: Sequence[str],
        *,
        timeout: int = 180,
        check: bool = True,
    ) -> CliRun:
        return run_flowai_harness(
            args,
            cwd=self.package_root,
            env=self.env,
            timeout=timeout,
            check=check,
            progress=self.progress,
        )


@pytest.fixture(scope="session")
def package_root() -> Path:
    return Path(__file__).resolve().parents[2]


@pytest.fixture(scope="session")
def repo_root(package_root: Path) -> Path:
    return package_root.parent


@pytest.fixture(scope="session")
def retail_scenario() -> Scenario:
    return load_scenario("retail_revenue")


@pytest.fixture
def neon_e2e_run(package_root: Path, repo_root: Path, retail_scenario: Scenario):
    base_config = E2EConfig.from_env()
    config = replace(base_config, run_root=package_root / base_config.run_root)
    progress = Progress()
    created_at = datetime.now(timezone.utc)
    git_sha = _git_sha(repo_root)
    suffix = uuid.uuid4().hex[:6]
    branch_name = branch_name_for(
        created_at=created_at,
        git_sha=git_sha,
        scenario=retail_scenario.name,
        suffix=suffix,
    )
    progress.log(
        f"preparing Neon run for scenario={retail_scenario.name} "
        f"project={config.neon_project_id} parent={config.parent_branch}"
    )
    progress.log(f"connection budget: {_format_connection_budget(_e2e_connection_budget())}")
    client = NeonClient(project_id=config.neon_project_id, progress=progress)
    with progress.step(f"create temporary Neon branch {branch_name}"):
        branch = client.create_branch(
            name=branch_name,
            parent=config.parent_branch,
            expires_at=config.expires_at(now=created_at),
        )
    run_id = branch.name
    with progress.step("resolve Neon connection string for target database"):
        target_url = client.connection_string(
            branch=branch.name,
            database=config.target_database,
            role=config.neon_role,
        )
    with progress.step("validate target fixture rows and relationships"):
        try:
            import psycopg
        except ImportError as error:
            raise RuntimeError(
                "psycopg[binary] is required for Neon e2e target validation; "
                "install with `uv sync --extra dev`"
            ) from error
        snapshot = validate_target_fixture(
            psycopg,
            target_url,
            retail_scenario,
            progress=progress,
        )
        progress.log(
            f"target fixture ready: {len(snapshot.tables)} tables, "
            f"{sum(snapshot.row_counts.values())} rows"
        )
    with progress.step("resolve Neon connection string for catalog database"):
        catalog_url = client.connection_string(
            branch=branch.name,
            database=config.catalog_database,
            role=config.neon_role,
        )
    with progress.step("resolve Neon connection string for KV database"):
        kv_url = client.connection_string(
            branch=branch.name,
            database=config.kv_database,
            role=config.neon_role,
        )
    env = {
        "FLOWAI_E2E_TARGET_DATABASE_URL": target_url,
        "FLOWAI_E2E_CATALOG_DATABASE_URL": catalog_url,
        "FLOWAI_E2E_KV_DATABASE_URL": kv_url,
    }
    data_environment = build_data_environment(config, run_id=run_id)
    with progress.step("write scoped data-environment file"):
        data_environment_path = write_data_environment(
            data_environment,
            config.run_root / run_id / "data-environment.json",
        )
        progress.log(f"data environment: {data_environment_path}")

    context = NeonE2ERun(
        package_root=package_root,
        repo_root=repo_root,
        config=config,
        scenario=retail_scenario,
        branch=branch,
        run_id=run_id,
        data_environment=data_environment,
        data_environment_path=data_environment_path,
        env=env,
        progress=progress,
    )
    try:
        yield context
    finally:
        with progress.step(f"best-effort delete Neon branch {branch.name}"):
            client.delete_branch(branch)


def _git_sha(repo_root: Path) -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "--short=12", "HEAD"],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=True,
    )
    return completed.stdout.strip()


def _e2e_connection_budget() -> dict[str, int]:
    return {
        "python_validator_connection_attempts_per_try": 1,
        "neon_connection_string_cli_calls": 3,
        "rust_profile_table_concurrency": 4,
        "sqlx_default_max_pool_connections_per_postgres_handle": 10,
    }


def _format_connection_budget(budget: Mapping[str, int]) -> str:
    return ", ".join(f"{key}={value}" for key, value in budget.items())
