# Retail Revenue Policy

Net revenue is calculated from completed orders only. Use the order date for
the reporting period.

For each order item, start with `quantity * unit_price`, subtract
`discount_amount`, and subtract any refund amount tied to that order item. Tax
is excluded from net revenue.

Cancelled orders do not contribute to gross revenue or net revenue.
