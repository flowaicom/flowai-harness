DROP SCHEMA IF EXISTS retail CASCADE;
CREATE SCHEMA retail;

CREATE TABLE retail.regions (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    vat_reporting BOOLEAN NOT NULL
);

CREATE TABLE retail.stores (
    id INTEGER PRIMARY KEY,
    region_id INTEGER NOT NULL REFERENCES retail.regions(id),
    name TEXT NOT NULL,
    channel TEXT NOT NULL
);

CREATE TABLE retail.customers (
    id INTEGER PRIMARY KEY,
    external_id TEXT NOT NULL UNIQUE,
    segment TEXT NOT NULL,
    region_id INTEGER NOT NULL REFERENCES retail.regions(id)
);

CREATE TABLE retail.products (
    id INTEGER PRIMARY KEY,
    sku TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    category TEXT NOT NULL
);

CREATE TABLE retail.campaigns (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    channel TEXT NOT NULL
);

CREATE TABLE retail.orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES retail.customers(id),
    store_id INTEGER NOT NULL REFERENCES retail.stores(id),
    campaign_id INTEGER REFERENCES retail.campaigns(id),
    ordered_at DATE NOT NULL,
    status TEXT NOT NULL,
    currency TEXT NOT NULL
);

CREATE TABLE retail.order_items (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL REFERENCES retail.orders(id),
    product_id INTEGER NOT NULL REFERENCES retail.products(id),
    quantity INTEGER NOT NULL,
    unit_price NUMERIC(12, 2) NOT NULL,
    discount_amount NUMERIC(12, 2) NOT NULL DEFAULT 0,
    tax_amount NUMERIC(12, 2) NOT NULL DEFAULT 0
);

CREATE TABLE retail.refunds (
    id INTEGER PRIMARY KEY,
    order_item_id INTEGER NOT NULL REFERENCES retail.order_items(id),
    refunded_at DATE NOT NULL,
    quantity INTEGER NOT NULL,
    refund_amount NUMERIC(12, 2) NOT NULL,
    reason TEXT NOT NULL
);
