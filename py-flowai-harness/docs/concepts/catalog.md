# Catalog

The catalog is the semantic layer that connects user intent to the data schema
and business knowledge in a workspace.

It gives agents a grounded representation of what the user is asking for, which
data entities can answer that request, and which business rules, metrics,
documents, and relationships should shape the result. Agents use this context
to resolve better plans before they query live data or propose an action.

The catalog stores metadata, semantic descriptions, relationships, and
retrieval projections. It helps agents find the right tables, columns, joins,
metrics, documents, knowledge items, enum values, and data-quality findings
without guessing names or inventing schema context.

The catalog is not the target database. It describes data and knowledge. The
target database is where read-only samples and SQL queries run.

## Why the catalog exists

Data agents need a grounded path from a user question to a query or answer.
Without a catalog, the model has to infer table names, field meanings, joins,
metric definitions, and policy context from prompts alone.

The catalog gives agents a safer workflow:

```text
discover candidates
  -> hydrate selected entities
  -> inspect fields
  -> inspect relations and paths
  -> sample target data when needed
  -> execute read-only SQL only after context is confirmed
```

This keeps broad discovery separate from authoritative details. A search result
is a candidate. A hydrated catalog entity, field listing, or relation path is
the context an agent should use for SQL planning.

## What lives in the catalog

Catalog entries are typed entities. The public catalog tools expose these
kinds:

| Kind | What it represents |
| --- | --- |
| `table` | A table, view, or preferred query surface. |
| `column` | A field belonging to a table or query surface. |
| `relationship` | A join or semantic relationship between entities. |
| `enum_value` | A known categorical value for a field. |
| `metric` | A named calculation or business measure. |
| `document` | A document projected from knowledge ingestion. |
| `knowledge` | An extracted fact, rule, policy, or note. |
| `data_quality_finding` | A quality issue, warning, or profiling finding. |

Every entry has a stable shape:

| Field | Purpose |
| --- | --- |
| `id` | Stable catalog id. Prefer this when calling follow-up tools. |
| `itemType` | Entry kind, such as `table` or `column`. |
| `name` | Human-readable short name. |
| `qualified_name` | Fully qualified name when the entity has one. |
| `content` | Compact description or semantic summary. |
| `tags` | Search and filtering labels. |
| `related` | Links to other catalog entries. |
| `metadata` | Typed details such as database id, schema, data type, or row count. |

## Scope and identity

Catalog data is scoped by tenant and workspace. Shared catalog backends should
store that scope as first-class storage fields so two workspaces can share the
same backend without mixing entries.

`database_id` is different. It identifies a logical target database inside a
workspace, such as `warehouse` or `billing`. Use the same `database_id` when
profiling a database and ingesting knowledge that links to that database.

Do not use `database_id` as an authorization boundary, and do not confuse it
with the catalog storage database.

## Catalog tools

Agents access the catalog through the built-in `catalog` toolkit. Attach it
only to agents that should inspect catalog metadata or run read-only data
queries:

```python
analyst = define_specialist(
    name="data_analyst",
    model="claude-sonnet-4-6",
    prompt="Use catalog tools to answer data questions.",
    toolkits=["catalog"],
)
```

The tools form a staged workflow:

| Tool | Use it for |
| --- | --- |
| `search_catalog` | Discover candidate entities from a phrase or identifier. |
| `get_catalog_entities` | Hydrate selected ids or qualified names into typed details. |
| `list_schema_fields` | Inspect columns, data types, keys, and field profiles. |
| `get_catalog_relations` | Fetch adjacent graph context for selected entities. |
| `get_relation_paths_between` | Find join paths or semantic paths between endpoints. |
| `sample_table_data` | Read a small exploratory sample from a selected table. |
| `execute_query` | Run a validated read-only `SELECT` or `WITH` query. |

Use `execute_query` as the final read step, after the agent has confirmed the
tables, fields, joins, filters, and semantic rules it needs.

## Storage setup

Catalog tools get their storage and data dependencies from
`create_runtime(..., data_environment=...)`. Each field has a separate job:

| Field | Supported kinds | Purpose |
| --- | --- | --- |
| `catalog` | `empty`, `inline`, `sqlite`, `postgres` | Catalog entities and relations. |
| `catalog_search` | local index config | Search index for `search_catalog`. |
| `target_database` | `sqlite`, `postgres` | Read-only source for samples and SQL. |
| `target_database_url` | URL shorthand | Simple target database connection shortcut. |
| `kv` | `memory`, `sqlite`, `postgres`, `redis` | Full document and knowledge payload storage. |

`catalog` and `catalog_search` are separate. The catalog stores entities. The
search index makes those entities discoverable.

Use `inline` catalogs for tests, examples, and committed export artifacts. Use
`sqlite` or `postgres` catalogs when profiling, Studio, or knowledge ingestion
should write durable entries.

### Local durable setup

Use local SQLite files when developing against a local target database:

```python
data_environment = {
    "tenant_id": "acme",
    "workspace_id": "analytics",
    "target_database": {
        "kind": "sqlite",
        "url": "sqlite:.data/target.db",
    },
    "catalog": {
        "kind": "sqlite",
        "url": "sqlite:.data/catalog.db",
        "ensure_schema": True,
    },
    "catalog_search": {
        "index_path": ".data/catalog-index",
        "rebuild_on_start": True,
        "write_through": True,
    },
}
```

### Reproducible artifact setup

Use an inline catalog when the catalog is an input artifact, such as an exported
`catalog.entries.json` committed with an example or test:

```python
data_environment = {
    "target_database_url": "sqlite:.data/target.db",
    "catalog": {
        "kind": "inline",
        "entries": catalog_entries,
    },
    "catalog_search": {
        "index_path": ".data/catalog-index",
        "rebuild_on_start": True,
    },
}
```

Inline catalogs are read-only runtime inputs. Profiling and knowledge ingestion
need a writable `sqlite` or `postgres` catalog.

### Shared deployment setup

Use environment-backed Postgres and Redis descriptors when the application
runs against shared services:

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
        "kind": "postgres",
        "url_env": "FLOWAI_CATALOG_URL",
        "ensure_schema": True,
    },
    "kv": {
        "kind": "redis",
        "url_env": "FLOWAI_REDIS_URL",
        "prefix": "acme:analytics",
    },
    "catalog_search": {
        "index_path": "/var/lib/flowai/catalog-index",
        "write_through": True,
    },
}
```

Use `url_env` for credentialed services so connection strings stay out of
checked-in config.

## Lifecycle

Catalog entries usually come from one of four paths:

- profiling a target database into a durable catalog
- ingesting documents or extracted knowledge into KV and catalog projections
- loading an exported `catalog.entries.json` artifact as an inline catalog
- writing entries through a backend-owned workflow

After entries change, keep `catalog_search` in sync by using `write_through`,
`rebuild_on_start`, or the catalog index rebuild command.

## Boundaries

Catalog tools are read-oriented. They help agents discover metadata, inspect
relationships, sample target data, and run read-only SQL. They do not perform
platform writes.

For business mutations, model the change as a typed plan, require approval when
needed, and apply the approved action through the action dispatcher.

## Common mistakes

- Treating `search_catalog` results as final query context.
- Running SQL before hydrating selected entities and inspecting fields.
- Inventing joins instead of using catalog relations or relation paths.
- Confusing `catalog` storage with the `target_database`.
- Using `inline` or `empty` catalogs as profiling or ingestion sinks.
- Enabling `search_catalog` without configuring `catalog_search`.
- Changing `database_id` between profiling and knowledge ingestion.

## See also

- [Tools](tools.md) for built-in toolkit behavior.
- [Tenants](tenant.md) for runtime identity and scope.
- [Configure a data environment](../guides/data-environment.md) for setup details.
- [Profile and export a catalog](../guides/catalog-profiling.md) for profiling workflows.
- [Knowledge and documents](../guides/knowledge.md) for document ingestion.
