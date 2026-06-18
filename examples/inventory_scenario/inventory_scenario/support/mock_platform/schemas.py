from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


Priority = Literal["low", "medium", "high"]


class ApiModel(BaseModel):
    model_config = ConfigDict(populate_by_name=True)


class ProductState(ApiModel):
    product_id: str
    product_name: str
    brand_name: str
    segment_name: str
    region: str
    channel_name: str
    on_hand: int
    safety_stock: int
    reorder_point: int
    holdback_units: int = 0


class ResolveProductsRequest(ApiModel):
    product_ids: list[str] = Field(min_length=1)


class InventoryPreviewRequest(ApiModel):
    product_ids: list[str] = Field(min_length=1)
    quantity_delta: int = 0
    safety_stock: int | None = None
    holdback_delta: int = 0


class ReplenishmentRequest(ApiModel):
    product_ids: list[str] = Field(min_length=1)
    quantity: int = Field(gt=0)
    reason: str = Field(min_length=1)
    priority: Priority = "medium"
    idempotency_key: str | None = None


class SafetyStockRequest(ApiModel):
    product_ids: list[str] = Field(min_length=1)
    safety_stock: int = Field(ge=0)
    reason: str = Field(min_length=1)
    priority: Priority = "medium"
    idempotency_key: str | None = None


class PromotionHoldbackRequest(ApiModel):
    product_ids: list[str] = Field(min_length=1)
    holdback_units: int = Field(ge=0)
    reason: str = Field(min_length=1)
    priority: Priority = "medium"
    idempotency_key: str | None = None


class ActionResponse(ApiModel):
    action_id: str
    action_type: str
    created: bool
    product_count: int
    idempotency_key: str | None = None
    status: str = "applied"
