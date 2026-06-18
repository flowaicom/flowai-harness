#!/usr/bin/env python3
"""Validate the lightweight Harness Studio v1 contract fixtures.

This intentionally uses only the Python standard library. It is not a full
OpenAPI validator; it catches fixture drift before implementation work starts.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / "fixtures"
OPENAPI = ROOT / "openapi.yaml"
VERSION = "harness-studio/v1"

REQUIRED_PATHS = {
    "/api/status",
    "/api/workspaces",
    "/api/workspaces/{workspace_key}/runtime",
    "/api/workspaces/{workspace_key}/capabilities",
    "/api/workspaces/{workspace_key}/agents",
    "/api/workspaces/{workspace_key}/agents/{agent_id}/stream",
    "/api/workspaces/{workspace_key}/threads",
    "/api/workspaces/{workspace_key}/threads/{thread_id}",
    "/api/workspaces/{workspace_key}/threads/{thread_id}/messages",
    "/api/workspaces/{workspace_key}/approvals/{approval_id}/respond",
    "/api/runtime",
    "/api/agents",
    "/api/agents/{agent_id}/stream",
}

REQUIRED_SSE_KINDS = {
    "message.delta",
    "tool.call.started",
    "tool.call.completed",
    "sub_agent.call.started",
    "approval.required",
    "profile.progress",
    "eval.case.completed",
    "run.completed",
}

SECRET_KEYS = {
    "apiKey",
    "api_key",
    "password",
    "databaseUrl",
    "database_url",
    "token",
    "secret",
}


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def walk_keys(value: Any) -> set[str]:
    if isinstance(value, dict):
        keys = set(value)
        for child in value.values():
            keys.update(walk_keys(child))
        return keys
    if isinstance(value, list):
        keys: set[str] = set()
        for child in value:
            keys.update(walk_keys(child))
        return keys
    return set()


def main() -> None:
    openapi_text = OPENAPI.read_text(encoding="utf-8")
    missing_paths = sorted(path for path in REQUIRED_PATHS if path not in openapi_text)
    if missing_paths:
        raise SystemExit(f"missing OpenAPI paths: {missing_paths}")
    if "version: harness-studio/v1" not in openapi_text:
        raise SystemExit("OpenAPI version is not harness-studio/v1")
    if "ErrorResponse:" not in openapi_text:
        raise SystemExit("OpenAPI is missing ErrorResponse schema")

    status = load_json(FIXTURES / "status-response.json")
    if status.get("studioApiVersion") != VERSION:
        raise SystemExit("status fixture has wrong studioApiVersion")
    if VERSION not in status.get("supportedVersions", []):
        raise SystemExit("status fixture does not advertise harness-studio/v1")

    all_json_paths = sorted(FIXTURES.rglob("*.json"))
    for path in all_json_paths:
        value = load_json(path)
        leaked_keys = walk_keys(value).intersection(SECRET_KEYS)
        if leaked_keys:
            raise SystemExit(f"{path.relative_to(ROOT)} contains secret-shaped keys: {sorted(leaked_keys)}")

    sse_kinds: set[str] = set()
    for path in sorted((FIXTURES / "sse").glob("*.json")):
        event = load_json(path)
        if event.get("schemaVersion") != VERSION:
            raise SystemExit(f"{path.name} has wrong schemaVersion")
        kind = event.get("kind")
        if not isinstance(kind, str):
            raise SystemExit(f"{path.name} is missing string kind")
        sse_kinds.add(kind)
        if not isinstance(event.get("seq"), int):
            raise SystemExit(f"{path.name} is missing integer seq")
        if "payload" not in event:
            raise SystemExit(f"{path.name} is missing payload")

    missing_kinds = sorted(REQUIRED_SSE_KINDS - sse_kinds)
    if missing_kinds:
        raise SystemExit(f"missing SSE fixture kinds: {missing_kinds}")

    print("harness-studio/v1 contract fixtures ok")


if __name__ == "__main__":
    main()
