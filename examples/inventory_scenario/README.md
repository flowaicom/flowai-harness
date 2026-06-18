# Inventory Scenario Example

Advanced local data-agent example for `flowai-harness`.

For the explanatory walkthrough, see the
[Inventory Scenario tutorial](https://flow-ai.com/docs/tutorials/inventory-scenario).

## What this example demonstrates

- Static SQLite artifact setup instead of direct warehouse access.
- Built-in `catalog` tools over local SQLite data.
- Query-backed `InventoryProductSet` references for large product selections.
- Compact glimpses that let agents reason without seeing every product id.
- Typed `InventoryScenarioPlan` actions for replenishment, safety stock, and
  promotion holdbacks.
- Runtime-managed plan approval and `executePlan` reference hydration.
- A Python action dispatcher that applies approved actions through a mutable
  mock platform.
- Studio chat, Connect, run inspection, and offline smoke verification against
  a prepared app.

## Prerequisites

From the repository root, install the source checkout:

```bash
./scripts/check-env.sh
./scripts/install.sh
```

Then enter the example:

```bash
cd examples/inventory_scenario
```

## Quick start

Seed the published target artifact and create the runtime data-environment
descriptor:

```bash
uv run inventory-scenario seed
uv run inventory-scenario data-env --out .data/inventory_scenario/data-environment.json
```

Seed creates:

```text
.data/inventory_scenario/target.db
.data/inventory_scenario/platform.db
.data/inventory_scenario/artifacts/
.data/inventory_scenario/data-environment.json
```

Run the Flow AI data operations that create catalog, KV, and search-index state:

```bash
export INVENTORY_DATA_ENV=.data/inventory_scenario/data-environment.json

PROFILE_TABLES=(
  --table dim_companies
  --table dim_brands
  --table dim_segments
  --table dim_subsegments
  --table dim_coordinates
  --table dim_sales_channels
  --table dim_time_periods
  --table dim_products
  --table dim_inventory
)

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output json \
  data profile estimate --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --schema main --sample-size 1 \
  "${PROFILE_TABLES[@]}"

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output ndjson \
  data profile database --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --schema main --sample-size 1 \
  "${PROFILE_TABLES[@]}" --schema-only

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output ndjson \
  data knowledge ingest --tenant-id inventory_scenario --workspace-id default \
  --database-id inventory_scenario --local-dir data/knowledge --ext md

uv run flowai-harness --data-environment "$INVENTORY_DATA_ENV" --output json \
  data catalog index rebuild --tenant-id inventory_scenario --workspace-id default
```

Those commands create:

```text
.data/inventory_scenario/catalog.db
.data/inventory_scenario/kv.db
.data/inventory_scenario/catalog-index/
```

All `.data/` files are generated local state and should not be committed.

## Verify locally

Run smoke verification:

```bash
uv run inventory-scenario smoke
```

Expected: exits 0 and prints a concise success line that includes target DB,
catalog query, mock platform, and scripted runtime checks.

Run offline tests:

```bash
uv run pytest tests/test_public_artifact.py tests/test_seed.py tests/test_platform_api.py tests/test_runtime_smoke.py -q
```

Expected: all tests pass without `ANTHROPIC_API_KEY`.

## Run Studio

Studio uses the Anthropic interpreter by default, so export a provider key:

```bash
export ANTHROPIC_API_KEY
uv run flowai-harness dev --app inventory_scenario.app:runtime
```

Open:

```text
http://127.0.0.1:4111
```

Suggested prompts:

- "Use the catalog tools to find the inventory fields that identify low-stock products."
- "Draft a reorder plan for online products below their reorder point."
- "Execute the approved inventory plan."
- "Show me what changed in the mock platform after execution."

These cover the explorer specialist, planner, executor, and coordinator routing
flow.

## Optional mock platform server

The runtime uses the generated `platform.db` directly by default, so Studio and
offline tests do not need a separate platform server.

Start the FastAPI mock platform only when you want to inspect or exercise the
HTTP API:

```bash
uv run inventory-scenario platform --host 127.0.0.1 --port 8123
```

Open:

```text
http://127.0.0.1:8123
```

Set the runtime platform URL only when you intentionally want runtime tools to
call the HTTP mock platform instead of local `platform.db`:

```bash
export INVENTORY_SCENARIO_PLATFORM_URL=http://127.0.0.1:8123
```

## Source layout

```text
inventory_scenario/
├── app.py                         # Studio import target
├── runtime.py                     # coordinator/planner/executor assembly
├── plans.py                       # typed plan and reference contracts
├── product_sets.py                # query-backed InventoryProductSet tool
├── action_dispatcher.py           # approved writes into the mock platform
├── prompts.py                     # agent role prompts
├── cli.py                         # tutorial command facade
└── support/
    ├── data_environment.py        # generated local data descriptor
    ├── seed.py                    # local target/platform setup
    ├── smoke.py                   # deterministic verification
    ├── mock_platform/             # optional FastAPI platform and local client
    └── dataset_artifacts/         # maintainer-only artifact publication tools
```

## Maintainer-only artifact publication

Ordinary users consume the published artifact. Dataset regeneration and
publication are maintainer-only operations.

For a new dataset version, publish the generated bundle to public object
storage, then update `data/manifest.example.json` with:

- public `.target.sqlite.zst` URL
- SHA-256 digest
- uncompressed byte size
- table row counts

Before calling the dataset public-ready, run:

```bash
uv run inventory-scenario verify-artifact --check-url
```

Expected: exits 0 only when the manifest has real artifact metadata and a
reachable public object.

## Common issues

| Symptom | Fix |
| --- | --- |
| Checksum mismatch | Delete `.data/inventory_scenario/artifacts/` and rerun seed. If it still fails, the manifest and public object do not match. |
| Missing `zstd` support | Install the `zstandard` Python package or the `zstd` CLI, then rerun seed. |
| Catalog search unavailable | Run the profile, knowledge ingest, and catalog index rebuild commands from Quick start. |
| `ANTHROPIC_API_KEY` missing during enrichment | Keep `--schema-only` profiling and omit `--extract-knowledge`, or export `ANTHROPIC_API_KEY` before enabling LLM enrichment. |
| `ANTHROPIC_API_KEY` missing when starting Studio | Export `ANTHROPIC_API_KEY` before building the default inventory scenario runtime. |
| Mock platform port conflict | Start Uvicorn with another `--port` and export the matching `INVENTORY_SCENARIO_PLATFORM_URL`. |
| Stale target or platform state | Rerun seed with `--reset`; this does not delete catalog/KV/index state. |
| Public artifact verifier fails | Publish the object storage artifact, replace placeholder manifest values with real values, then rerun the verifier. |

## Links

- [Tutorial walkthrough](https://flow-ai.com/docs/tutorials/inventory-scenario)
- [Configure a data environment](https://flow-ai.com/docs/guides/data-environment)
- [Profile and export a catalog](https://flow-ai.com/docs/guides/catalog-profiling)
- [Knowledge and documents](https://flow-ai.com/docs/guides/knowledge)
- [References and glimpses](https://flow-ai.com/docs/concepts/references)
- [Execute approved actions](https://flow-ai.com/docs/guides/action-dispatcher)
