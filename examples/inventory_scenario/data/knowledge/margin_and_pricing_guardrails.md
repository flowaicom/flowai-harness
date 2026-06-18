# Margin and Pricing Guardrails

## Scope

Scenario analysis should consider margin, but margin should not be used as the
only decision criterion. The dataset provides projected revenue and
`gross_margin_pct`, which are useful directional signals. It does not include a
full profit-and-loss model, supplier funding, freight, labor, markdown budgets,
or carrying cost.

Use margin language carefully. Prefer "gross margin percentage signal" or
"margin context" rather than "profit" unless additional finance data is
available.

## Margin Interpretation

Classify margin signals consistently:

- High margin: `gross_margin_pct >= 0.55`.
- Moderate margin: `0.35 <= gross_margin_pct < 0.55`.
- Low margin: `gross_margin_pct < 0.35`.

These thresholds are planning heuristics, not accounting rules. A low-margin
product may still need urgent replenishment if it protects customer service or
supports an important channel. A high-margin product may still be deferred if
inventory is healthy.

## Promotion Holdback Guidance

Promotion holdbacks should be considered when forecast demand would consume
inventory needed for higher-margin baseline demand or for channels with stricter
service expectations. A holdback recommendation should explain which demand is
being protected and what inventory risk would remain after the holdback.

Consider a holdback when:

- the promotion scenario has materially lower `gross_margin_pct` than baseline,
- current inventory is close to `safety_stock`,
- projected ending inventory would fall below `reorder_point`,
- the product has constrained supply in a specific market or channel,
- there is evidence that a short promotion would displace higher-priority
  demand.

Avoid holdbacks that create or worsen a stockout. If the product is already
below `safety_stock`, replenishment or allocation review should be evaluated
before recommending a promotion holdback.

## Pricing Language

The scenario dataset can support pricing analysis, but it does not represent a
pricing execution system. Analysts may describe a scenario as margin-dilutive,
margin-accretive, or margin-neutral based on `gross_margin_pct` and projected
revenue. They should not state that a price change has been applied unless a
separate pricing record confirms it.

Use these phrases:

- "projected revenue" instead of "recognized revenue",
- "gross margin percentage signal" instead of "profit",
- "protect inventory for higher-margin demand" instead of "maximize margin",
- "pricing hypothesis" instead of "price change" when no executed price record
  is present.

## Escalation Criteria

Escalate to a category manager or finance partner when a recommendation trades
off service level against margin, affects a large product set, or conflicts with
known promotion commitments. The escalation note should include the affected
products, scenario name, inventory risk category, projected revenue, and margin
signal.
