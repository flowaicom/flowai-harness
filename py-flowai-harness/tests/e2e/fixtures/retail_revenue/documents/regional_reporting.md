# Regional Reporting Guide

Revenue by region uses the store region, not the customer home region. Join
`orders.store_id` to `stores.id`, then join `stores.region_id` to `regions.id`.

The region code is the reporting label. EMEA uses VAT-inclusive invoices, but
VAT and other tax amounts are excluded from net revenue.
