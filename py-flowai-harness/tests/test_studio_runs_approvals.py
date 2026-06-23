"""Runs inspection + approval inbox routes."""

from __future__ import annotations

from fastapi.testclient import TestClient

from flowai_harness import (
    AgentSpec,
    define_app,
    define_runtime,
    define_tenant,
    define_workspace_runtime,
)
from flowai_harness.studio import StudioStore
from flowai_harness.studio.sse import project_runtime_event
from tests.studio_test_client import create_studio_test_client


class ApprovalRuntime:
    """Fake runtime that emits an approval-required event then finishes."""

    def __init__(self):
        self.events = [
            {"type": "tool-invocation", "state": "call", "toolName": "search", "toolCallId": "tc1"},
            {
                "type": "approval-required",
                "data": {"id": "appr-1", "kind": "tool", "title": "Approve search", "target": "search"},
            },
            {"type": "finish", "reason": "stop"},
        ]
        self.approval_calls = []

    def query(self, prompt, thread_id, resume=None):
        return self._stream()

    def run_specialist(self, specialist, prompt, thread_id=None):
        return self._stream()

    async def respond_to_approval(self, approval_id, outcome, feedback=None, partial=None):
        self.approval_calls.append((approval_id, outcome, feedback, partial))

    async def _stream(self):
        for event in self.events:
            yield event


def _client(store: StudioStore, runtime=None) -> TestClient:
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
    binding = define_workspace_runtime(runtime_spec=spec, runtime=runtime or ApprovalRuntime())
    app = define_app(name="demo", workspaces={"default": binding}, default_workspace="default")
    return create_studio_test_client(app, store=store)


def _seed_chat(client: TestClient, thread_id="thread-1", message="Search"):
    resp = client.post(
        "/api/workspaces/default/agents/coordinator/stream",
        json={"message": message, "threadId": thread_id},
    )
    assert resp.status_code == 200
    _ = resp.text


def test_runs_list_and_detail():
    client = _client(StudioStore(None))
    _seed_chat(client)
    runs = client.get("/api/workspaces/default/runs").json()["runs"]
    assert len(runs) == 1
    run_id = runs[0]["runId"]
    assert runs[0]["operation"] == "chat"
    assert runs[0]["agentId"] == "coordinator"
    assert runs[0]["status"] == "completed"

    detail = client.get(f"/api/workspaces/default/runs/{run_id}")
    assert detail.status_code == 200
    assert detail.json()["run"]["runId"] == run_id


def test_run_events_and_since_seq_reconnect():
    client = _client(StudioStore(None))
    _seed_chat(client)
    run_id = client.get("/api/workspaces/default/runs").json()["runs"][0]["runId"]

    events = client.get(f"/api/workspaces/default/runs/{run_id}/events").json()["events"]
    assert len(events) >= 3
    last_seq = events[1]["seq"]
    tail = client.get(
        f"/api/workspaces/default/runs/{run_id}/events?since_seq={last_seq}"
    ).json()["events"]
    assert all(e["seq"] > last_seq for e in tail)


def test_run_not_found_is_404():
    client = _client(StudioStore(None))
    assert client.get("/api/workspaces/default/runs/missing").status_code == 404
    assert client.get("/api/workspaces/default/runs/missing/events").status_code == 404


def test_runs_are_workspace_scoped():
    store = StudioStore(None)
    client = _client(store)
    _seed_chat(client)
    run_id = client.get("/api/workspaces/default/runs").json()["runs"][0]["runId"]
    # A different workspace key is unknown to this single-workspace app.
    assert client.get(f"/api/workspaces/other/runs/{run_id}").status_code == 404


# --- Approval inbox ----------------------------------------------------


def test_approval_is_captured_and_listed():
    runtime = ApprovalRuntime()
    client = _client(StudioStore(None), runtime=runtime)
    _seed_chat(client)

    approvals = client.get("/api/workspaces/default/approvals").json()["approvals"]
    assert len(approvals) == 1
    assert approvals[0]["approvalId"] == "appr-1"
    assert approvals[0]["status"] == "pending"
    assert approvals[0]["threadId"] == "thread-1"

    detail = client.get("/api/workspaces/default/approvals/appr-1")
    assert detail.status_code == 200
    assert detail.json()["approval"]["payload"]["approvalId"] == "appr-1"


def test_approval_respond_resolves_and_delegates():
    runtime = ApprovalRuntime()
    client = _client(StudioStore(None), runtime=runtime)
    _seed_chat(client)

    resp = client.post(
        "/api/workspaces/default/approvals/appr-1/respond",
        json={"outcome": "approve", "feedback": "ok"},
    )
    assert resp.status_code == 200
    assert runtime.approval_calls == [("appr-1", "approve", "ok", None)]
    listed = client.get("/api/workspaces/default/approvals").json()["approvals"]
    assert listed[0]["status"] == "approve"


def test_approval_decision_projects_to_studio_event():
    kind, payload = project_runtime_event(
        {"type": "approval-decision", "data": {"id": "appr-1", "outcome": {"outcome": "approve"}}}
    )

    assert kind == "approval.decision"
    assert payload["approvalId"] == "appr-1"
    assert payload["status"] == "approve"


def test_approval_not_found_is_404():
    client = _client(StudioStore(None))
    assert client.get("/api/workspaces/default/approvals/missing").status_code == 404
