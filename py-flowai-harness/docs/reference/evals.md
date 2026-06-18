# Evals

Python eval authoring types, helpers, result models, and the synchronous
runner. For task-oriented walkthroughs, start with the
[Evals guide](../guides/evals.md) and
[Final-response judge evals](../guides/judge-evals.md); this page is the API
contract.

Contract notes:

- **Scorer presets:** Use a preset to choose the default scoring shape for the
  eval run. Supported presets are `trajectory_only`, `planner`, `executor`,
  `sequential`, `specialist`, and `test_case_builder`. If a test case includes
  `final_response=define_final_response_eval(...)`, final-response scoring is
  added in addition to the preset's trajectory/action scoring.
- **Action matching:** Expected actions match actual actions only when their
  `type` values are the same. This check is strict even when payload matching
  uses subset mode.
- **Payload matching:** By default, an expected action payload must exactly
  match the actual action payload. Use `payload_match="subset"` with
  `define_expected_actions(...)` when the expected payload should only assert
  the fields you care about and ignore additional runtime-generated fields.
  Both modes compare semantic JSON: object key order is ignored and `1` equals
  `1.0`. Exact mode keeps array order significant; subset mode matches scalar
  arrays by value regardless of order.
- `runtime.run_eval(...)` returns the raw artifact dict. Validate it with
  `EvalArtifact.model_validate(...)`, or call `run_eval_sync(...)`, which
  returns a validated `EvalArtifact`.

## Common Helpers

::: flowai_harness.evals.define_eval_config

::: flowai_harness.evals.define_eval_request

::: flowai_harness.evals.define_test_case

::: flowai_harness.evals.define_expected_actions

::: flowai_harness.evals.define_ground_truth

::: flowai_harness.evals.define_action_ground_truth

::: flowai_harness.evals.define_expected_action

::: flowai_harness.evals.define_resolved_action

::: flowai_harness.evals.define_final_response_eval

::: flowai_harness.evals.define_scorer_preset

::: flowai_harness.evals.define_trajectory_scorer_config

::: flowai_harness.evals.define_final_response_scorer_config

::: flowai_harness.evals.score_sample

::: flowai_harness.evals.run_eval_sync

::: flowai_harness.evals.final_response_judge_verdicts_extra

## Config And Test Cases

::: flowai_harness.evals.EvalConfig

::: flowai_harness.evals.EvalRequest

::: flowai_harness.evals.EvalTestCase

::: flowai_harness.evals.GroundTruth

::: flowai_harness.evals.ExpectedAction

::: flowai_harness.evals.ResolvedAction

::: flowai_harness.evals.RawSampleOutput

::: flowai_harness.evals.ScoreWeights

::: flowai_harness.evals.ScorerPreset

::: flowai_harness.evals.FinalResponseEval

::: flowai_harness.evals.FinalResponseScorerConfig

### Final-response scorer constructors

Use the `ResponseScorer` classmethods for the supported final-response scorer
shapes:

```python
ResponseScorer.exact(id="answer", expected="42")
ResponseScorer.contains(id="mentions_metric", text="revenue", case_sensitive=False)
ResponseScorer.regex(id="currency", pattern=r"\$[0-9,]+")
ResponseScorer.judge(id="rubric", instructions="Pass when the answer cites the policy.")
```

::: flowai_harness.evals.ResponseScorer
    options:
      members:
        - judge
        - exact
        - contains
        - regex

::: flowai_harness.evals.JudgeVerdict

::: flowai_harness.evals.TrajectoryScorerConfig

## Results And Events

::: flowai_harness.evals.ScoredSample

::: flowai_harness.evals.ScorerResult

::: flowai_harness.evals.EvalArtifact

::: flowai_harness.evals.EvalArtifactSummary

::: flowai_harness.evals.TestCaseArtifact

::: flowai_harness.evals.SampleArtifact

::: flowai_harness.evals.ArtifactMetadata

::: flowai_harness.evals.ModelInvocation

::: flowai_harness.evals.TokenUsageSummary

::: flowai_harness.evals.SampleCost

::: flowai_harness.evals.SummaryCost

::: flowai_harness.evals.CostAgentBreakdown

::: flowai_harness.evals.SampleLatency

::: flowai_harness.evals.SummaryLatency

::: flowai_harness.evals.PassAtKResult

::: flowai_harness.evals.HarnessEvalEventEnvelope

## Type Aliases

These are `Literal` string aliases, not classes. Pass the string values
directly.

### `ActionPayloadMatchMode`

`Literal["subset", "exact"]` — how `define_expected_actions(...)` compares an
expected payload against the actual payload. `"exact"` (the default) requires
the payloads to match exactly; `"subset"` matches the expected payload as a
deep subset of the actual payload, so generated runtime fields can be omitted.
Both modes compare semantically: object key order is ignored and numbers match
by value (`1` == `1.0`). In subset mode, scalar arrays match by value in any
order; exact mode keeps array order significant.

### `ScorerName`

`Literal["trajectory", "planned_actions", "executed_actions", "final_response"]`
— the scorer identifiers used in `ScoreWeights` and `ScorerResult`.

### `EvalMode`

`Literal["planner", "executor", "sequential", "specialist", "testCaseBuilder"]`
— the eval execution mode used by `EvalConfig.mode` and `EvalArtifact.mode`.

### `TrajectoryMode`

`Literal["strict", "unordered", "subset", "superset", "subsequence"]` — how
`score_sample(...)` and eval runs compare the expected trajectory to the
observed tool/action trajectory.

### `AggregationStrategy`

`Literal["passRate", "meanScore"]` — summary aggregation strategy for
`EvalConfig.aggregation_strategy`.

### `ScorerPresetName`

`Literal["trajectory_only", "planner", "executor", "sequential", "specialist", "test_case_builder"]`
— named scorer weight presets accepted by `define_scorer_preset(...)`,
`define_eval_request(...)`, and `score_sample(...)`.

### `ResponseScorerMethod`

`Literal["judge", "exact", "contains", "regex"]` — discriminator for
`ResponseScorer` final-response scoring methods. Prefer the
`ResponseScorer.judge(...)`, `.exact(...)`, `.contains(...)`, and `.regex(...)`
constructors instead of setting this field manually.
