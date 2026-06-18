from __future__ import annotations

from copy import deepcopy
from types import UnionType
from typing import Any, Union, get_args, get_origin

from pydantic import BaseModel, TypeAdapter

_SCHEMA_DISCRIMINATORS = {
    "$schema",
    "properties",
    "$ref",
    "type",
}


def normalize_schema(schema: Any) -> dict[str, Any]:
    """Normalize a Python schema input into JSON Schema.

    Accepted inputs:
    - JSON Schema dictionaries.
    - Pydantic ``BaseModel`` classes.
    - Simple type maps such as ``{"query": str, "limit": int}``.
    - Other Python type hints that Pydantic's ``TypeAdapter`` can export.
    """

    if isinstance(schema, dict):
        if _looks_like_json_schema(schema):
            return _validate_json_schema_dict(schema)
        return _type_map_to_json_schema(schema)

    if isinstance(schema, type) and issubclass(schema, BaseModel):
        return _inline_json_schema_refs(schema.model_json_schema())

    try:
        return _inline_json_schema_refs(TypeAdapter(schema).json_schema())
    except Exception as exc:  # pragma: no cover - defensive path
        raise TypeError(f"unsupported schema input: {schema!r}") from exc


def _looks_like_json_schema(value: dict[str, Any]) -> bool:
    return bool(_SCHEMA_DISCRIMINATORS.intersection(value.keys()))


def _validate_json_schema_dict(schema: dict[str, Any]) -> dict[str, Any]:
    normalized = deepcopy(schema)
    schema_type = normalized.get("type")
    if schema_type is not None and not isinstance(schema_type, (str, list)):
        raise ValueError("schema.type must be a string or list of strings")
    properties = normalized.get("properties")
    if properties is not None and not isinstance(properties, dict):
        raise ValueError("schema.properties must be an object")
    return _inline_json_schema_refs(normalized)


def _type_map_to_json_schema(type_map: dict[str, Any]) -> dict[str, Any]:
    properties: dict[str, Any] = {}
    required: list[str] = []
    for name, type_hint in type_map.items():
        if not isinstance(name, str):
            raise TypeError("schema type-map keys must be strings")
        properties[name] = _type_hint_to_schema(type_hint)
        required.append(name)
    schema: dict[str, Any] = {"type": "object", "properties": properties}
    if required:
        schema["required"] = required
    return schema


def _type_hint_to_schema(type_hint: Any) -> dict[str, Any]:
    if isinstance(type_hint, dict):
        return normalize_schema(type_hint)

    origin = get_origin(type_hint)
    args = get_args(type_hint)

    if type_hint is str:
        return {"type": "string"}
    if type_hint is int:
        return {"type": "integer"}
    if type_hint is float:
        return {"type": "number"}
    if type_hint is bool:
        return {"type": "boolean"}
    if type_hint is dict or origin is dict:
        return {"type": "object"}
    if type_hint is list or origin is list:
        item_schema = _type_hint_to_schema(args[0]) if args else {}
        return {"type": "array", "items": item_schema}
    if origin in (Union, UnionType):
        non_none = [arg for arg in args if arg is not type(None)]
        if len(non_none) == 1:
            schema = _type_hint_to_schema(non_none[0])
            return {"anyOf": [schema, {"type": "null"}]}

    try:
        return _inline_json_schema_refs(TypeAdapter(type_hint).json_schema())
    except Exception:
        return {"type": "object"}


def _inline_json_schema_refs(schema: dict[str, Any]) -> dict[str, Any]:
    normalized = deepcopy(schema)
    defs = normalized.pop("$defs", {})
    unresolved_recursive_ref = False

    def resolve(value: Any, stack: set[str]) -> Any:
        nonlocal unresolved_recursive_ref
        if isinstance(value, dict):
            ref = value.get("$ref")
            if isinstance(ref, str) and ref.startswith("#/$defs/"):
                name = ref.rsplit("/", 1)[-1]
                if name in stack:
                    unresolved_recursive_ref = True
                    return {"$ref": ref}
                if name in defs:
                    return resolve(defs[name], stack | {name})
            return {key: resolve(item, stack) for key, item in value.items()}
        if isinstance(value, list):
            return [resolve(item, stack) for item in value]
        return value

    resolved = resolve(normalized, set())
    if unresolved_recursive_ref and defs:
        resolved["$defs"] = defs
    return resolved
