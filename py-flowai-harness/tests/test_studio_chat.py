import asyncio
import json
from types import SimpleNamespace

import pytest
from fastapi.testclient import TestClient

from flowai_harness import (
    define_app,
    define_coordinator,
    define_plan,
    define_planner,
    define_runtime,
    define_specialist,
    define_tenant,
    define_workspace_runtime,
)
from flowai_harness.studio import StudioStore
from flowai_harness.studio.server import _stream_chat
from tests.studio_test_client import create_studio_test_client


class FakeRuntime:
    def __init__(self, events=None):
        self.events = events or [
            {"type": "step-start"},
            {"type": "text", "text": "hello from runtime"},
            {"type": "finish", "reason": "stop"},
        ]
        self.query_calls = []
        self.specialist_calls = []
        self.approval_calls = []

    def query(self, prompt, thread_id, resume=None):
        self.query_calls.append(
            {"prompt": prompt, "thread_id": thread_id, "resume": resume}
        )
        return self._stream()

    def run_specialist(self, specialist, prompt, thread_id=None):
        self.specialist_calls.append(
            {"specialist": specialist, "prompt": prompt, "thread_id": thread_id}
        )
        return self._stream()

    async def respond_to_approval(self, approval_id, outcome, feedback=None, partial=None):
        self.approval_calls.append(
            {
                "approval_id": approval_id,
                "outcome": outcome,
                "feedback": feedback,
                "partial": partial,
            }
        )

    async def _stream(self):
        for event in self.events:
            yield event


class FakeCancelableStream:
    def __init__(self):
        self.cancelled = False

    def cancel(self):
        self.cancelled = True


class FakeCancelledRuntime(FakeRuntime):
    async def _stream(self):
        yield {"type": "text", "text": "partial answer"}
        raise asyncio.CancelledError()


class FakeRequest:
    async def is_disconnected(self):
        return False


def _runtime_spec():
    scenario_plan = define_plan("ScenarioPlan", {"type": "object"})
    specialist = define_specialist(
        name="insights",
        model="claude-sonnet-4-6",
        prompt="You inspect data.",
    )
    planner = define_planner(
        name="planner",
        model="claude-sonnet-4-6",
        prompt="You plan.",
        plan=scenario_plan,
    )
    coordinator = define_coordinator(
        name="scenario_coordinator",
        model="claude-sonnet-4-6",
        prompt="You coordinate work.",
        routes=["insights"],
    )
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, specialist, planner],
    )


def _client(fake_runtime=None, *, workspace_key="default", store=None):
    binding = define_workspace_runtime(
        runtime_spec=_runtime_spec(),
        runtime=fake_runtime or FakeRuntime(),
    )
    app = define_app(
        name="demo",
        workspaces={workspace_key: binding},
        default_workspace=workspace_key,
    )
    return create_studio_test_client(app, store=store or StudioStore(None))


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


def test_coordinator_chat_stream_persists_thread_messages_and_events():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Draft a plan", "threadId": "thread-1"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert [event["kind"] for event in events] == [
        "runtime.event",
        "message.delta",
        "runtime.finish",
        "run.completed",
    ]
    assert fake_runtime.query_calls == [
        {"prompt": "Draft a plan", "thread_id": "thread-1", "resume": None}
    ]

    threads = client.get("/api/workspaces/default/threads").json()["threads"]
    assert threads[0]["threadId"] == "thread-1"
    assert threads[0]["title"] == "Draft a plan"

    messages = client.get(
        "/api/workspaces/default/threads/thread-1/messages"
    ).json()["messages"]
    assert [(message["role"], message["content"]) for message in messages] == [
        ("user", "Draft a plan"),
        ("assistant", "hello from runtime"),
    ]
    assert messages[1]["metadata"]["parts"] == [{"type": "text", "text": "hello from runtime"}]


def test_chat_stream_uses_client_supplied_run_id():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Draft a plan", "threadId": "thread-1", "runId": "run-client"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert {event["runId"] for event in events} == {"run-client"}


def test_cancel_run_endpoint_invokes_active_runtime_stream_cancel():
    store = StudioStore(None)
    client = _client(store=store)
    stream = FakeCancelableStream()
    client.app.state.active_chat_runs[("default", "run-cancel")] = SimpleNamespace(
        stream=stream,
        thread_id="thread-1",
        agent_id="scenario_coordinator",
        cancel_recorded=False,
    )

    response = client.post("/api/workspaces/default/runs/run-cancel/cancel")

    assert response.status_code == 200
    assert response.json() == {
        "workspaceKey": "default",
        "runId": "run-cancel",
        "status": "cancelled",
        "cancelled": True,
    }
    assert stream.cancelled is True
    assert client.app.state.active_chat_runs[("default", "run-cancel")].cancel_recorded is True


def test_chat_stream_persists_partial_assistant_message_when_cancelled_mid_stream():
    store = StudioStore(None)
    runtime = FakeCancelledRuntime()
    binding = define_workspace_runtime(runtime_spec=_runtime_spec(), runtime=runtime)
    app = define_app(
        name="demo",
        workspaces={"default": binding},
        default_workspace="default",
    )
    active_chat_runs = {}

    async def collect_until_cancelled():
        chunks = []
        with pytest.raises(asyncio.CancelledError):
            async for chunk in _stream_chat(
                app=app,
                binding=binding,
                agent=SimpleNamespace(name="scenario_coordinator", role="coordinator"),
                prompt="Start a long answer",
                thread_id="thread-cancelled-partial",
                run_id="run-cancelled-partial",
                request=FakeRequest(),
                store=store,
                legacy_messages=False,
                active_chat_runs=active_chat_runs,
            ):
                chunks.append(chunk)
        return chunks

    chunks = asyncio.run(collect_until_cancelled())

    assert [event["kind"] for event in _sse_events("".join(chunks))] == [
        "message.delta"
    ]
    messages = store.list_messages(
        app_id="demo", workspace_key="default", thread_id="thread-cancelled-partial"
    )
    assert [(message["role"], message["content"]) for message in messages] == [
        ("user", "Start a long answer"),
        ("assistant", "partial answer"),
    ]
    assert messages[1]["metadata"] == {
        "runId": "run-cancelled-partial",
        "parts": [{"type": "text", "text": "partial answer"}],
        "status": "cancelled",
    }


def test_chat_stream_persists_partial_assistant_message_when_client_closes_stream():
    store = StudioStore(None)
    runtime = FakeRuntime(events=[{"type": "text", "text": "partial answer"}])
    binding = define_workspace_runtime(runtime_spec=_runtime_spec(), runtime=runtime)
    app = define_app(
        name="demo",
        workspaces={"default": binding},
        default_workspace="default",
    )
    active_chat_runs = {}

    async def receive_one_chunk_then_close():
        stream = _stream_chat(
            app=app,
            binding=binding,
            agent=SimpleNamespace(name="scenario_coordinator", role="coordinator"),
            prompt="Start a long answer",
            thread_id="thread-client-closed-partial",
            run_id="run-client-closed-partial",
            request=FakeRequest(),
            store=store,
            legacy_messages=False,
            active_chat_runs=active_chat_runs,
        )
        first_chunk = await anext(stream)
        await stream.aclose()
        return first_chunk

    first_chunk = asyncio.run(receive_one_chunk_then_close())

    assert [event["kind"] for event in _sse_events(first_chunk)] == ["message.delta"]
    messages = store.list_messages(
        app_id="demo", workspace_key="default", thread_id="thread-client-closed-partial"
    )
    assert [(message["role"], message["content"]) for message in messages] == [
        ("user", "Start a long answer"),
        ("assistant", "partial answer"),
    ]
    assert messages[1]["metadata"] == {
        "runId": "run-client-closed-partial",
        "parts": [{"type": "text", "text": "partial answer"}],
        "status": "cancelled",
    }


def test_chat_stream_persists_partial_assistant_message_when_cancelled_runtime_reports_error():
    store = StudioStore(None)
    runtime = FakeRuntime(
        events=[
            {"type": "text", "text": "partial answer"},
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "call_agent",
                "toolInvocationId": "call-agent-1",
                "args": {"agent": "data_analyst", "prompt": "List all products"},
            },
            {
                "type": "tool-agent",
                "state": "call",
                "agentName": "data_analyst",
                "toolInvocationId": "call-agent-1",
            },
            {"type": "error", "error": {"message": "Request cancelled"}},
        ]
    )
    binding = define_workspace_runtime(runtime_spec=_runtime_spec(), runtime=runtime)
    app = define_app(
        name="demo",
        workspaces={"default": binding},
        default_workspace="default",
    )
    active_chat_runs = {}

    async def receive_one_chunk_then_runtime_cancellation_error():
        stream = _stream_chat(
            app=app,
            binding=binding,
            agent=SimpleNamespace(name="scenario_coordinator", role="coordinator"),
            prompt="Start a long answer",
            thread_id="thread-cancel-error-partial",
            run_id="run-cancel-error-partial",
            request=FakeRequest(),
            store=store,
            legacy_messages=False,
            active_chat_runs=active_chat_runs,
        )
        chunks = [await anext(stream)]
        active_chat_runs[("default", "run-cancel-error-partial")].cancel_recorded = True
        async for chunk in stream:
            chunks.append(chunk)
        return chunks

    chunks = asyncio.run(receive_one_chunk_then_runtime_cancellation_error())

    assert [event["kind"] for event in _sse_events("".join(chunks))] == [
        "message.delta",
        "tool.call.started",
        "sub_agent.call.started",
        "run.cancelled",
    ]
    messages = store.list_messages(
        app_id="demo", workspace_key="default", thread_id="thread-cancel-error-partial"
    )
    assert [(message["role"], message["content"]) for message in messages] == [
        ("user", "Start a long answer"),
        ("assistant", "partial answer"),
    ]
    assert messages[1]["metadata"] == {
        "runId": "run-cancel-error-partial",
        "parts": [
            {"type": "text", "text": "partial answer"},
            {
                "type": "tool-invocation",
                "toolCallId": "call-agent-1",
                "toolName": "call_agent",
                "args": {"agent": "data_analyst", "prompt": "List all products"},
                "state": "cancelled",
            },
            {
                "type": "tool-agent",
                "toolCallId": "call-agent-1",
                "agentName": "data_analyst",
                "state": "cancelled",
            },
        ],
        "status": "cancelled",
    }
    assert (
        store.get_run(
            app_id="demo",
            workspace_key="default",
            run_id="run-cancel-error-partial",
        )["status"]
        == "cancelled"
    )


def test_delete_thread_removes_persisted_thread_messages_and_events():
    store = StudioStore(None)
    store.upsert_thread(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        title="Delete me",
    )
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        role="user",
        content="hello",
    )
    store.append_run_event(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        run_id="run-delete",
        seq=1,
        kind="message.delta",
        event={"seq": 1, "kind": "message.delta"},
        raw_event={"type": "text", "text": "hello"},
    )

    store.delete_thread(app_id="demo", workspace_key="default", thread_id="thread-delete")

    assert store.list_threads(app_id="demo", workspace_key="default") == []
    assert (
        store.list_messages(
            app_id="demo", workspace_key="default", thread_id="thread-delete"
        )
        == []
    )
    assert (
        store.list_thread_run_events(
            app_id="demo", workspace_key="default", thread_id="thread-delete"
        )
        == []
    )


def test_delete_thread_endpoint_removes_thread_and_returns_not_found_afterwards():
    store = StudioStore(None)
    client = _client(store=store)
    store.upsert_thread(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        title="Delete me",
    )
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        role="user",
        content="hello",
    )
    store.append_run_event(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-delete",
        run_id="run-delete",
        seq=1,
        kind="message.delta",
        event={"seq": 1, "kind": "message.delta"},
        raw_event={"type": "text", "text": "hello"},
    )

    response = client.delete("/api/workspaces/default/threads/thread-delete")

    assert response.status_code == 200
    assert response.json() == {
        "workspaceKey": "default",
        "threadId": "thread-delete",
        "deleted": True,
    }
    assert client.get("/api/workspaces/default/threads/thread-delete").status_code == 404
    assert client.get("/api/workspaces/default/threads/thread-delete/messages").status_code == 404
    assert (
        store.list_thread_run_events(
            app_id="demo", workspace_key="default", thread_id="thread-delete"
        )
        == []
    )


def test_chat_stream_keeps_sub_agent_completion_after_nested_finish():
    fake_runtime = FakeRuntime(
        events=[
            {
                "type": "tool-agent",
                "state": "call",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
            {"type": "finish", "reason": "stop"},
            {
                "type": "tool-agent",
                "state": "result",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
        ]
    )
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Ask insights", "threadId": "thread-agent"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert [event["kind"] for event in events] == [
        "sub_agent.call.started",
        "runtime.finish",
        "sub_agent.call.completed",
        "run.completed",
    ]
    assert events[0]["payload"]["toolCallId"] == "agent-call-1"
    assert events[2]["payload"]["toolCallId"] == "agent-call-1"


def test_chat_persistence_hides_nested_text_and_preserves_tool_parts():
    fake_runtime = FakeRuntime(
        events=[
            {"type": "text", "text": "Routing to analyst. "},
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "call_agent",
                "toolInvocationId": "call-agent-1",
                "args": {"agent": "insights"},
            },
            {
                "type": "tool-agent",
                "state": "call",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
            {"type": "text", "text": "Nested analyst answer that should not persist. "},
            {
                "type": "tool-agent",
                "state": "result",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
            {
                "type": "tool-invocation",
                "state": "result",
                "toolName": "call_agent",
                "toolInvocationId": "call-agent-1",
                "result": {"response": "Nested analyst answer that should not persist."},
            },
            {"type": "text", "text": "Coordinator final answer."},
            {"type": "finish", "reason": "stop"},
        ]
    )
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Ask insights", "threadId": "thread-parts"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert [event["kind"] for event in events] == [
        "message.delta",
        "tool.call.started",
        "sub_agent.call.started",
        "runtime.event",
        "sub_agent.call.completed",
        "tool.call.completed",
        "message.delta",
        "runtime.finish",
        "run.completed",
    ]

    messages = client.get(
        "/api/workspaces/default/threads/thread-parts/messages"
    ).json()["messages"]
    assistant = messages[1]
    assert assistant["content"] == "Routing to analyst. Coordinator final answer."
    assert assistant["metadata"]["parts"] == [
        {"type": "text", "text": "Routing to analyst. "},
        {
            "type": "tool-agent",
            "toolCallId": "agent-call-1",
            "agentName": "insights",
            "state": "result",
        },
        {
            "type": "tool-invocation",
            "toolCallId": "call-agent-1",
            "toolName": "call_agent",
            "args": {"agent": "insights"},
            "state": "result",
            "result": {"response": "Nested analyst answer that should not persist."},
        },
        {"type": "text", "text": "Coordinator final answer."},
    ]


def test_chat_stream_hides_entrypoint_agent_lifecycle():
    fake_runtime = FakeRuntime(
        events=[
            {
                "type": "tool-agent",
                "state": "call",
                "agentName": "scenario_coordinator",
                "toolInvocationId": "coordinator-call-1",
            },
            {"type": "text", "text": "Routing to analyst. "},
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "call_agent",
                "toolInvocationId": "call-agent-1",
                "args": {"agent": "insights", "prompt": "Summarize products"},
            },
            {
                "type": "tool-agent",
                "state": "call",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
            {"type": "text", "text": "Nested analyst answer that should not persist. "},
            {
                "type": "tool-agent",
                "state": "result",
                "agentName": "insights",
                "toolInvocationId": "agent-call-1",
            },
            {
                "type": "tool-invocation",
                "state": "result",
                "toolName": "call_agent",
                "toolInvocationId": "call-agent-1",
                "result": {"response": "Products summarized."},
            },
            {"type": "text", "text": "Coordinator final answer."},
            {
                "type": "tool-agent",
                "state": "result",
                "agentName": "scenario_coordinator",
                "toolInvocationId": "coordinator-call-1",
            },
            {"type": "finish", "reason": "stop"},
        ]
    )
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Ask insights", "threadId": "thread-entrypoint"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert [event["kind"] for event in events] == [
        "message.delta",
        "tool.call.started",
        "sub_agent.call.started",
        "runtime.event",
        "sub_agent.call.completed",
        "tool.call.completed",
        "message.delta",
        "runtime.finish",
        "run.completed",
    ]
    assert all(
        event["payload"].get("targetAgentId") != "scenario_coordinator"
        for event in events
        if isinstance(event.get("payload"), dict)
    )

    messages = client.get(
        "/api/workspaces/default/threads/thread-entrypoint/messages"
    ).json()["messages"]
    assistant = messages[1]
    assert assistant["content"] == "Routing to analyst. Coordinator final answer."
    assert all(
        part.get("agentName") != "scenario_coordinator"
        for part in assistant["metadata"]["parts"]
    )
    assert any(
        part.get("type") == "tool-agent" and part.get("agentName") == "insights"
        for part in assistant["metadata"]["parts"]
    )


def test_messages_endpoint_hides_persisted_entrypoint_agent_lifecycle_parts():
    store = StudioStore(None)
    client = _client(store=store)
    store.upsert_thread(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-historical-entrypoint",
        title="Historical entrypoint part",
    )
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-historical-entrypoint",
        role="assistant",
        content="Historical response.",
        metadata={
            "parts": [
                {
                    "type": "tool-agent",
                    "toolCallId": "agent-call-1",
                    "agentName": "insights",
                    "state": "result",
                },
                {
                    "type": "tool-agent",
                    "toolCallId": "coordinator-call-1",
                    "agentName": "scenario_coordinator",
                    "state": "result",
                },
                {"type": "text", "text": "Historical response."},
            ]
        },
    )

    response = client.get(
        "/api/workspaces/default/threads/thread-historical-entrypoint/messages"
    )

    assert response.status_code == 200
    assistant = response.json()["messages"][0]
    assert [part.get("agentName") for part in assistant["parts"]] == ["insights", None]
    assert [part.get("agentName") for part in assistant["metadata"]["parts"]] == [
        "insights",
        None,
    ]


def test_chat_stream_deduplicates_repeated_tool_lifecycle_events():
    store = StudioStore(None)
    fake_runtime = FakeRuntime(
        events=[
            {"type": "text", "text": "Searching. "},
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "search_catalog",
                "toolInvocationId": "tool-1",
                "args": {"query": "products"},
            },
            {
                "type": "tool-invocation",
                "state": "call",
                "toolName": "search_catalog",
                "toolInvocationId": "tool-1",
                "args": {"query": "products"},
            },
            {
                "type": "tool-invocation",
                "state": "result",
                "toolName": "search_catalog",
                "toolInvocationId": "tool-1",
                "args": {"query": "products"},
                "result": {"results": [{"name": "dim_products"}]},
            },
            {
                "type": "tool-invocation",
                "state": "result",
                "toolName": "search_catalog",
                "toolInvocationId": "tool-1",
                "args": {"query": "products"},
                "result": {"results": [{"name": "dim_products"}]},
            },
            {"type": "text", "text": "Done."},
            {"type": "finish", "reason": "stop"},
        ]
    )
    client = _client(fake_runtime, store=store)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={"message": "Find products", "threadId": "thread-dedupe"},
    )

    assert response.status_code == 200
    events = _sse_events(response.text)
    assert [event["kind"] for event in events] == [
        "message.delta",
        "tool.call.started",
        "tool.call.completed",
        "message.delta",
        "runtime.finish",
        "run.completed",
    ]

    persisted_events = store.list_thread_run_events(
        app_id="demo", workspace_key="default", thread_id="thread-dedupe"
    )
    assert [
        event["kind"]
        for event in persisted_events
        if event["kind"] in {"tool.call.started", "tool.call.completed"}
    ] == ["tool.call.started", "tool.call.completed"]

    messages = client.get(
        "/api/workspaces/default/threads/thread-dedupe/messages"
    ).json()["messages"]
    assistant = messages[1]
    tool_parts = [
        part
        for part in assistant["metadata"]["parts"]
        if part["type"] == "tool-invocation"
    ]
    assert tool_parts == [
        {
            "type": "tool-invocation",
            "toolCallId": "tool-1",
            "toolName": "search_catalog",
            "args": {"query": "products"},
            "state": "result",
            "result": {"results": [{"name": "dim_products"}]},
        }
    ]


def test_list_messages_rehydrates_legacy_empty_tool_args_from_run_events():
    store = StudioStore(None)
    client = _client(store=store)
    store.upsert_thread(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-legacy-args",
        title="Legacy args",
    )
    store.append_message(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-legacy-args",
        role="assistant",
        content="done",
        metadata={
            "runId": "run-legacy-args",
            "parts": [
                {
                    "type": "tool-invocation",
                    "toolCallId": "tool-1",
                    "toolName": "execute_query",
                    "args": {},
                    "state": "result",
                    "result": {"row_count": 1},
                }
            ],
        },
    )
    store.append_run_event(
        app_id="demo",
        workspace_key="default",
        thread_id="thread-legacy-args",
        run_id="run-legacy-args",
        seq=1,
        kind="tool.call.started",
        event={
            "schemaVersion": "harness-studio/v1",
            "workspaceKey": "default",
            "runId": "run-legacy-args",
            "threadId": "thread-legacy-args",
            "agentId": "scenario_coordinator",
            "seq": 1,
            "kind": "tool.call.started",
            "payload": {
                "toolCallId": "tool-1",
                "toolName": "execute_query",
                "arguments": {"query": "select * from orders"},
            },
        },
        raw_event={},
        operation="chat",
        agent_id="scenario_coordinator",
    )

    response = client.get("/api/workspaces/default/threads/thread-legacy-args/messages")

    assert response.status_code == 200
    messages = response.json()["messages"]
    assert messages[0]["metadata"]["parts"][0]["args"] == {
        "query": "select * from orders"
    }


def test_legacy_messages_payload_uses_only_final_user_message():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/scenario_coordinator/stream",
        json={
            "threadId": "thread-legacy",
            "messages": [
                {"role": "user", "content": "old prompt"},
                {"role": "assistant", "content": "old answer"},
                {"role": "user", "content": "new prompt"},
            ],
        },
    )

    assert response.status_code == 200
    assert fake_runtime.query_calls[0]["prompt"] == "new prompt"
    messages = client.get(
        "/api/workspaces/default/threads/thread-legacy/messages"
    ).json()["messages"]
    assert messages[0]["content"] == "new prompt"
    assert messages[0]["metadata"] == {"legacyMessages": True}


def test_direct_specialist_chat_uses_run_specialist():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/agents/insights/stream",
        json={"message": "Inspect the data", "threadId": "thread-specialist"},
    )

    assert response.status_code == 200
    assert fake_runtime.query_calls == []
    assert fake_runtime.specialist_calls == [
        {
            "specialist": "insights",
            "prompt": "Inspect the data",
            "thread_id": "thread-specialist",
        }
    ]


def test_planner_direct_chat_is_rejected_until_runtime_api_exists():
    client = _client()

    response = client.post(
        "/api/workspaces/default/agents/planner/stream",
        json={"message": "Plan directly", "threadId": "thread-planner"},
    )

    assert response.status_code == 409
    assert response.json()["error"]["code"] == "agent.unsupported_entrypoint"


def test_thread_metadata_is_scoped_by_workspace():
    store = StudioStore(None)
    default_runtime = FakeRuntime()
    other_runtime = FakeRuntime()
    app = define_app(
        name="demo",
        workspaces={
            "default": define_workspace_runtime(
                runtime_spec=_runtime_spec(),
                runtime=default_runtime,
            ),
            "customer-b": define_workspace_runtime(
                runtime_spec=_runtime_spec(),
                runtime=other_runtime,
            ),
        },
        default_workspace="default",
    )
    client = create_studio_test_client(app, store=store)

    assert (
        client.post(
            "/api/workspaces/default/agents/scenario_coordinator/stream",
            json={"message": "default prompt", "threadId": "same-thread"},
        ).status_code
        == 200
    )
    assert (
        client.post(
            "/api/workspaces/customer-b/agents/scenario_coordinator/stream",
            json={"message": "customer prompt", "threadId": "same-thread"},
        ).status_code
        == 200
    )

    default_messages = client.get(
        "/api/workspaces/default/threads/same-thread/messages"
    ).json()["messages"]
    other_messages = client.get(
        "/api/workspaces/customer-b/threads/same-thread/messages"
    ).json()["messages"]
    assert default_messages[0]["content"] == "default prompt"
    assert other_messages[0]["content"] == "customer prompt"


def test_thread_messages_reject_unknown_thread():
    client = _client()

    response = client.get("/api/workspaces/default/threads/missing/messages")

    assert response.status_code == 404
    assert response.json()["error"]["code"] == "thread.not_found"


def test_approval_response_delegates_to_runtime():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/approvals/approval-1/respond",
        json={"decision": "approved", "reason": "looks good"},
    )

    assert response.status_code == 200
    assert response.json()["status"] == "approve"
    assert fake_runtime.approval_calls == [
        {
            "approval_id": "approval-1",
            "outcome": "approve",
            "feedback": "looks good",
            "partial": None,
        }
    ]


def test_approval_decision_alias_is_removed():
    fake_runtime = FakeRuntime()
    client = _client(fake_runtime)

    response = client.post(
        "/api/workspaces/default/approvals/approval-2/decision",
        json={"decision": "rejected", "reason": "unsafe"},
    )

    assert response.status_code == 404
    assert fake_runtime.approval_calls == []
