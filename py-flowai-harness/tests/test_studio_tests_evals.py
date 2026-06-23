"""Studio tests & evals routes.

Run/stream exercise the genuine native ``run_eval``/``stream_eval`` bridge via the
mock interpreter, so there is no live model dependency.
"""

from __future__ import annotations

import json

from fastapi.testclient import TestClient

from flowai_harness import (
    AgentSpec,
    create_runtime,
    define_app,
    define_runtime,
    define_tenant,
    define_tool,
    define_workspace_runtime,
)
from flowai_harness.studio import StudioStore
from tests.studio_test_client import create_studio_test_client


def _spec():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Coordinate.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, planner],
        providers={"anthropic": {"apiKey": "unused"}},
    )


def _spec_with_eval_roles():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Coordinate.",
        routes=["planner", "executor", "insights"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    executor = AgentSpec(
        name="executor",
        role="executor",
        model="claude-sonnet-4-6",
        system_prompt="Execute.",
    )
    specialist = AgentSpec(
        name="insights",
        role="specialist",
        model="claude-sonnet-4-6",
        system_prompt="Analyze.",
    )
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, planner, executor, specialist],
        providers={"anthropic": {"apiKey": "unused"}},
    )


def _client(
    store: StudioStore,
    *,
    workspaces=("default",),
    spec_factory=_spec,
    runtime_kwargs: dict | None = None,
) -> TestClient:
    bindings = {}
    for key in workspaces:
        spec = spec_factory()
        kwargs = runtime_kwargs or {"testing": {"mock_response": "mocked eval response"}}
        runtime = create_runtime(spec, **kwargs)
        bindings[key] = define_workspace_runtime(runtime_spec=spec, runtime=runtime)
    app = define_app(
        name="demo", workspaces=bindings, default_workspace=workspaces[0]
    )
    return create_studio_test_client(app, store=store)


def _sse_events(text: str) -> list[dict]:
    events = []
    for chunk in text.strip().split("\n\n"):
        data = [
            line.removeprefix("data: ")
            for line in chunk.splitlines()
            if line.startswith("data: ")
        ]
        if data:
            events.append(json.loads("".join(data)))
    return events


def _test_case(test_id: str, **extra) -> dict:
    body = {"id": test_id, "input": "Assess the requested change"}
    body.update(extra)
    return body


# --- Tests routes -------------------------------------------------------


def test_test_case_crud_round_trips_arbitrary_ground_truth():
    client = _client(StudioStore(None))
    payload = _test_case(
        "tc-1",
        tags=["pricing"],
        expectedTrajectory=["buildPlan", "executePlan"],
        trajectoryMode="subsequence",
        groundTruth={"kind": "structured", "schema": "x.y", "payload": {"any": {"nested": [1, 2]}}},
    )
    created = client.post("/api/workspaces/default/tests", json=payload)
    assert created.status_code == 200
    stored = created.json()["test"]["testCase"]
    assert stored["structuredGroundTruth"]["data"] == {"any": {"nested": [1, 2]}}

    listed = client.get("/api/workspaces/default/tests").json()["tests"]
    assert [t["id"] for t in listed] == ["tc-1"]

    fetched = client.get("/api/workspaces/default/tests/tc-1").json()["test"]
    assert fetched["testCase"]["expectedTrajectory"] == ["buildPlan", "executePlan"]


def test_test_case_update_and_id_mismatch():
    client = _client(StudioStore(None))
    client.post("/api/workspaces/default/tests", json=_test_case("tc-1"))

    updated = client.put(
        "/api/workspaces/default/tests/tc-1",
        json=_test_case("tc-1", input="Updated prompt"),
    )
    assert updated.status_code == 200
    assert updated.json()["test"]["testCase"]["input"] == "Updated prompt"

    mismatch = client.put(
        "/api/workspaces/default/tests/tc-1", json=_test_case("tc-OTHER")
    )
    assert mismatch.status_code == 400
    assert mismatch.json()["error"]["code"] == "test.id_mismatch"


def test_test_case_delete_and_missing():
    client = _client(StudioStore(None))
    client.post("/api/workspaces/default/tests", json=_test_case("tc-1"))
    assert client.delete("/api/workspaces/default/tests/tc-1").status_code == 200
    assert client.get("/api/workspaces/default/tests/tc-1").status_code == 404
    assert client.delete("/api/workspaces/default/tests/tc-1").status_code == 404


def test_invalid_test_case_is_rejected():
    client = _client(StudioStore(None))
    # Missing required `input`.
    resp = client.post("/api/workspaces/default/tests", json={"id": "tc-1"})
    assert resp.status_code == 400
    assert resp.json()["error"]["code"] == "test.invalid"


def test_validate_scores_a_known_trajectory_sample():
    client = _client(StudioStore(None))
    client.post(
        "/api/workspaces/default/tests",
        json=_test_case("tc-1", expectedTrajectory=["a", "b"], trajectoryMode="subsequence"),
    )
    resp = client.post(
        "/api/workspaces/default/tests/tc-1/validate",
        json={"sample": {"actualTrajectory": ["a", "b"]}, "scorerPreset": "trajectory_only"},
    )
    assert resp.status_code == 200
    scored = resp.json()["scored"]
    assert scored["aggregate"] == 1.0
    assert scored["componentScores"][0]["scorerName"] == "trajectory"


def test_tools_endpoint_returns_inventory():
    client = _client(StudioStore(None))
    resp = client.get("/api/workspaces/default/tests/tools")
    assert resp.status_code == 200
    assert isinstance(resp.json()["tools"], list)


def test_tools_endpoint_returns_catalog_toolkit_inventory(tmp_path):
    def spec_factory():
        specialist = AgentSpec(
            name="catalog_reader",
            role="specialist",
            model="claude-sonnet-4-6",
            system_prompt="Use catalog tools.",
            toolkits=["catalog"],
        )
        return define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            providers={"anthropic": {"apiKey": "unused"}},
        )

    client = _client(
        StudioStore(None),
        spec_factory=spec_factory,
        runtime_kwargs={
            "testing": {"mock_response": "mocked eval response"},
            "data_environment": {
                "catalog": {"kind": "empty"},
                "catalog_search": {"index_path": str(tmp_path / "catalog-index")},
            },
        },
    )

    resp = client.get("/api/workspaces/default/tests/tools")

    assert resp.status_code == 200
    names = {tool["name"] for tool in resp.json()["tools"]}
    assert {
        "search_catalog",
        "get_catalog_entities",
        "list_schema_fields",
        "get_catalog_relations",
        "get_relation_paths_between",
        "sample_table_data",
        "execute_query",
    }.issubset(names)


def test_tests_are_workspace_scoped_across_routes():
    client = _client(StudioStore(None), workspaces=("default", "other"))
    client.post("/api/workspaces/default/tests", json=_test_case("tc-1"))
    assert client.get("/api/workspaces/other/tests").json()["tests"] == []
    assert client.get("/api/workspaces/other/tests/tc-1").status_code == 404


# --- Evals routes -------------------------------------------------------


def test_eval_capabilities_reflect_registered_agents():
    client = _client(StudioStore(None), spec_factory=_spec_with_eval_roles)

    resp = client.get("/api/workspaces/default/eval-capabilities")

    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["workspaceKey"] == "default"
    modes = body["modes"]
    by_mode = {mode["mode"]: mode for mode in modes if mode["mode"] != "specialist"}
    assert by_mode["sequential"]["agentId"] == "coordinator"
    assert by_mode["planner"]["agentId"] == "planner"
    assert by_mode["executor"]["agentId"] == "executor"
    specialists = [mode for mode in modes if mode["mode"] == "specialist"]
    assert specialists == [
        {
            "mode": "specialist",
            "label": "Insights",
            "description": "Evaluate the insights specialist directly.",
            "agentId": "insights",
            "role": "specialist",
            "targetAgentId": "insights",
        }
    ]


def _create_eval(client: TestClient, *, eval_id="ev-1", test_case_ids=("tc-1",)) -> dict:
    client.post("/api/workspaces/default/tests", json=_test_case("tc-1"))
    body = {
        "id": eval_id,
        "config": {"mode": "sequential", "samplesPerCase": 1, "concurrency": 1},
        "testCaseIds": list(test_case_ids),
    }
    resp = client.post("/api/workspaces/default/evals", json=body)
    assert resp.status_code == 200, resp.text
    return resp.json()["eval"]


def test_eval_create_list_get():
    client = _client(StudioStore(None))
    created = _create_eval(client)
    assert created["id"] == "ev-1"
    assert created["status"] == "created"
    assert created["testCaseIds"] == ["tc-1"]

    listed = client.get("/api/workspaces/default/evals").json()["evals"]
    assert [e["id"] for e in listed] == ["ev-1"]
    got = client.get("/api/workspaces/default/evals/ev-1").json()
    assert got["eval"]["config"]["samplesPerCase"] == 1


def test_eval_run_persists_artifact_and_status():
    store = StudioStore(None)
    client = _client(store)
    _create_eval(client)
    resp = client.post("/api/workspaces/default/evals/ev-1/run")
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["artifact"]["tenantId"] == "acme"
    assert body["artifact"]["testCases"][0]["testCaseId"] == "tc-1"

    got = client.get("/api/workspaces/default/evals/ev-1").json()
    assert got["eval"]["status"] == "completed"
    assert len(got["runs"]) == 1
    assert got["runs"][0]["runId"] == body["runId"]


def test_specialist_eval_run_uses_target_agent():
    client = _client(StudioStore(None), spec_factory=_spec_with_eval_roles)
    client.post(
        "/api/workspaces/default/tests",
        json=_test_case(
            "tc-1",
            finalResponse={
                "scorers": [
                    {
                        "id": "mentions_mocked_response",
                        "method": "contains",
                        "text": "mocked eval response",
                    }
                ],
                "passThreshold": 1.0,
            },
        ),
    )
    create = client.post(
        "/api/workspaces/default/evals",
        json={
            "id": "ev-specialist",
            "config": {
                "mode": "specialist",
                "targetAgentId": "insights",
                "samplesPerCase": 1,
                "concurrency": 1,
            },
            "testCaseIds": ["tc-1"],
        },
    )
    assert create.status_code == 200, create.text

    resp = client.post("/api/workspaces/default/evals/ev-specialist/run")

    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["artifact"]["mode"] == "specialist"
    assert body["artifact"]["metadata"]["scorerPreset"] == "specialist"
    assert body["artifact"]["metadata"]["scoreWeights"] == {"final_response": 1.0}
    sample = body["artifact"]["testCases"][0]["samples"][0]
    assert "mocked eval response" in sample["responseText"]


def test_specialist_eval_without_scoreable_expectations_is_rejected():
    client = _client(StudioStore(None), spec_factory=_spec_with_eval_roles)
    client.post("/api/workspaces/default/tests", json=_test_case("tc-empty"))
    create = client.post(
        "/api/workspaces/default/evals",
        json={
            "id": "ev-specialist-empty",
            "config": {
                "mode": "specialist",
                "targetAgentId": "insights",
                "samplesPerCase": 1,
                "concurrency": 1,
            },
            "testCaseIds": ["tc-empty"],
        },
    )
    assert create.status_code == 200, create.text

    resp = client.post("/api/workspaces/default/evals/ev-specialist-empty/run")

    assert resp.status_code == 400
    body = resp.json()
    assert body["error"]["code"] == "eval.run_failed"
    assert "no scoreable expectations" in body["error"]["message"]


def test_specialist_eval_captures_direct_tool_calls_in_trace():
    @define_tool("echo", {"value": str}, approval="never")
    async def echo(args, ctx):
        return {"echo": args["value"], "toolUseId": ctx["tool_use_id"]}

    def spec_factory():
        specialist = AgentSpec(
            name="insights",
            role="specialist",
            model="claude-sonnet-4-6",
            system_prompt="Use the requested tool.",
            tools=[echo],
        )
        return define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            providers={"anthropic": {"apiKey": "unused"}},
        )

    store = StudioStore(None)
    client = _client(store, spec_factory=spec_factory, runtime_kwargs={"interpreter": "scripted"})
    client.post(
        "/api/workspaces/default/tests",
        json=_test_case(
            "tc-tool",
            input=json.dumps({"tool": "echo", "args": {"value": "hello"}}),
            expectedTrajectory=["echo"],
        ),
    )
    create = client.post(
        "/api/workspaces/default/evals",
        json={
            "id": "ev-specialist-tool",
            "config": {
                "mode": "specialist",
                "targetAgentId": "insights",
                "samplesPerCase": 1,
                "concurrency": 1,
            },
            "testCaseIds": ["tc-tool"],
        },
    )
    assert create.status_code == 200, create.text

    resp = client.post("/api/workspaces/default/evals/ev-specialist-tool/run")

    assert resp.status_code == 200, resp.text
    sample = resp.json()["artifact"]["testCases"][0]["samples"][0]
    assert sample["actualTrajectory"] == ["echo"]
    assert sample["passed"] is True
    trace_id = sample["trace"]["traceId"]
    trace_resp = client.get(f"/api/workspaces/default/traces/{trace_id}")
    assert trace_resp.status_code == 200, trace_resp.text
    steps = trace_resp.json()["trace"]["trace"]["steps"]
    assert [step["toolName"] for step in steps] == ["echo"]
    assert steps[0]["arguments"]["value"] == {"value": "hello"}
    assert steps[0]["result"]["value"]["echo"] == "hello"


def test_specialist_eval_captures_catalog_tool_calls_in_trace(tmp_path):
    def spec_factory():
        specialist = AgentSpec(
            name="catalog_reader",
            role="specialist",
            model="claude-sonnet-4-6",
            system_prompt="Use the requested catalog tool.",
            toolkits=["catalog"],
        )
        return define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            providers={"anthropic": {"apiKey": "unused"}},
        )

    data_environment = {
        "catalog": {
            "kind": "inline",
            "entries": [
                {
                    "id": "table:products",
                    "itemType": "table",
                    "name": "products",
                    "qualified_name": "main.products",
                    "content": "Product sales and catalog attributes.",
                    "tags": ["sales"],
                    "related": [],
                    "metadata": {
                        "databaseId": "warehouse",
                        "schemaName": "main",
                        "tableName": "products",
                        "relationType": "base_table",
                        "rowCount": 10,
                        "columnCount": 2,
                        "preferredQuerySurface": True,
                    },
                }
            ],
        },
        "catalog_search": {"index_path": str(tmp_path / "catalog-index")},
    }
    store = StudioStore(None)
    client = _client(
        store,
        spec_factory=spec_factory,
        runtime_kwargs={"interpreter": "scripted", "data_environment": data_environment},
    )
    client.post(
        "/api/workspaces/default/tests",
        json=_test_case(
            "tc-catalog-tool",
            input=json.dumps(
                {
                    "tool": "get_catalog_entities",
                    "args": {"refs": [{"id": "table:products", "kind": "table"}]},
                }
            ),
            expectedTrajectory=["get_catalog_entities"],
        ),
    )
    create = client.post(
        "/api/workspaces/default/evals",
        json={
            "id": "ev-specialist-catalog-tool",
            "config": {
                "mode": "specialist",
                "targetAgentId": "catalog_reader",
                "samplesPerCase": 1,
                "concurrency": 1,
            },
            "testCaseIds": ["tc-catalog-tool"],
        },
    )
    assert create.status_code == 200, create.text

    resp = client.post("/api/workspaces/default/evals/ev-specialist-catalog-tool/run")

    assert resp.status_code == 200, resp.text
    sample = resp.json()["artifact"]["testCases"][0]["samples"][0]
    assert sample["actualTrajectory"] == ["get_catalog_entities"]
    assert sample["passed"] is True
    trace_id = sample["trace"]["traceId"]
    trace_resp = client.get(f"/api/workspaces/default/traces/{trace_id}")
    assert trace_resp.status_code == 200, trace_resp.text
    steps = trace_resp.json()["trace"]["trace"]["steps"]
    assert [step["toolName"] for step in steps] == ["get_catalog_entities"]
    assert steps[0]["arguments"]["value"]["refs"][0]["id"] == "table:products"
    assert steps[0]["result"]["value"]["entities"][0]["id"] == "table:products"


def test_specialist_eval_unknown_target_is_rejected():
    client = _client(StudioStore(None), spec_factory=_spec_with_eval_roles)
    client.post("/api/workspaces/default/tests", json=_test_case("tc-1"))
    create = client.post(
        "/api/workspaces/default/evals",
        json={
            "id": "ev-missing-specialist",
            "config": {
                "mode": "specialist",
                "targetAgentId": "missing",
                "samplesPerCase": 1,
                "concurrency": 1,
            },
            "testCaseIds": ["tc-1"],
        },
    )
    assert create.status_code == 200, create.text

    resp = client.post("/api/workspaces/default/evals/ev-missing-specialist/run")

    assert resp.status_code == 400
    assert resp.json()["error"]["code"] == "eval.invalid_target_agent"


def test_eval_run_persists_sample_trace_detail():
    store = StudioStore(None)
    client = _client(store)
    _create_eval(client)
    body = client.post("/api/workspaces/default/evals/ev-1/run").json()
    sample = body["artifact"]["testCases"][0]["samples"][0]
    trace_ref = sample["trace"]
    assert trace_ref["traceId"].startswith("trace-")

    trace_resp = client.get(f"/api/workspaces/default/traces/{trace_ref['traceId']}")
    assert trace_resp.status_code == 200, trace_resp.text
    trace_row = trace_resp.json()["trace"]
    assert trace_row["traceId"] == trace_ref["traceId"]
    assert trace_row["evalRunId"] == body["runId"]
    assert trace_row["testCaseId"] == "tc-1"
    assert trace_row["sampleIndex"] == 0
    assert trace_row["trace"]["scope"]["evalRunId"] == body["runId"]
    assert trace_row["trace"]["scope"]["testCaseId"] == "tc-1"
    assert trace_row["trace"]["status"] == "completed"

    listed = client.get(
        f"/api/workspaces/default/traces?evalRunId={body['runId']}&testCaseId=tc-1"
    ).json()["traces"]
    assert [row["traceId"] for row in listed] == [trace_ref["traceId"]]


def test_eval_run_persists_sample_chat_messages():
    client = _client(StudioStore(None))
    _create_eval(client)
    body = client.post("/api/workspaces/default/evals/ev-1/run").json()
    sample = body["artifact"]["testCases"][0]["samples"][0]
    assert "mocked eval response" in sample["responseText"]

    resp = client.get(f"/api/workspaces/default/threads/{sample['threadId']}/messages")
    assert resp.status_code == 200, resp.text
    messages = resp.json()["messages"]
    assert [message["role"] for message in messages] == ["user", "assistant"]
    assert messages[0]["content"] == "Assess the requested change"
    assert messages[1]["content"] == sample["responseText"]
    assert messages[1]["metadata"]["source"] == "eval"
    assert messages[1]["metadata"]["runId"] == body["runId"]


def test_eval_sample_chat_messages_rehydrate_tool_calls_from_trace():
    store = StudioStore(None)
    client = _client(store)
    store.upsert_thread(
        app_id="demo",
        workspace_key="default",
        thread_id="eval-thread-1",
        title="Eval sample",
    )
    metadata = {
        "source": "eval",
        "runId": "eval-run-1",
        "testCaseId": "tc-1",
        "sampleIndex": 0,
    }
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="eval-thread-1",
        role="user",
        content="Which product has the highest revenue?",
        metadata=metadata,
    )
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="eval-thread-1",
        role="assistant",
        content="Sparkling Water 12pk has the highest revenue.",
        metadata=metadata,
    )
    store.upsert_trace(
        app_id="demo",
        workspace_key="default",
        trace={
            "traceId": "trace-eval-chat",
            "workspaceId": "default",
            "stage": "runtime",
            "status": "completed",
            "scope": {
                "evalRunId": "eval-run-1",
                "testCaseId": "tc-1",
                "threadId": "eval-thread-1",
                "sampleIndex": 0,
            },
            "steps": [
                {
                    "ordinal": 0,
                    "actor": "assistant",
                    "toolName": "execute_query",
                    "toolCallId": "tool-call-1",
                    "arguments": {
                        "kind": "inline",
                        "value": {"query": "select product_name, revenue from orders"},
                    },
                    "result": {
                        "kind": "inline",
                        "value": {
                            "rows": [
                                {
                                    "product_name": "Sparkling Water 12pk",
                                    "revenue": 363.48,
                                }
                            ]
                        },
                    },
                }
            ],
            "startedAt": None,
            "completedAt": "2026-06-02T00:00:00Z",
            "provenance": {
                "kind": "eval_sample",
                "evalRunId": "eval-run-1",
                "testCaseId": "tc-1",
                "sampleIndex": 0,
                "stage": "runtime",
                "threadId": "eval-thread-1",
            },
        },
    )

    resp = client.get("/api/workspaces/default/threads/eval-thread-1/messages")

    assert resp.status_code == 200, resp.text
    assistant = resp.json()["messages"][1]
    assert assistant["parts"] == [
        {
            "type": "tool-invocation",
            "toolCallId": "tool-call-1",
            "toolName": "execute_query",
            "args": {"query": "select product_name, revenue from orders"},
            "state": "result",
            "result": {
                "rows": [
                    {
                        "product_name": "Sparkling Water 12pk",
                        "revenue": 363.48,
                    }
                ]
            },
        },
        {"type": "text", "text": "Sparkling Water 12pk has the highest revenue."},
    ]
    assert assistant["metadata"]["traceId"] == "trace-eval-chat"


def test_eval_stream_emits_envelopes_and_persists_final_artifact():
    client = _client(StudioStore(None))
    _create_eval(client)
    resp = client.get("/api/workspaces/default/evals/ev-1/stream")
    assert resp.status_code == 200
    events = _sse_events(resp.text)
    types = [e["type"] for e in events]
    assert types == [
        "evalStarted",
        "testCaseStarted",
        "sampleCompleted",
        "testCaseCompleted",
        "evalCompleted",
    ]
    runs = client.get("/api/workspaces/default/evals/ev-1").json()["runs"]
    assert len(runs) == 1


def test_eval_compare_two_runs():
    client = _client(StudioStore(None))
    _create_eval(client)
    first = client.post("/api/workspaces/default/evals/ev-1/run").json()["runId"]
    second = client.post("/api/workspaces/default/evals/ev-1/rerun").json()["runId"]
    assert first != second
    resp = client.get(
        f"/api/workspaces/default/evals/compare?left={first}&right={second}"
    )
    assert resp.status_code == 200
    comparison = resp.json()["comparison"]
    assert comparison["left"]["runId"] == first
    assert comparison["right"]["runId"] == second
    assert [tc["testCaseId"] for tc in comparison["testCases"]] == ["tc-1"]


def test_eval_cancel_records_cancelled_status():
    client = _client(StudioStore(None))
    _create_eval(client)
    resp = client.post("/api/workspaces/default/evals/ev-1/cancel")
    assert resp.status_code == 200
    assert resp.json()["status"] == "cancelled"
    assert client.get("/api/workspaces/default/evals/ev-1").json()["eval"]["status"] == "cancelled"
    # Unknown eval still 404s.
    assert client.post("/api/workspaces/default/evals/nope/cancel").status_code == 404


def test_eval_run_is_captured_in_run_summaries():
    # Eval streams persist normalized run events tagged eval.
    store = StudioStore(None)
    client = _client(store)
    _create_eval(client)
    resp = client.get("/api/workspaces/default/evals/ev-1/stream")
    assert resp.status_code == 200
    _ = resp.text  # drain the stream so events persist
    eval_runs = [
        r for r in store.list_runs(app_id="demo", workspace_key="default")
        if r["operation"] == "eval"
    ]
    assert len(eval_runs) == 1
    assert eval_runs[0]["status"] == "completed"


def test_eval_delete_removes_run():
    client = _client(StudioStore(None))
    _create_eval(client)
    assert client.delete("/api/workspaces/default/evals/ev-1").status_code == 200
    assert client.get("/api/workspaces/default/evals/ev-1").status_code == 404
    assert client.delete("/api/workspaces/default/evals/ev-1").status_code == 404


def test_eval_invalid_config_is_rejected():
    client = _client(StudioStore(None))
    resp = client.post(
        "/api/workspaces/default/evals",
        json={"config": {"scoreWeights": {"bogus_scorer": 1.0}}, "testCaseIds": []},
    )
    assert resp.status_code == 400
    assert resp.json()["error"]["code"] == "eval.invalid_config"


def test_eval_run_without_test_cases_is_rejected():
    client = _client(StudioStore(None))
    client.post(
        "/api/workspaces/default/evals",
        json={"id": "ev-empty", "config": {"mode": "sequential"}, "testCaseIds": []},
    )
    resp = client.post("/api/workspaces/default/evals/ev-empty/run")
    assert resp.status_code == 400
    assert resp.json()["error"]["code"] == "eval.no_test_cases"
