import json
from typing import Literal

import pytest
from pydantic import BaseModel, TypeAdapter, ValidationError

from flowai_harness import TaggedUnion, normalize_schema


class PriceChange(BaseModel):
    kind: str = "price_change"
    product_id: str
    new_price: float


class PromotionLaunch(BaseModel):
    kind: str = "promotion_launch"
    product_ids: list[str]
    discount_pct: float


def test_tagged_union_validates_payload_with_shorthand_kind_defaults():
    ScenarioAction = TaggedUnion(PriceChange, PromotionLaunch)

    action = TypeAdapter(ScenarioAction).validate_python(
        {"kind": "price_change", "product_id": "p-1", "new_price": 9.99}
    )

    assert isinstance(action, PriceChange)
    assert action.kind == "price_change"
    assert action.product_id == "p-1"


def test_tagged_union_surfaces_clear_error_for_missing_and_unknown_discriminator():
    ScenarioAction = TaggedUnion(PriceChange, PromotionLaunch)
    adapter = TypeAdapter(ScenarioAction)

    with pytest.raises(ValidationError) as missing:
        adapter.validate_python({"product_id": "p-1", "new_price": 9.99})
    assert "kind" in str(missing.value)

    with pytest.raises(ValidationError) as unknown:
        adapter.validate_python({"kind": "unknown", "product_id": "p-1"})
    assert "unknown" in str(unknown.value)
    assert "price_change" in str(unknown.value)


def test_tagged_union_schema_contains_discriminator():
    ScenarioAction = TaggedUnion(PriceChange, PromotionLaunch)

    class ScenarioPlan(BaseModel):
        actions: list[ScenarioAction]

    schema = normalize_schema(ScenarioPlan)

    assert _contains_key(schema, "discriminator")
    assert "Tagged" not in json.dumps(schema)


def test_tagged_union_schema_requires_discriminator_in_each_variant():
    ScenarioAction = TaggedUnion(PriceChange, PromotionLaunch)

    class ScenarioPlan(BaseModel):
        actions: list[ScenarioAction]

    schema = normalize_schema(ScenarioPlan)
    variants = schema["properties"]["actions"]["items"]["oneOf"]

    assert all("kind" in variant["required"] for variant in variants)


def test_tagged_union_round_trips_json_payload():
    ScenarioAction = TaggedUnion(PriceChange, PromotionLaunch)
    adapter = TypeAdapter(ScenarioAction)
    payload = {"kind": "promotion_launch", "product_ids": ["p-1"], "discount_pct": 10.0}

    validated = adapter.validate_json(json.dumps(payload))
    dumped = adapter.dump_json(validated)

    assert json.loads(dumped) == payload


def test_tagged_union_supports_custom_discriminator_name():
    class PriceChangeByType(BaseModel):
        action_type: Literal["price_change"]
        product_id: str

    class PromotionLaunchByType(BaseModel):
        action_type: str = "promotion_launch"
        discount_pct: float

    ScenarioAction = TaggedUnion(
        PriceChangeByType,
        PromotionLaunchByType,
        discriminator="action_type",
    )

    action = TypeAdapter(ScenarioAction).validate_python(
        {"action_type": "promotion_launch", "discount_pct": 5.0}
    )

    assert isinstance(action, PromotionLaunchByType)


def test_tagged_union_rejects_duplicate_discriminator_values():
    class DuplicatePriceChange(BaseModel):
        kind: str = "price_change"
        other: str

    with pytest.raises(ValueError, match="duplicate discriminator"):
        TaggedUnion(PriceChange, DuplicatePriceChange)


def _contains_key(value, key):
    if isinstance(value, dict):
        return key in value or any(_contains_key(item, key) for item in value.values())
    if isinstance(value, list):
        return any(_contains_key(item, key) for item in value)
    return False
