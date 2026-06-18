from __future__ import annotations

import os
import re
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path


DEFAULT_PARENT_BRANCH = "e2e-base"
DEFAULT_ROLE = "e2e_owner"
DEFAULT_TARGET_DATABASE = "flowai_e2e_target"
DEFAULT_CATALOG_DATABASE = "flowai_e2e_catalog"
DEFAULT_KV_DATABASE = "flowai_e2e_kv"
DEFAULT_TARGET_SCHEMA = "retail"
DEFAULT_TENANT_ID = "flowai-e2e"
DEFAULT_WORKSPACE_ID = "retail_revenue"
DEFAULT_BRANCH_TTL_HOURS = 24


@dataclass(frozen=True)
class E2EConfig:
    neon_project_id: str
    neon_role: str = DEFAULT_ROLE
    parent_branch: str = DEFAULT_PARENT_BRANCH
    branch_ttl_hours: int = DEFAULT_BRANCH_TTL_HOURS
    target_database: str = DEFAULT_TARGET_DATABASE
    catalog_database: str = DEFAULT_CATALOG_DATABASE
    kv_database: str = DEFAULT_KV_DATABASE
    target_schema: str = DEFAULT_TARGET_SCHEMA
    tenant_id: str = DEFAULT_TENANT_ID
    workspace_id: str = DEFAULT_WORKSPACE_ID
    run_root: Path = Path(".data/e2e")

    @classmethod
    def from_env(cls) -> "E2EConfig":
        project_id = os.environ.get("FLOWAI_E2E_NEON_PROJECT_ID", "").strip()
        if not project_id:
            raise RuntimeError("FLOWAI_E2E_NEON_PROJECT_ID is required for Neon e2e tests")
        return cls(
            neon_project_id=project_id,
            neon_role=os.environ.get("FLOWAI_E2E_NEON_ROLE", DEFAULT_ROLE),
            parent_branch=os.environ.get(
                "FLOWAI_E2E_NEON_PARENT_BRANCH",
                DEFAULT_PARENT_BRANCH,
            ),
            branch_ttl_hours=int(
                os.environ.get("FLOWAI_E2E_BRANCH_TTL_HOURS", str(DEFAULT_BRANCH_TTL_HOURS))
            ),
            target_database=os.environ.get(
                "FLOWAI_E2E_TARGET_DATABASE",
                DEFAULT_TARGET_DATABASE,
            ),
            catalog_database=os.environ.get(
                "FLOWAI_E2E_CATALOG_DATABASE",
                DEFAULT_CATALOG_DATABASE,
            ),
            kv_database=os.environ.get("FLOWAI_E2E_KV_DATABASE", DEFAULT_KV_DATABASE),
            target_schema=os.environ.get("FLOWAI_E2E_TARGET_SCHEMA", DEFAULT_TARGET_SCHEMA),
            run_root=Path(os.environ.get("FLOWAI_E2E_RUN_ROOT", ".data/e2e")),
        )

    def expires_at(self, *, now: datetime | None = None) -> str:
        current = now or datetime.now(timezone.utc)
        if current.tzinfo is None:
            current = current.replace(tzinfo=timezone.utc)
        expires = current.astimezone(timezone.utc) + timedelta(hours=self.branch_ttl_hours)
        return expires.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def branch_name_for(
    *,
    created_at: datetime,
    git_sha: str,
    scenario: str,
    suffix: str,
    max_len: int = 63,
) -> str:
    if created_at.tzinfo is None:
        created_at = created_at.replace(tzinfo=timezone.utc)
    date = created_at.astimezone(timezone.utc).strftime("%Y%m%d")
    sha = _slug(git_sha)[:7] or "unknown"
    scenario_slug = _slug(scenario)
    suffix_slug = _slug(suffix)
    fixed = f"e2e-{date}-{sha}-"
    tail = f"-{suffix_slug}" if suffix_slug else ""
    room = max_len - len(fixed) - len(tail)
    if room < 1:
        room = 1
    scenario_part = scenario_slug[:room].strip("-") or "scenario"
    name = f"{fixed}{scenario_part}{tail}".strip("-")
    return name[:max_len].strip("-")


def _slug(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", value.lower()).strip("-")
    return re.sub(r"-+", "-", slug)
