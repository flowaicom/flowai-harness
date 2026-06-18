from __future__ import annotations

import asyncio
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from inventory_scenario.support.mock_platform.schemas import (
    PromotionHoldbackRequest,
    ReplenishmentRequest,
    SafetyStockRequest,
)
from inventory_scenario.support.mock_platform.store import PlatformStore


class HttpPlatformClient:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")

    async def replenishment(self, payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(
            self._json_request,
            "/actions/replenishment",
            payload,
        )

    async def safety_stock(self, payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(
            self._json_request,
            "/actions/safety-stock",
            payload,
        )

    async def holdback(self, payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(
            self._json_request,
            "/actions/holdback",
            payload,
        )

    async def list_products(
        self,
        *,
        limit: int = 5000,
        offset: int = 0,
    ) -> dict[str, Any]:
        return await asyncio.to_thread(
            self._json_get,
            "/products",
            {"limit": limit, "offset": offset},
        )

    def _json_get(self, path: str, query: dict[str, Any]) -> dict[str, Any]:
        encoded = urllib.parse.urlencode(query)
        request = urllib.request.Request(
            f"{self.base_url}{path}?{encoded}",
            headers={"accept": "application/json"},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"platform request failed: {exc.code} {body}") from exc

    def _json_request(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        data = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=data,
            headers={"content-type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"platform request failed: {exc.code} {body}") from exc


class LocalPlatformClient:
    def __init__(self, platform_db: Path):
        self.store = PlatformStore(platform_db)

    async def replenishment(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = ReplenishmentRequest.model_validate(payload)
        return await asyncio.to_thread(self.store.apply_replenishment, request)

    async def safety_stock(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = SafetyStockRequest.model_validate(payload)
        return await asyncio.to_thread(self.store.apply_safety_stock, request)

    async def holdback(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = PromotionHoldbackRequest.model_validate(payload)
        return await asyncio.to_thread(self.store.apply_holdback, request)

    async def list_products(
        self,
        *,
        limit: int = 5000,
        offset: int = 0,
    ) -> dict[str, Any]:
        return await asyncio.to_thread(
            self.store.list_products,
            limit=limit,
            offset=offset,
        )


def default_platform_client(data_environment: dict[str, Any]) -> Any:
    base_url = os.environ.get("INVENTORY_SCENARIO_PLATFORM_URL")
    if base_url:
        return HttpPlatformClient(base_url)
    return LocalPlatformClient(_platform_db_from_data_environment(data_environment))


def _platform_db_from_data_environment(data_environment: dict[str, Any]) -> Path:
    target_database = data_environment.get("target_database") or data_environment.get(
        "targetDatabase"
    )
    if not isinstance(target_database, dict):
        raise ValueError("data_environment target_database is required for local platform")
    url = target_database.get("url")
    if not isinstance(url, str) or not url.startswith("sqlite:"):
        raise ValueError("local platform requires a sqlite target_database url")
    return Path(url.removeprefix("sqlite:")).parent / "platform.db"
