import json

from fastapi.testclient import TestClient

import flowai_harness.studio.server as studio_server
from flowai_harness import (
    define_app,
    define_coordinator,
    define_runtime,
    define_tenant,
    define_workspace_runtime,
)
from flowai_harness.studio import StudioStore, create_studio_app


def _runtime_spec():
    coordinator = define_coordinator(
        name="coordinator",
        model="claude-sonnet-4-6",
        prompt="You coordinate data work.",
        routes=["analyst"],
    )
    analyst = {
        "name": "analyst",
        "role": "specialist",
        "model": "claude-haiku-4-5",
        "systemPrompt": "Inspect data.",
    }
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, analyst],
    )


def _data_environment(tmp_path):
    return {
        "target_database": {
            "kind": "sqlite",
            "url": f"sqlite:{tmp_path / 'target.db'}",
        },
        "catalog": {
            "kind": "sqlite",
            "url": f"sqlite:{tmp_path / 'catalog.db'}",
        },
        "kv": {
            "kind": "sqlite",
            "url": f"sqlite:{tmp_path / 'kv.db'}",
            "ensure_schema": True,
        },
    }


def _client(tmp_path, *, data_environment=True):
    store = StudioStore(None)
    binding = define_workspace_runtime(
        runtime_spec=_runtime_spec(),
        data_environment=_data_environment(tmp_path) if data_environment else None,
    )
    app = define_app(
        name="demo",
        workspaces={"default": binding},
        default_workspace="default",
    )
    return TestClient(create_studio_app(app, store=store)), store


def _sse_events(response_text):
    events = []
    for chunk in response_text.strip().split("\n\n"):
        data_lines = [
            line.removeprefix("data: ")
            for line in chunk.splitlines()
            if line.startswith("data: ")
        ]
        if data_lines:
            events.append(json.loads("".join(data_lines)))
    return events


def test_data_sources_expose_workspace_runtime_source(tmp_path):
    client, _store = _client(tmp_path)

    response = client.get("/api/workspaces/default/data/sources")

    assert response.status_code == 200
    source = response.json()["sources"][0]
    assert source["id"] == "workspace-runtime"
    assert source["databaseType"] == "sqlite"
    assert source["isActive"] is True

    capabilities = client.get("/api/workspaces/default/capabilities").json()["capabilities"]
    enabled = {capability["id"] for capability in capabilities if capability["enabled"]}
    assert {"data.sources", "data.profile", "knowledge.ingest"} <= enabled


def test_data_routes_reject_workspace_without_data_environment(tmp_path):
    client, _store = _client(tmp_path, data_environment=False)

    response = client.get("/api/workspaces/default/data/sources")

    assert response.status_code == 409
    assert response.json()["error"]["code"] == "data.environment_missing"


def test_discovery_tables_delegate_to_native_data_bridge(tmp_path, monkeypatch):
    client, _store = _client(tmp_path)
    calls = []

    def fake_list_tables(data_environment_json, schema):
        calls.append((json.loads(data_environment_json), schema))
        return json.dumps(
            {
                "tables": [
                    {
                        "schemaName": "main",
                        "tableName": "orders",
                        "tableType": "base_table",
                        "rowCount": 10,
                        "columnCount": 3,
                        "description": None,
                    }
                ]
            }
        )

    monkeypatch.setattr(
        studio_server._internal,
        "data_list_tables",
        fake_list_tables,
        raising=False,
    )

    response = client.get("/api/workspaces/default/data/discovery/tables?schema=main")

    assert response.status_code == 200
    assert response.json()["tables"][0]["tableName"] == "orders"
    assert calls[0][0]["targetDatabase"]["kind"] == "sqlite"
    assert calls[0][1] == "main"


def test_profile_table_stream_persists_run_events(tmp_path, monkeypatch):
    client, store = _client(tmp_path)

    def fake_profile_table(*_args):
        return json.dumps(
            {
                "jobId": "profile-table-1",
                "events": [
                    {"type": "started", "jobId": "profile-table-1"},
                    {
                        "type": "completed",
                        "summary": {
                            "tablesDiscovered": 1,
                            "columnsProfiled": 3,
                            "enumsExtracted": 0,
                            "relationshipsFound": 0,
                            "catalogItemsIndexed": 1,
                            "durationMs": 1,
                        },
                    },
                ],
            }
        )

    monkeypatch.setattr(
        studio_server._internal,
        "data_profile_table",
        fake_profile_table,
        raising=False,
    )

    response = client.post(
        "/api/workspaces/default/data/profile/table",
        json={"sourceId": "workspace-runtime", "tableName": "orders"},
    )

    assert response.status_code == 200
    assert [event["type"] for event in _sse_events(response.text)] == ["started", "completed"]
    persisted = store.list_run_events(
        app_id="demo",
        workspace_key="default",
        run_id="profile-table-1",
    )
    assert [event["kind"] for event in persisted] == ["started", "completed"]


def test_knowledge_ingest_projects_runtime_events_to_frontend_shape(tmp_path, monkeypatch):
    client, _store = _client(tmp_path)

    def fake_ingest_knowledge(*_args):
        return json.dumps(
            {
                "jobId": "knowledge-ingest-1",
                "events": [
                    {"type": "discovered", "total": 2},
                    {"type": "ingesting", "current": 1, "total": 2, "name": "rules.md"},
                    {
                        "type": "completed",
                        "scanned": 2,
                        "new": 1,
                        "skippedDuplicate": 1,
                        "errors": [],
                    },
                ],
            }
        )

    monkeypatch.setattr(
        studio_server._internal,
        "data_ingest_knowledge",
        fake_ingest_knowledge,
        raising=False,
    )

    response = client.post(
        "/api/workspaces/default/data/knowledge/ingest",
        json={"source": {"type": "localDirectory", "path": "/tmp/docs"}},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert events[0] == {"type": "discovered", "totalFiles": 2}
    assert events[1]["fileName"] == "rules.md"
    assert events[2]["documentsIngested"] == 1
    assert events[2]["documentsSkipped"] == 1


def test_connect_search_tools_and_metrics_delegate_to_native_bridge(tmp_path, monkeypatch):
    client, _store = _client(tmp_path)
    calls = []

    def fake_search(data_environment_json, query_json):
        calls.append(("search", json.loads(data_environment_json), json.loads(query_json)))
        return json.dumps({"search": {"items": [], "totalCount": 0, "queryTimeMs": 1}})

    def fake_list_tools(data_environment_json):
        calls.append(("tools", json.loads(data_environment_json)))
        return json.dumps(
            {
                "tools": [
                    {
                        "id": "search_catalog",
                        "toolId": "search_catalog",
                        "name": "search catalog",
                        "description": "Find tables",
                        "inputSchema": {"type": "object"},
                    }
                ]
            }
        )

    def fake_execute_tool(data_environment_json, tool_id, input_json):
        calls.append(("execute", json.loads(data_environment_json), tool_id, json.loads(input_json)))
        return json.dumps(
            {
                "result": {
                    "toolId": tool_id,
                    "success": True,
                    "data": {"results": []},
                }
            }
        )

    def fake_metrics(data_environment_json, query_json):
        calls.append(("metrics", json.loads(data_environment_json), json.loads(query_json)))
        return json.dumps({"metrics": [], "totalCount": 0})

    monkeypatch.setattr(studio_server._internal, "data_search_catalog", fake_search, raising=False)
    monkeypatch.setattr(studio_server._internal, "data_list_tools", fake_list_tools, raising=False)
    monkeypatch.setattr(studio_server._internal, "data_execute_tool", fake_execute_tool, raising=False)
    monkeypatch.setattr(studio_server._internal, "data_list_metrics", fake_metrics, raising=False)

    assert client.post("/api/workspaces/default/data/search", json={"query": "orders"}).status_code == 200
    assert client.get("/api/workspaces/default/tools").json()["tools"][0]["toolId"] == "search_catalog"
    assert (
        client.post(
            "/api/workspaces/default/tools/search_catalog/execute",
            json={"input": {"query": "orders"}},
        ).json()["result"]["success"]
        is True
    )
    assert client.get("/api/workspaces/default/data/metrics?query=revenue").status_code == 200
    assert client.post("/api/workspaces/default/data/import", json={}).status_code == 404

    assert [call[0] for call in calls] == ["search", "tools", "execute", "metrics"]
    assert calls[0][2]["query"] == "orders"
    assert calls[2][2] == "search_catalog"
    assert calls[3][2]["query"] == "revenue"
