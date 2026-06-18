from __future__ import annotations

import argparse
from pathlib import Path

from flowai_harness import EvalTestCase
from flowai_harness.studio import StudioStore

from coordinator_planner_executor.app import (
    EXECUTOR_EVAL_TEST_CASE,
    PLANNER_EVAL_TEST_CASE,
    app,
)


def seed_eval_tests(*, workspace_key: str = "acme", store_path: str | Path = ".flowai/studio.db"):
    store = StudioStore(store_path)
    try:
        saved = []
        for payload in (PLANNER_EVAL_TEST_CASE, EXECUTOR_EVAL_TEST_CASE):
            test_case = EvalTestCase.model_validate(payload)
            saved.append(
                store.upsert_test_case(
                    app_id=app.app_id,
                    workspace_key=workspace_key,
                    test_case_id=test_case.id,
                    payload=test_case.model_dump(by_alias=True, mode="json"),
                )
            )
        return saved
    finally:
        store.close()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Seed planner/executor eval test cases for coordinator-planner-executor."
    )
    parser.add_argument("--workspace", default="acme", help="Studio workspace key to seed.")
    parser.add_argument(
        "--store",
        default=".flowai/studio.db",
        help="Path to the local Studio SQLite store.",
    )
    args = parser.parse_args()

    saved = seed_eval_tests(workspace_key=args.workspace, store_path=args.store)
    for row in saved:
        print(f"seeded {row['testCase']['id']} in workspace {args.workspace}")


if __name__ == "__main__":
    main()
