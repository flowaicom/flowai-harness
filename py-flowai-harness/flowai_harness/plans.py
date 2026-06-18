from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator
from pydantic.alias_generators import to_camel

from flowai_harness._schema import normalize_schema

PlanStatus = Literal["draft", "approved", "executing", "executed", "failed"]
_ALLOWED_STATUSES = {"draft", "approved", "executing", "executed", "failed"}


class PlanDisplayAlias(BaseModel):
    """Display alias for one fixed Flow AI plan lifecycle status."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    status: PlanStatus
    alias: str


class PlanSpec(BaseModel):
    """Plan declaration compiled by `flowai-runtime` to Plan<HarnessAction>."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    name: str
    schema_: dict[str, Any] = Field(alias="schema", serialization_alias="schema")
    display_aliases: list[PlanDisplayAlias] = Field(default_factory=list)

    @field_validator("schema_", mode="before")
    @classmethod
    def _normalize_schema(cls, value: Any) -> dict[str, Any]:
        return normalize_schema(value)

    @field_validator("display_aliases", mode="before")
    @classmethod
    def _normalize_display_aliases(cls, value: Any) -> list[Any]:
        if value is None:
            return []
        if isinstance(value, Mapping):
            return [_alias_from_pair(status, alias) for status, alias in value.items()]
        return [_alias_from_value(item) for item in value]


def define_plan(
    name: str,
    schema: Any,
    display_aliases: Mapping[str, str] | Iterable[PlanDisplayAlias | Mapping[str, str]] = (),
) -> PlanSpec:
    """Create a validated Flow AI plan spec.

    Args:
        name: Unique plan name within the runtime spec.
        schema: Action schema for plan items: a JSON Schema mapping, a
            Pydantic model class, a simple type map, or any type hint
            Pydantic can export.
        display_aliases: Display names for fixed plan lifecycle statuses,
            either a ``{status: alias}`` mapping or an iterable of
            ``PlanDisplayAlias`` / mappings. Statuses are limited to
            ``draft``, ``approved``, ``executing``, ``executed``, and
            ``failed``.

    Returns:
        A frozen, validated ``PlanSpec``.

    Raises:
        ValueError: If a display alias names an unsupported status.
        TypeError: If the schema input cannot be normalized to JSON Schema.
    """

    return PlanSpec(name=name, schema=schema, display_aliases=display_aliases)


def _alias_from_pair(status: Any, alias: Any) -> dict[str, Any]:
    _validate_status(status)
    return {"status": status, "alias": alias}


def _alias_from_value(item: PlanDisplayAlias | Mapping[str, Any]) -> Any:
    if isinstance(item, PlanDisplayAlias):
        return item
    status = item.get("status") if isinstance(item, Mapping) else None
    _validate_status(status)
    return item


def _validate_status(status: Any) -> None:
    if status not in _ALLOWED_STATUSES:
        raise ValueError(
            f"unsupported plan display status `{status}`; expected one of {sorted(_ALLOWED_STATUSES)}"
        )
