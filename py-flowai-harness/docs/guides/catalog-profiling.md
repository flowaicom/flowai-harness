# Profile and export a catalog

Use this guide when you want to prepare a database for agents before runtime
execution.

Profiling is a development or operator workflow. You profile a read-only target
database once, persist the resulting catalog to a durable backend, and then your
application runtime consumes that catalog through the
[`data_environment`](data-environment.md).

```text
target database
    |
    | profile
    v
durable catalog (sqlite/postgres)
    |
    | export
    v
catalog.entries.json
    |
    | load as an inline catalog
    v
create_runtime(..., data_environment=...)
```

By the end, you should have a portable catalog artifact that your runtime can
load through the data environment.

## When to use this guide

Use this guide when agents need database context such as tables, columns,
relationships, descriptions, or preferred query surfaces. Start with
[Configure a data environment](data-environment.md) if you have not configured a
target database and durable catalog yet.

## The catalog lifecycle

### 1. Configure a data environment

Profiling needs a `target_database` to read and a durable `catalog` to write.
`inline` and `empty` catalogs are read-only runtime inputs and are rejected for
writes.

```json title="data-environment.json"
{
  "target_database": { "kind": "sqlite", "url": "sqlite:.data/acme.db" },
  "catalog": { "kind": "sqlite", "url": "sqlite:.data/catalog.db", "ensure_schema": true }
}
```

### 2. Estimate (optional)

Estimate token/cost/duration before paying for LLM enrichment:

```bash
flowai-harness --data-environment data-environment.json \
  data profile estimate --database-id acme
```

### 3. Profile

Profile a single table or a whole database. Profiling writes catalog entries
(tables, columns, relationships, …) into the configured durable catalog.

```bash
# one table
flowai-harness --data-environment data-environment.json \
  data profile table --database-id acme --table products

# the whole database (or a subset with repeated --table)
flowai-harness --data-environment data-environment.json \
  data profile database --database-id acme
```

#### Enrichment modes

| Mode | How | Output |
| --- | --- | --- |
| Anthropic (default) | `ANTHROPIC_API_KEY` set, or `--anthropic-api-key` | LLM-written semantic descriptions |
| Schema-only | `--schema-only` | Deterministic fallback, no LLM call |

The model can be overridden with `--anthropic-model` or
`FLOWAI_PROFILE_ANTHROPIC_MODEL`, and a compatible gateway with
`--anthropic-base-url` or `ANTHROPIC_BASE_URL`. Use `--schema-only` for
hermetic, reproducible runs in CI and examples.

### Target database id contract

`--database-id` is the stable logical id for the target database being
profiled. It is not the catalog storage database and it is not a tenant or
workspace boundary. Use the same non-empty value for every command that creates
or links schema-scoped catalog facts for the same target database:

```bash
flowai-harness --data-environment data-environment.json \
  data catalog profile --database-id warehouse

flowai-harness --data-environment data-environment.json \
  data knowledge ingest --database-id warehouse --source docs/
```

Tables, columns, relationship vertices, data-quality findings, and knowledge
scope links all use this id when resolving catalog relations. Using a different
or blank value can create links that apply to no schema object, or to an object
from the wrong target database. Profile commands reject blank `--database-id`
values before ingestion starts.

### 4. Maintain the search index

The catalog search index is separate from catalog storage. Rebuild or
health-check it after profiling:

```bash
flowai-harness --data-environment data-environment.json data catalog index rebuild
flowai-harness --data-environment data-environment.json data catalog index doctor
```

The doctor/check flow should report orphaned or mismatched catalog relation
counts with sample source ids, target ids, and relation kinds. Re-profile or
re-ingest with the correct `--database-id` to repair bad catalog data.

### 5. Export a portable artifact

Export the durable catalog to a committed, reviewable `catalog.entries.json`.
This **reads the existing catalog** — it does not re-profile the target
database, so it needs no target connection and no API key.

```bash
flowai-harness --data-environment data-environment.json \
  data catalog export --out data/catalogs/acme/catalog.entries.json
```

The artifact is:

- **Deterministic** — entries are ordered by `(kind, qualified_name, name, id)`,
  so repeated exports of the same catalog are byte-identical and
  snapshot-testable.
- **Secret-safe** — entries carry no connection strings, and any error message
  redacts credentials in target/catalog URLs.

`--output text|json|ndjson` controls the *summary* written to stdout (the entry
array always goes to `--out`). Scope flags `--tenant-id` / `--workspace-id`
select which catalog scope to export, matching the other `data` commands.

### 6. Consume from the runtime

Point your application's runtime at whichever catalog the workflow produced —
the durable backend directly, or the exported JSON loaded `inline` (ideal for
committed reference verticals and reproducible reviews):

```python
import json
from flowai_harness import create_runtime

# (a) consume the durable catalog directly
runtime = create_runtime(
    runtime_spec,
    data_environment={
        "target_database": {"kind": "sqlite", "url": "sqlite:.data/acme.db"},
        "catalog": {"kind": "sqlite", "url": "sqlite:.data/catalog.db"},
    },
)

# (b) consume the exported artifact inline
entries = json.loads(open("data/catalogs/acme/catalog.entries.json").read())
runtime = create_runtime(
    runtime_spec,
    data_environment={
        "target_database": {"kind": "sqlite", "url": "sqlite:.data/acme.db"},
        "catalog": {"kind": "inline", "entries": entries},
    },
)
```

The exported entries use the same shape as inline catalog entries (`itemType`,
`qualified_name`, `related`, `metadata`, …), so an export round-trips into an
`inline` catalog without transformation.

## Verify it works

Check that the exported file exists, contains entries, and can be loaded by the
runtime as an inline catalog:

```python
import json

entries = json.loads(open("data/catalogs/acme/catalog.entries.json").read())
assert len(entries) > 0
```

Then start a runtime with the exported entries and call a catalog tool such as
`search_catalog` or `get_catalog_entities`.

## Common errors

| Error | Fix |
| --- | --- |
| `ANTHROPIC_API_KEY is required for LLM enrichment; pass --schema-only for deterministic fallback` | Set `ANTHROPIC_API_KEY`, pass `--anthropic-api-key`, or use `--schema-only` for the deterministic no-LLM path. |
| `profiling/ingestion requires a durable catalog backend; data_environment.catalog kind=inline is read-only` (same for `kind=empty`) | Point `catalog` at a writable `sqlite` or `postgres` backend. `inline` and `empty` catalogs are read-only runtime inputs, not profiling sinks. |
| `failed to read data-environment file '...'` | Pass `--data-environment <path>` pointing at an existing JSON/TOML file; every `data …` command resolves its storage from that file. |

## See also

- [Configure a data environment](data-environment.md) — catalog and target descriptors.
- [Knowledge and documents](knowledge.md) — document ingestion and catalog projection.
- [Inventory scenario example](../tutorials/inventory-scenario.md) — seeded local data setup.
- [`create_runtime` reference](../reference/runtime.md#flowai_harness.runtime.create_runtime)
