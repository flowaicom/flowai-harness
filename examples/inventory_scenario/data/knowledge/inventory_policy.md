# Inventory Policy

## Purpose

This policy defines how inventory teams classify stock risk and prepare
recommendations for replenishment, transfer, safety stock changes, and promotion
protection. It is intended for analysts working with the inventory scenario
dataset and for operators reviewing recommended changes before they are entered
into downstream planning systems.

The policy prioritizes customer service and continuity of supply. Margin,
storage capacity, and working capital are important constraints, but they should
not mask a clear stockout risk for active demand.

## Stock Position Categories

Use current inventory and forecast demand together. A product's stock position
should not be classified from `on_hand` alone.

- Critical: current `on_hand` is below `safety_stock`, or projected ending
  inventory is negative in the active planning period.
- At risk: current `on_hand` is at or above `safety_stock` but below
  `reorder_point`, or projected ending inventory falls below `safety_stock`.
- Watch list: current stock is healthy, but projected ending inventory falls
  below `reorder_point`.
- Healthy: projected ending inventory remains at or above `reorder_point`.

Projected ending inventory can be estimated as:

```text
projected_ending_inventory = on_hand - projected_units
```

When forecast units are missing, use the current stock category and flag the row
for forecast review rather than assuming demand is zero.

## Recommendation Principles

Recommendations should be grouped by a common operating reason. Do not combine
products with unrelated causes into one recommendation simply because they share
a segment or brand.

Good grouping reasons include:

- stockout prevention for products below safety stock,
- regional rebalance for products with uneven market coverage,
- safety stock correction for products with outdated buffers,
- promotion protection for products with constrained supply,
- demand surge response for products with unusually high projected units.

Every recommendation should include the affected `product_id` values, the
scenario or forecast evidence, the expected inventory impact, and the priority.
If the recommendation covers many products, include a concise summary of why the
whole product set belongs together and call out any exceptions.

## Priority Guidance

Use high priority when a product is critical, when projected ending inventory is
negative, or when a customer-facing promotion depends on constrained inventory.
Use medium priority when the product is at risk but not yet below safety stock.
Use low priority for watch-list products where the expected action can wait for
the next planning review.

Priority should be raised when the product serves a high-visibility channel,
when there is no substitute item, or when several markets show the same risk.
Priority can be lowered when the product is seasonal, discontinued, or has a
documented acceptable stockout window.

## Analyst Review Checklist

Before submitting a recommendation, check:

- product IDs resolve to active products,
- the scenario name matches the question being answered,
- inventory and scenario rows refer to the same product, location, and channel,
- projected units are present and plausible,
- safety stock and reorder point are non-negative,
- duplicates have been removed from the product list,
- the rationale explains the business reason, not just the SQL result.

If these checks fail, record the exception and avoid presenting the result as a
ready-to-execute recommendation.
