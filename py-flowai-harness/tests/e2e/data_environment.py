from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from tests.e2e.config import E2EConfig


def build_data_environment(config: E2EConfig, *, run_id: str) -> dict[str, Any]:
    return {
        "tenantId": config.tenant_id,
        "workspaceId": config.workspace_id,
        "targetDatabase": {
            "kind": "postgres",
            "urlEnv": "FLOWAI_E2E_TARGET_DATABASE_URL",
            "schema": config.target_schema,
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
            "indexPath": str(config.run_root / run_id / "catalog-index"),
            "rebuildOnStart": True,
            "writeThrough": True,
        },
    }


def write_data_environment(payload: dict[str, Any], path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return path
