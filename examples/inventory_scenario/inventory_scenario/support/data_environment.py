from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


TENANT_ID = "inventory_scenario"
WORKSPACE_ID = "default"


def default_data_root() -> Path:
    return Path(".data") / "inventory_scenario"


def sqlite_url(path: Path) -> str:
    return f"sqlite:{path}"


def build_data_environment(data_root: Path | None = None) -> dict[str, Any]:
    root = data_root or default_data_root()
    return {
        "tenant_id": TENANT_ID,
        "workspace_id": WORKSPACE_ID,
        "target_database": {
            "kind": "sqlite",
            "url": sqlite_url(root / "target.db"),
        },
        "catalog": {
            "kind": "sqlite",
            "url": sqlite_url(root / "catalog.db"),
            "ensure_schema": True,
        },
        "kv": {
            "kind": "sqlite",
            "url": sqlite_url(root / "kv.db"),
            "ensure_schema": True,
        },
        "catalog_search": {
            "index_path": str(root / "catalog-index"),
            "rebuild_on_start": False,
            "write_through": False,
        },
    }


def write_data_environment_file(
    out_path: Path,
    *,
    data_root: Path | None = None,
) -> dict[str, Any]:
    payload = build_data_environment(data_root)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return payload


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Write the inventory scenario Flow AI data-environment descriptor.",
    )
    parser.add_argument("--data-root", type=Path, default=default_data_root())
    parser.add_argument(
        "--out",
        type=Path,
        default=default_data_root() / "data-environment.json",
    )
    args = parser.parse_args(argv)

    payload = write_data_environment_file(args.out, data_root=args.data_root)
    print(
        json.dumps(
            {
                "data_environment": str(args.out),
                "target_db": payload["target_database"]["url"],
                "catalog_db": payload["catalog"]["url"],
                "kv_db": payload["kv"]["url"],
                "catalog_index": payload["catalog_search"]["index_path"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
