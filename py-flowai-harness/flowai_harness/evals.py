from __future__ import annotations

import asyncio
import json
import math
import re
from collections.abc import Mapping
from typing import Annotated, Any, Literal

from pydantic import (
    AliasChoices,
    BaseModel,
    ConfigDict,
    Field,
    TypeAdapter,
    field_validator,
    model_serializer,
    model_validator,
)
from pydantic.alias_generators import to_camel

from flowai_harness import _internal
from flowai_harness._native import call_native

EvalMode = Literal["planner", "executor", "sequential", "specialist", "testCaseBuilder"]
TrajectoryMode = Literal["strict", "unordered", "subset", "superset", "subsequence"]
AggregationStrategy = Literal["passRate", "meanScore"]
ScorerPresetName = Literal[
    "trajectory_only",
    "planner",
    "executor",
    "sequential",
    "specialist",
    "test_case_builder",
]
ScorerName = Literal["trajectory", "planned_actions", "executed_actions", "final_response"]
ResponseScorerMethod = Literal["judge", "exact", "contains", "regex"]
ActionPayloadMatchMode = Literal["subset", "exact"]
FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY = "finalResponseJudgeVerdicts"

# Canonical scorer names. The harness is new, so there are no compatibility
# aliases: weight keys must be one of these exact names.
_SCORER_ALIASES: dict[str, ScorerName] = {
    "trajectory": "trajectory",
    "planned_actions": "planned_actions",
    "executed_actions": "executed_actions",
    "final_response": "final_response",
}
_SCORER_NAME_HELP = ", ".join(_SCORER_ALIASES)


class _EvalModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        frozen=True,
        extra="forbid",
        hide_input_in_errors=True,
    )


class TokenUsageSummary(_EvalModel):
    input_tokens: int = 0
    output_tokens: int = 0
    cached_tokens: int = 0
    cache_creation_tokens: int = 0


class ScoreWeights(_EvalModel):
    weights: dict[ScorerName, float]

    def __init__(self, weights: Mapping[str, float] | None = None, **data: Any) -> None:
        if weights is not None:
            if "weights" in data:
                raise TypeError("ScoreWeights accepts either positional weights or weights=, not both")
            data["weights"] = weights
        super().__init__(**data)

    @model_validator(mode="before")
    @classmethod
    def _coerce_mapping(cls, value: Any) -> Any:
        if isinstance(value, ScoreWeights):
            return value
        if isinstance(value, Mapping) and "weights" not in value:
            return {"weights": dict(value)}
        return value

    @field_validator("weights", mode="before")
    @classmethod
    def _normalize_weights(cls, value: Any) -> dict[str, float]:
        if not isinstance(value, Mapping):
            raise TypeError("score weights must be a mapping")
        result: dict[str, float] = {}
        for raw_name, raw_weight in value.items():
            name = _SCORER_ALIASES.get(str(raw_name))
            if name is None:
                raise ValueError(
                    f"unknown scorer weight key '{raw_name}'; expected one of: "
                    f"{_SCORER_NAME_HELP}"
                )
            weight = float(raw_weight)
            if not math.isfinite(weight):
                raise ValueError("score weights must be finite")
            if weight < 0:
                raise ValueError("score weights must be non-negative")
            result[name] = result.get(name, 0.0) + weight
        if not result:
            raise ValueError("score weights must be non-empty")
        if not any(weight > 0 for weight in result.values()):
            raise ValueError("at least one score weight must be positive")
        return result

    def as_dict(self) -> dict[str, float]:
        return dict(self.weights)

    @model_serializer(mode="plain")
    def _serialize_weights(self) -> dict[str, float]:
        return self.as_dict()


class ScorerPreset(_EvalModel):
    name: ScorerPresetName
    weights: ScoreWeights | None = None


class ResponseScorer(_EvalModel):
    id: str
    method: ResponseScorerMethod
    weight: float = 1.0
    required: bool = False
    instructions: str | None = None
    reference_response: str | None = None
    rubric: dict[int, str] | None = None
    context: dict[str, Any] | None = None
    expected: str | None = None
    text: str | None = None
    pattern: str | None = None
    case_sensitive: bool = True

    @classmethod
    def judge(
        cls,
        *,
        id: str,
        instructions: str,
        reference_response: str | None = None,
        rubric: Mapping[int, str] | None = None,
        context: Mapping[str, Any] | None = None,
        required: bool = False,
        weight: float = 1.0,
    ) -> ResponseScorer:
        return cls(
            id=id,
            method="judge",
            instructions=instructions,
            reference_response=reference_response,
            rubric=dict(rubric) if rubric is not None else None,
            context=dict(context) if context is not None else None,
            required=required,
            weight=weight,
        )

    @classmethod
    def exact(
        cls,
        *,
        id: str,
        expected: str,
        required: bool = False,
        weight: float = 1.0,
    ) -> ResponseScorer:
        return cls(
            id=id,
            method="exact",
            expected=expected,
            required=required,
            weight=weight,
        )

    @classmethod
    def contains(
        cls,
        *,
        id: str,
        text: str,
        case_sensitive: bool = True,
        required: bool = False,
        weight: float = 1.0,
    ) -> ResponseScorer:
        return cls(
            id=id,
            method="contains",
            text=text,
            case_sensitive=case_sensitive,
            required=required,
            weight=weight,
        )

    @classmethod
    def regex(
        cls,
        *,
        id: str,
        pattern: str,
        required: bool = False,
        weight: float = 1.0,
    ) -> ResponseScorer:
        return cls(
            id=id,
            method="regex",
            pattern=pattern,
            required=required,
            weight=weight,
        )

    @model_validator(mode="after")
    def _validate_scorer(self) -> ResponseScorer:
        if not self.id.strip():
            raise ValueError("response scorer id must be non-empty")
        if self.weight <= 0 or not math.isfinite(self.weight):
            raise ValueError("response scorer weight must be positive and finite")

        if self.method == "judge":
            if not (self.instructions or "").strip():
                raise ValueError("judge response scorer instructions must be non-empty")
            if self.rubric is not None:
                if set(self.rubric) != {0, 1}:
                    raise ValueError("judge response scorer rubric must define exactly 0 and 1")
                if not all(value.strip() for value in self.rubric.values()):
                    raise ValueError("judge response scorer rubric descriptions must be non-empty")
        elif self.method == "exact":
            if self.expected is None:
                raise ValueError("exact response scorer expected text must be provided")
        elif self.method == "contains":
            if self.text is None or not self.text:
                raise ValueError("contains response scorer text must be non-empty")
        elif self.method == "regex":
            if self.pattern is None or not self.pattern:
                raise ValueError("regex response scorer pattern must be non-empty")
            try:
                re.compile(self.pattern)
            except re.error as error:
                raise ValueError(f"invalid regex response scorer pattern: {error}") from error
        return self


class FinalResponseEval(_EvalModel):
    scorers: list[ResponseScorer] = Field(min_length=1)
    pass_threshold: float = 1.0

    @model_validator(mode="after")
    def _validate_final_response_eval(self) -> FinalResponseEval:
        if not 0 <= self.pass_threshold <= 1:
            raise ValueError("final response pass_threshold must be between 0 and 1")
        ids = [scorer.id for scorer in self.scorers]
        if len(ids) != len(set(ids)):
            raise ValueError("response scorer ids must be unique")
        return self


class JudgeVerdict(_EvalModel):
    passed: bool
    selected_rubric_score: int
    reason: str

    @field_validator("reason")
    @classmethod
    def _validate_reason(cls, value: str) -> str:
        reason = value.strip()
        if not reason:
            raise ValueError("judge verdict reason must be non-empty")
        return reason

    @model_validator(mode="after")
    def _validate_binary_verdict(self) -> JudgeVerdict:
        if self.selected_rubric_score not in (0, 1):
            raise ValueError("judge verdict selected_rubric_score must be 0 or 1")
        if self.passed != (self.selected_rubric_score == 1):
            raise ValueError(
                "judge verdict passed must be true exactly when selected_rubric_score is 1"
            )
        return self

    def as_extra_value(self) -> dict[str, Any]:
        return {
            "passed": self.passed,
            "selected_rubric_score": self.selected_rubric_score,
            "reason": self.reason,
        }


class TrajectoryScorerConfig(_EvalModel):
    include_sub_agents: bool = False
    ignore_tools: list[str] = Field(default_factory=list)

    @field_validator("ignore_tools", mode="before")
    @classmethod
    def _normalize_ignore_tools(cls, value: Any) -> list[str]:
        tools = list(value or [])
        normalized = [str(tool).strip() for tool in tools]
        if any(not tool for tool in normalized):
            raise ValueError("trajectory ignore_tools entries must be non-empty")
        return list(dict.fromkeys(normalized))


class FinalResponseScorerConfig(_EvalModel):
    include_judge_trace: bool = False


class ExpectedAction(_EvalModel):
    type: str
    payload: dict[str, Any] = Field(default_factory=dict)


class FlatActionGroundTruthPayload(_EvalModel):
    kind: Literal["flat"] = "flat"
    planned_actions: list[ExpectedAction] = Field(default_factory=list)
    executed_actions: list[ExpectedAction] = Field(default_factory=list)
    payload_match: ActionPayloadMatchMode = "exact"

    @model_validator(mode="after")
    def _require_at_least_one_bucket(self) -> FlatActionGroundTruthPayload:
        if not self.planned_actions and not self.executed_actions:
            raise ValueError(
                "expected actions must declare at least one of "
                "planned_actions or executed_actions"
            )
        return self


ActionGroundTruthPayload = Annotated[
    FlatActionGroundTruthPayload,
    Field(discriminator="kind"),
]
_ACTION_GROUND_TRUTH_PAYLOAD_ADAPTER = TypeAdapter(ActionGroundTruthPayload)


class GroundTruth(_EvalModel):
    kind: Literal["structured"] = "structured"
    payload: ActionGroundTruthPayload | dict[str, Any]
    schema_: str | None = Field(default=None, alias="schema", serialization_alias="schema")

    @model_validator(mode="before")
    @classmethod
    def _normalize_wire_shape(cls, value: Any) -> Any:
        if not isinstance(value, Mapping):
            return value
        data = dict(value)
        kind = data.get("kind")
        if kind == "structured":
            if "payload" not in data and "data" in data:
                data["payload"] = data.pop("data")
            return data
        return data

    @model_validator(mode="after")
    def _validate_action_payloads(self) -> GroundTruth:
        if isinstance(self.payload, Mapping) and self.payload.get("kind") == "flat":
            object.__setattr__(
                self,
                "payload",
                _ACTION_GROUND_TRUTH_PAYLOAD_ADAPTER.validate_python(self.payload),
            )
        return self

    @model_serializer(mode="plain")
    def _serialize_ground_truth(self) -> dict[str, Any]:
        return {
            "kind": "structured",
            "data": _dump_model(self.payload),
        }


class ResolvedAction(_EvalModel):
    type: str
    payload: dict[str, Any] = Field(default_factory=dict)


class RawSampleOutput(_EvalModel):
    actual_trajectory: list[str] = Field(default_factory=list)
    response_text: str | None = None
    extra: dict[str, Any] = Field(default_factory=dict)

    @field_validator("extra", mode="before")
    @classmethod
    def _normalize_extra(cls, value: Any) -> dict[str, Any]:
        data = dict(value or {})
        if "resolvedActions" in data:
            data["resolvedActions"] = [
                _dump_model(ResolvedAction.model_validate(action))
                for action in data["resolvedActions"]
            ]
        return data

    def with_judge_verdicts(
        self,
        verdicts: Mapping[str, JudgeVerdict | Mapping[str, Any]],
    ) -> RawSampleOutput:
        extra = dict(self.extra)
        extra.update(final_response_judge_verdicts_extra(verdicts))
        return RawSampleOutput(
            actual_trajectory=list(self.actual_trajectory),
            response_text=self.response_text,
            extra=extra,
        )


class EvalConfig(_EvalModel):
    """Eval run configuration. Build with ``define_eval_config(...)``."""

    mode: EvalMode = Field(
        default="sequential",
        description="Eval mode: planner, executor, sequential, specialist, or testCaseBuilder.",
    )
    target_agent_id: str | None = Field(
        default=None,
        description="Agent evaluated directly in specialist mode, bypassing coordinator routing.",
    )
    test_case_set_id: str = Field(
        default="",
        description="Identifier of a persisted test case set; empty for inline test cases.",
    )
    test_case_ids: list[str] | None = Field(
        default=None,
        description="Optional subset of test case ids to run.",
    )
    samples_per_case: int = Field(
        default=3,
        description="Number of samples generated per test case.",
    )
    pass_threshold: float = Field(
        default=0.7,
        description="Minimum aggregate score in [0, 1] for a sample to pass.",
    )
    concurrency: int = Field(
        default=2,
        description="Maximum number of samples executed concurrently.",
    )
    k_values: list[int] = Field(
        default_factory=lambda: [1, 3],
        description="k values reported for pass@k.",
    )
    provider: str | None = Field(
        default=None,
        description="Provider override for the eval run; also used by judge scorers when set.",
    )
    model: str | None = Field(
        default=None,
        description="Model override for the eval run, paired with provider.",
    )
    timeout_per_sample_secs: int | None = Field(
        default=120,
        description="Per-sample timeout in seconds.",
    )
    tags_filter: list[str] | None = Field(
        default=None,
        description="Tag filter applied to test case selection.",
    )
    aggregation_strategy: AggregationStrategy = Field(
        default="passRate",
        description="Summary aggregation strategy: passRate or meanScore.",
    )
    score_weights: ScoreWeights | None = Field(
        default=None,
        description="Per-scorer weights; normalized by the total positive weight.",
    )
    scorer_config: dict[str, Any] | None = Field(
        default=None,
        description="Per-scorer configuration, e.g. trajectory projection and judge trace options.",
    )
    request_overrides: dict[str, Any] | None = Field(
        default=None,
        description="Raw request override payload forwarded to the native eval runner.",
    )

    @field_validator("score_weights", mode="before")
    @classmethod
    def _normalize_score_weights(cls, value: Any) -> Any:
        if value is None or isinstance(value, ScoreWeights):
            return value
        return ScoreWeights.model_validate(value)


class EvalTestCase(_EvalModel):
    """One eval test case. Build with ``define_test_case(...)``."""

    id: str = Field(description="Unique test case identifier.")
    tags: list[str] = Field(
        default_factory=list,
        description="Free-form tags used by EvalConfig.tags_filter.",
    )
    input: str = Field(
        description="User prompt sent to the agent under test.",
    )
    expected_trajectory: list[str] = Field(
        default_factory=list,
        description="Expected tool names compared per trajectory_mode.",
    )
    trajectory_mode: TrajectoryMode = Field(
        default="unordered",
        description="Trajectory comparison mode: strict, unordered, subset, superset, or subsequence.",
    )
    ground_truth: GroundTruth | None = Field(
        default=None,
        alias="structuredGroundTruth",
        validation_alias=AliasChoices(
            "structuredGroundTruth",
            "groundTruth",
            "expectedActions",
            "expected_actions",
        ),
        description="Structured action ground truth from define_expected_actions(...).",
    )
    final_response: FinalResponseEval | None = Field(
        default=None,
        description="How to score the final user-facing response text.",
    )
    source_thread_id: str | None = Field(
        default=None,
        description="Provenance of an authored test case; not reused as the eval execution thread.",
    )

    @model_validator(mode="before")
    @classmethod
    def _reject_ambiguous_action_expectations(cls, value: Any) -> Any:
        if not isinstance(value, Mapping):
            return value
        has_expected_actions = any(
            key in value for key in ("expectedActions", "expected_actions")
        )
        has_ground_truth = any(
            key in value
            for key in ("structuredGroundTruth", "groundTruth", "ground_truth")
        )
        if has_expected_actions and has_ground_truth:
            raise ValueError("provide either expected_actions or ground_truth, not both")
        return value


class EvalRequest(_EvalModel):
    tenant_id: str
    workspace_id: str
    config: EvalConfig
    test_cases: list[EvalTestCase]
    scorer_preset: str | None = None
    score_weights: ScoreWeights | None = None

    @field_validator("score_weights", mode="before")
    @classmethod
    def _normalize_score_weights(cls, value: Any) -> Any:
        if value is None or isinstance(value, ScoreWeights):
            return value
        return ScoreWeights.model_validate(value)

    @model_validator(mode="after")
    def _validate_preset_case_compatibility(self) -> EvalRequest:
        if self.scorer_preset == "trajectory_only" and any(
            test_case.final_response is not None for test_case in self.test_cases
        ):
            raise ValueError(
                "scorer_preset='trajectory_only' cannot be used with final_response evals"
            )
        return self


class PassAtKResult(_EvalModel):
    k: int
    simple_estimate: float
    unbiased_estimate: float | None = None
    num_samples: int
    num_correct: int


class ScorerResult(_EvalModel):
    scorer_name: str
    score: float
    details: dict[str, Any] | None = None


class ScoredSample(_EvalModel):
    aggregate: float
    component_scores: list[ScorerResult] = Field(min_length=1)


class ModelInvocation(_EvalModel):
    agent: str
    provider: str | None = None
    model: str
    input_tokens: int
    output_tokens: int
    cached_tokens: int
    cache_creation_tokens: int = 0
    estimated_cost_usd: float | None = None


class SampleCost(_EvalModel):
    llm_cost_usd: float | None = None
    non_llm_cost_usd: float | None = None
    total_cost_usd: float | None = None


class SampleLatency(_EvalModel):
    total_ms: int
    first_token_ms: int | None = None
    model_ms: int | None = None
    tool_ms: int | None = None


class SummaryLatency(_EvalModel):
    p50_ms: int | None = None
    p95_ms: int | None = None
    p99_ms: int | None = None
    min_ms: int | None = None
    max_ms: int | None = None


class EvalTraceRef(_EvalModel):
    trace_id: str
    thread_id: str | None = None
    url: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


class SampleArtifact(_EvalModel):
    sample_index: int
    passed: bool
    aggregate_score: float
    component_scores: list[ScorerResult]
    response_text: str | None = Field(default=None, exclude_if=lambda value: value is None)
    actual_trajectory: list[str]
    final_response_eval: dict[str, Any] | None = None
    planned_actions: list[ResolvedAction] = Field(default_factory=list)
    resolved_actions: list[ResolvedAction] = Field(default_factory=list)
    duration_ms: int
    model_invocations: list[ModelInvocation] = Field(default_factory=list)
    token_usage: TokenUsageSummary
    cost: SampleCost | None = None
    latency: SampleLatency | None = None
    thread_id: str | None = None
    trace: EvalTraceRef | None = None
    metadata: dict[str, Any] | None = None
    error: str | None = None


class TestCaseArtifact(_EvalModel):
    test_case_id: str
    input: str | None = None
    samples: list[SampleArtifact]
    pass_at_k: list[PassAtKResult] = Field(default_factory=list)
    aggregate_score: float


class CostAgentBreakdown(_EvalModel):
    agent: str
    provider: str | None = None
    model: str
    usage: TokenUsageSummary
    estimated_cost_usd: float | None = None


class SummaryCost(_EvalModel):
    estimated_cost_usd: float
    per_agent: list[CostAgentBreakdown] = Field(default_factory=list)


class EvalArtifactSummary(_EvalModel):
    total_test_cases: int
    passed: int
    failed: int
    skipped: int = 0
    aggregate_score: float
    pass_rate: float
    pass_at_k: list[PassAtKResult] = Field(default_factory=list)
    total_duration_ms: int
    total_usage: TokenUsageSummary
    cost: SummaryCost | None = None
    latency: SummaryLatency | None = None
    metadata: dict[str, Any] | None = None


class ArtifactMetadata(_EvalModel):
    schema_version: int = 1
    scorer_preset: str
    score_weights: dict[str, float]


class EvalArtifact(_EvalModel):
    run_id: str
    tenant_id: str
    workspace_id: str
    mode: EvalMode
    summary: EvalArtifactSummary
    test_cases: list[TestCaseArtifact]
    metadata: ArtifactMetadata


class EvalStartedData(_EvalModel):
    artifact: EvalArtifact


class EvalStarted(_EvalModel):
    type: Literal["evalStarted"] = "evalStarted"
    data: EvalStartedData


class TestCaseStartedData(_EvalModel):
    test_case_id: str


class TestCaseStarted(_EvalModel):
    type: Literal["testCaseStarted"] = "testCaseStarted"
    data: TestCaseStartedData


class SampleCompletedData(_EvalModel):
    sample: SampleArtifact


class SampleCompleted(_EvalModel):
    type: Literal["sampleCompleted"] = "sampleCompleted"
    data: SampleCompletedData


class TestCaseCompletedData(_EvalModel):
    test_case: TestCaseArtifact


class TestCaseCompleted(_EvalModel):
    type: Literal["testCaseCompleted"] = "testCaseCompleted"
    data: TestCaseCompletedData


class EvalCompletedData(_EvalModel):
    artifact: EvalArtifact


class EvalCompleted(_EvalModel):
    type: Literal["evalCompleted"] = "evalCompleted"
    data: EvalCompletedData


class EvalFailedData(_EvalModel):
    error: str


class EvalFailed(_EvalModel):
    type: Literal["evalFailed"] = "evalFailed"
    data: EvalFailedData


class EvalCancelledData(_EvalModel):
    reason: str


class EvalCancelled(_EvalModel):
    type: Literal["evalCancelled"] = "evalCancelled"
    data: EvalCancelledData


HarnessEvalEvent = Annotated[
    EvalStarted
    | TestCaseStarted
    | SampleCompleted
    | TestCaseCompleted
    | EvalCompleted
    | EvalFailed
    | EvalCancelled,
    Field(discriminator="type"),
]


class HarnessEvalEventEnvelope(_EvalModel):
    run_id: str
    sequence: int
    event: HarnessEvalEvent

    @model_validator(mode="before")
    @classmethod
    def _unflatten_event(cls, value: Any) -> Any:
        if not isinstance(value, Mapping):
            return value
        data = dict(value)
        event_type = data.pop("type", None)
        event_data = data.pop("data", None)
        if event_type is not None:
            data["event"] = {"type": event_type, "data": event_data or {}}
        return data

    @model_serializer(mode="plain")
    def _serialize_envelope(self) -> dict[str, Any]:
        event = self.event
        return {
            "runId": self.run_id,
            "sequence": self.sequence,
            "type": event.type,
            "data": _dump_model(event.data),
        }


def define_eval_config(
    *,
    mode: EvalMode = "sequential",
    target_agent_id: str | None = None,
    test_case_set_id: str = "",
    test_case_ids: list[str] | None = None,
    samples_per_case: int = 3,
    pass_threshold: float = 0.7,
    concurrency: int = 2,
    k_values: list[int] | None = None,
    provider: str | None = None,
    model: str | None = None,
    timeout_per_sample_secs: int | None = 120,
    tags_filter: list[str] | None = None,
    aggregation_strategy: AggregationStrategy = "passRate",
    score_weights: ScoreWeights | Mapping[str, float] | None = None,
    scorer_config: Mapping[str, Any] | None = None,
) -> EvalConfig:
    """Create a validated eval run configuration.

    Args:
        mode: Eval mode: ``"planner"``, ``"executor"``, ``"sequential"``,
            ``"specialist"``, or ``"testCaseBuilder"``. Defaults to
            ``"sequential"``.
        target_agent_id: Agent evaluated directly in ``"specialist"`` mode,
            bypassing coordinator routing.
        test_case_set_id: Identifier of a persisted test case set. Leave
            empty when test cases are supplied inline on the request.
        test_case_ids: Optional subset of test case ids to run.
        samples_per_case: Number of samples generated per test case.
        pass_threshold: Minimum aggregate score for a sample to pass, in
            ``[0, 1]``. Applies to the overall sample aggregate; the
            final-response ``pass_threshold`` applies separately inside
            ``FinalResponseEval``.
        concurrency: Maximum number of samples executed concurrently.
        k_values: ``k`` values reported for pass@k. Defaults to ``[1, 3]``.
        provider: Provider override for the eval run. Judge scorers use this
            when set; otherwise they fall back to the coordinator model,
            then the first registered agent model.
        model: Model override for the eval run, paired with ``provider``.
        timeout_per_sample_secs: Per-sample timeout in seconds.
        tags_filter: Tag filter applied to test case selection.
        aggregation_strategy: Summary aggregation: ``"passRate"`` or
            ``"meanScore"``.
        score_weights: Per-scorer weights keyed by ``trajectory``,
            ``planned_actions``, ``executed_actions``, or
            ``final_response``. Weights are normalized by the total positive
            weight, so they do not need to sum to 1.0.
        scorer_config: Per-scorer configuration mapping; merge the outputs
            of ``define_trajectory_scorer_config(...)`` and
            ``define_final_response_scorer_config(...)``.

    Returns:
        A frozen, validated ``EvalConfig``.

    Raises:
        pydantic.ValidationError: If a ``score_weights`` key is not a known
            scorer name or a weight is negative or non-finite.
    """
    return EvalConfig(
        mode=mode,
        target_agent_id=target_agent_id,
        test_case_set_id=test_case_set_id,
        test_case_ids=test_case_ids,
        samples_per_case=samples_per_case,
        pass_threshold=pass_threshold,
        concurrency=concurrency,
        k_values=k_values or [1, 3],
        provider=provider,
        model=model,
        timeout_per_sample_secs=timeout_per_sample_secs,
        tags_filter=tags_filter,
        aggregation_strategy=aggregation_strategy,
        score_weights=score_weights,
        scorer_config=dict(scorer_config) if scorer_config is not None else None,
    )


def define_trajectory_scorer_config(
    *,
    include_sub_agents: bool = False,
    ignore_tools: list[str] | None = None,
) -> dict[str, Any]:
    """Create the trajectory scorer entry for ``EvalConfig.scorer_config``.

    Args:
        include_sub_agents: Include tool calls emitted inside sub-agent runs
            in the scored trajectory projection.
        ignore_tools: Tool names removed from the scored projection only;
            entries must be non-empty and duplicates are dropped. The raw
            ``actual_trajectory`` recorded in eval artifacts is not mutated.

    Returns:
        A ``{"trajectory": {...}}`` mapping suitable for merging into
        ``scorer_config``.

    Raises:
        pydantic.ValidationError: If an ``ignore_tools`` entry is empty.
    """
    config = TrajectoryScorerConfig(
        include_sub_agents=include_sub_agents,
        ignore_tools=ignore_tools or [],
    )
    return {
        "trajectory": config.model_dump(by_alias=True, mode="json"),
    }


def define_final_response_scorer_config(
    *,
    include_judge_trace: bool = False,
) -> dict[str, Any]:
    """Create the final-response scorer entry for ``EvalConfig.scorer_config``.

    Args:
        include_judge_trace: When true, judge scorer details include
            ``judgeTrace.prompt`` and ``judgeTrace.response``. Leave off for
            normal runs: the trace can contain final responses, reference
            answers, rubric text, and other test data.

    Returns:
        A ``{"finalResponse": {...}}`` mapping suitable for merging into
        ``scorer_config``.
    """
    config = FinalResponseScorerConfig(include_judge_trace=include_judge_trace)
    return {
        "finalResponse": config.model_dump(by_alias=True, mode="json"),
    }


def define_test_case(
    id: str,
    input: str,
    *,
    tags: list[str] | None = None,
    expected_trajectory: list[str] | None = None,
    trajectory_mode: TrajectoryMode = "unordered",
    expected_actions: GroundTruth | Mapping[str, Any] | None = None,
    ground_truth: GroundTruth | Mapping[str, Any] | None = None,
    final_response: FinalResponseEval | Mapping[str, Any] | None = None,
    source_thread_id: str | None = None,
) -> EvalTestCase:
    """Create a validated eval test case.

    Args:
        id: Unique test case identifier.
        input: User prompt the eval run sends to the agent under test.
        tags: Free-form tags used by ``EvalConfig.tags_filter``.
        expected_trajectory: Expected tool names; compared to the observed
            trajectory using ``trajectory_mode``.
        trajectory_mode: Trajectory comparison mode: ``"strict"`` (same
            sequence), ``"unordered"`` (same multiset), ``"subset"``,
            ``"superset"``, or ``"subsequence"``. Defaults to
            ``"unordered"``.
        expected_actions: Action ground truth from
            ``define_expected_actions(...)``; scores planned and/or executed
            business actions.
        ground_truth: Structured ground-truth envelope; alternative spelling
            of ``expected_actions``. Mutually exclusive with it.
        final_response: ``FinalResponseEval`` describing how to score the
            final user-facing text.
        source_thread_id: Provenance of an authored test case. It is not
            reused as the eval execution thread.

    Returns:
        A frozen, validated ``EvalTestCase``.

    Raises:
        ValueError: If both ``expected_actions`` and ``ground_truth`` are
            provided.
    """
    if expected_actions is not None and ground_truth is not None:
        raise ValueError("provide either expected_actions or ground_truth, not both")
    return EvalTestCase(
        id=id,
        input=input,
        tags=tags or [],
        expected_trajectory=expected_trajectory or [],
        trajectory_mode=trajectory_mode,
        ground_truth=expected_actions if expected_actions is not None else ground_truth,
        final_response=final_response,
        source_thread_id=source_thread_id,
    )


def define_eval_request(
    runtime: Any,
    *,
    workspace_id: str,
    test_cases: list[EvalTestCase | Mapping[str, Any]],
    config: EvalConfig | Mapping[str, Any] | None = None,
    scorer_preset: str | None = None,
    score_weights: ScoreWeights | Mapping[str, float] | None = None,
    tenant_id: str | None = None,
) -> EvalRequest:
    """Create a validated eval request bound to a runtime tenant.

    Args:
        runtime: Native ``Runtime`` handle. Its ``resource_id`` supplies the
            tenant id when ``tenant_id`` is not given.
        workspace_id: Workspace the eval run is recorded under.
        test_cases: ``EvalTestCase`` values or mappings validated as such.
        config: ``EvalConfig`` or mapping. Defaults to
            ``define_eval_config()``.
        scorer_preset: Scorer preset name, e.g. ``"trajectory_only"``,
            ``"planner"``, ``"executor"``, ``"sequential"``,
            ``"specialist"``, or ``"test_case_builder"``.
        score_weights: Request-level scorer weights; normalized by the total
            positive weight.
        tenant_id: Explicit tenant id override when the runtime handle does
            not expose ``resource_id``.

    Returns:
        A frozen, validated ``EvalRequest``.

    Raises:
        ValueError: If ``tenant_id`` is omitted and the runtime has no
            usable ``resource_id``.
        pydantic.ValidationError: If ``scorer_preset="trajectory_only"`` is
            combined with test cases that author ``final_response`` evals.
    """
    if tenant_id is None:
        runtime_resource_id = getattr(runtime, "resource_id", None)
        if not isinstance(runtime_resource_id, str) or not runtime_resource_id.strip():
            raise ValueError(
                "define_eval_request requires a runtime with resource_id; pass tenant_id explicitly"
            )
        tenant_id = runtime_resource_id

    return EvalRequest(
        tenant_id=tenant_id,
        workspace_id=workspace_id,
        config=config if config is not None else define_eval_config(),
        test_cases=list(test_cases),
        scorer_preset=scorer_preset,
        score_weights=score_weights,
    )


def define_ground_truth(
    payload: Mapping[str, Any],
    *,
    schema: str | None = None,
    kind: Literal["structured"] = "structured",
) -> GroundTruth:
    """Wrap a payload mapping in the structured ground-truth envelope.

    Use ``define_expected_actions(...)`` for action-based evals; this helper
    keeps the generic envelope available for raw structured payloads.

    Args:
        payload: Ground-truth payload mapping. A payload with
            ``kind="flat"`` is validated as action ground truth.
        schema: Optional schema identifier carried alongside the payload.
        kind: Envelope kind. Only ``"structured"`` is supported.

    Returns:
        A frozen ``GroundTruth`` envelope.
    """
    return GroundTruth(kind=kind, payload=dict(payload), schema=schema)


def define_final_response_eval(
    *,
    scorers: list[ResponseScorer | Mapping[str, Any]],
    pass_threshold: float = 1.0,
) -> FinalResponseEval:
    """Declare how to evaluate the coordinator's final text response.

    The final-response score is the weighted average of the scorer results,
    normalized by the total positive weight. ``required=True`` scorers act
    as gates, not weight multipliers: a failed required scorer fails the
    final-response eval even when the raw weighted score is non-zero. Judge
    scorers fail closed: judge execution errors are recorded as
    ``passed=false`` with ``score=0.0`` and an explicit ``errorKind``, and
    they stay in the weighted aggregate.

    Args:
        scorers: ``ResponseScorer`` values (or mappings); at least one, with
            unique ids. Build them with ``ResponseScorer.judge(...)``,
            ``.exact(...)``, ``.contains(...)``, or ``.regex(...)``.
        pass_threshold: Minimum weighted score in ``[0, 1]`` for the
            final-response scorer to pass. Applies inside this eval; the
            request-level ``EvalConfig.pass_threshold`` applies later to the
            overall sample aggregate.

    Returns:
        A frozen, validated ``FinalResponseEval``.

    Raises:
        pydantic.ValidationError: If ``scorers`` is empty, scorer ids are
            not unique, or ``pass_threshold`` is outside ``[0, 1]``.
    """
    return FinalResponseEval(
        scorers=list(scorers),
        pass_threshold=pass_threshold,
    )


def define_expected_actions(
    *,
    planned_actions: list[ExpectedAction | Mapping[str, Any]] | None = None,
    executed_actions: list[ExpectedAction | Mapping[str, Any]] | None = None,
    payload_match: ActionPayloadMatchMode = "exact",
) -> GroundTruth:
    """Declare expected planned and/or executed business actions to score.

    Provide ``planned_actions`` to score against the stored plan,
    ``executed_actions`` to score against the execution result, or both.
    Presence of a bucket signals intent to score that source. Action
    matching is strict on ``type``; expected payloads are compared per
    ``payload_match``. Extra action items are penalized in both modes, and
    action list comparison is order-insensitive.

    Args:
        planned_actions: Expected actions scored against the stored plan.
        executed_actions: Expected actions scored against the execution
            result.
        payload_match: ``"exact"`` (default) requires expected payloads to
            equal actual payloads exactly; ``"subset"`` lets an expected
            payload match as a deep subset of the actual payload, so
            runtime-generated fields can be omitted.

    Returns:
        A frozen ``GroundTruth`` envelope for ``define_test_case(...)``.

    Raises:
        pydantic.ValidationError: If both buckets are empty.
    """
    payload = FlatActionGroundTruthPayload(
        kind="flat",
        planned_actions=planned_actions or [],
        executed_actions=executed_actions or [],
        payload_match=payload_match,
    )
    return GroundTruth(kind="structured", payload=payload)


def define_action_ground_truth(
    *,
    planned_actions: list[ExpectedAction | Mapping[str, Any]] | None = None,
    executed_actions: list[ExpectedAction | Mapping[str, Any]] | None = None,
    payload_match: ActionPayloadMatchMode = "exact",
    kind: Literal["flat"] = "flat",
) -> GroundTruth:
    """Declare expected business actions to score.

    ``define_expected_actions(...)`` is the preferred Python authoring helper.
    This helper keeps the generic ground-truth terminology available for callers
    that want to work close to the structured wire shape.

    Args:
        planned_actions: Expected actions scored against the stored plan.
        executed_actions: Expected actions scored against the execution
            result.
        payload_match: ``"exact"`` (default) requires expected payloads to
            equal actual payloads exactly; ``"subset"`` matches expected
            payloads as a deep subset of actual payloads.
        kind: Payload kind. Only ``"flat"`` is supported.

    Returns:
        A frozen ``GroundTruth`` envelope.

    Raises:
        pydantic.ValidationError: If both buckets are empty.
    """
    payload = FlatActionGroundTruthPayload(
        kind=kind,
        planned_actions=planned_actions or [],
        executed_actions=executed_actions or [],
        payload_match=payload_match,
    )
    return GroundTruth(kind="structured", payload=payload)


def define_expected_action(
    type: str,
    payload: Mapping[str, Any] | None = None,
) -> ExpectedAction:
    """Create one expected business action for action ground truth.

    Args:
        type: Action type. Matched strictly against the actual action type.
        payload: Expected action payload. Compared per the enclosing
            ``payload_match`` mode (exact by default).

    Returns:
        A frozen ``ExpectedAction``.
    """
    return ExpectedAction(
        type=type,
        payload=dict(payload or {}),
    )


def define_resolved_action(
    type: str,
    payload: Mapping[str, Any] | None = None,
) -> ResolvedAction:
    """Create one resolved (observed) action in the wire shape.

    Use the result's ``model_dump(by_alias=True, mode="json")`` output in
    ``RawSampleOutput.extra`` under the camelCase keys ``plannedActions`` or
    ``resolvedActions``.

    Args:
        type: Action type as produced by the runtime.
        payload: Action payload as produced by the runtime.

    Returns:
        A frozen ``ResolvedAction``.
    """
    return ResolvedAction(
        type=type,
        payload=dict(payload or {}),
    )


def final_response_judge_verdicts_extra(
    verdicts: Mapping[str, JudgeVerdict | Mapping[str, Any]],
) -> dict[str, Any]:
    """Build the ``finalResponseJudgeVerdicts`` extra entry for offline scoring.

    ``score_sample(...)`` never calls a judge model; precomputed judge
    verdicts are supplied through this ``RawSampleOutput.extra`` key.
    Prefer ``RawSampleOutput.with_judge_verdicts(...)``.

    Args:
        verdicts: Mapping from response scorer id to a ``JudgeVerdict`` or
            verdict mapping with ``passed``, ``selected_rubric_score``, and
            ``reason``.

    Returns:
        A one-key mapping ``{"finalResponseJudgeVerdicts": {...}}``.

    Raises:
        TypeError: If ``verdicts`` is not a mapping.
        ValueError: If a scorer id is empty.
        pydantic.ValidationError: If a verdict fails validation.
    """
    if not isinstance(verdicts, Mapping):
        raise TypeError("judge verdicts must be a mapping keyed by response scorer id")

    normalized: dict[str, Any] = {}
    for raw_id, raw_verdict in verdicts.items():
        scorer_id = str(raw_id).strip()
        if not scorer_id:
            raise ValueError("judge verdict response scorer ids must be non-empty")
        normalized[scorer_id] = JudgeVerdict.model_validate(raw_verdict).as_extra_value()
    return {
        FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY: normalized,
    }


def define_scorer_preset(
    name: ScorerPresetName,
    *,
    weights: ScoreWeights | Mapping[str, float] | None = None,
) -> ScorerPreset:
    """Create a named scorer preset with optional weight overrides.

    Args:
        name: Preset name: ``"trajectory_only"``, ``"planner"``,
            ``"executor"``, ``"sequential"``, ``"specialist"``, or
            ``"test_case_builder"``.
        weights: Optional scorer weights overriding the preset defaults,
            keyed by ``trajectory``, ``planned_actions``,
            ``executed_actions``, or ``final_response``. Normalized by the
            total positive weight.

    Returns:
        A frozen ``ScorerPreset``.

    Raises:
        pydantic.ValidationError: If the name is not a known preset or a
            weight key/value is invalid.
    """
    return ScorerPreset(name=name, weights=weights)


def score_sample(
    test_case: EvalTestCase | Mapping[str, Any],
    output: RawSampleOutput | Mapping[str, Any],
    *,
    mode: EvalMode | None = None,
    score_weights: ScoreWeights | Mapping[str, float] | None = None,
    scorer_preset: ScorerPreset | ScorerPresetName | Mapping[str, Any] | None = None,
    scorer_config: Mapping[str, Any] | None = None,
) -> ScoredSample:
    """Score one known sample deterministically through the Rust scorer.

    This is offline scoring: it does not run a runtime and never calls a
    judge model. Judge scorers require precomputed verdicts supplied via
    ``RawSampleOutput.with_judge_verdicts(...)``.

    Args:
        test_case: ``EvalTestCase`` or mapping validated as one.
        output: ``RawSampleOutput`` or mapping with ``actual_trajectory``,
            optional ``response_text``, and ``extra``. Scorer payloads in
            ``extra`` must use the camelCase keys ``plannedActions``,
            ``resolvedActions``, ``trajectoryEvents``, and
            ``finalResponseJudgeVerdicts``; unknown keys are ignored by
            scorers, so misspellings can look like genuine score failures.
        mode: Optional eval mode that selects the default scorer
            composition.
        score_weights: Scorer weights normalized by the total positive
            weight. Mutually exclusive with weights carried by
            ``scorer_preset``.
        scorer_preset: Preset name, ``ScorerPreset``, or mapping selecting
            the scorer composition.
        scorer_config: Per-scorer configuration; see
            ``define_trajectory_scorer_config(...)`` and
            ``define_final_response_scorer_config(...)``.

    Returns:
        A ``ScoredSample`` with ``aggregate`` and ``component_scores``. It
        has no ``passed`` field; offline callers apply their own threshold.

    Raises:
        ValueError: If weights are supplied both directly and through the
            preset, or the native scorer rejects the inputs.
        pydantic.ValidationError: If the test case or output fail
            validation.
    """
    test_case_model = EvalTestCase.model_validate(test_case)
    output_model = RawSampleOutput.model_validate(output)

    preset_name: str | None
    preset_weights: ScoreWeights | None = None
    if scorer_preset is None:
        preset_name = None
    elif isinstance(scorer_preset, str):
        preset_name = scorer_preset
    else:
        preset = ScorerPreset.model_validate(scorer_preset)
        preset_name = preset.name
        preset_weights = preset.weights

    if score_weights is not None and preset_weights is not None:
        raise ValueError(
            "score_sample accepts weights either in scorer_preset or score_weights, not both"
        )

    weights = score_weights if score_weights is not None else preset_weights
    if weights is not None and not isinstance(weights, ScoreWeights):
        weights = ScoreWeights.model_validate(weights)

    options = {
        "mode": mode,
        "scorerPreset": preset_name,
        "scoreWeights": weights.as_dict() if isinstance(weights, ScoreWeights) else None,
        "scorerConfig": dict(scorer_config) if scorer_config is not None else None,
    }
    raw = call_native(
        _internal.score_eval_sample,
        json.dumps(test_case_model.model_dump(by_alias=True, mode="json")),
        json.dumps(output_model.model_dump(by_alias=True, mode="json")),
        json.dumps({key: value for key, value in options.items() if value is not None}),
    )
    return ScoredSample.model_validate(raw)


def run_eval_sync(runtime: Any, request: EvalRequest | Mapping[str, Any]) -> EvalArtifact:
    """Run a runtime-backed eval to completion and block until it finishes.

    Wraps ``runtime.run_eval(...)`` in ``asyncio.run(...)`` and validates the
    returned artifact. Use ``runtime.stream_eval(...)`` for incremental
    event envelopes instead.

    Args:
        runtime: Native ``Runtime`` handle.
        request: ``EvalRequest`` or mapping validated as one.

    Returns:
        The validated ``EvalArtifact`` for the completed run.

    Raises:
        RuntimeError: If called from a thread with a running asyncio event
            loop, or the native eval run fails.
        pydantic.ValidationError: If the request or returned artifact fail
            validation.
    """
    request_model = EvalRequest.model_validate(request)

    async def _run_eval() -> Any:
        return await runtime.run_eval(request_model)

    return EvalArtifact.model_validate(asyncio.run(_run_eval()))


def _dump_model(value: Any) -> Any:
    if isinstance(value, BaseModel):
        return value.model_dump(by_alias=True, mode="json")
    return value


def _rename(data: dict[str, Any], old: str, new: str) -> None:
    if old in data and new not in data:
        data[new] = data.pop(old)
