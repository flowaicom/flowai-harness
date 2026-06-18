from __future__ import annotations

from typing import Any


def build_action_dispatcher(platform: Any):
    async def dispatch_actions(
        actions: list[dict[str, Any]],
        ctx: dict[str, Any],
    ) -> dict[str, Any]:
        results = []
        for action in actions:
            product_ids = _product_ids_for_action(action, ctx)
            payload = dict(action.get("payload") or {})
            kind = action.get("kind")
            if kind == "reorder_products":
                result = await _maybe_await(
                    platform.replenishment(
                        {
                            "product_ids": product_ids,
                            "quantity": payload["quantity"],
                            "reason": payload["reason"],
                        }
                    )
                )
            elif kind == "hold_inventory":
                result = await _maybe_await(
                    platform.holdback(
                        {
                            "product_ids": product_ids,
                            "holdback_units": payload["holdbackUnits"],
                            "reason": payload["reason"],
                        }
                    )
                )
            else:
                raise ValueError(f"unsupported inventory action kind: {kind}")
            results.append(
                {
                    "kind": kind,
                    "name": payload.get("name"),
                    "productCount": len(product_ids),
                    "result": result,
                }
            )
        return {
            "entitiesAffected": len(results),
            "summary": f"Applied {len(results)} inventory action(s).",
            "details": {"actions": results},
        }

    return dispatch_actions


def _product_ids_for_action(action: dict[str, Any], ctx: dict[str, Any]) -> list[str]:
    references = action.get("references") or []
    product_refs = [
        ref
        for ref in references
        if isinstance(ref, dict) and ref.get("kind") == "InventoryProductSet"
    ]
    if len(product_refs) != 1:
        raise ValueError(
            "inventory actions must include exactly one InventoryProductSet reference"
        )
    ref_id = product_refs[0].get("id")
    if not isinstance(ref_id, str) or not ref_id:
        raise ValueError("InventoryProductSet reference id is required")
    resolved = ((ctx.get("resolved_refs") or {}).get("InventoryProductSet") or {}).get(
        ref_id
    )
    if not isinstance(resolved, dict):
        raise ValueError(f"missing hydrated InventoryProductSet reference: {ref_id}")
    product_ids = resolved.get("productIds") or resolved.get("product_ids")
    if not isinstance(product_ids, list):
        raise ValueError(f"InventoryProductSet {ref_id} did not hydrate product ids")
    return [str(product_id) for product_id in product_ids]


async def _maybe_await(value):
    if hasattr(value, "__await__"):
        return await value
    return value
