from __future__ import annotations

from collections.abc import Iterable, Mapping, Sequence
from dataclasses import asdict, is_dataclass
from decimal import Decimal
from datetime import date, datetime
from typing import Any


def glimpse(value: Any, *, max_items: int = 3) -> dict[str, Any]:
    """Build a small schema-neutral glimpse from customer-provided data.

    Domain-specific fields belong in customer code. For example, a customer can
    pass a hand-built summary dict from ``ReferenceSpec.glimpse`` and this helper
    will only normalize it to JSON-compatible values.
    """

    if max_items < 0:
        raise ValueError("max_items must be non-negative")

    normalized = _to_jsonable(value)
    if isinstance(normalized, Mapping):
        return dict(normalized)
    if _is_sequence_like(normalized):
        items = list(normalized)
        return {
            "count": len(items),
            "sample": items[:max_items],
        }
    return {"value": normalized}


def _to_jsonable(value: Any) -> Any:
    if hasattr(value, "model_dump") and callable(value.model_dump):
        return _to_jsonable(value.model_dump(mode="json"))
    if is_dataclass(value) and not isinstance(value, type):
        return _to_jsonable(asdict(value))
    if isinstance(value, Mapping):
        return {str(key): _to_jsonable(item) for key, item in value.items()}
    if _is_sequence_like(value):
        return [_to_jsonable(item) for item in value]
    if isinstance(value, Decimal):
        return format(value, "f")
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    return value


def _is_sequence_like(value: Any) -> bool:
    return isinstance(value, (Sequence, Iterable)) and not isinstance(
        value,
        (str, bytes, bytearray, Mapping),
    )
