from __future__ import annotations

import json
import sqlite3
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


class StudioStore:
    """Local Studio metadata store scoped by app and workspace.

    Runtime memory remains owned by `flowai-runtime`; this store is only for UI
    listing, message display, reconnectable run/event metadata, and approval
    references.
    """

    def __init__(self, path: str | Path | None = ".flowai/studio.db") -> None:
        if path is None:
            path = ":memory:"
        self.path = str(path)
        if self.path != ":memory:":
            Path(self.path).parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(self.path, check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        self._lock = threading.Lock()
        self._migrate()

    def close(self) -> None:
        with self._lock:
            self._conn.close()

    def upsert_thread(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
        title: str,
    ) -> dict[str, Any]:
        now = _now()
        with self._lock:
            self._conn.execute(
                """
                insert into threads(app_id, workspace_key, thread_id, title, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?)
                on conflict(app_id, workspace_key, thread_id)
                do update set title = coalesce(threads.title, excluded.title), updated_at = excluded.updated_at
                """,
                (app_id, workspace_key, thread_id, title, now, now),
            )
            self._conn.commit()
        return self.get_thread(app_id=app_id, workspace_key=workspace_key, thread_id=thread_id)

    def touch_thread(self, *, app_id: str, workspace_key: str, thread_id: str) -> None:
        with self._lock:
            self._conn.execute(
                """
                update threads set updated_at = ? where app_id = ? and workspace_key = ? and thread_id = ?
                """,
                (_now(), app_id, workspace_key, thread_id),
            )
            self._conn.commit()

    def list_threads(self, *, app_id: str, workspace_key: str) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select thread_id, title, created_at, updated_at
                from threads
                where app_id = ? and workspace_key = ?
                order by updated_at desc, created_at desc
                """,
                (app_id, workspace_key),
            ).fetchall()
        return [_thread_row(row) for row in rows]

    def get_thread(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select thread_id, title, created_at, updated_at
                from threads
                where app_id = ? and workspace_key = ? and thread_id = ?
                """,
                (app_id, workspace_key, thread_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"thread {thread_id!r} is not registered")
        return _thread_row(row)

    def delete_thread(self, *, app_id: str, workspace_key: str, thread_id: str) -> None:
        with self._lock:
            cursor = self._conn.execute(
                """
                delete from threads
                where app_id = ? and workspace_key = ? and thread_id = ?
                """,
                (app_id, workspace_key, thread_id),
            )
            self._conn.execute(
                "delete from messages where app_id = ? and workspace_key = ? and thread_id = ?",
                (app_id, workspace_key, thread_id),
            )
            self._conn.execute(
                "delete from run_events where app_id = ? and workspace_key = ? and thread_id = ?",
                (app_id, workspace_key, thread_id),
            )
            self._conn.execute(
                "delete from approval_refs where app_id = ? and workspace_key = ? and thread_id = ?",
                (app_id, workspace_key, thread_id),
            )
            self._conn.execute(
                "delete from traces where app_id = ? and workspace_key = ? and thread_id = ?",
                (app_id, workspace_key, thread_id),
            )
            self._conn.commit()
            if cursor.rowcount == 0:
                raise KeyError(f"thread {thread_id!r} is not registered")

    def append_message(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
        role: str,
        content: str,
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        now = _now()
        metadata_json = json.dumps(metadata or {}, sort_keys=True)
        with self._lock:
            cursor = self._conn.execute(
                """
                insert into messages(app_id, workspace_key, thread_id, role, content, metadata_json, created_at)
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (app_id, workspace_key, thread_id, role, content, metadata_json, now),
            )
            self._conn.execute(
                """
                update threads set updated_at = ? where app_id = ? and workspace_key = ? and thread_id = ?
                """,
                (now, app_id, workspace_key, thread_id),
            )
            self._conn.commit()
            message_id = int(cursor.lastrowid)
        return {
            "messageId": str(message_id),
            "threadId": thread_id,
            "role": role,
            "content": content,
            "metadata": metadata or {},
            "createdAt": now,
        }

    def list_messages(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
    ) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select id, thread_id, role, content, metadata_json, created_at
                from messages
                where app_id = ? and workspace_key = ? and thread_id = ?
                order by id asc
                """,
                (app_id, workspace_key, thread_id),
            ).fetchall()
        return [_message_row(row) for row in rows]

    def append_run_event(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
        run_id: str,
        seq: int,
        kind: str,
        event: dict[str, Any],
        raw_event: dict[str, Any] | None = None,
        operation: str = "chat",
        agent_id: str = "",
    ) -> None:
        with self._lock:
            self._conn.execute(
                """
                insert into run_events(
                    app_id, workspace_key, thread_id, run_id, seq, kind,
                    operation, agent_id, event_json, raw_json, created_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    app_id,
                    workspace_key,
                    thread_id,
                    run_id,
                    seq,
                    kind,
                    operation,
                    agent_id,
                    json.dumps(event, sort_keys=True),
                    json.dumps(raw_event or {}, sort_keys=True),
                    _now(),
                ),
            )
            self._conn.commit()

    def list_run_events(
        self,
        *,
        app_id: str,
        workspace_key: str,
        run_id: str,
        since_seq: int | None = None,
    ) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select seq, kind, event_json, raw_json, created_at
                from run_events
                where app_id = ? and workspace_key = ? and run_id = ?
                    and (? is null or seq > ?)
                order by seq asc
                """,
                (app_id, workspace_key, run_id, since_seq, since_seq),
            ).fetchall()
        return [_run_event_row(row) for row in rows]

    def list_thread_run_events(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
    ) -> list[dict[str, Any]]:
        """All run events for a thread, in chronological (insertion) order.

        A thread may span several runs (turns); ordering by ``id`` keeps tool
        calls and sub-agent calls in the order they actually happened.
        """

        with self._lock:
            rows = self._conn.execute(
                """
                select seq, kind, event_json, raw_json, created_at
                from run_events
                where app_id = ? and workspace_key = ? and thread_id = ?
                order by id asc
                """,
                (app_id, workspace_key, thread_id),
            ).fetchall()
        return [_run_event_row(row) for row in rows]

    def list_runs(self, *, app_id: str, workspace_key: str) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                _RUN_SUMMARY_SQL + " order by max(r.created_at) desc, r.run_id asc",
                (app_id, workspace_key, None, None),
            ).fetchall()
        return [_run_summary_row(row) for row in rows]

    def get_run(
        self,
        *,
        app_id: str,
        workspace_key: str,
        run_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                _RUN_SUMMARY_SQL,
                (app_id, workspace_key, run_id, run_id),
            ).fetchone()
        if row is None or row["run_id"] is None:
            raise KeyError(f"run {run_id!r} is not registered")
        return _run_summary_row(row)

    # ------------------------------------------------------------------
    # Traces (runtime/eval canonical trace records)
    # ------------------------------------------------------------------

    def upsert_trace(
        self,
        *,
        app_id: str,
        workspace_key: str,
        trace: dict[str, Any],
    ) -> dict[str, Any]:
        trace_id = str(trace.get("traceId") or "")
        if not trace_id:
            raise ValueError("trace must include traceId")
        scope = trace.get("scope") if isinstance(trace.get("scope"), dict) else {}
        eval_run_id = _string_or_none(scope.get("evalRunId"))
        test_case_id = _string_or_none(scope.get("testCaseId"))
        thread_id = _string_or_none(scope.get("threadId"))
        sample_index = scope.get("sampleIndex")
        sample_index_value = sample_index if isinstance(sample_index, int) else None
        now = _now()
        trace_json = json.dumps(trace, sort_keys=True)
        with self._lock:
            self._conn.execute(
                """
                insert into traces(
                    app_id, workspace_key, trace_id, eval_run_id, test_case_id,
                    thread_id, sample_index, trace_json, created_at, updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                on conflict(app_id, workspace_key, trace_id)
                do update set
                    eval_run_id = excluded.eval_run_id,
                    test_case_id = excluded.test_case_id,
                    thread_id = excluded.thread_id,
                    sample_index = excluded.sample_index,
                    trace_json = excluded.trace_json,
                    updated_at = excluded.updated_at
                """,
                (
                    app_id,
                    workspace_key,
                    trace_id,
                    eval_run_id,
                    test_case_id,
                    thread_id,
                    sample_index_value,
                    trace_json,
                    now,
                    now,
                ),
            )
            self._conn.commit()
        return self.get_trace(app_id=app_id, workspace_key=workspace_key, trace_id=trace_id)

    def get_trace(
        self,
        *,
        app_id: str,
        workspace_key: str,
        trace_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select trace_id, eval_run_id, test_case_id, thread_id, sample_index,
                       trace_json, created_at, updated_at
                from traces
                where app_id = ? and workspace_key = ? and trace_id = ?
                """,
                (app_id, workspace_key, trace_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"trace {trace_id!r} is not registered")
        return _trace_row(row)

    def list_traces(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_run_id: str | None = None,
        test_case_id: str | None = None,
        thread_id: str | None = None,
    ) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select trace_id, eval_run_id, test_case_id, thread_id, sample_index,
                       trace_json, created_at, updated_at
                from traces
                where app_id = ? and workspace_key = ?
                    and (? is null or eval_run_id = ?)
                    and (? is null or test_case_id = ?)
                    and (? is null or thread_id = ?)
                order by updated_at desc, trace_id asc
                """,
                (
                    app_id,
                    workspace_key,
                    eval_run_id,
                    eval_run_id,
                    test_case_id,
                    test_case_id,
                    thread_id,
                    thread_id,
                ),
            ).fetchall()
        return [_trace_row(row) for row in rows]

    def upsert_approval_ref(
        self,
        *,
        app_id: str,
        workspace_key: str,
        thread_id: str,
        run_id: str,
        approval_id: str,
        payload: dict[str, Any],
    ) -> None:
        with self._lock:
            self._conn.execute(
                """
                insert into approval_refs(
                    app_id, workspace_key, thread_id, run_id, approval_id, status, payload_json, created_at, updated_at
                )
                values (?, ?, ?, ?, ?, 'pending', ?, ?, ?)
                on conflict(app_id, workspace_key, approval_id)
                do update set payload_json = excluded.payload_json, updated_at = excluded.updated_at
                """,
                (
                    app_id,
                    workspace_key,
                    thread_id,
                    run_id,
                    approval_id,
                    json.dumps(payload, sort_keys=True),
                    _now(),
                    _now(),
                ),
            )
            self._conn.commit()

    def update_approval_ref(
        self,
        *,
        app_id: str,
        workspace_key: str,
        approval_id: str,
        status: str,
    ) -> None:
        with self._lock:
            self._conn.execute(
                """
                update approval_refs
                set status = ?, updated_at = ?
                where app_id = ? and workspace_key = ? and approval_id = ?
                """,
                (status, _now(), app_id, workspace_key, approval_id),
            )
            self._conn.commit()

    def list_approvals(self, *, app_id: str, workspace_key: str) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select thread_id, run_id, approval_id, status, payload_json, created_at, updated_at
                from approval_refs
                where app_id = ? and workspace_key = ?
                order by (case when status = 'pending' then 0 else 1 end),
                         updated_at desc, approval_id asc
                """,
                (app_id, workspace_key),
            ).fetchall()
        return [_approval_row(row) for row in rows]

    def get_approval(
        self,
        *,
        app_id: str,
        workspace_key: str,
        approval_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select thread_id, run_id, approval_id, status, payload_json, created_at, updated_at
                from approval_refs
                where app_id = ? and workspace_key = ? and approval_id = ?
                """,
                (app_id, workspace_key, approval_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"approval {approval_id!r} is not registered")
        return _approval_row(row)

    # ------------------------------------------------------------------
    # Test cases (Studio eval storage / M7.1)
    #
    # ``payload`` is the full harness ``EvalTestCase`` wire object (camelCase,
    # opaque structured ``groundTruth``). The store keeps it verbatim; shape
    # validation belongs to the harness, not here.
    # ------------------------------------------------------------------

    def upsert_test_case(
        self,
        *,
        app_id: str,
        workspace_key: str,
        test_case_id: str,
        payload: dict[str, Any],
    ) -> dict[str, Any]:
        now = _now()
        payload_json = json.dumps(payload, sort_keys=True)
        with self._lock:
            self._conn.execute(
                """
                insert into test_cases(app_id, workspace_key, id, payload_json, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?)
                on conflict(app_id, workspace_key, id)
                do update set payload_json = excluded.payload_json, updated_at = excluded.updated_at
                """,
                (app_id, workspace_key, test_case_id, payload_json, now, now),
            )
            self._conn.commit()
        return self.get_test_case(
            app_id=app_id, workspace_key=workspace_key, test_case_id=test_case_id
        )

    def get_test_case(
        self,
        *,
        app_id: str,
        workspace_key: str,
        test_case_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select id, payload_json, created_at, updated_at
                from test_cases
                where app_id = ? and workspace_key = ? and id = ?
                """,
                (app_id, workspace_key, test_case_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"test case {test_case_id!r} is not registered")
        return _test_case_row(row)

    def list_test_cases(self, *, app_id: str, workspace_key: str) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select id, payload_json, created_at, updated_at
                from test_cases
                where app_id = ? and workspace_key = ?
                order by updated_at desc, id asc
                """,
                (app_id, workspace_key),
            ).fetchall()
        return [_test_case_row(row) for row in rows]

    def delete_test_case(
        self,
        *,
        app_id: str,
        workspace_key: str,
        test_case_id: str,
    ) -> None:
        with self._lock:
            cursor = self._conn.execute(
                """
                delete from test_cases
                where app_id = ? and workspace_key = ? and id = ?
                """,
                (app_id, workspace_key, test_case_id),
            )
            self._conn.commit()
            if cursor.rowcount == 0:
                raise KeyError(f"test case {test_case_id!r} is not registered")

    # ------------------------------------------------------------------
    # Eval runs (Studio eval storage)
    #
    # An eval row is a saved config + the selected test-case ids + a status.
    # Each execution appends an artifact (see ``append_eval_artifact``).
    # ------------------------------------------------------------------

    def upsert_eval_run(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
        config: dict[str, Any],
        test_case_ids: list[str],
        status: str = "created",
    ) -> dict[str, Any]:
        now = _now()
        config_json = json.dumps(config, sort_keys=True)
        ids_json = json.dumps(list(test_case_ids))
        with self._lock:
            self._conn.execute(
                """
                insert into eval_runs(
                    app_id, workspace_key, id, config_json, test_case_ids_json, status, created_at, updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?)
                on conflict(app_id, workspace_key, id)
                do update set
                    config_json = excluded.config_json,
                    test_case_ids_json = excluded.test_case_ids_json,
                    status = excluded.status,
                    updated_at = excluded.updated_at
                """,
                (app_id, workspace_key, eval_id, config_json, ids_json, status, now, now),
            )
            self._conn.commit()
        return self.get_eval_run(
            app_id=app_id, workspace_key=workspace_key, eval_id=eval_id
        )

    def get_eval_run(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select id, config_json, test_case_ids_json, status, created_at, updated_at
                from eval_runs
                where app_id = ? and workspace_key = ? and id = ?
                """,
                (app_id, workspace_key, eval_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"eval {eval_id!r} is not registered")
        return _eval_run_row(row)

    def list_eval_runs(self, *, app_id: str, workspace_key: str) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select id, config_json, test_case_ids_json, status, created_at, updated_at
                from eval_runs
                where app_id = ? and workspace_key = ?
                order by updated_at desc, id asc
                """,
                (app_id, workspace_key),
            ).fetchall()
        return [_eval_run_row(row) for row in rows]

    def update_eval_run_status(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
        status: str,
    ) -> None:
        with self._lock:
            cursor = self._conn.execute(
                """
                update eval_runs
                set status = ?, updated_at = ?
                where app_id = ? and workspace_key = ? and id = ?
                """,
                (status, _now(), app_id, workspace_key, eval_id),
            )
            self._conn.commit()
            if cursor.rowcount == 0:
                raise KeyError(f"eval {eval_id!r} is not registered")

    def delete_eval_run(self, *, app_id: str, workspace_key: str, eval_id: str) -> None:
        with self._lock:
            cursor = self._conn.execute(
                "delete from eval_runs where app_id = ? and workspace_key = ? and id = ?",
                (app_id, workspace_key, eval_id),
            )
            self._conn.execute(
                "delete from eval_artifacts where app_id = ? and workspace_key = ? and eval_id = ?",
                (app_id, workspace_key, eval_id),
            )
            self._conn.commit()
            if cursor.rowcount == 0:
                raise KeyError(f"eval {eval_id!r} is not registered")

    # ------------------------------------------------------------------
    # Eval artifacts (Studio eval storage)
    #
    # One artifact per ``(eval_id, run_id)``. Re-appending the same ``run_id``
    # overwrites it, so incremental snapshots during a streaming run converge to
    # the final artifact without growing history.
    # ------------------------------------------------------------------

    def append_eval_artifact(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
        run_id: str,
        artifact: dict[str, Any],
    ) -> dict[str, Any]:
        now = _now()
        artifact_json = json.dumps(artifact, sort_keys=True)
        with self._lock:
            self._conn.execute(
                """
                insert into eval_artifacts(app_id, workspace_key, eval_id, run_id, artifact_json, created_at)
                values (?, ?, ?, ?, ?, ?)
                on conflict(app_id, workspace_key, eval_id, run_id)
                do update set artifact_json = excluded.artifact_json
                """,
                (app_id, workspace_key, eval_id, run_id, artifact_json, now),
            )
            self._conn.commit()
        return self.get_eval_artifact(
            app_id=app_id, workspace_key=workspace_key, eval_id=eval_id, run_id=run_id
        )

    def get_eval_artifact(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
        run_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select eval_id, run_id, artifact_json, created_at
                from eval_artifacts
                where app_id = ? and workspace_key = ? and eval_id = ? and run_id = ?
                """,
                (app_id, workspace_key, eval_id, run_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"artifact for eval {eval_id!r} run {run_id!r} not found")
        return _eval_artifact_row(row)

    def list_eval_artifacts(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
    ) -> list[dict[str, Any]]:
        with self._lock:
            rows = self._conn.execute(
                """
                select eval_id, run_id, artifact_json, created_at
                from eval_artifacts
                where app_id = ? and workspace_key = ? and eval_id = ?
                order by created_at asc, run_id asc
                """,
                (app_id, workspace_key, eval_id),
            ).fetchall()
        return [_eval_artifact_row(row) for row in rows]

    def get_eval_artifact_by_run(
        self,
        *,
        app_id: str,
        workspace_key: str,
        run_id: str,
    ) -> dict[str, Any]:
        with self._lock:
            row = self._conn.execute(
                """
                select eval_id, run_id, artifact_json, created_at
                from eval_artifacts
                where app_id = ? and workspace_key = ? and run_id = ?
                """,
                (app_id, workspace_key, run_id),
            ).fetchone()
        if row is None:
            raise KeyError(f"artifact for run {run_id!r} not found")
        return _eval_artifact_row(row)

    def latest_eval_summary(
        self,
        *,
        app_id: str,
        workspace_key: str,
        eval_id: str,
    ) -> dict[str, Any] | None:
        """Summary of the most recent artifact for an eval (None if no runs)."""

        with self._lock:
            row = self._conn.execute(
                """
                select artifact_json
                from eval_artifacts
                where app_id = ? and workspace_key = ? and eval_id = ?
                order by created_at desc, run_id desc
                limit 1
                """,
                (app_id, workspace_key, eval_id),
            ).fetchone()
        if row is None:
            return None
        artifact = json.loads(row["artifact_json"])
        summary = artifact.get("summary") if isinstance(artifact, dict) else None
        return summary if isinstance(summary, dict) else None

    def _migrate(self) -> None:
        with self._lock:
            self._conn.executescript(
                """
                create table if not exists threads (
                    app_id text not null,
                    workspace_key text not null,
                    thread_id text not null,
                    title text not null,
                    created_at text not null,
                    updated_at text not null,
                    primary key(app_id, workspace_key, thread_id)
                );

                create table if not exists messages (
                    id integer primary key autoincrement,
                    app_id text not null,
                    workspace_key text not null,
                    thread_id text not null,
                    role text not null,
                    content text not null,
                    metadata_json text not null,
                    created_at text not null
                );

                create table if not exists run_events (
                    id integer primary key autoincrement,
                    app_id text not null,
                    workspace_key text not null,
                    thread_id text not null,
                    run_id text not null,
                    seq integer not null,
                    kind text not null,
                    operation text not null default 'chat',
                    agent_id text not null default '',
                    event_json text not null,
                    raw_json text not null,
                    created_at text not null
                );

                create table if not exists approval_refs (
                    app_id text not null,
                    workspace_key text not null,
                    thread_id text not null,
                    run_id text not null,
                    approval_id text not null,
                    status text not null,
                    payload_json text not null,
                    created_at text not null,
                    updated_at text not null,
                    primary key(app_id, workspace_key, approval_id)
                );

                create table if not exists traces (
                    app_id text not null,
                    workspace_key text not null,
                    trace_id text not null,
                    eval_run_id text,
                    test_case_id text,
                    thread_id text,
                    sample_index integer,
                    trace_json text not null,
                    created_at text not null,
                    updated_at text not null,
                    primary key(app_id, workspace_key, trace_id)
                );

                create table if not exists test_cases (
                    app_id text not null,
                    workspace_key text not null,
                    id text not null,
                    payload_json text not null,
                    created_at text not null,
                    updated_at text not null,
                    primary key(app_id, workspace_key, id)
                );

                create table if not exists eval_runs (
                    app_id text not null,
                    workspace_key text not null,
                    id text not null,
                    config_json text not null,
                    test_case_ids_json text not null,
                    status text not null,
                    created_at text not null,
                    updated_at text not null,
                    primary key(app_id, workspace_key, id)
                );

                create table if not exists eval_artifacts (
                    app_id text not null,
                    workspace_key text not null,
                    eval_id text not null,
                    run_id text not null,
                    artifact_json text not null,
                    created_at text not null,
                    primary key(app_id, workspace_key, eval_id, run_id)
                );

                create index if not exists idx_messages_thread
                    on messages(app_id, workspace_key, thread_id, id);
                create index if not exists idx_run_events_run
                    on run_events(app_id, workspace_key, run_id, seq);
                create index if not exists idx_traces_eval
                    on traces(app_id, workspace_key, eval_run_id, test_case_id, sample_index);
                create index if not exists idx_traces_thread
                    on traces(app_id, workspace_key, thread_id);
                create index if not exists idx_test_cases_ws
                    on test_cases(app_id, workspace_key, updated_at);
                create index if not exists idx_eval_runs_ws
                    on eval_runs(app_id, workspace_key, updated_at);
                create index if not exists idx_eval_artifacts_eval
                    on eval_artifacts(app_id, workspace_key, eval_id, created_at);
                """
            )
            # (run-event schema migration). ``create table if not exists`` never alters an existing
            # table, so add the columns explicitly when missing.
            existing = {
                row["name"]
                for row in self._conn.execute("pragma table_info(run_events)").fetchall()
            }
            if "operation" not in existing:
                self._conn.execute(
                    "alter table run_events add column operation text not null default 'chat'"
                )
            if "agent_id" not in existing:
                self._conn.execute(
                    "alter table run_events add column agent_id text not null default ''"
                )
            self._conn.commit()


# SQL for one summary row per run. The optional run filter
# ``(? is null or r.run_id = ?)`` lets ``get_run`` reuse the same query.
_RUN_SUMMARY_SQL = """
    select
        r.run_id as run_id,
        (select operation from run_events x
         where x.app_id = r.app_id and x.workspace_key = r.workspace_key
           and x.run_id = r.run_id order by x.seq asc limit 1) as operation,
        (select thread_id from run_events x
         where x.app_id = r.app_id and x.workspace_key = r.workspace_key
           and x.run_id = r.run_id order by x.seq asc limit 1) as thread_id,
        (select agent_id from run_events x
         where x.app_id = r.app_id and x.workspace_key = r.workspace_key
           and x.run_id = r.run_id order by x.seq asc limit 1) as agent_id,
        (select kind from run_events x
         where x.app_id = r.app_id and x.workspace_key = r.workspace_key
           and x.run_id = r.run_id order by x.seq desc limit 1) as last_kind,
        min(r.seq) as first_seq,
        max(r.seq) as last_seq,
        count(*) as event_count,
        min(r.created_at) as created_at,
        max(r.created_at) as updated_at
    from run_events r
    where r.app_id = ? and r.workspace_key = ? and (? is null or r.run_id = ?)
    group by r.run_id
"""

_RUN_STATUS_BY_KIND = {
    "run.completed": "completed",
    "evalCompleted": "completed",
    "runtime.finish": "completed",
    "finish": "completed",
    "completed": "completed",
    "run.failed": "failed",
    "evalFailed": "failed",
    "error": "failed",
    "evalCancelled": "cancelled",
    "run.cancelled": "cancelled",
}


def _derive_run_status(last_kind: str | None) -> str:
    return _RUN_STATUS_BY_KIND.get(str(last_kind or ""), "running")


def _approval_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "approvalId": row["approval_id"],
        "threadId": row["thread_id"],
        "runId": row["run_id"],
        "status": row["status"],
        "payload": json.loads(row["payload_json"]),
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _run_summary_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "runId": row["run_id"],
        "operation": row["operation"],
        "threadId": row["thread_id"],
        "agentId": row["agent_id"],
        "status": _derive_run_status(row["last_kind"]),
        "firstSeq": row["first_seq"],
        "lastSeq": row["last_seq"],
        "eventCount": row["event_count"],
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _thread_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "threadId": row["thread_id"],
        "title": row["title"],
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _message_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "messageId": str(row["id"]),
        "threadId": row["thread_id"],
        "role": row["role"],
        "content": row["content"],
        "metadata": json.loads(row["metadata_json"]),
        "createdAt": row["created_at"],
    }


def _run_event_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "seq": row["seq"],
        "kind": row["kind"],
        "event": json.loads(row["event_json"]),
        "raw": json.loads(row["raw_json"]),
        "createdAt": row["created_at"],
    }


def _trace_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "traceId": row["trace_id"],
        "evalRunId": row["eval_run_id"],
        "testCaseId": row["test_case_id"],
        "threadId": row["thread_id"],
        "sampleIndex": row["sample_index"],
        "trace": json.loads(row["trace_json"]),
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _test_case_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "testCase": json.loads(row["payload_json"]),
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _eval_run_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "config": json.loads(row["config_json"]),
        "testCaseIds": json.loads(row["test_case_ids_json"]),
        "status": row["status"],
        "createdAt": row["created_at"],
        "updatedAt": row["updated_at"],
    }


def _eval_artifact_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "evalId": row["eval_id"],
        "runId": row["run_id"],
        "artifact": json.loads(row["artifact_json"]),
        "createdAt": row["created_at"],
    }


def _string_or_none(value: Any) -> str | None:
    return value if isinstance(value, str) and value else None


def _now() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
