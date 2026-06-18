from __future__ import annotations

from pathlib import Path

from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from inventory_scenario.support.data_environment import default_data_root
from inventory_scenario.support.mock_platform.schemas import (
    InventoryPreviewRequest,
    PromotionHoldbackRequest,
    ReplenishmentRequest,
    ResolveProductsRequest,
    SafetyStockRequest,
)
from inventory_scenario.support.mock_platform.store import PlatformStore


def create_default_app() -> FastAPI:
    return create_app(default_data_root() / "platform.db")


def create_app(platform_db: Path) -> FastAPI:
    store = PlatformStore(platform_db)
    app = FastAPI(title="Inventory Scenario Platform")
    static_dir = Path(__file__).resolve().parent / "static"
    app.mount("/static", StaticFiles(directory=static_dir), name="static")

    @app.get("/")
    def index():
        return FileResponse(static_dir / "index.html")

    @app.get("/health")
    def health():
        return {"ok": True}

    @app.get("/state/summary")
    def summary():
        return store.summary()

    @app.get("/products")
    def products(limit: int = 50, offset: int = 0):
        return store.list_products(limit=limit, offset=offset)

    @app.post("/products/resolve")
    def resolve_products(request: ResolveProductsRequest):
        return store.resolve_products(request.product_ids)

    @app.post("/inventory/preview")
    def preview(request: InventoryPreviewRequest):
        return store.preview(request)

    @app.post("/actions/replenishment")
    def replenishment(request: ReplenishmentRequest):
        return _apply(lambda: store.apply_replenishment(request))

    @app.post("/actions/safety-stock")
    def safety_stock(request: SafetyStockRequest):
        return _apply(lambda: store.apply_safety_stock(request))

    @app.post("/actions/holdback")
    def holdback(request: PromotionHoldbackRequest):
        return _apply(lambda: store.apply_holdback(request))

    @app.get("/actions")
    def actions():
        return store.list_actions()

    @app.get("/actions/{action_id}")
    def action(action_id: str):
        result = store.get_action(action_id)
        if result is None:
            raise HTTPException(status_code=404, detail="action not found")
        return result

    return app


def _apply(callback):
    try:
        return callback()
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
