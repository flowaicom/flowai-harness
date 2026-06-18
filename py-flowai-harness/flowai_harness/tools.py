from __future__ import annotations

from collections.abc import Callable, Mapping
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from pydantic.alias_generators import to_camel

from flowai_harness._schema import normalize_schema


class ToolSpec(BaseModel):
    """Language-neutral tool specification plus optional Python binding."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
        arbitrary_types_allowed=True,
    )

    name: str
    description: str = ""
    input_schema: dict[str, Any]
    approval: dict[str, Any] = Field(default_factory=lambda: {"kind": "never"})
    output_schema: dict[str, Any] | None = None
    binding_id: str | None = None
    handler: Callable[..., Any] | None = Field(default=None, exclude=True)
    approval_handler: Callable[..., bool] | None = Field(default=None, exclude=True)

    @model_validator(mode="before")
    @classmethod
    def _normalize_approval(cls, value: Any) -> Any:
        if not isinstance(value, dict):
            return value
        data = dict(value)
        approval = data.get("approval", "never")
        name = data.get("name")
        binding_id = data.get("binding_id", data.get("bindingId"))
        if callable(approval) and data.get("approval_handler") is None:
            data["approval_handler"] = approval
        data["approval"] = _approval_to_wire(approval, name=name, binding_id=binding_id)
        return data

    @field_validator("input_schema", "output_schema", mode="before")
    @classmethod
    def _normalize_schema(cls, value: Any) -> dict[str, Any] | None:
        if value is None:
            return None
        return normalize_schema(value)

    def bind(self, handler: Callable[..., Any]) -> ToolSpec:
        """Return a copy of this spec with a Python handler attached."""

        return type(self)(
            name=self.name,
            description=self.description,
            input_schema=self.input_schema,
            approval=self.approval,
            output_schema=self.output_schema,
            binding_id=self.binding_id,
            handler=handler,
            approval_handler=self.approval_handler,
        )

    def __call__(self, handler: Callable[..., Any]) -> ToolSpec:
        return self.bind(handler)


def define_tool(
    name: str,
    input_schema: Any,
    description: str = "",
    approval: str | Mapping[str, Any] | Callable[..., bool] = "never",
    output_schema: Any | None = None,
    binding_id: str | None = None,
) -> ToolSpec:
    """Create a validated Flow AI tool spec.

    The returned value is callable, so it can be used as a decorator:

    ``@define_tool(name="search", input_schema={"query": str})``.

    Args:
        name: Tool name presented to the model.
        input_schema: Tool input schema: a JSON Schema mapping, a Pydantic
            model class, a simple type map such as ``{"query": str}``, or
            any type hint Pydantic can export.
        description: Tool description presented to the model.
        approval: Approval policy: ``"never"`` (default), ``"always"``, a
            mapping ``{"kind": "dynamic", "value": predicate_id}``, or a
            callable predicate. A callable becomes a dynamic policy whose id
            is ``binding_id`` or ``"<name>_approval"``, with the callable
            attached as the approval handler.
        output_schema: Optional output schema, normalized like
            ``input_schema``.
        binding_id: Stable binding key for handler registration; defaults to
            the tool name.

    Returns:
        A frozen ``ToolSpec``. Bind a Python handler with ``.bind(handler)``
        or by calling the spec as a decorator.

    Raises:
        ValueError: If ``approval`` is not a recognized policy, or a
            callable approval is supplied without a tool name.
        TypeError: If a schema input cannot be normalized to JSON Schema.
    """

    return ToolSpec(
        name=name,
        description=description,
        input_schema=input_schema,
        approval=approval,
        output_schema=output_schema,
        binding_id=binding_id,
    )


def _approval_to_wire(
    approval: str | Mapping[str, Any] | Callable[..., bool],
    *,
    name: Any,
    binding_id: Any,
) -> dict[str, Any]:
    if callable(approval):
        if not isinstance(name, str) or name == "":
            raise ValueError("approval callable requires a non-empty tool name")
        dynamic_id = binding_id if binding_id is not None else f"{name}_approval"
        return {"kind": "dynamic", "value": dynamic_id}

    if isinstance(approval, str):
        if approval in {"never", "always"}:
            return {"kind": approval}
        raise ValueError("approval must be 'never', 'always', or a callable dynamic policy")

    if isinstance(approval, Mapping):
        kind = approval.get("kind")
        if kind in {"never", "always"}:
            return {"kind": kind}
        if kind == "dynamic":
            value = approval.get("value")
            if not isinstance(value, str) or value == "":
                raise ValueError("approval dynamic value must be a non-empty binding id")
            return {"kind": "dynamic", "value": value}

    raise ValueError("approval must be 'never', 'always', or a callable dynamic policy")
