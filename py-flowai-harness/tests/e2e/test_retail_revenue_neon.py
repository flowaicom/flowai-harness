from __future__ import annotations

import asyncio
import json
import re
from decimal import Decimal

import pytest

from tests.e2e.runtime_helpers import catalog_runtime, collect, run_tool

pytestmark = [pytest.mark.e2e, pytest.mark.neon]


def test_retail_revenue_deterministic_neon_e2e(neon_e2e_run, monkeypatch):
    neon_e2e_run.progress.log("test start: deterministic retail revenue e2e")
    _prepare_catalog(neon_e2e_run)
    with neon_e2e_run.progress.step("install runtime connection env vars"):
        _install_runtime_env(monkeypatch, neon_e2e_run.env)

    with neon_e2e_run.progress.step("create scripted catalog runtime"):
        runtime = catalog_runtime(neon_e2e_run.data_environment, interpreter="scripted")
    scenario = neon_e2e_run.scenario
    metric = scenario.question("q1_net_revenue_by_region")

    with neon_e2e_run.progress.step("scripted tool search_catalog"):
        search = run_tool(
            runtime,
            "retail_reader",
            "search_catalog",
            {"query": "net revenue refund policy", "limit": 10},
        )
    search_names = {result["name"] for result in search["results"]}
    assert "Revenue Policy" in search_names
    assert "Refund Policy" in search_names

    with neon_e2e_run.progress.step("scripted tool search_catalog orders table"):
        table_search = run_tool(
            runtime,
            "retail_reader",
            "search_catalog",
            {
                "query": "orders table completed order status store id campaign",
                "kinds": ["table"],
                "limit": 5,
            },
        )
    assert any(
        result["qualified_name"] == f"{scenario.target_schema}.orders"
        for result in table_search["results"]
    )

    with neon_e2e_run.progress.step("scripted tool get_catalog_entities retail tables"):
        entities = run_tool(
            runtime,
            "retail_reader",
            "get_catalog_entities",
            {
                "refs": [
                    {
                        "qualified_name": f"{scenario.target_schema}.orders",
                        "kind": "table",
                    },
                    {
                        "qualified_name": f"{scenario.target_schema}.refunds",
                        "kind": "table",
                    },
                ]
            },
        )
    entity_names = {entity["name"] for entity in entities["entities"]}
    assert entity_names == {"orders", "refunds"}
    assert entities.get("missing", []) == []

    with neon_e2e_run.progress.step("scripted tool list_schema_fields all retail tables"):
        fields = run_tool(
            runtime,
            "retail_reader",
            "list_schema_fields",
            {
                "tables": [
                    {
                        "qualified_name": f"{scenario.target_schema}.{table}",
                        "kind": "table",
                    }
                    for table in scenario.profile_tables
                ],
                "limit_per_table": 50,
            },
        )
    _assert_schema_fields_match_scenario(scenario, fields)

    with neon_e2e_run.progress.step("scripted tool get_catalog_relations orders"):
        relations = run_tool(
            runtime,
            "retail_reader",
            "get_catalog_relations",
            {
                "refs": [
                    {
                        "qualified_name": f"{scenario.target_schema}.orders",
                        "kind": "table",
                    }
                ],
                "target_kinds": ["table"],
                "limit_per_ref": 50,
            },
        )
    _assert_relations_include(
        relations,
        [
            ("orders", "customers"),
            ("orders", "stores"),
            ("orders", "campaigns"),
            ("order_items", "orders"),
        ],
    )

    with neon_e2e_run.progress.step("scripted tool get_relation_paths_between orders to regions"):
        path = run_tool(
            runtime,
            "retail_reader",
            "get_relation_paths_between",
            {
                "from": {
                    "qualified_name": f"{scenario.target_schema}.orders",
                    "kind": "table",
                },
                "to": [
                    {
                        "qualified_name": f"{scenario.target_schema}.regions",
                        "kind": "table",
                    }
                ],
                "path_type": "any",
                "max_depth": 4,
            },
        )
    assert path["paths"][0]["found"] is True
    assert _path_step_table_names(path) == ["stores", "regions"]

    with neon_e2e_run.progress.step("scripted tool sample_table_data orders"):
        sample = run_tool(
            runtime,
            "retail_reader",
            "sample_table_data",
            {
                "table": {
                    "qualified_name": f"{scenario.target_schema}.orders",
                    "kind": "table",
                },
                "columns": ["id", "status", "store_id"],
                "limit": 3,
            },
        )
    assert sample["columns"] == ["id", "status", "store_id"]
    assert sample["row_count"] == 3
    assert all(row["status"] in {"completed", "cancelled"} for row in sample["rows"])

    with neon_e2e_run.progress.step("scripted tool execute_query metric validation"):
        rows = run_tool(
            runtime,
            "retail_reader",
            "execute_query",
            {
                "sql": metric.validation_sql,
                "limit": 10,
                "purpose": "verify Q1 2026 net revenue by region fixture truth",
            },
        )
    assert rows["columns"] == ["region", "net_revenue"]
    assert _row_map(rows) == metric.expected_rows

    with neon_e2e_run.progress.step("scripted tool execute_query write rejection"):
        write_attempt = run_tool(
            runtime,
            "retail_reader",
            "execute_query",
            {"sql": "DROP TABLE retail.orders"},
        )
    assert "error" in write_attempt
    assert "select" in write_attempt["error"].lower() or "read-only" in write_attempt["error"].lower()

    with neon_e2e_run.progress.step("scripted tool execute_query target intact check"):
        count = run_tool(
            runtime,
            "retail_reader",
            "execute_query",
            {"sql": "SELECT COUNT(*)::text AS count FROM retail.orders"},
        )
    assert count["rows"] == [{"count": "5"}]


@pytest.mark.live_llm
def test_live_llm_answers_discovery_and_metric_questions(neon_e2e_run, monkeypatch):
    neon_e2e_run.progress.log("test start: live LLM retail revenue e2e")
    _prepare_catalog(neon_e2e_run)
    with neon_e2e_run.progress.step("install runtime connection env vars"):
        _install_runtime_env(monkeypatch, neon_e2e_run.env)

    with neon_e2e_run.progress.step("create live Anthropic catalog runtime"):
        runtime = catalog_runtime(neon_e2e_run.data_environment, interpreter="anthropic")
    scenario = neon_e2e_run.scenario
    discovery = scenario.question("discover_net_revenue_inputs")
    metric = scenario.question("q1_net_revenue_by_region")

    with neon_e2e_run.progress.step("live LLM discovery question"):
        discovery_text, discovery_events = _run_live_specialist(
            runtime,
            f"{discovery.question}\nUse catalog tools before answering.",
        )
    assert _tool_names(discovery_events) & {"search_catalog", "list_schema_fields"}
    discovery_lower = discovery_text.lower()
    for table in discovery.expected_tables:
        assert table in discovery_lower
    _assert_mentions_any(discovery_lower, ["refund", "refunds", "refunded"], "refund policy")
    _assert_mentions_any(discovery_lower, ["tax", "vat"], "tax exclusion")
    _assert_store_region_path(discovery_lower)

    with neon_e2e_run.progress.step("live LLM metric question"):
        metric_text, metric_events = _run_live_specialist(
            runtime,
            f"{metric.question}\nUse catalog tools and execute read-only SQL before answering.",
        )
    _assert_live_metric_trajectory(metric_events)
    metric_lower = metric_text.lower()
    for region, expected in metric.expected_rows.items():
        assert region.lower() in metric_lower
        assert Decimal(expected) in _decimal_values(metric_text)
    assert Decimal(metric.expected_total) in _decimal_values(metric_text)
    assert "refund" in metric_lower


def _prepare_catalog(neon_e2e_run):
    scenario = neon_e2e_run.scenario
    env_path = str(neon_e2e_run.data_environment_path)

    with neon_e2e_run.progress.step("CLI profile estimate"):
        estimate = neon_e2e_run.run_cli(
            [
                "--data-environment",
                env_path,
                "--output",
                "json",
                "data",
                "profile",
                "estimate",
                "--tenant-id",
                neon_e2e_run.config.tenant_id,
                "--workspace-id",
                neon_e2e_run.config.workspace_id,
                *_profile_scope_args(scenario),
            ]
        )
    estimate_payload = json.loads(estimate.stdout)
    _assert_profile_estimate_matches_scenario(scenario, estimate_payload)

    profile_args = [
        "--data-environment",
        env_path,
        "--output",
        "ndjson",
        "data",
        "profile",
        "database",
        "--tenant-id",
        neon_e2e_run.config.tenant_id,
        "--workspace-id",
        neon_e2e_run.config.workspace_id,
        *_profile_scope_args(scenario),
        "--sample-size",
        "3",
    ]
    if not _live_enrichment_enabled():
        profile_args.append("--schema-only")
    with neon_e2e_run.progress.step("CLI profile database"):
        profile = neon_e2e_run.run_cli(profile_args, timeout=300)
    _assert_profile_events_match_scenario(scenario, _ndjson_events(profile.stdout))

    with neon_e2e_run.progress.step("CLI knowledge ingest documents"):
        ingest = neon_e2e_run.run_cli(
            [
                "--data-environment",
                env_path,
                "--output",
                "ndjson",
                "data",
                "knowledge",
                "ingest",
                "--tenant-id",
                neon_e2e_run.config.tenant_id,
                "--workspace-id",
                neon_e2e_run.config.workspace_id,
                "--database-id",
                scenario.database_id,
                "--local-dir",
                str(scenario.documents_dir),
                "--ext",
                "md",
            ],
            timeout=180,
        )
    _assert_ingest_events_match_scenario(scenario, _ndjson_events(ingest.stdout))

    with neon_e2e_run.progress.step("CLI catalog index rebuild"):
        rebuild = neon_e2e_run.run_cli(
            [
                "--data-environment",
                env_path,
                "--output",
                "json",
                "data",
                "catalog",
                "index",
                "rebuild",
                "--tenant-id",
                neon_e2e_run.config.tenant_id,
                "--workspace-id",
                neon_e2e_run.config.workspace_id,
            ],
            timeout=180,
        )
    rebuild_payload = json.loads(rebuild.stdout)
    assert rebuild_payload["indexedEntries"] >= len(scenario.profile_tables)

    with neon_e2e_run.progress.step("CLI catalog index doctor"):
        doctor = neon_e2e_run.run_cli(
            [
                "--data-environment",
                env_path,
                "--output",
                "json",
                "data",
                "catalog",
                "index",
                "doctor",
                "--tenant-id",
                neon_e2e_run.config.tenant_id,
                "--workspace-id",
                neon_e2e_run.config.workspace_id,
            ]
        )
    doctor_payload = json.loads(doctor.stdout)
    assert doctor_payload["health"]["status"] == "ready"


def _profile_scope_args(scenario):
    return [
        "--database-id",
        scenario.database_id,
        "--schema",
        scenario.target_schema,
    ]


def _row_map(result):
    return {
        row["region"]: f"{Decimal(str(row['net_revenue'])):.2f}"
        for row in result["rows"]
    }


def _ndjson_events(stdout):
    return [json.loads(line) for line in stdout.splitlines() if line.strip()]


def _assert_profile_estimate_matches_scenario(scenario, payload):
    assert payload["tableCount"] == len(scenario.profile_tables), (
        f"tableCount should match target fixture tables; got {payload}"
    )
    assert payload["columnCount"] == scenario.expected_column_count, (
        f"columnCount should match target fixture columns; got {payload}"
    )
    assert payload["tableCount"] <= scenario.limits["max_tables"]
    assert payload["columnCount"] <= scenario.limits["max_columns"]
    assert (
        payload["estimatedInputTokens"]
        <= scenario.limits["max_estimated_input_tokens"]
    )


def _assert_profile_events_match_scenario(scenario, events):
    profiled_columns = {
        event["tableName"]: event["columns"]
        for event in events
        if event.get("type") == "tableProfiled"
    }
    completed_tables = {
        event["tableName"]
        for event in events
        if event.get("type") == "tableCompleted"
    }
    expected_tables = set(scenario.profile_tables)
    missing_completed = sorted(expected_tables - completed_tables)
    assert not missing_completed, f"missing completed tables: {missing_completed}"
    extra_completed = sorted(completed_tables - expected_tables)
    assert not extra_completed, f"unexpected completed tables: {extra_completed}"

    for table, expected_columns in scenario.expected_columns.items():
        assert profiled_columns.get(table) == len(expected_columns), (
            f"{table} should profile {len(expected_columns)} columns; "
            f"got {profiled_columns.get(table)}"
        )

    completed_event = _single_event(events, "completed")
    summary = completed_event["summary"]
    assert summary["tablesDiscovered"] == len(scenario.profile_tables)
    assert summary["columnsProfiled"] == scenario.expected_column_count
    assert summary["relationshipsFound"] >= len(scenario.expected_relationships)
    assert summary["catalogItemsIndexed"] >= (
        len(scenario.profile_tables)
        + scenario.expected_column_count
        + len(scenario.expected_relationships)
    )


def _assert_ingest_events_match_scenario(scenario, events):
    expected_names = {
        document_path.rsplit("/", 1)[-1]
        for document_path in scenario.document_paths
    }
    ingested_names = {
        event["name"]
        for event in events
        if event.get("type") == "ingesting"
    }
    missing = sorted(expected_names - ingested_names)
    assert not missing, f"missing ingested documents: {missing}"

    completed = _single_event(events, "completed")
    summary = completed.get("summary", completed)
    assert summary["scanned"] == len(expected_names)
    assert summary["new"] == len(expected_names)
    assert summary["errors"] == []


def _single_event(events, event_type):
    matches = [event for event in events if event.get("type") == event_type]
    assert len(matches) == 1, f"expected one {event_type} event, got {matches}"
    return matches[0]


def _assert_schema_fields_match_scenario(scenario, result):
    tables = {
        table["table"]["name"]: [field["name"] for field in table["fields"]]
        for table in result["tables"]
    }
    assert set(tables) == set(scenario.profile_tables)
    for table, expected_columns in scenario.expected_columns.items():
        assert tables[table] == expected_columns


def _assert_relations_include(result, expected_pairs):
    edges = set()
    for item in result["results"]:
        source_table = item["source"]["details"].get("table_name")
        for relation in item["relations"]:
            target_table = relation["target"]["details"].get("table_name")
            relationship = relation.get("relationship")
            if relationship:
                details = relationship["details"]
                edges.add((details.get("source_table"), details.get("target_table")))
            if source_table and target_table:
                if relation["direction"] == "outgoing":
                    edges.add((source_table, target_table))
                elif relation["direction"] == "incoming":
                    edges.add((target_table, source_table))
    missing = sorted(set(expected_pairs) - edges)
    assert not missing, f"missing catalog relation edges: {missing}; got {sorted(edges)}"


def _path_step_table_names(result):
    return [
        step["entity"]["details"].get("table_name")
        for path in result["paths"]
        for step in path["steps"]
    ]


def _assert_mentions_any(text, alternatives, label):
    if not any(alternative in text for alternative in alternatives):
        raise AssertionError(f"expected {label}; looked for one of {alternatives!r} in {text!r}")


def _assert_store_region_path(text):
    direct_phrases = [
        "store region",
        "store's region",
        "orders.store_id",
        "store_id",
        "stores.region_id",
        "stores to regions",
        "stores and regions",
    ]
    if any(phrase in text for phrase in direct_phrases):
        return
    if "stores" in text and "regions" in text and ("join" in text or "region_id" in text):
        return
    raise AssertionError(
        "expected evidence that region reporting uses the store-to-region path"
    )


def _decimal_values(text):
    values = set()
    for match in re.finditer(r"(?<![A-Za-z0-9])\$?([0-9][0-9,]*(?:\.[0-9]+)?)(?![A-Za-z0-9])", text):
        values.add(Decimal(match.group(1).replace(",", "")))
    return values


def _install_runtime_env(monkeypatch, env):
    for key, value in env.items():
        monkeypatch.setenv(key, value)


def _run_live_specialist(runtime, prompt):
    events = asyncio.run(collect(runtime.run_specialist("retail_reader", prompt, thread_id="e2e-live")))
    text = "".join(event.get("text", "") for event in events if event.get("type") == "text")
    return text, events


def _tool_names(events):
    return {
        event["toolName"]
        for event in events
        if event.get("type") == "tool-invocation" and "toolName" in event
    }


def _assert_live_metric_trajectory(events):
    order = [
        event["toolName"]
        for event in events
        if event.get("type") == "tool-invocation" and "toolName" in event
    ]
    catalog_tools = {
        "search_catalog",
        "get_catalog_entities",
        "list_schema_fields",
        "get_catalog_relations",
        "get_relation_paths_between",
    }
    assert "execute_query" in order
    used_catalog_tools = [name for name in order if name in catalog_tools]
    assert used_catalog_tools, f"expected catalog tool before SQL; got {order}"
    assert min(order.index(name) for name in used_catalog_tools) < order.index("execute_query"), (
        f"expected catalog discovery before execute_query; got {order}"
    )


def _live_enrichment_enabled():
    import os

    return os.environ.get("FLOWAI_E2E_LIVE_ENRICHMENT") == "1"
