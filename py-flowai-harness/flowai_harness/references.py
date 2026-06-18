from __future__ import annotations

from collections.abc import Callable
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, field_validator
from pydantic.alias_generators import to_camel

from flowai_harness._schema import normalize_schema


class ReferenceSpec(BaseModel):
    """Named typed memory pointer declaration.

    ``glimpse`` is a Python-only customer callback. It is excluded from the
    Rust wire spec; hosts call it before storing a reference and pass the
    resulting JSON glimpse to the runtime.
    """

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
        arbitrary_types_allowed=True,
    )

    name: str
    schema_: dict[str, Any] = Field(alias="schema", serialization_alias="schema")
    ttl_ms: int | None = Field(default=None, ge=0)
    glimpse: Callable[[Any], Any] | None = Field(default=None, exclude=True)

    @field_validator("schema_", mode="before")
    @classmethod
    def _normalize_schema(cls, value: Any) -> dict[str, Any]:
        return normalize_schema(value)


def define_reference(
    name: str,
    schema: Any,
    ttl_ms: int | None = None,
    glimpse: Callable[[Any], Any] | None = None,
) -> ReferenceSpec:
    """Create a validated Flow AI reference spec with optional customer glimpse code.

    Args:
        name: Reference kind name used when creating and resolving
            references.
        schema: Schema of the referenced value: a JSON Schema mapping, a
            Pydantic model class, a simple type map, or any type hint
            Pydantic can export.
        ttl_ms: Optional time-to-live in milliseconds for stored values;
            must be non-negative when set.
        glimpse: Optional Python callback that derives the stored glimpse
            from the full value. It runs once before storing and is excluded
            from the Rust wire spec.

    Returns:
        A frozen, validated ``ReferenceSpec``.

    Raises:
        TypeError: If the schema input cannot be normalized to JSON Schema.
        pydantic.ValidationError: If ``ttl_ms`` is negative.
    """

    return ReferenceSpec(name=name, schema=schema, ttl_ms=ttl_ms, glimpse=glimpse)
