from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from pydantic import BaseModel, ConfigDict, Field
from pydantic.alias_generators import to_camel


class TenantIdentity(BaseModel):
    """Runtime tenant identity aligned with `agent_fw_core::TenantId`."""

    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
    )

    resource_id: str = Field(
        min_length=1,
        description="Stable tenant resource identifier; scopes runtime storage and data access.",
    )
    version: str = Field(
        min_length=1,
        description="Tenant configuration version label, e.g. 'v1'.",
    )


def define_tenant(
    tenant: str | Mapping[str, Any] | TenantIdentity | None = None,
    /,
    version: str | None = None,
    *,
    resource_id: str | None = None,
    **kwargs: Any,
) -> TenantIdentity:
    """Create a validated Flow AI tenant identity.

    Accepts ``define_tenant("acme", "v1")``, a mapping, an existing
    ``TenantIdentity``, or keyword arguments.

    Args:
        tenant: Positional resource id string, mapping of tenant fields, or
            an existing ``TenantIdentity`` to copy from.
        version: Tenant configuration version label, e.g. ``"v1"``.
        resource_id: Keyword override for the tenant resource id.
        **kwargs: Additional fields merged into the validated payload.

    Returns:
        A frozen, validated ``TenantIdentity``.

    Raises:
        pydantic.ValidationError: If ``resource_id`` or ``version`` is
            missing or empty.
    """

    if tenant is None:
        data: dict[str, Any] = {}
    elif isinstance(tenant, TenantIdentity):
        data = tenant.model_dump(by_alias=True, mode="json")
    elif isinstance(tenant, Mapping):
        data = dict(tenant)
    else:
        data = {"resource_id": tenant}

    if resource_id is not None:
        data["resource_id"] = resource_id
    if version is not None:
        data["version"] = version
    data.update(kwargs)
    if "resource_id" in data:
        data["resourceId"] = data.pop("resource_id")
    return TenantIdentity.model_validate(data)
