# Knowledge and documents

Use this guide when you want agents to retrieve source material, documents, or
extracted knowledge through built-in data tools.

Knowledge ingestion creates indexed source material. The data environment makes
that knowledge available to runtime tools. Agents access it through the built-in
`catalog` toolkit.

## When to use this guide

Use this guide when agents need retrieval-backed context from files such as
Markdown, text notes, policies, product docs, runbooks, or extracted knowledge.

By the end, you should have ingested documents, projected them into a catalog
when configured, and attached catalog tools to an agent.

## How knowledge reaches agents

Flow AI supports workspace-local knowledge documents in two phases:

1. `flowai-harness data knowledge ingest` imports local files into the configured
   KV store and optionally extracts structured knowledge items.
2. When a writable catalog is configured, ingestion projects document and
   extracted knowledge entries into the tenant/workspace catalog scope. Agents
   inspect those entries through the built-in `catalog` toolkit.

Documents and knowledge are catalog entity kinds surfaced by `get_catalog_entities`,
`get_catalog_relations`, and `search_catalog` when `catalog_search` is
configured.

## What gets indexed

Ingestion can store:

- source documents
- document metadata
- extracted knowledge items
- catalog projections for documents and knowledge
- relations between knowledge and profiled database entities

## Configure storage and search

Knowledge ingestion always requires `kv`. If `catalog` is omitted, ingestion is
KV-only and the runtime can still hydrate entries that another process projected
into the catalog. If `catalog` is present during ingestion, it must be writable:
use `sqlite` or `postgres`, not `inline` or `empty`.

```json
{
  "tenant_id": "acme",
  "workspace_id": "analytics",
  "kv": {
    "kind": "sqlite",
    "url": "sqlite:.data/flowai-kv.db",
    "ensure_schema": true
  },
  "catalog": {
    "kind": "sqlite",
    "url": "sqlite:.data/flowai-catalog.db",
    "ensure_schema": true
  },
  "catalog_search": {
    "index_path": ".data/catalog-index",
    "rebuild_on_start": true,
    "write_through": true
  },
  "target_database": {
    "kind": "postgres",
    "url_env": "ACME_WAREHOUSE_URL",
    "schema": "public"
  }
}
```

In `target_database`, `url_env` names the environment variable the connection
URL is read from at startup, so the credentialed URL itself stays out of the
config file.

The catalog is scoped by tenant and workspace. Knowledge projection uses that
scope when generating document and knowledge catalog ids, so two workspaces can
share the same catalog backend without colliding.

## Ingest documents

Preview the directory before running an ingest command. Ingestion writes durable
state, and `--extract-knowledge` can call an LLM.

```bash
find ./knowledge -type f
```

Document-only ingest stores `DocumentItem` payloads and content hashes in KV:

```bash
flowai-harness --data-environment data-environment.json --output ndjson \
  data knowledge ingest \
  --tenant-id acme \
  --workspace-id analytics \
  --database-id warehouse \
  --local-dir ./knowledge \
  --ext md \
  --ext txt
```

With `--extract-knowledge`, the command also extracts `KnowledgeItem` payloads.
It requires `ANTHROPIC_API_KEY` or `--anthropic-api-key`. Use
`FLOWAI_KNOWLEDGE_ANTHROPIC_MODEL` or `--anthropic-model` to select the
extraction model.

```bash
flowai-harness --data-environment data-environment.json --output ndjson \
  data knowledge ingest \
  --tenant-id acme \
  --workspace-id analytics \
  --database-id warehouse \
  --local-dir ./knowledge \
  --ext md \
  --extract-knowledge
```

When a writable catalog is configured, completion means both KV persistence and
catalog projection succeeded. If catalog projection fails, the command emits an
error instead of a completed event.

## Verify it works

With `--output ndjson`, a successful run ends with a `completed` event that
accounts for every scanned file:

```json
{
  "type": "completed",
  "scanned": 1,
  "new": 1,
  "skippedDuplicate": 0,
  "errors": []
}
```

To confirm the catalog projection, export the same catalog scope and check the
summary for a `document` entry:

```bash
flowai-harness --data-environment data-environment.json \
  data catalog export \
  --tenant-id acme \
  --workspace-id analytics \
  --out catalog.entries.json
```

```text
output_path: catalog.entries.json
entries_written: 1
  document: 1
```

Agents see the same entry through the `catalog` toolkit: `search_catalog` for
discovery, or `get_catalog_entities` once ids are known.

## Attach knowledge to runtime

Add `catalog` to any specialist that should retrieve workspace documents or
extracted knowledge catalog entries:

```python
from flowai_harness import (
    create_runtime,
    define_runtime,
    define_specialist,
    define_tenant,
)


tenant = define_tenant("acme", "v1")
knowledge_reader = define_specialist(
    "knowledge_reader",
    model="claude-sonnet-4-6",
    prompt="Use workspace knowledge before answering data-policy questions.",
    toolkits=["catalog"],
)

runtime = create_runtime(
    define_runtime(
        tenant=tenant,
        agents=[knowledge_reader],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    ),
    data_environment={
        "tenant_id": "acme",
        "workspace_id": "analytics",
        "kv": {"kind": "sqlite", "url": "sqlite:.data/flowai-kv.db"},
        "catalog": {"kind": "sqlite", "url": "sqlite:.data/flowai-catalog.db"},
        "catalog_search": {
            "index_path": ".data/catalog-index",
            "rebuild_on_start": True,
            "write_through": True,
        },
    },
)
```

## Query knowledge from an agent

Inline catalogs are useful for tests and examples. They can drive catalog
hydration, graph tools, and `search_catalog` when paired with
`catalog_search`, but they cannot receive ingestion output:

```python
runtime = create_runtime(
    define_runtime(
        tenant=tenant,
        agents=[knowledge_reader],
        providers={"anthropic": {"apiKey": "unused"}},
    ),
    interpreter="scripted",
    data_environment={
        "kv": {"kind": "memory"},
        "catalog": {
            "kind": "inline",
            "entries": [
                {
                    "id": "document:revenue-guide",
                    "itemType": "document",
                    "name": "Revenue Guide",
                    "qualified_name": None,
                    "content": "Catalog preview for revenue guidance.",
                    "tags": ["[TYPE:document]"],
                    "related": [],
                    "metadata": {
                        "sourceDocumentId": "doc-1",
                        "extractionStatus": "processed"
                    },
                }
            ],
        },
        "catalog_search": {
            "index_path": ".data/catalog-index",
            "rebuild_on_start": True,
        },
    },
)
```

## Toolkit tools

Use the built-in `catalog` toolkit for document and knowledge entries:

| Tool                    | Purpose                                                                               |
| ----------------------- | ------------------------------------------------------------------------------------- |
| `get_catalog_entities`  | Hydrate known document or knowledge catalog ids and return typed details.             |
| `get_catalog_relations` | Traverse document/knowledge relations, including extracted-from and applies-to edges. |
| `search_catalog`        | Search documents and knowledge through the configured catalog search index.           |

Example input for known ids:

```json
{
  "refs": [
    { "id": "document:revenue-guide" },
    { "id": "knowledge:revenue-rule" }
  ]
}
```

The response includes catalog entities. Full document bodies remain in the
ingestion KV store; the public catalog toolkit returns the catalog projection
and typed metadata, not the old KV-hydrated document-content envelope.

```json
{
  "entities": [
    {
      "id": "document:revenue-guide",
      "kind": "document",
      "name": "Revenue Guide"
    }
  ],
  "missing": [],
  "warnings": []
}
```

## Common errors

| Symptom                                                                  | Fix                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--database-id <DATABASE_ID>` is missing                                 | Pass the catalog database id used when profiling the target schema, for example `--database-id warehouse`.                                                                                                              |
| `knowledge ingestion database_id must not be blank`                      | Pass a non-empty `--database-id`; there is no default fallback because schema links must target a known catalog database.                                                                                               |
| `knowledge catalog projection found ... missing scope targets`           | Profile the target schema first, or use the same `--database-id` used by profiling.                                                                                                                                     |
| `knowledge ingestion requires data_environment.kv`                       | Add `kv` to the data environment.                                                                                                                                                                                       |
| `kind=inline is read-only` or `kind=empty is read-only` during ingestion | Use a writable `catalog` backend or omit `catalog` for KV-only ingestion.                                                                                                                                               |
| Toolkit returns a `DataCatalog` missing-dependency error                 | Attach `data_environment["catalog"]` to the runtime or MCP server.                                                                                                                                                      |
| `create_runtime` reports that catalog search is not configured           | Add `catalog_search.index_path` to the data environment for agents that select the `catalog` toolkit. Use `rebuild_on_start = true` or run `flowai-harness data catalog index rebuild` before the first search request. |
| `ANTHROPIC_API_KEY is required when --extract-knowledge is enabled`      | Load `.env`, set `ANTHROPIC_API_KEY`, or pass `--anthropic-api-key`.                                                                                                                                                    |

## See also

- [Configure a data environment](data-environment.md)
- [Profile and export a catalog](catalog-profiling.md)
- [Inventory scenario example](../tutorials/inventory-scenario.md)
