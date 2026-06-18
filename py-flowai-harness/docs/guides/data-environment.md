# Configure a data environment

A data environment connects runtime tools to the data dependencies they need:
catalog metadata, catalog search, key-value storage, target databases, and
knowledge/document stores.

Use this guide when an agent needs built-in data tools such as catalog search,
schema inspection, read-only query execution, or document retrieval. Skip it for
pure Python callback tools that only use services passed through
`create_runtime(..., services=...)`.

## What you will configure

By the end of this guide, you should have a `data_environment` mapping that can
be passed to `create_runtime(...)` and used by agents with the `catalog`
toolkit.

## Which dependency do I need?

| Need | Configure |
| --- | --- |
| Agents inspect database structure | `catalog` |
| Agents search indexed metadata | `catalog_search` |
| Agents run read-only SQL or sample rows | `target_database` or `target_database_url` |
| Agents retrieve documents or extracted knowledge | `kv`, `catalog`, and `catalog_search` |
| Runtime-owned durable data tools | `kv` or a durable `catalog` backend |

The exact config fields are in
[`DataEnvironmentConfig`](../reference/runtime.md#flowai_harness.runtime.DataEnvironmentConfig).

## Minimal setup

For a local SQLite target database and inline catalog:

```python
runtime = create_runtime(
    runtime_spec,
    data_environment={
        "target_database_url": "sqlite:/path/to/acme.db",
        "catalog": {
            "kind": "inline",
            "entries": [
                {
                    "id": "table:products",
                    "itemType": "table",
                    "name": "products",
                    "qualified_name": "main.products",
                    "content": "Product catalog and revenue table.",
                    "tags": ["sales"],
                    "related": [],
                    "metadata": {
                        "databaseId": "warehouse",
                        "schemaName": "main",
                        "tableName": "products",
                        "relationType": "base_table",
                        "preferredQuerySurface": True,
                    },
                }
            ],
        },
        "catalog_search": {
            "index_path": "/path/to/catalog-index",
            "rebuild_on_start": True,
        },
    },
)
```

Attach the `catalog` toolkit to the agent that should use these dependencies:

```python
reader = define_specialist(
    name="reader",
    model="claude-sonnet-4-6",
    prompt="Use catalog tools to answer data questions.",
    toolkits=["catalog"],
)
```

## Use a durable catalog

Use a durable catalog when profiling output, Studio data exploration, or
knowledge ingestion should persist across runs.

```python
data_environment = {
    "tenant_id": "acme",
    "workspace_id": "analytics",
    "target_database": {
        "kind": "postgres",
        "url_env": "ACME_WAREHOUSE_URL",
        "schema": "public",
    },
    "catalog": {
        "kind": "sqlite",
        "url": "sqlite:.flowai/catalog.db",
        "ensure_schema": True,
    },
    "catalog_search": {
        "index_path": ".flowai/catalog-index",
        "rebuild_on_start": True,
        "write_through": True,
    },
}
```

Set `tenant_id` only as a guardrail for shared config files. It must match the
runtime tenant.

## Add knowledge dependencies

Knowledge ingestion needs KV storage. If you want ingested documents projected
into catalog search, add a writable catalog and catalog search.

```python
data_environment = {
    "kv": {
        "kind": "sqlite",
        "url": "sqlite:.flowai/kv.db",
        "ensure_schema": True,
    },
    "catalog": {
        "kind": "sqlite",
        "url": "sqlite:.flowai/catalog.db",
        "ensure_schema": True,
    },
    "catalog_search": {
        "index_path": ".flowai/catalog-index",
        "rebuild_on_start": True,
        "write_through": True,
    },
}
```

See [Knowledge and documents](knowledge.md) for the ingestion flow.

## Verify it works

With the scripted interpreter, call a catalog tool and confirm you receive a
structured result rather than a missing-dependency error.

```python
prompt = '{"tool": "search_catalog", "args": {"query": "products", "limit": 5}}'
events = [event async for event in runtime.run_specialist("reader", prompt, thread_id="t-1")]

assert any(
    event["type"] == "tool-invocation"
    and event["state"] == "result"
    and "error" not in event.get("result", {})
    for event in events
)
```

For a complete local setup with `target.db`, `catalog.db`, `kv.db`,
`catalog-index/`, and Studio, see the
[Inventory scenario example](../tutorials/inventory-scenario.md).

## Common errors

| Symptom | Fix |
| --- | --- |
| `create_runtime` reports catalog search is not configured | Add `catalog_search.index_path` when an agent selects the `catalog` toolkit. |
| Catalog tools return a missing `DataCatalog` dependency | Add `data_environment["catalog"]`. |
| Query tools return a missing target database dependency | Add `target_database` or `target_database_url`. |
| Profiling or knowledge ingestion rejects `kind=inline` | Use a writable `sqlite` or `postgres` catalog for writes. Inline catalogs are runtime inputs. |
| Tenant mismatch at startup | Make `data_environment["tenant_id"]` match `define_tenant(...).resource_id`, or omit it. |

## See also

- [Profile and export a catalog](catalog-profiling.md)
- [Knowledge and documents](knowledge.md)
- [Studio](studio.md)
- [`DataEnvironmentConfig` reference](../reference/runtime.md#flowai_harness.runtime.DataEnvironmentConfig)
