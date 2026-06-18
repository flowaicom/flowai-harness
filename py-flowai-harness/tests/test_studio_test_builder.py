"""tests/from-chat + builder trace extraction over run events."""

from __future__ import annotations

from fastapi.testclient import TestClient

from flowai_harness import (
    AgentSpec,
    define_app,
    define_runtime,
    define_tenant,
    define_workspace_runtime,
)
from flowai_harness.studio import StudioStore, create_studio_app


class ToolEmittingRuntime:
    """Fake runtime that emits a tool trajectory then finishes."""

    def __init__(self):
        self.events = [
            {"type": "tool-invocation", "state": "call", "toolName": "search", "toolCallId": "tc1", "args": {"q": "x"}},
            {"type": "tool-invocation", "state": "result", "toolName": "search", "toolCallId": "tc1", "result": {"ok": True}},
            {"type": "tool-agent", "state": "call", "agentName": "planner", "prompt": "plan it"},
            {"type": "tool-invocation", "state": "call", "toolName": "executePlan", "toolCallId": "tc2", "args": {}},
            {"type": "finish", "reason": "stop"},
        ]

    def query(self, prompt, thread_id, resume=None):
        return self._stream()

    def run_specialist(self, specialist, prompt, thread_id=None):
        return self._stream()

    async def respond_to_approval(self, *args, **kwargs):
        return None

    async def _stream(self):
        for event in self.events:
            yield event


def _client(store: StudioStore) -> TestClient:
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
    spec = define_runtime(tenant=define_tenant("acme", "v1"), agents=[coordinator, planner])
    binding = define_workspace_runtime(runtime_spec=spec, runtime=ToolEmittingRuntime())
    app = define_app(name="demo", workspaces={"default": binding}, default_workspace="default")
    return TestClient(create_studio_app(app, store=store))


def _seed_chat(client: TestClient, *, thread_id="thread-1", message="Search products") -> None:
    resp = client.post(
        "/api/workspaces/default/agents/coordinator/stream",
        json={"message": message, "threadId": thread_id},
    )
    assert resp.status_code == 200
    # drain the SSE stream so all events are persisted
    _ = resp.text


def test_thread_trace_extracts_tool_and_sub_agent_calls():
    client = _client(StudioStore(None))
    _seed_chat(client)
    trace = client.get(
        "/api/workspaces/default/tests/builder/threads/thread-1/trace"
    ).json()["trace"]
    assert trace["trajectory"] == ["search", "executePlan"]
    assert {c["toolName"] for c in trace["toolCalls"]} == {"search", "executePlan"}
    search_call = next(c for c in trace["toolCalls"] if c["toolName"] == "search")
    assert search_call["status"] == "completed"
    assert [s["targetAgentId"] for s in trace["subAgentCalls"]] == ["planner"]
    assert trace["resolvedActions"] == []


def test_thread_trace_unknown_thread_is_404():
    client = _client(StudioStore(None))
    assert (
        client.get(
            "/api/workspaces/default/tests/builder/threads/missing/trace"
        ).status_code
        == 404
    )


def test_from_chat_creates_draft_test_case():
    client = _client(StudioStore(None))
    _seed_chat(client, message="Search products")
    resp = client.post(
        "/api/workspaces/default/tests/from-chat", json={"threadId": "thread-1", "id": "tc-from-chat"}
    )
    assert resp.status_code == 200, resp.text
    test_case = resp.json()["test"]["testCase"]
    assert test_case["id"] == "tc-from-chat"
    assert test_case["input"] == "Search products"
    assert test_case["expectedTrajectory"] == ["search", "executePlan"]
    assert test_case["sourceThreadId"] == "thread-1"

    # The draft is persisted and listable.
    listed = client.get("/api/workspaces/default/tests").json()["tests"]
    assert [t["id"] for t in listed] == ["tc-from-chat"]


def test_chat_run_is_recorded_with_operation_and_agent():
    # Chat capture tags run events with operation + agent.
    store = StudioStore(None)
    client = _client(store)
    _seed_chat(client, thread_id="thread-1")
    runs = store.list_runs(app_id="demo", workspace_key="default")
    assert len(runs) == 1
    assert runs[0]["operation"] == "chat"
    assert runs[0]["agentId"] == "coordinator"
    assert runs[0]["status"] == "completed"  # terminal `finish` → run.completed


def test_from_chat_requires_thread_id_and_existing_thread():
    client = _client(StudioStore(None))
    assert client.post("/api/workspaces/default/tests/from-chat", json={}).status_code == 400
    assert (
        client.post(
            "/api/workspaces/default/tests/from-chat", json={"threadId": "missing"}
        ).status_code
        == 404
    )


class PlanStoringRuntime(ToolEmittingRuntime):
    """Fake runtime whose stream stores a typed plan via storePlan."""

    PLAN_ACTIONS = {
        "head": {
            "kind": "promotion_launch",
            "payload": {"product_ids": ["SKU-1", "SKU-2"], "discount_pct": 15},
            "references": [],
        },
        "tail": [
            {
                "kind": "price_change",
                "payload": {"product_id": "SKU-1", "new_price": 79.99},
                "references": [],
            }
        ],
    }

    def __init__(self):
        super().__init__()
        self.events = [
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "storePlan",
                "toolCallId": "tc1",
                "args": {"specName": "PromoPlan", "planId": "plan-1"},
            },
            {
                "type": "tool-invocation",
                "state": "result",
                "toolName": "storePlan",
                "toolCallId": "tc1",
                "result": {"actions": self.PLAN_ACTIONS, "approvedAt": None},
            },
            {"type": "finish", "reason": "stop"},
        ]


def test_from_chat_prefills_planned_action_ground_truth():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Coordinate.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner", role="planner", model="claude-sonnet-4-6", system_prompt="Plan."
    )
    spec = define_runtime(tenant=define_tenant("acme", "v1"), agents=[coordinator, planner])
    binding = define_workspace_runtime(runtime_spec=spec, runtime=PlanStoringRuntime())
    app = define_app(name="demo", workspaces={"default": binding}, default_workspace="default")
    client = TestClient(create_studio_app(app, store=StudioStore(None)))
    _seed_chat(client, message="Run the summer sale")

    resp = client.post(
        "/api/workspaces/default/tests/from-chat", json={"threadId": "thread-1"}
    )
    assert resp.status_code == 200, resp.text
    test_case = resp.json()["test"]["testCase"]

    ground_truth = test_case["structuredGroundTruth"]
    assert ground_truth["kind"] == "structured"
    payload = ground_truth["data"]
    assert payload["payloadMatch"] == "subset"
    assert payload["plannedActions"] == [
        {
            "type": "promotion_launch",
            "payload": {"product_ids": ["SKU-1", "SKU-2"], "discount_pct": 15},
        },
        {
            "type": "price_change",
            "payload": {"product_id": "SKU-1", "new_price": 79.99},
        },
    ]


def test_from_chat_without_plan_leaves_ground_truth_empty():
    client = _client(StudioStore(None))
    _seed_chat(client, message="Search products")
    resp = client.post(
        "/api/workspaces/default/tests/from-chat", json={"threadId": "thread-1"}
    )
    assert resp.status_code == 200, resp.text
    assert resp.json()["test"]["testCase"].get("structuredGroundTruth") is None


def test_tool_catalog_includes_role_default_tools():
    # Role-default tools are composed at runtime and not listed in
    # ``toolkits``; the catalog must still surface them so planner/executor
    # trajectory steps like storePlan/executePlan do not validate as Unknown.
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Coordinate.",
        routes=["planner", "executor"],
    )
    planner = AgentSpec(
        name="planner", role="planner", model="claude-sonnet-4-6", system_prompt="Plan."
    )
    executor = AgentSpec(
        name="executor", role="executor", model="claude-sonnet-4-6", system_prompt="Run."
    )
    spec = define_runtime(
        tenant=define_tenant("acme", "v1"), agents=[coordinator, planner, executor]
    )
    binding = define_workspace_runtime(runtime_spec=spec, runtime=ToolEmittingRuntime())
    app = define_app(name="demo", workspaces={"default": binding}, default_workspace="default")
    client = TestClient(create_studio_app(app, store=StudioStore(None)))

    resp = client.get("/api/workspaces/default/tests/tools")
    assert resp.status_code == 200, resp.text
    names = {tool["name"] for tool in resp.json()["tools"]}
    assert {
        "call_agent",
        "storePlan",
        "getPlan",
        "executePlan",
        "resolveRef",
        "glimpseRef",
    } <= names
