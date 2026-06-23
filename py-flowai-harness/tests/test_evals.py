import asyncio
import json

import pytest
from pydantic import ValidationError

import flowai_harness.evals as evals_module
from flowai_harness import (
    AgentSpec,
    EvalArtifact,
    EvalRequest,
    EvalTestCase,
    FinalResponseEval,
    FinalResponseScorerConfig,
    GroundTruth,
    HarnessEvalEventEnvelope,
    JudgeVerdict,
    RawSampleOutput,
    ResponseScorer,
    ScoreWeights,
    ScorerPreset,
    define_action_ground_truth,
    define_coordinator,
    define_executor,
    define_eval_config,
    define_eval_request,
    define_expected_actions,
    define_expected_action,
    define_final_response_eval,
    define_final_response_scorer_config,
    define_plan,
    define_planner,
    define_resolved_action,
    define_runtime,
    define_scorer_preset,
    define_specialist,
    define_tenant,
    define_test_case,
    define_tool,
    define_trajectory_scorer_config,
    final_response_judge_verdicts_extra,
    create_runtime,
    run_eval_sync,
    score_sample,
)


def test_eval_config_defaults_match_contract_wire_shape():
    config = define_eval_config()

    assert config.model_dump(by_alias=True, mode="json") == {
        "mode": "sequential",
        "targetAgentId": None,
        "testCaseSetId": "",
        "testCaseIds": None,
        "samplesPerCase": 3,
        "passThreshold": 0.7,
        "concurrency": 2,
        "kValues": [1, 3],
        "provider": None,
        "model": None,
        "timeoutPerSampleSecs": 120,
        "tagsFilter": None,
        "aggregationStrategy": "passRate",
        "scoreWeights": None,
        "scorerConfig": None,
        "requestOverrides": None,
    }


def test_eval_config_accepts_specialist_target_agent():
    config = define_eval_config(mode="specialist", target_agent_id="insights")

    assert config.model_dump(by_alias=True, mode="json")["mode"] == "specialist"
    assert config.model_dump(by_alias=True, mode="json")["targetAgentId"] == "insights"


def test_eval_request_wire_shape_uses_camel_case_and_structured_ground_truth():
    action = define_expected_action(
        "availability_change",
        {
            "targetAvailability": 100,
            "productIds": ["p1"],
            "context": {"channels": ["ONLINE"]},
        },
    )
    expected_actions = define_expected_actions(executed_actions=[action])
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(
            score_weights=ScoreWeights(
                {"trajectory": 0.5, "executed_actions": 0.5}
            )
        ),
        test_cases=[
            define_test_case(
                "tc-1",
                "Assess price changes",
                expected_trajectory=["buildPlan", "executePlan"],
                expected_actions=expected_actions,
                source_thread_id="thread-source",
            )
        ],
        scorer_preset="sequential",
    )

    dumped = request.model_dump(by_alias=True, mode="json")

    assert dumped["tenantId"] == "tenant-acme"
    assert dumped["workspaceId"] == "workspace-main"
    assert dumped["config"]["scoreWeights"] == {
        "trajectory": 0.5,
        "executed_actions": 0.5,
    }
    assert dumped["testCases"][0]["sourceThreadId"] == "thread-source"
    assert dumped["testCases"][0]["structuredGroundTruth"] == {
        "kind": "structured",
        "data": {
            "kind": "flat",
            "plannedActions": [],
            "payloadMatch": "exact",
            "executedActions": [
                {
                    "type": "availability_change",
                    "payload": {
                        "targetAvailability": 100,
                        "productIds": ["p1"],
                        "context": {"channels": ["ONLINE"]},
                    },
                }
            ],
        },
    }


def test_final_response_eval_wire_shape_and_validation():
    final_response = define_final_response_eval(
        scorers=[
            ResponseScorer.judge(
                id="matches_reference",
                instructions="Compare the final response to the reference response.",
                reference_response="The billing contact was updated.",
                rubric={
                    0: "The response is not similar or misses key facts.",
                    1: "The response is similar and contains the key facts.",
                },
                required=True,
                weight=2,
            ),
            ResponseScorer.regex(
                id="mentions_email",
                pattern=r"\bjane@example\.com\b",
            ),
        ],
        pass_threshold=0.75,
    )

    test_case = define_test_case(
        "tc-final-response",
        "Tell me what changed",
        final_response=final_response,
    )

    dumped = test_case.model_dump(by_alias=True, mode="json")

    assert dumped["finalResponse"] == {
        "scorers": [
            {
                "id": "matches_reference",
                "method": "judge",
                "weight": 2.0,
                "required": True,
                "instructions": "Compare the final response to the reference response.",
                "referenceResponse": "The billing contact was updated.",
                "rubric": {
                    "0": "The response is not similar or misses key facts.",
                    "1": "The response is similar and contains the key facts.",
                },
                "context": None,
                "expected": None,
                "text": None,
                "pattern": None,
                "caseSensitive": True,
            },
            {
                "id": "mentions_email",
                "method": "regex",
                "weight": 1.0,
                "required": False,
                "instructions": None,
                "referenceResponse": None,
                "rubric": None,
                "context": None,
                "expected": None,
                "text": None,
                "pattern": r"\bjane@example\.com\b",
                "caseSensitive": True,
            },
        ],
        "passThreshold": 0.75,
    }

    with pytest.raises(ValidationError, match="rubric must define exactly 0 and 1"):
        ResponseScorer.judge(
            id="bad_rubric",
            instructions="Compare the response.",
            rubric={1: "passes"},
        )

    with pytest.raises(ValidationError, match="invalid regex"):
        ResponseScorer.regex(id="bad_regex", pattern="[")


def test_final_response_judge_verdicts_extra_wire_shape_and_validation():
    extra = final_response_judge_verdicts_extra(
        {
            "matches_reference": JudgeVerdict(
                passed=True,
                selected_rubric_score=1,
                reason=" The response matches. ",
            )
        }
    )

    assert extra == {
        "finalResponseJudgeVerdicts": {
            "matches_reference": {
                "passed": True,
                "selected_rubric_score": 1,
                "reason": "The response matches.",
            }
        }
    }

    with pytest.raises(ValidationError, match="selected_rubric_score must be 0 or 1"):
        JudgeVerdict(passed=True, selected_rubric_score=2, reason="Bad score.")

    with pytest.raises(ValidationError, match="passed must be true exactly"):
        JudgeVerdict(passed=True, selected_rubric_score=0, reason="Inconsistent.")

    with pytest.raises(ValueError, match="response scorer ids must be non-empty"):
        final_response_judge_verdicts_extra(
            {"": {"passed": True, "selected_rubric_score": 1, "reason": "Ok."}}
        )


def test_final_response_scorer_config_wire_shape():
    assert FinalResponseScorerConfig().model_dump(by_alias=True, mode="json") == {
        "includeJudgeTrace": False,
    }
    assert define_final_response_scorer_config(include_judge_trace=True) == {
        "finalResponse": {
            "includeJudgeTrace": True,
        }
    }

    config = define_eval_config(
        scorer_config={
            **define_trajectory_scorer_config(
                include_sub_agents=True,
                ignore_tools=["call_agent"],
            ),
            **define_final_response_scorer_config(include_judge_trace=True),
        }
    )

    assert config.model_dump(by_alias=True, mode="json")["scorerConfig"] == {
        "trajectory": {
            "includeSubAgents": True,
            "ignoreTools": ["call_agent"],
        },
        "finalResponse": {
            "includeJudgeTrace": True,
        },
    }


def test_score_weights_use_canonical_names_and_reject_invalid_values():
    weights = ScoreWeights(
        {
            "trajectory": 0.5,
            "planned_actions": 0.25,
            "executed_actions": 0.2,
            "final_response": 0.05,
        }
    )

    assert weights.as_dict() == {
        "trajectory": 0.5,
        "planned_actions": 0.25,
        "executed_actions": 0.2,
        "final_response": 0.05,
    }

    with pytest.raises(ValidationError, match="non-negative"):
        ScoreWeights({"trajectory": -1})

    with pytest.raises(ValidationError, match="unknown scorer"):
        ScoreWeights({"planner": 1})

    # The harness is new: legacy aliases are no longer accepted.
    with pytest.raises(ValidationError, match="unknown scorer"):
        ScoreWeights({"fusedExecutor": 1})


def test_raw_sample_output_normalizes_generic_resolved_actions():
    output = RawSampleOutput(
        actual_trajectory=["executePlan"],
        extra={
            "resolvedActions": [
                {
                    "type": "availability_change",
                    "payload": {
                        "targetAvailability": 100,
                        "productIds": ["p1"],
                    },
                }
            ],
            "customerMetadata": {"note": "kept"},
        },
    )

    assert output.model_dump(by_alias=True, mode="json")["extra"] == {
        "resolvedActions": [
            {
                "type": "availability_change",
                "payload": {
                    "targetAvailability": 100,
                    "productIds": ["p1"],
                },
            }
        ],
        "customerMetadata": {"note": "kept"},
    }


def test_raw_sample_output_rejects_old_action_shape():
    with pytest.raises(ValidationError):
        RawSampleOutput(
            actual_trajectory=["executePlan"],
            extra={
                "resolvedActions": [
                    {
                        "type": "price_change",
                        "changeType": "absolute",
                        "value": 10.0,
                    }
                ]
            },
        )


def test_legacy_text_only_ground_truth_is_rejected():
    with pytest.raises(ValidationError):
        GroundTruth.model_validate(
            {"kind": "textOnly", "text": "Legacy narrative expectation"}
        )


def test_old_action_ground_truth_fields_are_rejected():
    flat_shape = {
        "kind": "flat",
        "expectedActions": [
            {
                "actionType": "price_change",
                "payload": {"changeType": "absolute", "value": 10.0},
                "productIds": ["legacy-p1"],
                "productFingerprints": ["legacy-fp1"],
                "scope": {"channels": ["ONLINE"], "region": ["EU"]},
            }
        ],
        "expectedScope": {"channels": ["ONLINE"], "region": ["EU"]},
    }

    with pytest.raises(ValidationError):
        GroundTruth.model_validate(flat_shape)


def test_ground_truth_constructor_validates_flat_shape():
    # At least one bucket must be non-empty.
    with pytest.raises(ValidationError):
        define_expected_actions()

    action = define_expected_action("price_change", {"value": 1.0})
    planned = define_expected_actions(planned_actions=[action])
    assert planned.payload.kind == "flat"
    assert planned.payload.payload_match == "exact"
    assert planned.payload.planned_actions[0].type == "price_change"
    assert planned.payload.executed_actions == []

    executed = define_expected_actions(executed_actions=[action])
    assert executed.payload.executed_actions[0].type == "price_change"

    exact = define_expected_actions(executed_actions=[action], payload_match="exact")
    assert exact.payload.payload_match == "exact"
    assert exact.model_dump(by_alias=True, mode="json")["data"]["payloadMatch"] == "exact"

    with pytest.raises(ValidationError):
        define_expected_actions(
            executed_actions=[action],
            payload_match="strict",  # type: ignore[arg-type]
        )

    with pytest.raises(ValidationError):
        define_action_ground_truth(kind="multiGroup")  # type: ignore[arg-type]


def test_test_case_rejects_ambiguous_action_expectation_names():
    action = define_expected_action("price_change", {"value": 1.0})
    expected_actions = define_expected_actions(executed_actions=[action])

    with pytest.raises(ValueError, match="expected_actions or ground_truth"):
        define_test_case(
            "tc-ambiguous",
            "hello",
            expected_actions=expected_actions,
            ground_truth=expected_actions,
        )

    with pytest.raises(ValidationError, match="expected_actions or ground_truth"):
        EvalTestCase.model_validate(
            {
                "id": "tc-ambiguous",
                "input": "hello",
                "expectedActions": expected_actions.model_dump(
                    by_alias=True,
                    mode="json",
                ),
                "structuredGroundTruth": expected_actions.model_dump(
                    by_alias=True,
                    mode="json",
                ),
            }
        )


def test_scorer_preset_constructor_normalizes_weights():
    preset = define_scorer_preset(
        "sequential",
        weights={"trajectory": 0.5, "executed_actions": 0.5},
    )

    assert preset.model_dump(by_alias=True, mode="json") == {
        "name": "sequential",
        "weights": {"trajectory": 0.5, "executed_actions": 0.5},
    }


def test_scorer_preset_direct_construction_normalizes_weights():
    preset = ScorerPreset(
        name="sequential",
        weights={"trajectory": 0.5, "executed_actions": 0.5},
    )

    assert preset.model_dump(by_alias=True, mode="json")["weights"] == {
        "trajectory": 0.5,
        "executed_actions": 0.5,
    }


def test_eval_artifact_and_event_envelope_shapes_match_rust_contract():
    artifact_json = {
        "runId": "eval-run-id",
        "tenantId": "tenant-acme",
        "workspaceId": "workspace-main",
        "mode": "sequential",
        "summary": {
            "totalTestCases": 1,
            "passed": 1,
            "failed": 0,
            "skipped": 0,
            "aggregateScore": 1.0,
            "passRate": 1.0,
            "passAtK": [
                {
                    "k": 1,
                    "simpleEstimate": 1.0,
                    "unbiasedEstimate": 1.0,
                    "numSamples": 1,
                    "numCorrect": 1,
                }
            ],
            "totalDurationMs": 10,
            "totalUsage": {
                "inputTokens": 1,
                "outputTokens": 2,
                "cachedTokens": 0,
                "cacheCreationTokens": 0,
            },
            "cost": None,
            "latency": None,
            "metadata": None,
        },
        "testCases": [
            {
                "testCaseId": "tc-1",
                "input": "hello",
                "samples": [
                    {
                        "sampleIndex": 0,
                        "passed": True,
                        "aggregateScore": 1.0,
                        "componentScores": [
                            {
                                "scorerName": "trajectory",
                                "score": 1.0,
                                "details": {
                                    "mode": "unordered",
                                    "passed": True,
                                    "expected": ["plan"],
                                    "actual": ["plan"],
                                    "matched": ["plan"],
                                    "missing": [],
                                    "unexpected": [],
                                    "diagnostics": {
                                        "precision": 1.0,
                                        "recall": 1.0,
                                        "f1": 1.0,
                                        "f2": 1.0,
                                    },
                                },
                            }
                        ],
                        "actualTrajectory": ["plan"],
                        "responseText": "I updated Acme Corp's billing contact.",
                        "finalResponseEval": None,
                        "plannedActions": [],
                        "resolvedActions": [],
                        "durationMs": 10,
                        "modelInvocations": [],
                        "tokenUsage": {
                            "inputTokens": 1,
                            "outputTokens": 2,
                            "cachedTokens": 0,
                            "cacheCreationTokens": 0,
                        },
                        "cost": None,
                        "latency": None,
                        "threadId": "eval-run-id-tc-1-0",
                        "trace": None,
                        "metadata": None,
                        "error": None,
                    }
                ],
                "passAtK": [
                    {
                        "k": 1,
                        "simpleEstimate": 1.0,
                        "unbiasedEstimate": 1.0,
                        "numSamples": 1,
                        "numCorrect": 1,
                    }
                ],
                "aggregateScore": 1.0,
            }
        ],
        "metadata": {
            "schemaVersion": 1,
            "scorerPreset": "sequential",
            "scoreWeights": {"trajectory": 0.5, "executed_actions": 0.5},
        },
    }
    artifact = EvalArtifact.model_validate(artifact_json)
    envelope = HarnessEvalEventEnvelope.model_validate(
        {
            "runId": "eval-run-id",
            "sequence": 0,
            "type": "evalStarted",
            "data": {"artifact": artifact_json},
        }
    )

    assert artifact.model_dump(by_alias=True, mode="json") == artifact_json
    assert envelope.event.data.artifact == artifact
    assert envelope.model_dump(by_alias=True, mode="json") == {
        "runId": "eval-run-id",
        "sequence": 0,
        "type": "evalStarted",
        "data": {"artifact": artifact_json},
    }
    assert json.loads(envelope.model_dump_json(by_alias=True)) == {
        "runId": "eval-run-id",
        "sequence": 0,
        "type": "evalStarted",
        "data": {"artifact": artifact_json},
    }
    assert artifact.summary.pass_at_k[0].simple_estimate == 1.0
    assert (
        artifact.test_cases[0]
        .samples[0]
        .component_scores[0]
        .details["diagnostics"]["f2"]
        == 1.0
    )


def test_eval_event_data_rejects_unknown_variant_keys():
    with pytest.raises(ValidationError):
        HarnessEvalEventEnvelope.model_validate(
            {
                "runId": "eval-run-id",
                "sequence": 0,
                "type": "testCaseStarted",
                "data": {"frobnicate": "tc-1"},
            }
        )


def test_ground_truth_action_payload_dicts_are_typed_and_validated():
    # Both buckets empty is rejected by the at-least-one validator.
    with pytest.raises(ValidationError):
        GroundTruth.model_validate(
            {
                "kind": "structured",
                "data": {"kind": "flat", "plannedActions": [], "executedActions": []},
            }
        )

    ground_truth = GroundTruth.model_validate(
        {
            "kind": "structured",
            "data": {
                "kind": "flat",
                "executedActions": [
                    {"type": "price_change", "payload": {"value": 1.0}}
                ],
            },
        }
    )

    assert ground_truth.payload.executed_actions[0].type == "price_change"


def test_resolved_action_constructor_uses_camel_case_wire_shape():
    action = define_resolved_action(
        "availability_change",
        {
            "targetAvailability": 100,
            "productIds": ["p1"],
            "context": {"channels": ["ONLINE"]},
        },
    )

    assert action.model_dump(by_alias=True, mode="json") == {
        "type": "availability_change",
        "payload": {
            "targetAvailability": 100,
            "productIds": ["p1"],
            "context": {"channels": ["ONLINE"]},
        },
    }


@pytest.mark.parametrize(
    "trajectory_mode",
    ["strict", "unordered", "subset", "superset", "subsequence"],
)
def test_test_case_trajectory_modes_use_current_canonical_wire_names(trajectory_mode):
    test_case = define_test_case(
        "tc-1",
        "hello",
        expected_trajectory=["plan", "execute"],
        trajectory_mode=trajectory_mode,
    )

    assert test_case.model_dump(by_alias=True, mode="json")["trajectoryMode"] == trajectory_mode


def test_score_sample_scores_known_trajectory_without_runtime():
    scored = score_sample(
        define_test_case("tc-1", "hello", expected_trajectory=["plan", "answer"]),
        RawSampleOutput(actual_trajectory=["plan", "answer"]),
        scorer_preset="trajectory_only",
    )

    assert scored.aggregate == 1.0
    assert scored.component_scores[0].scorer_name == "trajectory"
    assert scored.component_scores[0].score == 1.0
    details = scored.component_scores[0].details
    assert details["passed"] is True
    assert details["expected"] == ["plan", "answer"]
    assert details["actual"] == ["plan", "answer"]
    assert details["matched"] == ["plan", "answer"]
    assert details["missing"] == []
    assert details["unexpected"] == []
    assert details["diagnostics"]["f2"] == 1.0
    assert "fScore" not in details


def test_score_sample_reports_stale_native_extension_version(monkeypatch):
    monkeypatch.setattr(evals_module._internal, "native_api_version", lambda: 1)

    with pytest.raises(RuntimeError, match="rebuild/reinstall the extension") as exc:
        score_sample(
            define_test_case("tc-1", "hello", expected_trajectory=["plan"]),
            RawSampleOutput(actual_trajectory=["plan"]),
            scorer_preset="trajectory_only",
        )

    assert "expected native API version" in str(exc.value)


def test_score_sample_wraps_native_unknown_field_errors(monkeypatch):
    def raise_unknown_field(*_args):
        raise ValueError("unknown field `finalResponse`, expected `id`")

    monkeypatch.setattr(evals_module._internal, "score_eval_sample", raise_unknown_field)

    with pytest.raises(RuntimeError, match="rebuild/reinstall the extension") as exc:
        score_sample(
            define_test_case("tc-1", "hello", expected_trajectory=["plan"]),
            RawSampleOutput(actual_trajectory=["plan"]),
            scorer_preset="trajectory_only",
        )

    message = str(exc.value)
    assert "Raw native detail" in message
    assert "unknown field `finalResponse`" in message


def test_score_sample_scores_final_response_deterministic_scorers():
    scored = score_sample(
        define_test_case(
            "tc-response",
            "Update Acme's billing contact.",
            final_response=FinalResponseEval(
                scorers=[
                    ResponseScorer.contains(
                        id="mentions_email",
                        text="jane@example.com",
                        weight=2,
                    ),
                    ResponseScorer.regex(
                        id="mentions_ticket",
                        pattern=r"TICKET-[0-9]+",
                    ),
                ],
                pass_threshold=0.5,
            ),
        ),
        RawSampleOutput(
            response_text="Updated Acme's billing contact to jane@example.com."
        ),
        score_weights={"final_response": 1},
    )

    final_response_score = scored.component_scores[0]
    assert final_response_score.scorer_name == "final_response"
    assert final_response_score.score == pytest.approx(2 / 3)
    assert final_response_score.details["passed"] is True
    assert final_response_score.details["score"] == pytest.approx(2 / 3)
    assert final_response_score.details["responseScorers"][0]["id"] == "mentions_email"
    assert scored.aggregate == pytest.approx(2 / 3)


def test_score_sample_scores_precomputed_judge_final_response():
    output = RawSampleOutput(
        response_text="Updated Acme's billing contact to jane@example.com."
    ).with_judge_verdicts(
        {
            "reports_success": JudgeVerdict(
                passed=True,
                selected_rubric_score=1,
                reason="The response reports the successful billing update.",
            ),
            "does_not_claim_refund": JudgeVerdict(
                passed=False,
                selected_rubric_score=0,
                reason="The response is silent about refunds.",
            ),
        }
    )

    scored = score_sample(
        define_test_case(
            "tc-response",
            "Update Acme's billing contact.",
            final_response=FinalResponseEval(
                scorers=[
                    ResponseScorer.judge(
                        id="reports_success",
                        instructions="Pass when the response reports the update.",
                        weight=2,
                    ),
                    ResponseScorer.judge(
                        id="does_not_claim_refund",
                        instructions="Pass when the response does not claim a refund.",
                    ),
                    ResponseScorer.contains(
                        id="mentions_email",
                        text="jane@example.com",
                    ),
                ],
                pass_threshold=0.75,
            ),
        ),
        output,
        score_weights={"final_response": 1},
    )

    final_response_score = scored.component_scores[0]
    assert final_response_score.scorer_name == "final_response"
    assert final_response_score.score == pytest.approx(0.75)
    assert final_response_score.details["passed"] is True
    assert (
        final_response_score.details["responseScorers"][0]["details"]["verdict"][
            "reason"
        ]
        == "The response reports the successful billing update."
    )
    assert scored.aggregate == pytest.approx(0.75)


def test_score_sample_required_precomputed_judge_failure_gates_final_response():
    output = RawSampleOutput(response_text="Billing was updated.").with_judge_verdicts(
        {
            "reports_success": {
                "passed": True,
                "selected_rubric_score": 1,
                "reason": "The response reports success.",
            },
            "does_not_claim_refund": {
                "passed": False,
                "selected_rubric_score": 0,
                "reason": "The response incorrectly claims a refund.",
            },
        }
    )

    scored = score_sample(
        define_test_case(
            "tc-response-required",
            "Update billing and do not mention refunds.",
            final_response=FinalResponseEval(
                scorers=[
                    ResponseScorer.judge(
                        id="reports_success",
                        instructions="Pass when the response reports success.",
                        weight=4,
                    ),
                    ResponseScorer.judge(
                        id="does_not_claim_refund",
                        instructions="Pass when the response does not claim a refund.",
                        required=True,
                    ),
                ],
                pass_threshold=0.5,
            ),
        ),
        output,
        score_weights={"final_response": 1},
    )

    final_response_score = scored.component_scores[0]
    assert final_response_score.score == 0.0
    assert final_response_score.details["score"] == pytest.approx(0.8)
    assert final_response_score.details["passed"] is False
    assert final_response_score.details["requiredFailed"] == ["does_not_claim_refund"]
    assert scored.aggregate == 0.0


def test_score_sample_honors_trajectory_mode_semantics():
    actual = RawSampleOutput(actual_trajectory=["a", "x", "b"])

    unordered = score_sample(
        define_test_case(
            "tc-unordered",
            "hello",
            expected_trajectory=["a", "b"],
            trajectory_mode="unordered",
        ),
        actual,
        scorer_preset="trajectory_only",
    )
    strict = score_sample(
        define_test_case(
            "tc-strict",
            "hello",
            expected_trajectory=["a", "b"],
            trajectory_mode="strict",
        ),
        actual,
        scorer_preset="trajectory_only",
    )
    superset = score_sample(
        define_test_case(
            "tc-superset",
            "hello",
            expected_trajectory=["a", "b"],
            trajectory_mode="superset",
        ),
        actual,
        scorer_preset="trajectory_only",
    )
    subsequence = score_sample(
        define_test_case(
            "tc-subsequence",
            "hello",
            expected_trajectory=["a", "b"],
            trajectory_mode="subsequence",
        ),
        actual,
        scorer_preset="trajectory_only",
    )

    assert unordered.aggregate == 0.0
    assert strict.aggregate == 0.0
    assert superset.aggregate == 1.0
    assert subsequence.aggregate == 1.0
    assert unordered.component_scores[0].details["mode"] == "unordered"
    assert strict.component_scores[0].details["mode"] == "strict"
    assert superset.component_scores[0].details["mode"] == "superset"
    assert subsequence.component_scores[0].details["mode"] == "subsequence"
    assert strict.component_scores[0].details["passed"] is False
    assert strict.component_scores[0].details["unexpected"] == ["x"]
    assert strict.component_scores[0].details["diagnostics"]["f2"] == pytest.approx(10 / 11)
    assert superset.component_scores[0].details["passed"] is True
    assert superset.component_scores[0].details["unexpected"] == ["x"]
    assert superset.component_scores[0].details["diagnostics"]["precision"] == pytest.approx(2 / 3)
    assert "fScore" not in superset.component_scores[0].details


def test_score_sample_projects_configured_sub_agent_trajectory():
    scored = score_sample(
        define_test_case(
            "tc-sub-agent-trajectory",
            "Update the customer through the planner and executor.",
            expected_trajectory=[
                "storePlan",
                "executePlan",
            ],
            trajectory_mode="subsequence",
        ),
        RawSampleOutput(
            actual_trajectory=["call_agent", "storePlan", "call_agent", "executePlan"],
            extra={
                "trajectoryEvents": [
                    {
                        "kind": "agent",
                        "name": "coordinator",
                        "invocationId": "agent-root",
                        "depth": 1,
                    },
                    {
                        "kind": "tool",
                        "name": "call_agent",
                        "agent": "coordinator",
                        "invocationId": "tool-1",
                        "depth": 1,
                    },
                    {
                        "kind": "agent",
                        "name": "planner",
                        "agent": "coordinator",
                        "invocationId": "agent-planner",
                        "depth": 2,
                    },
                    {
                        "kind": "tool",
                        "name": "storePlan",
                        "agent": "planner",
                        "invocationId": "tool-2",
                        "depth": 2,
                    },
                    {
                        "kind": "tool",
                        "name": "call_agent",
                        "agent": "coordinator",
                        "invocationId": "tool-3",
                        "depth": 1,
                    },
                    {
                        "kind": "agent",
                        "name": "executor",
                        "agent": "coordinator",
                        "invocationId": "agent-executor",
                        "depth": 2,
                    },
                    {
                        "kind": "tool",
                        "name": "executePlan",
                        "agent": "executor",
                        "invocationId": "tool-4",
                        "depth": 2,
                    },
                ]
            },
        ),
        scorer_preset="trajectory_only",
        scorer_config=define_trajectory_scorer_config(
            include_sub_agents=True,
            ignore_tools=["call_agent"],
        ),
    )

    detail = scored.component_scores[0].details
    assert scored.aggregate == 1.0
    assert detail["actual"] == ["storePlan", "executePlan"]
    assert detail["matched"] == ["storePlan", "executePlan"]
    assert detail["unexpected"] == []
    assert detail["projection"]["source"] == "trajectoryEvents"
    assert detail["projection"]["scoredTrajectory"] == ["storePlan", "executePlan"]


def test_score_sample_executor_preset_scores_authored_trajectory():
    scored = score_sample(
        define_test_case(
            "tc-executor-trajectory",
            "Update the customer billing contact.",
            expected_trajectory=["lookupCustomer", "updateCustomer"],
            trajectory_mode="subsequence",
            expected_actions=define_action_ground_truth(
                executed_actions=[
                    define_expected_action(
                        "update_customer",
                        {"customerId": "cust_123", "email": "jane@example.com"},
                    )
                ]
            ),
        ),
        RawSampleOutput(
            actual_trajectory=["lookupCustomer", "updateCustomer"],
            extra={
                "resolvedActions": [
                    define_resolved_action(
                        "update_customer",
                        {"customerId": "cust_123", "email": "jane@example.com"},
                    ).model_dump(by_alias=True, mode="json")
                ]
            },
        ),
        scorer_preset="executor",
    )

    names = [score.scorer_name for score in scored.component_scores]
    assert names == ["trajectory", "executed_actions", "composite"]
    assert scored.aggregate == 1.0


def test_score_sample_action_subset_matches_semantic_numbers():
    scored = score_sample(
        define_test_case(
            "tc-action-numeric",
            "Set product availability.",
            expected_actions=define_action_ground_truth(
                executed_actions=[
                    define_expected_action(
                        "availability_change",
                        {
                            "availabilityChanges": [
                                {"type": "SetAbsolute", "value": 1}
                            ]
                        },
                    )
                ],
                payload_match="subset",
            ),
        ),
        RawSampleOutput(
            extra={
                "resolvedActions": [
                    define_resolved_action(
                        "availability_change",
                        {
                            "availabilityChanges": [
                                {"type": "SetAbsolute", "value": 1.0}
                            ]
                        },
                    ).model_dump(by_alias=True, mode="json")
                ]
            },
        ),
        scorer_preset="executor",
    )

    executed = next(
        score for score in scored.component_scores if score.scorer_name == "executed_actions"
    )
    assert scored.aggregate == 1.0
    assert executed.details["summary"]["exactCount"] == 1


def test_score_sample_action_subset_matches_unordered_scalar_arrays():
    scored = score_sample(
        define_test_case(
            "tc-action-array-order",
            "Delist products.",
            expected_actions=define_action_ground_truth(
                executed_actions=[
                    define_expected_action(
                        "delist_products",
                        {"productIds": ["a", "b", "c"]},
                    )
                ],
                payload_match="subset",
            ),
        ),
        RawSampleOutput(
            extra={
                "resolvedActions": [
                    define_resolved_action(
                        "delist_products",
                        {
                            "productIds": ["c", "a", "b"],
                            "reason": "seasonal cleanup",
                        },
                    ).model_dump(by_alias=True, mode="json")
                ]
            },
        ),
        scorer_preset="executor",
    )

    executed = next(
        score for score in scored.component_scores if score.scorer_name == "executed_actions"
    )
    assert scored.aggregate == 1.0
    assert executed.details["summary"]["exactCount"] == 1


def test_score_sample_specialist_defaults_to_final_response_expectation():
    scored = score_sample(
        define_test_case(
            "tc-specialist-response",
            "Summarize the customer.",
            final_response=FinalResponseEval(
                scorers=[
                    ResponseScorer.contains(
                        id="mentions_customer",
                        text="customer",
                    )
                ]
            ),
        ),
        RawSampleOutput(response_text="The customer is ready for follow-up."),
        mode="specialist",
    )

    names = [score.scorer_name for score in scored.component_scores]
    assert names == ["final_response", "composite"]
    assert scored.aggregate == 1.0


def test_score_sample_specialist_defaults_to_trajectory_expectation():
    scored = score_sample(
        define_test_case(
            "tc-specialist-trajectory",
            "Read catalog metadata.",
            expected_trajectory=["get_catalog_entities"],
        ),
        RawSampleOutput(actual_trajectory=["get_catalog_entities"]),
        mode="specialist",
    )

    names = [score.scorer_name for score in scored.component_scores]
    assert names == ["trajectory", "composite"]
    assert scored.aggregate == 1.0


def test_score_sample_specialist_defaults_to_executed_action_expectation():
    scored = score_sample(
        define_test_case(
            "tc-specialist-action",
            "Update the customer.",
            expected_actions=define_action_ground_truth(
                executed_actions=[
                    define_expected_action(
                        "update_customer",
                        {"customerId": "cust_123"},
                    )
                ]
            ),
        ),
        RawSampleOutput(
            extra={
                "resolvedActions": [
                    define_resolved_action(
                        "update_customer",
                        {"customerId": "cust_123"},
                    ).model_dump(by_alias=True, mode="json")
                ]
            },
        ),
        mode="specialist",
    )

    names = [score.scorer_name for score in scored.component_scores]
    assert names == ["executed_actions", "composite"]
    assert scored.aggregate == 1.0


def test_score_sample_specialist_rejects_empty_default_expectations():
    with pytest.raises(ValueError, match="no scoreable expectations"):
        score_sample(
            define_test_case("tc-specialist-empty", "hello"),
            RawSampleOutput(actual_trajectory=[]),
            mode="specialist",
        )


def test_score_sample_specialist_explicit_trajectory_allows_empty_expected_trajectory():
    scored = score_sample(
        define_test_case("tc-specialist-no-tools", "Answer without tools."),
        RawSampleOutput(actual_trajectory=[]),
        mode="specialist",
        score_weights={"trajectory": 1.0},
    )

    names = [score.scorer_name for score in scored.component_scores]
    assert names == ["trajectory", "composite"]
    assert scored.aggregate == 1.0


def test_score_sample_specialist_final_response_weight_requires_spec():
    with pytest.raises(ValueError, match="finalResponse"):
        score_sample(
            define_test_case("tc-specialist-response-missing", "hello"),
            RawSampleOutput(response_text="hello"),
            mode="specialist",
            score_weights={"final_response": 1.0},
        )


def test_score_sample_rejects_specialist_mode_with_sequential_preset():
    with pytest.raises(ValueError, match="conflicts"):
        score_sample(
            define_test_case(
                "tc-specialist-trajectory",
                "Read catalog metadata.",
                expected_trajectory=["get_catalog_entities"],
            ),
            RawSampleOutput(actual_trajectory=["get_catalog_entities"]),
            mode="specialist",
            scorer_preset="sequential",
        )


def test_score_sample_rejects_conflicting_mode_and_preset():
    with pytest.raises(ValueError, match="conflicts"):
        score_sample(
            define_test_case("tc-1", "hello"),
            RawSampleOutput(actual_trajectory=[]),
            mode="planner",
            scorer_preset="executor",
        )


def test_eval_request_rejects_trajectory_only_with_final_response():
    with pytest.raises(ValidationError, match="trajectory_only.*final_response"):
        EvalRequest(
            tenant_id="tenant-acme",
            workspace_id="workspace-main",
            config=define_eval_config(samples_per_case=1, concurrency=1),
            scorer_preset="trajectory_only",
            test_cases=[
                define_test_case(
                    "tc-response",
                    "hello",
                    final_response=FinalResponseEval(
                        scorers=[
                            ResponseScorer.contains(
                                id="mentions_done",
                                text="done",
                            )
                        ]
                    ),
                )
            ],
        )


def test_runtime_run_eval_bridge_returns_typed_artifact():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=1, concurrency=1),
        test_cases=[define_test_case("tc-1", "hello")],
    )

    async def run_eval():
        return await runtime.run_eval(request)

    raw_artifact = asyncio.run(run_eval())
    artifact = EvalArtifact.model_validate(raw_artifact)

    assert artifact.tenant_id == "tenant-acme"
    assert artifact.workspace_id == "workspace-main"
    assert artifact.mode == "sequential"
    assert artifact.test_cases[0].test_case_id == "tc-1"
    assert artifact.test_cases[0].samples[0].resolved_actions == []
    assert artifact.test_cases[0].samples[0].planned_actions == []
    assert artifact.metadata.score_weights == {
        "trajectory": 0.5,
        "planned_actions": 0.25,
        "executed_actions": 0.25,
    }


def test_define_eval_request_defaults_tenant_from_runtime_and_run_eval_sync():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = define_eval_request(
        runtime,
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=1, concurrency=1),
        test_cases=[define_test_case("tc-1", "hello")],
    )

    artifact = run_eval_sync(runtime, request)

    assert request.tenant_id == "tenant-acme"
    assert runtime.resource_id == "tenant-acme"
    assert artifact.tenant_id == "tenant-acme"
    assert artifact.workspace_id == "workspace-main"
    assert artifact.test_cases[0].test_case_id == "tc-1"


def test_runtime_run_eval_scripted_captures_tool_trajectory():
    calls = []

    @define_tool("record_event", {"value": str}, approval="never")
    def record_event(args, ctx):
        calls.append((args, ctx["tool_use_id"]))
        return {"recorded": args["value"]}

    coordinator = define_coordinator(
        "coordinator",
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        routes=["helper"],
        tools=[record_event],
    )
    helper = define_specialist(
        "helper",
        model="claude-sonnet-4-6",
        prompt="Unused helper route.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, helper],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        interpreter="scripted",
    )
    request = define_eval_request(
        runtime,
        workspace_id="workspace-main",
        config=define_eval_config(
            samples_per_case=1,
            concurrency=1,
            score_weights={"trajectory": 1.0},
        ),
        test_cases=[
            define_test_case(
                "tc-scripted-tool",
                json.dumps({"tool": "record_event", "args": {"value": "ok"}}),
                expected_trajectory=["record_event"],
            )
        ],
    )

    artifact = run_eval_sync(runtime, request)
    sample = artifact.test_cases[0].samples[0]

    assert calls == [({"value": "ok"}, "scripted-tool-1")]
    assert sample.actual_trajectory == ["record_event"]
    assert sample.aggregate_score == 1.0
    assert sample.metadata["trajectoryEvents"][0]["name"] == "coordinator"
    assert sample.metadata["trajectoryEvents"][1]["name"] == "record_event"


def test_runtime_run_eval_scripted_captures_nested_sub_agent_tools():
    def run_once():
        plan_id = "eval-plan-1"
        plan_spec = define_plan(
            "EvalPlan",
            {
                "type": "object",
                "required": ["rationale", "actions"],
                "properties": {
                    "rationale": {"type": "string"},
                    "actions": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "required": ["kind"],
                            "properties": {
                                "kind": {"type": "string"},
                                "message": {"type": "string"},
                                "references": {"type": "array"},
                            },
                        },
                    },
                },
            },
        )
        plan_body = {
            "rationale": "deterministic eval plan",
            "actions": [{"kind": "record_counter", "message": "record eval action"}],
        }
        coordinator_script = json.dumps(
            {
                "script": [
                    {
                        "tool": "call_agent",
                        "args": {
                            "agent": "planner",
                            "prompt": json.dumps(
                                {
                                    "tool": "storePlan",
                                    "args": {
                                        "specName": "EvalPlan",
                                        "planId": plan_id,
                                        "body": plan_body,
                                    },
                                }
                            ),
                        },
                    },
                    {
                        "tool": "call_agent",
                        "args": {
                            "agent": "executor",
                            "prompt": json.dumps(
                                {"tool": "executePlan", "args": {"planId": plan_id}}
                            ),
                        },
                    },
                ]
            }
        )
        action_dispatches = []

        def dispatch_actions(actions, ctx):
            action_dispatches.append((actions, ctx))
            return {
                "entitiesAffected": len(actions),
                "summary": f"executed {len(actions)} action(s)",
                "details": None,
            }

        coordinator = define_coordinator(
            "coordinator",
            model="claude-sonnet-4-6",
            prompt="Delegate to planner and executor.",
            routes=["planner", "executor"],
        )
        planner = define_planner(
            "planner",
            model="claude-sonnet-4-6",
            prompt="Store the scripted plan.",
            plan=plan_spec,
        )
        executor = define_executor(
            "executor",
            model="claude-sonnet-4-6",
            prompt="Execute the scripted plan.",
            plan=plan_spec,
        )
        runtime = create_runtime(
            define_runtime(
                tenant=define_tenant("tenant-acme", "v1"),
                agents=[coordinator, planner, executor],
                providers={"anthropic": {"apiKey": "unused"}},
            ),
            action_dispatcher=dispatch_actions,
            interpreter="scripted",
        )
        request = define_eval_request(
            runtime,
            workspace_id="workspace-main",
            config=define_eval_config(
                samples_per_case=1,
                concurrency=1,
                score_weights={"trajectory": 0.5, "executed_actions": 0.5},
                scorer_config=define_trajectory_scorer_config(
                    include_sub_agents=True,
                    ignore_tools=["call_agent"],
                ),
            ),
            test_cases=[
                define_test_case(
                    "tc-nested-scripted-tools",
                    coordinator_script,
                    expected_trajectory=["storePlan", "executePlan"],
                    trajectory_mode="subsequence",
                    ground_truth=define_action_ground_truth(
                        executed_actions=[
                            define_expected_action(
                                "record_counter",
                                {"message": "record eval action"},
                            )
                        ],
                    ),
                )
            ],
        )

        artifact = run_eval_sync(runtime, request)
        sample = artifact.test_cases[0].samples[0]
        trajectory_score = next(
            score for score in sample.component_scores if score.scorer_name == "trajectory"
        )
        executed_score = next(
            score
            for score in sample.component_scores
            if score.scorer_name == "executed_actions"
        )
        signature = {
            "actualTrajectory": sample.actual_trajectory,
            "trajectoryEvents": sample.metadata["trajectoryEvents"],
            "scoredTrajectory": trajectory_score.details["projection"][
                "scoredTrajectory"
            ],
            "plannedActions": [
                action.model_dump(by_alias=True, mode="json")
                for action in sample.planned_actions
            ],
            "resolvedActions": [
                action.model_dump(by_alias=True, mode="json")
                for action in sample.resolved_actions
            ],
        }
        return sample, trajectory_score, executed_score, action_dispatches, signature

    signatures = []
    for _ in range(10):
        sample, trajectory_score, executed_score, action_dispatches, signature = run_once()
        events = signature["trajectoryEvents"]

        assert action_dispatches == [
            (
                [
                    {
                        "kind": "record_counter",
                        "payload": {"message": "record eval action"},
                        "references": [],
                    }
                ],
                {"resolved_refs": {}},
            )
        ]
        assert signature["actualTrajectory"] == [
            "call_agent",
            "storePlan",
            "call_agent",
            "executePlan",
        ]
        assert signature["scoredTrajectory"] == ["storePlan", "executePlan"]
        assert sum(
            1
            for event in events
            if event["kind"] == "tool"
            and event["name"] == "storePlan"
            and event.get("agent") == "planner"
            and event["depth"] == 2
        ) == 1
        assert sum(
            1
            for event in events
            if event["kind"] == "tool"
            and event["name"] == "executePlan"
            and event.get("agent") == "executor"
            and event["depth"] == 2
        ) == 1
        assert signature["plannedActions"] == [
            {
                "type": "record_counter",
                "payload": {"message": "record eval action"},
            }
        ]
        assert signature["resolvedActions"] == [
            {
                "type": "record_counter",
                "payload": {"message": "record eval action"},
            }
        ]
        assert trajectory_score.score == 1.0
        assert executed_score.score == 1.0
        assert sample.aggregate_score == 1.0
        signatures.append(signature)

    assert signatures == [signatures[0]] * len(signatures)


def test_runtime_run_eval_scores_final_response_artifact():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=1, concurrency=1),
        test_cases=[
            define_test_case(
                "tc-response",
                "hello",
                final_response=FinalResponseEval(
                    scorers=[
                        ResponseScorer.contains(
                            id="mentions_mocked_response",
                            text="mocked eval response",
                        )
                    ]
                ),
            )
        ],
    )

    async def run_eval():
        return await runtime.run_eval(request)

    raw_artifact = asyncio.run(run_eval())
    artifact = EvalArtifact.model_validate(raw_artifact)
    sample = artifact.test_cases[0].samples[0]

    assert artifact.metadata.score_weights["final_response"] == pytest.approx(1 / 3)
    assert "mocked eval response" in sample.response_text
    assert sample.final_response_eval["passed"] is True
    assert (
        sample.final_response_eval["responseScorers"][0]["id"]
        == "mentions_mocked_response"
    )


def test_runtime_run_eval_does_not_use_mock_response_as_judge_verdict():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=1, concurrency=1),
        test_cases=[
            define_test_case(
                "tc-response",
                "hello",
                final_response=FinalResponseEval(
                    scorers=[
                        ResponseScorer.judge(
                            id="judge_similarity",
                            instructions=(
                                "Pass when the response is similar to the reference."
                            ),
                            reference_response="The billing contact was updated.",
                        )
                    ]
                ),
            )
        ],
    )

    async def run_eval():
        return await runtime.run_eval(request)

    raw_artifact = asyncio.run(run_eval())
    artifact = EvalArtifact.model_validate(raw_artifact)
    sample = artifact.test_cases[0].samples[0]
    response_scorer = sample.final_response_eval["responseScorers"][0]

    assert sample.final_response_eval["passed"] is False
    assert response_scorer["passed"] is False
    assert (
        response_scorer["details"]["errorKind"]
        == "judge_provider_unavailable"
    )
    assert "judge-capable interpreter" in response_scorer["reason"]
    assert all(invocation.agent != "judge" for invocation in sample.model_invocations)


def test_runtime_stream_eval_bridge_returns_contract_events():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=1, concurrency=1),
        test_cases=[define_test_case("tc-1", "hello")],
    )

    async def collect_events():
        return [event async for event in runtime.stream_eval(request)]

    raw_events = asyncio.run(collect_events())
    events = [HarnessEvalEventEnvelope.model_validate(event) for event in raw_events]

    assert [event.event.type for event in events] == [
        "evalStarted",
        "testCaseStarted",
        "sampleCompleted",
        "testCaseCompleted",
        "evalCompleted",
    ]
    assert all(event.run_id == events[0].run_id for event in events)
    assert [event.sequence for event in events] == [0, 1, 2, 3, 4]
    assert raw_events[0]["data"]["artifact"]["tenantId"] == "tenant-acme"
    assert raw_events[2]["data"]["sample"]["resolvedActions"] == []


def test_runtime_stream_eval_can_be_cancelled_by_breaking_iteration():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a deterministic response.",
        routes=["planner"],
    )
    planner = AgentSpec(
        name="planner",
        role="planner",
        model="claude-sonnet-4-6",
        system_prompt="Plan.",
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("tenant-acme", "v1"),
            agents=[coordinator, planner],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        testing={"mock_response": "mocked eval response"},
    )
    request = EvalRequest(
        tenant_id="tenant-acme",
        workspace_id="workspace-main",
        config=define_eval_config(samples_per_case=2, concurrency=1),
        test_cases=[define_test_case("tc-1", "hello")],
    )

    async def first_event_only():
        async for event in runtime.stream_eval(request):
            return event
        raise AssertionError("stream ended before first event")

    event = HarnessEvalEventEnvelope.model_validate(asyncio.run(first_event_only()))

    assert event.event.type == "evalStarted"
    assert event.sequence == 0
