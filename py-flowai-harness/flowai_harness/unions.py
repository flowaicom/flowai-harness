from __future__ import annotations

from functools import reduce
from operator import or_
from typing import Annotated, Any, Literal, get_args, get_origin

from pydantic import BaseModel, Field, create_model


def TaggedUnion(
    *models: type[BaseModel],
    discriminator: str = "kind",
) -> Any:
    """Create a Pydantic discriminated union over `discriminator`.

    Models may use either Pydantic's native `Literal[...]` discriminator field
    or the public Harness shorthand `kind: str = "some_kind"`.

    Python type checkers cannot infer a precise alias from a runtime function
    call, so the return annotation is intentionally `Any`. Runtime validation
    and JSON schema generation still use Pydantic's discriminated union.
    """

    if len(models) < 2:
        raise ValueError("TaggedUnion requires at least two Pydantic models")

    values: dict[str, type[BaseModel]] = {}
    variants: list[type[BaseModel]] = []
    for model in models:
        if not isinstance(model, type) or not issubclass(model, BaseModel):
            raise TypeError("TaggedUnion variants must be Pydantic BaseModel classes")
        value = _discriminator_value(model, discriminator)
        if value in values:
            raise ValueError(
                f"duplicate discriminator value `{value}` for "
                f"{values[value].__name__} and {model.__name__}"
            )
        values[value] = model
        variants.append(_literal_variant(model, discriminator, value))

    union_type = reduce(or_, variants)
    return Annotated[union_type, Field(discriminator=discriminator)]


def _discriminator_value(model: type[BaseModel], discriminator: str) -> str:
    field = model.model_fields.get(discriminator)
    if field is None:
        raise ValueError(f"{model.__name__} is missing discriminator field `{discriminator}`")

    literal_values = _literal_values(field.annotation)
    if literal_values:
        if (
            len(literal_values) != 1
            or not isinstance(literal_values[0], str)
            or literal_values[0] == ""
        ):
            raise ValueError(
                f"{model.__name__}.{discriminator} must be a single non-empty string Literal"
            )
        return literal_values[0]

    if isinstance(field.default, str) and field.default != "":
        return field.default

    raise ValueError(
        f"{model.__name__}.{discriminator} must be a string Literal or have a "
        "non-empty string default"
    )


def _literal_values(annotation: Any) -> tuple[Any, ...]:
    origin = get_origin(annotation)
    if origin is Literal:
        return get_args(annotation)
    if origin is Annotated:
        return _literal_values(get_args(annotation)[0])
    return ()


def _literal_variant(
    model: type[BaseModel],
    discriminator: str,
    value: str,
) -> type[BaseModel]:
    if _literal_values(model.model_fields[discriminator].annotation) == (value,):
        return model

    literal_annotation = Literal[value]  # type: ignore[valid-type]
    variant = create_model(
        model.__name__,
        __base__=model,
        __module__=model.__module__,
        **{discriminator: (literal_annotation, ...)},
    )
    variant.__name__ = model.__name__
    variant.__qualname__ = model.__qualname__
    return variant
