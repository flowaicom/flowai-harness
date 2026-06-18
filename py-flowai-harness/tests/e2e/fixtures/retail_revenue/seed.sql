INSERT INTO retail.regions (id, code, name, vat_reporting) VALUES
    (1, 'EMEA', 'Europe, Middle East, and Africa', true),
    (2, 'NAM', 'North America', false),
    (3, 'APAC', 'Asia Pacific', false);

INSERT INTO retail.stores (id, region_id, name, channel) VALUES
    (1, 1, 'London Flagship', 'retail'),
    (2, 2, 'Austin Online', 'online'),
    (3, 3, 'Singapore Market', 'retail');

INSERT INTO retail.customers (id, external_id, segment, region_id) VALUES
    (1, 'C-EMEA-001', 'enterprise', 1),
    (2, 'C-NAM-001', 'consumer', 2),
    (3, 'C-APAC-001', 'consumer', 3),
    (4, 'C-EMEA-002', 'consumer', 1);

INSERT INTO retail.products (id, sku, name, category) VALUES
    (1, 'SKU-TEA-BOX', 'Premium Tea Box', 'pantry'),
    (2, 'SKU-MUG-SET', 'Ceramic Mug Set', 'home'),
    (3, 'SKU-BEANS', 'Coffee Beans', 'pantry');

INSERT INTO retail.campaigns (id, code, name, channel) VALUES
    (1, 'Q1-WELCOME', 'Q1 Welcome Offer', 'email'),
    (2, 'Q1-RETARGET', 'Q1 Retargeting', 'paid_search');

INSERT INTO retail.orders (id, customer_id, store_id, campaign_id, ordered_at, status, currency) VALUES
    (1001, 1, 1, 1, DATE '2026-01-15', 'completed', 'USD'),
    (1002, 2, 2, 2, DATE '2026-02-10', 'completed', 'USD'),
    (1003, 3, 3, 1, DATE '2026-03-05', 'completed', 'USD'),
    (1004, 4, 1, 1, DATE '2026-04-10', 'completed', 'USD'),
    (1005, 4, 1, 2, DATE '2026-02-18', 'cancelled', 'USD');

INSERT INTO retail.order_items (id, order_id, product_id, quantity, unit_price, discount_amount, tax_amount) VALUES
    (5001, 1001, 1, 2, 120.00, 20.00, 44.00),
    (5002, 1001, 2, 1, 80.00, 0.00, 16.00),
    (5003, 1002, 3, 3, 50.00, 15.00, 0.00),
    (5004, 1002, 1, 1, 200.00, 25.00, 0.00),
    (5005, 1003, 3, 5, 30.00, 0.00, 0.00),
    (5006, 1004, 1, 1, 500.00, 0.00, 100.00),
    (5007, 1005, 2, 2, 70.00, 10.00, 26.00);

INSERT INTO retail.refunds (id, order_item_id, refunded_at, quantity, refund_amount, reason) VALUES
    (9001, 5001, DATE '2026-02-01', 1, 60.00, 'damaged item'),
    (9002, 5004, DATE '2026-03-01', 1, 50.00, 'goodwill adjustment');
