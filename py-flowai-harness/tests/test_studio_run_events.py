"""RunEventStore — operation/agent_id columns, list/get/since_seq."""

from __future__ import annotations

import sqlite3

import pytest

from flowai_harness.studio.store import StudioStore


@pytest.fixture()
def store() -> StudioStore:
    s = StudioStore(path=None)
    try:
        yield s
    finally:
        s.close()


def _append(store, run_id, seq, kind, *, operation="chat", agent_id="", ws="default"):
    store.append_run_event(
        app_id="app",
        workspace_key=ws,
        thread_id="t-1",
        run_id=run_id,
        seq=seq,
        kind=kind,
        event={"seq": seq, "kind": kind},
        raw_event={"type": kind},
        operation=operation,
        agent_id=agent_id,
    )


def test_list_runs_summarizes_each_run(store: StudioStore) -> None:
    _append(store, "run-1", 1, "message.delta", operation="chat", agent_id="coordinator")
    _append(store, "run-1", 2, "finish", operation="chat", agent_id="coordinator")
    _append(store, "run-2", 1, "sampleCompleted", operation="eval", agent_id="planner")

    runs = {r["runId"]: r for r in store.list_runs(app_id="app", workspace_key="default")}
    assert set(runs) == {"run-1", "run-2"}

    one = runs["run-1"]
    assert one["operation"] == "chat"
    assert one["agentId"] == "coordinator"  # taken from the first event
    assert one["status"] == "completed"  # last kind is `finish`
    assert (one["firstSeq"], one["lastSeq"], one["eventCount"]) == (1, 2, 2)

    assert runs["run-2"]["operation"] == "eval"
    assert runs["run-2"]["status"] == "running"  # no terminal event yet


@pytest.mark.parametrize(
    ("last_kind", "expected"),
    [
        ("finish", "completed"),
        ("evalCompleted", "completed"),
        ("evalFailed", "failed"),
        ("run.failed", "failed"),
        ("evalCancelled", "cancelled"),
        ("message.delta", "running"),
    ],
)
def test_status_is_derived_from_terminal_event(store: StudioStore, last_kind, expected) -> None:
    _append(store, "r", 1, "step")
    _append(store, "r", 2, last_kind)
    run = store.get_run(app_id="app", workspace_key="default", run_id="r")
    assert run["status"] == expected


def test_get_run_missing_raises(store: StudioStore) -> None:
    with pytest.raises(KeyError):
        store.get_run(app_id="app", workspace_key="default", run_id="nope")


def test_since_seq_filters_run_events(store: StudioStore) -> None:
    for seq in range(1, 5):
        _append(store, "r", seq, f"k{seq}")
    all_events = store.list_run_events(app_id="app", workspace_key="default", run_id="r")
    assert [e["seq"] for e in all_events] == [1, 2, 3, 4]
    tail = store.list_run_events(
        app_id="app", workspace_key="default", run_id="r", since_seq=2
    )
    assert [e["seq"] for e in tail] == [3, 4]


def test_runs_are_workspace_scoped(store: StudioStore) -> None:
    _append(store, "run-a", 1, "finish", ws="ws-a")
    assert [r["runId"] for r in store.list_runs(app_id="app", workspace_key="ws-a")] == ["run-a"]
    assert store.list_runs(app_id="app", workspace_key="ws-b") == []
    with pytest.raises(KeyError):
        store.get_run(app_id="app", workspace_key="ws-b", run_id="run-a")


def test_default_operation_and_agent_when_omitted(store: StudioStore) -> None:
    store.append_run_event(
        app_id="app",
        workspace_key="default",
        thread_id="t",
        run_id="r",
        seq=1,
        kind="finish",
        event={},
    )
    run = store.get_run(app_id="app", workspace_key="default", run_id="r")
    assert run["operation"] == "chat"
    assert run["agentId"] == ""


def test_migration_backfills_columns_on_legacy_table(tmp_path) -> None:
    db_path = tmp_path / "legacy.db"
    conn = sqlite3.connect(db_path)
    conn.executescript(
        """
        create table run_events (
            id integer primary key autoincrement,
            app_id text not null,
            workspace_key text not null,
            thread_id text not null,
            run_id text not null,
            seq integer not null,
            kind text not null,
            event_json text not null,
            raw_json text not null,
            created_at text not null
        );
        insert into run_events(app_id, workspace_key, thread_id, run_id, seq, kind, event_json, raw_json, created_at)
        values ('app', 'default', 't', 'old-run', 1, 'finish', '{}', '{}', '1970-01-01T00:00:00Z');
        """
    )
    conn.commit()
    conn.close()

    store = StudioStore(path=str(db_path))  # triggers _migrate → ALTER TABLE
    try:
        run = store.get_run(app_id="app", workspace_key="default", run_id="old-run")
        assert run["operation"] == "chat"  # backfilled default
        assert run["agentId"] == ""
        assert run["status"] == "completed"
        # New writes that set the columns work alongside backfilled rows.
        _append(store, "new-run", 1, "sampleCompleted", operation="eval", agent_id="planner")
        assert store.get_run(app_id="app", workspace_key="default", run_id="new-run")["operation"] == "eval"
    finally:
        store.close()
