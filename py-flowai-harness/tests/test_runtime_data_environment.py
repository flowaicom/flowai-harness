import asyncio
import json
import sqlite3

import pytest

from flowai_harness import (
    create_runtime,
    define_tenant,
    define_runtime,
    define_specialist,
)


async def _collect(stream):
    events = []
    async for event in stream:
        events.append(event)
    return events


def _spec_with_agent(agent):
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[agent],
        providers={"anthropic": {"apiKey": "unused"}},
    )


def _specialist(name, toolkits):
    return define_specialist(
        name=name,
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        toolkits=toolkits,
    )


def _run_tool(runtime, specialist, tool, args):
    prompt = json.dumps({"tool": tool, "args": args})
    events = asyncio.run(
        _collect(runtime.run_specialist(specialist, prompt, thread_id="thread-1"))
    )
    return _tool_result(events, tool)


def _tool_result(events, tool_name):
    for event in events:
        if (
            event["type"] == "tool-invocation"
            and event["toolName"] == tool_name
            and event["state"] == "result"
        ):
            return event["result"]
    raise AssertionError(f"no result event for {tool_name}: {events}")


def _catalog_entry(name="products"):
    return {
        "id": f"table:{name}",
        "itemType": "table",
        "name": name,
        "qualified_name": f"main.{name}",
        "content": "Product sales and catalog attributes for revenue analysis.",
        "tags": ["sales"],
        "related": [],
        "metadata": {},
    }


def _semantic_table_entry(name="products"):
    entry = _catalog_entry(name)
    entry["metadata"] = {
        "databaseId": "warehouse",
        "schemaName": "main",
        "tableName": name,
        "relationType": "base_table",
        "rowCount": 10,
        "columnCount": 2,
        "preferredQuerySurface": True,
    }
    return entry


def _catalog_search(tmp_path, name="catalog-index"):
    return {"index_path": str(tmp_path / name)}


def _document_entry():
    return {
        "id": "document:guide",
        "itemType": "document",
        "name": "Revenue Guide",
        "qualified_name": None,
        "content": "Catalog preview for revenue guidance.",
        "tags": ["[TYPE:document]"],
        "related": [],
        "metadata": {
            "sourceDocumentId": "doc-1",
            "extractionStatus": "processed",
        },
    }


def _knowledge_entry():
    return {
        "id": "knowledge:revenue-rule",
        "itemType": "knowledge",
        "name": "Revenue Rule",
        "qualified_name": None,
        "content": "Catalog preview for revenue rules.",
        "tags": ["[TYPE:knowledge]"],
        "related": [
            {
                "target_id": "document:guide",
                "relationType": "extracted_from",
                "description": "Extracted from Revenue Guide",
            }
        ],
        "metadata": {
            "sourceKnowledgeId": "k-1",
            "sourceDocumentId": "doc-1",
            "knowledgeType": "business_rule",
            "scopeTables": ["fact_sales"],
            "scopeColumns": ["fact_sales.net_amount"],
            "sqlExpression": None,
            "synonyms": ["net revenue"],
        },
    }


def _seed_sqlite_kv(path, tenant, key, value):
    with sqlite3.connect(path) as conn:
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS kv_store (
                tenant TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                expires_at INTEGER,
                PRIMARY KEY (tenant, key)
            )
            """
        )
        conn.execute(
            "INSERT OR REPLACE INTO kv_store (tenant, key, value, expires_at) VALUES (?, ?, ?, NULL)",
            (tenant, key, json.dumps(value)),
        )


def test_catalog_execute_query_uses_sqlite_data_environment(tmp_path):
    db_path = tmp_path / "acme.db"
    with sqlite3.connect(db_path) as conn:
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, revenue REAL)")
        conn.executemany(
            "INSERT INTO products (name, revenue) VALUES (?, ?)",
            [("Tea", 12.5), ("Coffee", 20.0)],
        )

    runtime = create_runtime(
        _spec_with_agent(_specialist("reader", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "target_database_url": f"sqlite:{db_path}",
            "catalog": {"kind": "inline", "entries": [_catalog_entry()]},
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "reader",
        "execute_query",
        {"sql": "SELECT name, revenue FROM products ORDER BY id", "limit": 10},
    )

    assert result["row_count"] == 2
    assert result["columns"] == ["name", "revenue"]
    assert result["rows"][0] == {"name": "Tea", "revenue": 12.5}


def test_catalog_execute_query_uses_structured_sqlite_target_descriptor(tmp_path):
    db_path = tmp_path / "acme.db"
    with sqlite3.connect(db_path) as conn:
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO products (name) VALUES ('Tea')")

    runtime = create_runtime(
        _spec_with_agent(_specialist("reader", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "kv": {"kind": "memory"},
            "target_database": {"kind": "sqlite", "url": f"sqlite:{db_path}"},
            "catalog": {"kind": "inline", "entries": [_catalog_entry()]},
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "reader",
        "execute_query",
        {"sql": "SELECT name FROM products", "limit": 10},
    )

    assert result["row_count"] == 1
    assert result["rows"][0] == {"name": "Tea"}


def test_create_runtime_accepts_sqlite_kv_and_catalog_ensure_schema(tmp_path):
    runtime = create_runtime(
        _spec_with_agent(_specialist("searcher", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "kv": {
                "kind": "sqlite",
                "url": f"sqlite:{tmp_path / 'runtime-kv.db'}",
                "ensure_schema": True,
            },
            "catalog": {
                "kind": "sqlite",
                "url": f"sqlite:{tmp_path / 'catalog.db'}",
                "ensure_schema": True,
            },
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "searcher",
        "search_catalog",
        {"query": "products", "limit": 5},
    )

    assert "Catalog search index is unavailable" in result["error"]


def test_create_runtime_accepts_catalog_workspace_scope(tmp_path):
    runtime = create_runtime(
        _spec_with_agent(_specialist("searcher", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "tenant_id": "acme",
            "workspace_id": "analytics",
            "catalog": {
                "kind": "sqlite",
                "url": f"sqlite:{tmp_path / 'scoped-catalog.db'}",
                "ensure_schema": True,
            },
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "searcher",
        "search_catalog",
        {"query": "products", "limit": 5},
    )

    assert "Catalog search index is unavailable" in result["error"]


def test_create_runtime_rebuilds_catalog_search_backend_from_data_environment(tmp_path):
    runtime = create_runtime(
        _spec_with_agent(_specialist("searcher", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "tenant_id": "acme",
            "workspace_id": "analytics",
            "catalog": {
                "kind": "inline",
                "entries": [_semantic_table_entry("products")],
            },
            "catalog_search": {
                "index_path": str(tmp_path / "catalog-index"),
                "rebuild_on_start": True,
            },
        },
    )

    result = _run_tool(
        runtime,
        "searcher",
        "search_catalog",
        {"query": "products", "limit": 5},
    )

    assert result["results"][0]["id"] == "table:products"
    assert result["diagnostics"]["search_mode"] == "lexical"


def test_create_runtime_rejects_data_environment_tenant_mismatch(tmp_path):
    with pytest.raises(ValueError, match="data_environment.tenant_id.*runtime tenant"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "tenant_id": "other-tenant",
                "catalog": {
                    "kind": "sqlite",
                    "url": f"sqlite:{tmp_path / 'catalog.db'}",
                    "ensure_schema": True,
                },
            },
        )


def test_create_runtime_rejects_blank_data_environment_tenant_id(tmp_path):
    with pytest.raises(ValueError, match=r"data_environment\['tenant_id'\] must not be blank"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "tenant_id": "   ",
                "catalog": {
                    "kind": "sqlite",
                    "url": f"sqlite:{tmp_path / 'catalog.db'}",
                    "ensure_schema": True,
                },
            },
        )


def test_create_runtime_rejects_blank_data_environment_workspace_id(tmp_path):
    with pytest.raises(ValueError, match=r"data_environment\['workspace_id'\] must not be blank"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "workspace_id": "   ",
                "catalog": {
                    "kind": "sqlite",
                    "url": f"sqlite:{tmp_path / 'catalog.db'}",
                    "ensure_schema": True,
                },
            },
        )


def test_catalog_execute_query_rejects_write_before_sqlite_mutation(tmp_path):
    db_path = tmp_path / "acme.db"
    with sqlite3.connect(db_path) as conn:
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO products (name) VALUES ('Tea')")

    runtime = create_runtime(
        _spec_with_agent(_specialist("reader", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "target_database_url": f"sqlite:{db_path}",
            "catalog": {"kind": "inline", "entries": [_catalog_entry()]},
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "reader",
        "execute_query",
        {"sql": "DROP TABLE products"},
    )

    assert "error" in result
    assert "read-only" in result["error"].lower() or "select" in result["error"].lower()
    with sqlite3.connect(db_path) as conn:
        assert conn.execute("SELECT COUNT(*) FROM products").fetchone() == (1,)


def test_catalog_toolkit_requires_catalog_search_config():
    with pytest.raises(ValueError, match="catalog_search"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "catalog": {"kind": "inline", "entries": [_catalog_entry()]},
            },
        )


def test_catalog_entities_hydrate_document_and_knowledge_entries(tmp_path):
    runtime = create_runtime(
        _spec_with_agent(_specialist("catalog_reader", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "catalog": {
                "kind": "inline",
                "entries": [_document_entry(), _knowledge_entry()],
            },
            "catalog_search": _catalog_search(tmp_path),
        },
    )

    result = _run_tool(
        runtime,
        "catalog_reader",
        "get_catalog_entities",
        {"refs": [{"id": "document:guide"}, {"id": "knowledge:revenue-rule"}]},
    )
    assert [entity["id"] for entity in result["entities"]] == [
        "document:guide",
        "knowledge:revenue-rule",
    ]


def test_missing_data_environment_dependencies_surface_actionable_tool_errors(tmp_path):
    no_target_runtime = create_runtime(
        _spec_with_agent(_specialist("reader", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "catalog": {"kind": "inline", "entries": [_catalog_entry()]},
            "catalog_search": _catalog_search(tmp_path, "no-target-index"),
        },
    )
    target_error = _run_tool(
        no_target_runtime,
        "reader",
        "execute_query",
        {"sql": "SELECT 1"},
    )["error"]
    assert "TargetDatabase" in target_error
    assert "data_environment.target_database_url" in target_error

    no_catalog_runtime = create_runtime(
        _spec_with_agent(_specialist("searcher", ["catalog"])),
        interpreter="scripted",
        data_environment={
            "catalog_search": _catalog_search(tmp_path, "no-catalog-index"),
        },
    )
    catalog_error = _run_tool(
        no_catalog_runtime,
        "searcher",
        "get_catalog_entities",
        {"refs": [{"id": "table:products"}]},
    )["error"]
    assert "DataCatalog" in catalog_error
    assert "data_environment.catalog" in catalog_error


def test_create_runtime_rejects_unsupported_data_environment_shape():
    with pytest.raises(ValueError, match="sqlite.*postgres"):
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={"target_database_url": "mysql://localhost/db"},
        )


def test_create_runtime_accepts_structured_storage_descriptors_without_opening_missing_env():
    with pytest.raises(ValueError, match="FLOWAI_MISSING_TARGET_URL"):
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "kv": {"kind": "memory"},
                "catalog": {"kind": "empty"},
                "target_database": {
                    "kind": "postgres",
                    "url_env": "FLOWAI_MISSING_TARGET_URL",
                    "schema": "public",
                },
            },
        )


def test_create_runtime_accepts_camel_case_data_environment_aliases():
    with pytest.raises(ValueError, match="FLOWAI_MISSING_TARGET_URL"):
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "catalog": {"kind": "empty"},
                "targetDatabase": {
                    "kind": "postgres",
                    "urlEnv": "FLOWAI_MISSING_TARGET_URL",
                    "schema": "public",
                },
            },
        )


def test_create_runtime_accepts_remote_kv_descriptor_kinds_until_env_resolution():
    for kind, env_name in [
        ("postgres", "FLOWAI_MISSING_KV_POSTGRES_URL"),
        ("redis", "FLOWAI_MISSING_KV_REDIS_URL"),
    ]:
        with pytest.raises(ValueError, match=env_name):
            create_runtime(
                _spec_with_agent(_specialist("reader", ["catalog"])),
                interpreter="scripted",
                data_environment={
                    "kv": {"kind": kind, "url_env": env_name},
                    "catalog": {"kind": "empty"},
                },
            )


def test_create_runtime_accepts_remote_catalog_descriptor_until_env_resolution():
    with pytest.raises(ValueError, match="FLOWAI_MISSING_CATALOG_POSTGRES_URL"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "catalog": {
                    "kind": "postgres",
                    "url_env": "FLOWAI_MISSING_CATALOG_POSTGRES_URL",
                    "ensure_schema": True,
                },
            },
        )


def test_create_runtime_rejects_unsupported_storage_category_backend():
    with pytest.raises(ValueError, match="catalog.*redis"):
        create_runtime(
            _spec_with_agent(_specialist("searcher", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "catalog": {
                    "kind": "redis",
                    "url_env": "FLOWAI_REDIS_URL",
                },
            },
        )


def test_create_runtime_rejects_invalid_remote_identifiers_without_leaking_urls():
    with pytest.raises(ValueError) as kv_exc:
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "kv": {
                    "kind": "postgres",
                    "url": "postgresql://user:secret@example.invalid/db",
                    "table": "kv-store",
                },
            },
        )

    kv_message = str(kv_exc.value)
    assert "data_environment.kv.table" in kv_message
    assert "secret" not in kv_message
    assert "example.invalid" not in kv_message

    with pytest.raises(ValueError) as target_exc:
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "target_database": {
                    "kind": "postgres",
                    "url": "postgresql://user:secret@example.invalid/db",
                    "schema": "public;drop",
                },
            },
        )

    target_message = str(target_exc.value)
    assert "data_environment.target_database.schema" in target_message
    assert "secret" not in target_message
    assert "example.invalid" not in target_message


def test_create_runtime_descriptor_validation_hides_secret_connection_string_inputs():
    with pytest.raises(ValueError) as exc:
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "kv": {
                    "kind": "postgres",
                    "url": "postgresql://user:secret@example.invalid/db?password=abc",
                    "url_env": "FLOWAI_KV_DATABASE_URL",
                },
            },
        )

    message = str(exc.value)
    assert "either url or url_env" in message
    assert "secret" not in message
    assert "password=abc" not in message
    assert "example.invalid" not in message


def test_create_runtime_validates_storage_descriptor_types():
    with pytest.raises(
        TypeError,
        match="data_environment\\['target_database'\\] must be a mapping",
    ):
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={"target_database": "sqlite:/tmp/acme.db"},
        )


def test_create_runtime_redacts_secret_connection_string_values(monkeypatch):
    monkeypatch.setenv(
        "FLOWAI_BAD_TARGET_URL",
        "postgresql://user:secret@/db?password=abc",
    )

    with pytest.raises(ValueError) as exc:
        create_runtime(
            _spec_with_agent(_specialist("reader", ["catalog"])),
            interpreter="scripted",
            data_environment={
                "target_database": {
                    "kind": "postgres",
                    "url_env": "FLOWAI_BAD_TARGET_URL",
                },
            },
        )

    message = str(exc.value)
    assert "user:***@/db" in message
    assert "password=***" in message
    assert "secret" not in message
    assert "abc" not in message
