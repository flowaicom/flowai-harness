from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field
from pydantic.alias_generators import to_camel

from flowai_harness import TaggedUnion, define_plan, define_reference


class DomainModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        extra="forbid",
        populate_by_name=True,
    )

    def model_dump(self, *args, **kwargs):
        kwargs.setdefault("by_alias", True)
        return super().model_dump(*args, **kwargs)


class ProductSetPayload(DomainModel):
    product_ids: list[str]
    sql: str = Field(min_length=1)
    params: list[str | int | float | bool | None] = Field(default_factory=list)
    reason: str = Field(min_length=1)
    selection_summary: str | None = None
    sample: list[dict[str, Any]] = Field(default_factory=list)


class ProductSetRef(DomainModel):
    kind: Literal["InventoryProductSet"] = "InventoryProductSet"
    id: str = Field(min_length=1)


def _product_set_glimpse(value: ProductSetPayload) -> dict[str, Any]:
    return {
        "productCount": len(value.product_ids),
        "previewProductIds": value.product_ids[:3],
        "selectionSummary": value.selection_summary,
        "sample": value.sample[:3],
    }


ProductSet = define_reference(
    name="InventoryProductSet",
    schema=ProductSetPayload,
    ttl_ms=60 * 60 * 1000,
    glimpse=_product_set_glimpse,
)


class InventoryActionBase(DomainModel):
    name: str = Field(min_length=1)
    reason: str = Field(min_length=1)
    references: list[ProductSetRef] = Field(min_length=1, max_length=1)


class ReorderProductsAction(InventoryActionBase):
    kind: Literal["reorder_products"] = "reorder_products"
    quantity: int = Field(gt=0)


class HoldInventoryAction(InventoryActionBase):
    kind: Literal["hold_inventory"] = "hold_inventory"
    holdback_units: int = Field(ge=0)


InventoryScenarioAction = TaggedUnion(
    ReorderProductsAction,
    HoldInventoryAction,
)


class InventoryScenarioPlan(DomainModel):
    objective: str = Field(min_length=1)
    actions: list[InventoryScenarioAction] = Field(min_length=1) # type: ignore
    assumptions: list[str] = Field(default_factory=list)


inventory_scenario_plan = define_plan(
    "InventoryScenarioPlan",
    InventoryScenarioPlan,
    display_aliases={
        "draft": "Draft inventory plan",
        "approved": "Approved inventory plan",
        "executing": "Applying inventory actions",
        "executed": "Inventory plan applied",
        "failed": "Inventory plan failed",
    },
)
