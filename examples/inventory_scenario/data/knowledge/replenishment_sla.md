# Replenishment SLA

## Service-Level Objective

The replenishment process is designed to prevent avoidable stockouts while
keeping excess inventory within normal planning tolerances. Analysts should use
the active scenario forecast and the current inventory baseline to identify
products that require replenishment review.

This document describes when a replenishment request should be created, how it
should be prioritized, and what evidence should accompany the request.

## Priority Levels

High-priority replenishment should be reviewed the same business day. Use high
priority when any of the following are true:

- current `on_hand` is below `safety_stock`,
- projected ending inventory is negative,
- the product is below `reorder_point` and projected units exceed half of
  current `on_hand`,
- the product is tied to an active promotion or customer commitment,
- multiple markets show the same product below safety stock.

Medium-priority replenishment should be reviewed within two business days. Use
medium priority when current inventory is above `safety_stock` but below
`reorder_point`, or when projected demand would move the product below
`safety_stock` without creating a negative ending inventory.

Low-priority replenishment can be queued for the next planning cycle. Use low
priority when the product is on the watch list but still has enough inventory to
cover the active period.

## Quantity Guidance

Recommended replenishment quantity should normally restore projected ending
inventory to at least `reorder_point`.

```text
minimum_replenishment =
  max(0, reorder_point - (on_hand - projected_units))
```

Round up only when the business context supports it, such as case-pack minimums,
pallet constraints, supplier minimum order quantities, or pre-season inventory
builds. If those constraints are unknown, present the calculated minimum and
state that operational rounding may be required.

Avoid recommending quantities that create more than one additional period of
coverage unless the product is seasonal, supply constrained, or explicitly part
of a forward-buy plan.

## Evidence Required

Each replenishment request should include:

- `product_id` and product name,
- location and channel where the risk appears,
- current `on_hand`, `safety_stock`, and `reorder_point`,
- scenario name and projected units,
- calculated projected ending inventory,
- recommended quantity or review range,
- priority and rationale.

If the recommendation spans multiple products, include aggregate counts by
priority and list the highest-risk products first.

## Escalation Conditions

Escalate to a planning lead when the calculated replenishment quantity is more
than three times projected active-period demand, when the same product appears
with conflicting signals across channels, or when the forecast implies a major
promotion that is not visible in the planning calendar.
