"""StudioStore test-case / eval-run / eval-artifact tables."""

from __future__ import annotations

import pytest

from flowai_harness.studio.store import StudioStore


@pytest.fixture()
def store() -> StudioStore:
    s = StudioStore(path=None)  # in-memory
    try:
        yield s
    finally:
        s.close()


def _test_case_payload(test_case_id: str) -> dict:
    # Mirrors the harness EvalTestCase wire shape with an opaque structured
    # ground truth carrying arbitrary nested data.
    return {
        "id": test_case_id,
        "input": "Drop the price of enterprise SKUs by 10%",
        "tags": ["pricing", "smoke"],
        "expectedTrajectory": ["buildPlan", "executePlan"],
        "trajectoryMode": "subsequence",
        "structuredGroundTruth": {
            "kind": "structured",
            "data": {
                "kind": "flat",
                "plannedActions": [],
                "executedActions": [
                    {
                        "type": "price_change",
                        "payload": {"productIds": ["p1"], "deltaPct": -10, "nested": {"a": [1, 2, 3]}},
                    }
                ],
            },
        },
    }


# --- test cases ----------------------------------------------------------


def test_test_case_round_trips_arbitrary_payload(store: StudioStore) -> None:
    payload = _test_case_payload("tc-1")
    saved = store.upsert_test_case(
        app_id="app", workspace_key="default", test_case_id="tc-1", payload=payload
    )
    assert saved["id"] == "tc-1"
    assert saved["testCase"] == payload  # byte-for-byte (semantic) round-trip
    assert saved["createdAt"] and saved["updatedAt"]

    fetched = store.get_test_case(app_id="app", workspace_key="default", test_case_id="tc-1")
    assert fetched["testCase"] == payload


def test_test_case_upsert_updates_in_place(store: StudioStore) -> None:
    store.upsert_test_case(
        app_id="app", workspace_key="default", test_case_id="tc-1",
        payload=_test_case_payload("tc-1"),
    )
    changed = _test_case_payload("tc-1")
    changed["input"] = "Raise prices instead"
    store.upsert_test_case(
        app_id="app", workspace_key="default", test_case_id="tc-1", payload=changed
    )
    rows = store.list_test_cases(app_id="app", workspace_key="default")
    assert len(rows) == 1
    assert rows[0]["testCase"]["input"] == "Raise prices instead"


def test_test_case_delete_and_missing_raise(store: StudioStore) -> None:
    store.upsert_test_case(
        app_id="app", workspace_key="default", test_case_id="tc-1",
        payload=_test_case_payload("tc-1"),
    )
    store.delete_test_case(app_id="app", workspace_key="default", test_case_id="tc-1")
    assert store.list_test_cases(app_id="app", workspace_key="default") == []
    with pytest.raises(KeyError):
        store.get_test_case(app_id="app", workspace_key="default", test_case_id="tc-1")
    with pytest.raises(KeyError):
        store.delete_test_case(app_id="app", workspace_key="default", test_case_id="tc-1")


def test_test_cases_are_workspace_scoped(store: StudioStore) -> None:
    store.upsert_test_case(
        app_id="app", workspace_key="ws-a", test_case_id="tc-1",
        payload=_test_case_payload("tc-1"),
    )
    assert len(store.list_test_cases(app_id="app", workspace_key="ws-a")) == 1
    assert store.list_test_cases(app_id="app", workspace_key="ws-b") == []
    with pytest.raises(KeyError):
        store.get_test_case(app_id="app", workspace_key="ws-b", test_case_id="tc-1")


# --- eval runs -----------------------------------------------------------


def test_eval_run_create_get_list_and_status(store: StudioStore) -> None:
    config = {"mode": "sequential", "samplesPerCase": 3, "passThreshold": 0.7}
    saved = store.upsert_eval_run(
        app_id="app", workspace_key="default", eval_id="ev-1",
        config=config, test_case_ids=["tc-1", "tc-2"],
    )
    assert saved["id"] == "ev-1"
    assert saved["config"] == config
    assert saved["testCaseIds"] == ["tc-1", "tc-2"]
    assert saved["status"] == "created"

    store.update_eval_run_status(
        app_id="app", workspace_key="default", eval_id="ev-1", status="running"
    )
    assert store.get_eval_run(app_id="app", workspace_key="default", eval_id="ev-1")["status"] == "running"

    runs = store.list_eval_runs(app_id="app", workspace_key="default")
    assert [r["id"] for r in runs] == ["ev-1"]


def test_eval_run_missing_raises(store: StudioStore) -> None:
    with pytest.raises(KeyError):
        store.get_eval_run(app_id="app", workspace_key="default", eval_id="nope")
    with pytest.raises(KeyError):
        store.update_eval_run_status(
            app_id="app", workspace_key="default", eval_id="nope", status="running"
        )


def test_delete_eval_run_removes_run_and_artifacts(store: StudioStore) -> None:
    store.upsert_eval_run(
        app_id="app", workspace_key="default", eval_id="ev-1",
        config={"mode": "sequential"}, test_case_ids=["tc-1"],
    )
    store.append_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-a",
        artifact={"summary": {}},
    )
    store.delete_eval_run(app_id="app", workspace_key="default", eval_id="ev-1")
    with pytest.raises(KeyError):
        store.get_eval_run(app_id="app", workspace_key="default", eval_id="ev-1")
    assert store.list_eval_artifacts(app_id="app", workspace_key="default", eval_id="ev-1") == []
    with pytest.raises(KeyError):
        store.delete_eval_run(app_id="app", workspace_key="default", eval_id="ev-1")


def test_eval_runs_are_workspace_scoped(store: StudioStore) -> None:
    store.upsert_eval_run(
        app_id="app", workspace_key="ws-a", eval_id="ev-1",
        config={"mode": "sequential"}, test_case_ids=["tc-1"],
    )
    assert store.list_eval_runs(app_id="app", workspace_key="ws-b") == []


# --- eval artifacts ------------------------------------------------------


def test_eval_artifact_append_get_list(store: StudioStore) -> None:
    artifact_a = {"runId": "run-a", "summary": {"passRate": 1.0}, "testCases": []}
    artifact_b = {"runId": "run-b", "summary": {"passRate": 0.5}, "testCases": []}
    store.append_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-a", artifact=artifact_a
    )
    store.append_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-b", artifact=artifact_b
    )

    got = store.get_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-a"
    )
    assert got["artifact"] == artifact_a
    assert got["evalId"] == "ev-1" and got["runId"] == "run-a"

    history = store.list_eval_artifacts(app_id="app", workspace_key="default", eval_id="ev-1")
    assert [a["runId"] for a in history] == ["run-a", "run-b"]


def test_eval_artifact_same_run_overwrites(store: StudioStore) -> None:
    store.append_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-a",
        artifact={"summary": {"passed": 0}},
    )
    store.append_eval_artifact(
        app_id="app", workspace_key="default", eval_id="ev-1", run_id="run-a",
        artifact={"summary": {"passed": 2}},
    )
    history = store.list_eval_artifacts(app_id="app", workspace_key="default", eval_id="ev-1")
    assert len(history) == 1
    assert history[0]["artifact"]["summary"]["passed"] == 2


def test_eval_artifact_missing_raises(store: StudioStore) -> None:
    with pytest.raises(KeyError):
        store.get_eval_artifact(
            app_id="app", workspace_key="default", eval_id="ev-1", run_id="nope"
        )


# --- traces --------------------------------------------------------------


def test_trace_upsert_get_list_and_filters(store: StudioStore) -> None:
    trace = {
        "traceId": "trace-1",
        "workspaceId": "default",
        "stage": "runtime",
        "status": "completed",
        "scope": {
            "evalRunId": "run-1",
            "testCaseId": "tc-1",
            "threadId": "thread-1",
            "sampleIndex": 0,
        },
        "steps": [
            {
                "ordinal": 0,
                "actor": "assistant",
                "toolName": "search",
                "arguments": {"kind": "inline", "value": {"q": "revenue"}},
                "result": {"kind": "omitted", "reason": "notCaptured"},
            }
        ],
        "startedAt": None,
        "completedAt": "2026-06-02T00:00:00Z",
        "provenance": {"kind": "evalSample"},
    }
    saved = store.upsert_trace(app_id="app", workspace_key="default", trace=trace)
    assert saved["traceId"] == "trace-1"
    assert saved["evalRunId"] == "run-1"
    assert saved["testCaseId"] == "tc-1"
    assert saved["threadId"] == "thread-1"
    assert saved["sampleIndex"] == 0
    assert saved["trace"] == trace

    assert store.get_trace(app_id="app", workspace_key="default", trace_id="trace-1")["trace"] == trace
    assert [
        row["traceId"]
        for row in store.list_traces(app_id="app", workspace_key="default", eval_run_id="run-1")
    ] == ["trace-1"]
    assert [
        row["traceId"]
        for row in store.list_traces(app_id="app", workspace_key="default", test_case_id="tc-1")
    ] == ["trace-1"]
    assert [
        row["traceId"]
        for row in store.list_traces(app_id="app", workspace_key="default", thread_id="thread-1")
    ] == ["trace-1"]
    assert store.list_traces(app_id="app", workspace_key="default", eval_run_id="other") == []
    with pytest.raises(KeyError):
        store.get_trace(app_id="app", workspace_key="default", trace_id="missing")
