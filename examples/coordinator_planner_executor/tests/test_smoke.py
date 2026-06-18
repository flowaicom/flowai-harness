from coordinator_planner_executor.app import (
    CATALOG_ACTION_PLAN,
    EXECUTOR_EVAL_TEST_CASE,
    PLANNER_EVAL_TEST_CASE,
    _runtime_spec,
)
from coordinator_planner_executor.smoke import agent_calls, run_smoke, tool_result


def test_coordinator_planner_executor_smoke_runs_without_live_model():
    events = run_smoke()

    assert agent_calls(events)[:3] == ["coordinator", "planner", "executor"]
    assert tool_result(events, "executePlan")["entitiesAffected"] == 1


def test_seeded_eval_inputs_are_customer_queries():
    for test_case in (PLANNER_EVAL_TEST_CASE, EXECUTOR_EVAL_TEST_CASE):
        prompt = test_case["input"]

        assert "storePlan" not in prompt
        assert "executePlan" not in prompt
        assert "exactly once" not in prompt
        assert "{" not in prompt


def test_seeded_eval_inputs_reference_canonical_product_id():
    for test_case in (PLANNER_EVAL_TEST_CASE, EXECUTOR_EVAL_TEST_CASE):
        prompt = test_case["input"]

        assert "p-001" in prompt
        assert "Sparkling Water 12pk" in prompt
        assert "sparkling-water-12pk" not in prompt


def test_seeded_eval_ground_truth_matches_stable_action_fields():
    for test_case in (PLANNER_EVAL_TEST_CASE, EXECUTOR_EVAL_TEST_CASE):
        ground_truth = test_case["structuredGroundTruth"]["payload"]
        expected_actions = ground_truth.get("plannedActions") or ground_truth.get("executedActions")
        expected_payload = expected_actions[0]["payload"]

        assert ground_truth["payloadMatch"] == "subset"
        assert expected_payload == {"productId": "p-001", "newPrice": 6.49}


def test_catalog_action_plan_rejects_extra_action_fields():
    action_schema = CATALOG_ACTION_PLAN.schema_["properties"]["actions"]["items"]

    assert CATALOG_ACTION_PLAN.schema_["additionalProperties"] is False
    assert action_schema["additionalProperties"] is False


def test_catalog_action_plan_only_accepts_executable_price_change_actions():
    action_schema = CATALOG_ACTION_PLAN.schema_["properties"]["actions"]["items"]
    properties = action_schema["properties"]

    assert properties["kind"]["const"] == "price_change"
    assert properties["newPrice"]["type"] == "number"
    assert "never be null" in properties["newPrice"]["description"]


def test_planner_prompt_forbids_review_steps_in_stored_plan_actions():
    spec = _runtime_spec("prompt-contract")
    planner = next(agent for agent in spec.agents if agent.name == "planner")
    prompt = planner.system_prompt

    assert "REVIEW_CURRENT_PRICE" in prompt
    assert "workflow steps as actions" in prompt
    assert '"kind":"price_change"' in prompt
    assert '"newPrice":6.49' in prompt
