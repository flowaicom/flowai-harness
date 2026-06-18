# Evals

Use evals when you want a repeatable contract for an agent run: expected tool
trajectory, planned actions, executed actions, final response text, or a
combination of those signals.

Tests verify deterministic behavior. Evals measure quality, reliability, and
behavior across examples.

The examples below use the same public helpers you use in application tests:
offline scoring for known outputs, runtime-backed scoring for executed cases,
and scripted runtime scoring for deterministic tool flows. Use this page as
the first stop for writing an eval, then use the
[Evals reference](../reference/evals.md) for the complete model surface.

## Choose a path

| Path | Use it when | Returns |
| --- | --- | --- |
| `score_sample(...)` | You already have a recorded trajectory, action payload, or final response. | `ScoredSample` with `aggregate` and `component_scores`. |
| `runtime.run_eval(...)` | You want the runtime to execute the case from async code. | A raw artifact dict; validate it with `EvalArtifact.model_validate(...)`. |
| `run_eval_sync(runtime, request)` | Same as `run_eval`, from synchronous code. | A validated `EvalArtifact` with test cases, samples, `aggregate_score`, and `passed`. |
| `runtime.stream_eval(...)` | You want progress events while the eval is running. | Event envelopes plus the final artifact event. |

Start with offline scoring when you already have expected outputs. Use
runtime-backed evals when the runtime itself should execute the case.

## First offline eval

Start with `score_sample(...)` when you want deterministic scoring without
creating a runtime:

```python
from flowai_harness import RawSampleOutput, define_test_case, score_sample


case = define_test_case(
    "planner-basic",
    "Plan the requested change",
    expected_trajectory=["buildPlan", "explainPlan"],
)

scored = score_sample(
    case,
    RawSampleOutput(actual_trajectory=["buildPlan", "explainPlan"]),
    scorer_preset="trajectory_only",
)

assert scored.aggregate == 1.0
assert scored.component_scores[0].scorer_name == "trajectory"
```

`score_sample(...)` returns a `ScoredSample`. It has `aggregate` and
`component_scores`; it does not have a `passed` field. Apply your own threshold
when you use offline scoring:

```python
passed = scored.aggregate >= 0.7
```

Runtime-backed eval artifacts use different field names after Python model
validation: each `SampleArtifact` has `aggregate_score` and `passed`. If you
inspect the raw runtime dictionary before `EvalArtifact.model_validate(...)`,
the same score appears as the wire key `aggregateScore`.

## Score semantics

Use `aggregate` / `aggregate_score` as the authoritative score. Component
scores are diagnostics: they show which scorers contributed and what each
scorer reported, but callers should not recompute pass/fail from
`component_scores`.

For composite presets, the harness computes the aggregate as a weighted average
of child scorer aggregates. Weights are normalized by the total positive
weight, so they do not need to add up to `1.0`:

```python
from flowai_harness import (
    RawSampleOutput,
    ResponseScorer,
    define_final_response_eval,
    define_test_case,
    score_sample,
)


scored = score_sample(
    define_test_case(
        "response-weighting",
        "Update billing",
        final_response=define_final_response_eval(
            scorers=[
                ResponseScorer.contains(
                    id="mentions_update",
                    text="updated",
                    weight=2,
                ),
                ResponseScorer.contains(
                    id="mentions_email",
                    text="jane@example.com",
                    weight=1,
                ),
            ],
            pass_threshold=0.5,
        ),
    ),
    RawSampleOutput(response_text="Billing was updated."),
    score_weights={"final_response": 1},
)

assert scored.component_scores[0].details["score"] == 2 / 3
assert scored.aggregate == 2 / 3
```

Composite presets also include a synthetic `composite` entry in
`component_scores`. That entry reports the weighted aggregate produced from the
named child components. It is useful for diagnostics, but consumers should read
the named component entries for scorer detail and the sample-level aggregate
field for the final score.

`required=True` is a gate, not a weight multiplier. A required response scorer
that fails makes the final-response scorer fail even when the raw weighted
`score` is non-zero. In that case `score` remains available for diagnosis,
`requiredFailed` names the failed required scorer, and `effectiveScore` is `0`.

Two pass thresholds exist at different layers. The final-response
`pass_threshold` applies inside `FinalResponseEval`. The request-level
`EvalConfig.pass_threshold` applies later to the overall sample aggregate.

Judge response scorers have their own runtime and failure semantics; see
[Final-response judge evals](judge-evals.md), including its
[fail-closed behavior](judge-evals.md#fail-closed-semantics).

## Trajectory modes

`trajectory_mode` controls how expected and actual tool sequences are compared.
Default is `unordered`. The public mode names use standard set/sequence
terminology:

| Mode | Meaning |
|---|---|
| `strict` | Expected and actual are exactly the same sequence. |
| `unordered` | Expected and actual are the same multiset, ignoring order. |
| `subset` | Actual is a subset of expected. Extra actual tools fail. |
| `superset` | Actual is a superset of expected. Extra actual tools are allowed. |
| `subsequence` | Expected appears in actual in order. Gaps/extra actual tools are allowed. |

For live agent runs, `superset` is usually the milestone mode: required tools
must happen, but discovery or lookup tools can vary. Use `subsequence` when
those milestones must happen in a specific order.

```python
actual = RawSampleOutput(actual_trajectory=["a", "lookup", "b"])

strict = score_sample(
    define_test_case(
        "exact",
        "Run the tools",
        expected_trajectory=["a", "b"],
        trajectory_mode="strict",
    ),
    actual,
    scorer_preset="trajectory_only",
)
superset = score_sample(
    define_test_case(
        "milestones",
        "Run the tools",
        expected_trajectory=["a", "b"],
        trajectory_mode="superset",
    ),
    actual,
    scorer_preset="trajectory_only",
)

assert strict.aggregate == 0.0
assert superset.aggregate == 1.0
```

Trajectory scoring is a binary contract. The trajectory component `score` is
`1.0` when the selected mode passes and `0.0` when it fails. Similarity numbers
live under `details["diagnostics"]` and are explanatory only:

```python
detail = strict.component_scores[0].details

assert detail["passed"] is False
assert detail["matched"] == ["a", "b"]
assert detail["unexpected"] == ["lookup"]
assert detail["diagnostics"]["precision"] == 2 / 3
assert detail["diagnostics"]["recall"] == 1.0
```

`diagnostics["f1"]` balances precision and recall. `diagnostics["f2"]` uses the
same F-beta formula with `beta=2`, so it emphasizes recall: missing expected
milestone tools hurt more than extra observed tools. These diagnostic values do
not affect aggregation.

## Trajectory projection

Coordinator topologies usually emit `call_agent` in the top-level trajectory.
By default, sub-agent tool calls are not included in the scored projection.

!!! warning "Coordinator defaults"
    By default, sub-agent tool calls are excluded from the scored trajectory.
    A coordinator-routed planner/executor run may therefore score against
    `["call_agent", "call_agent"]` instead of `["storePlan", "executePlan"]`.
    Use `include_sub_agents=True` and `ignore_tools=["call_agent"]` when the
    expected milestones live inside routed sub-agents.

For milestone expectations inside planner and executor agents, include
sub-agent calls and ignore routing tools:

```python
from flowai_harness import define_trajectory_scorer_config


scorer_config = define_trajectory_scorer_config(
    include_sub_agents=True,
    ignore_tools=["call_agent"],
)
```

Use that config with either `score_sample(...)` or `define_eval_config(...)`:

```python
config = define_eval_config(
    samples_per_case=1,
    concurrency=1,
    score_weights={"trajectory": 1.0},
    scorer_config=scorer_config,
)
```

`include_sub_agents=True` includes tool calls emitted inside sub-agent runs.
`ignore_tools=[...]` removes named tools from the scored projection only; it
does not mutate `sample.actual_trajectory` in the eval artifact.

When projection is active, trajectory scorer `details["actual"]` is the scored
projection. `details["projection"]["observedTrajectory"]` preserves the raw
observed trajectory used to build that projection.

## Extra payload keys

`RawSampleOutput.extra` carries scorer payloads that do not fit in
`actual_trajectory` or `response_text`. Use the camelCase keys shown below,
even in Python code. Unknown or misspelled keys are currently ignored by the
scorers, which can look like a real score failure.

| Key | Purpose | Value shape |
| --- | --- | --- |
| `plannedActions` | Planned action list for planner and sequential scoring. | List of `define_resolved_action(...).model_dump(by_alias=True, mode="json")` dicts. |
| `resolvedActions` | Executed/resolved action list for executor and sequential scoring. | List of `define_resolved_action(...).model_dump(by_alias=True, mode="json")` dicts. |
| `trajectoryEvents` | Runtime trajectory projection metadata, including nested sub-agent tool calls. Usually produced by runtime evals, not handwritten. | List of trajectory event dicts. |
| `finalResponseJudgeVerdicts` | Precomputed judge verdicts for offline final-response scoring. Prefer `RawSampleOutput.with_judge_verdicts(...)`. | Mapping from response scorer id to judge verdict wire dict. |

`plannedActions` and `resolvedActions` use the same action wire shape:

```python
from flowai_harness import RawSampleOutput, define_resolved_action


action = define_resolved_action(
    "update_customer",
    {"customerId": "acme", "billingContact": "jane@example.com"},
).model_dump(by_alias=True, mode="json")

output = RawSampleOutput(
    actual_trajectory=["lookupCustomer", "updateCustomer"],
    extra={
        "resolvedActions": [action],
    },
)
```

For planner-side scoring, put the same shape under `plannedActions`:

```python
output = RawSampleOutput(
    actual_trajectory=["storePlan"],
    extra={
        "plannedActions": [action],
    },
)
```

## Expected action payload matching

Use `expected_actions=define_expected_actions(...)` to score planned and
executed business actions. By default, expected action payloads must match the
actual payload exactly.

```python
from flowai_harness import define_expected_action, define_expected_actions

expected_actions = define_expected_actions(
    executed_actions=[
        define_expected_action(
            "update_customer",
            {"customerId": "acme", "billingContact": "jane@example.com"},
        )
    ],
)
```

If the runtime may add generated IDs or other non-business fields, opt into
subset payload matching:

```python
expected_actions = define_expected_actions(
    executed_actions=[
        define_expected_action(
            "apply_discount",
            {"changeType": "discount", "value": 10},
        )
    ],
    payload_match="subset",
)
```

With `payload_match="subset"`, the expected payload must be a deep subset of the
actual payload. Extra action items are still penalized in both modes, and action
list order remains order-insensitive.

Payload comparison is semantic JSON, not string matching, in both modes: object
key order is ignored and numeric values compare by value, so `1` matches `1.0`.
In subset mode, scalar arrays inside the payload also match by value regardless
of order, so `["a", "b", "c"]` matches `["c", "a", "b"]`; extra or missing array
values still fail. Exact mode keeps array order significant.

## Executor trajectory opt-in

Executor evals can also score trajectory when the executor workflow matters.
The default executor signal remains `executed_actions`; authoring an
`expected_trajectory` opts the default executor scorer into a small trajectory
component. This is useful when the executor should call specific domain tools,
not just produce the right final action payload:

```python
from flowai_harness import (
    RawSampleOutput,
    define_expected_action,
    define_expected_actions,
    define_resolved_action,
    define_test_case,
    score_sample,
)

case = define_test_case(
    "executor-workflow",
    "Update the billing contact",
    expected_trajectory=["lookupCustomer", "updateCustomer"],
    trajectory_mode="subsequence",
    expected_actions=define_expected_actions(
        executed_actions=[
            define_expected_action(
                "update_customer",
                {"customerId": "acme", "billingContact": "jane@example.com"},
            )
        ]
    ),
)

output = RawSampleOutput(
    actual_trajectory=["lookupCustomer", "updateCustomer"],
    extra={
        "resolvedActions": [
            define_resolved_action(
                "update_customer",
                {"customerId": "acme", "billingContact": "jane@example.com"},
            ).model_dump(by_alias=True, mode="json")
        ]
    },
)

scored = score_sample(case, output, scorer_preset="executor")
assert scored.aggregate == 1.0
```

## Specialist eval scoring

Specialist evals execute a named specialist directly with
`define_eval_config(mode="specialist", target_agent_id="...")`. Scoring defaults
come from the expectations authored on each selected test case.

A data-answering specialist can score only final response quality:

```python
from flowai_harness import (
    ResponseScorer,
    define_eval_config,
    define_final_response_eval,
    define_test_case,
)

config = define_eval_config(mode="specialist", target_agent_id="insights")
case = define_test_case(
    "insights-answer",
    "Summarize customer risk",
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.contains(
                id="mentions_risk",
                text="risk",
            )
        ]
    ),
)
```

A catalog or tool specialist can score tool usage by authoring the expected
trajectory:

```python
case = define_test_case(
    "catalog-reader",
    "Inspect product metadata",
    expected_trajectory=["get_catalog_entities"],
)
```

An action-taking specialist can score executed actions directly:

```python
from flowai_harness import define_expected_action, define_expected_actions

case = define_test_case(
    "customer-update",
    "Update the billing contact",
    expected_actions=define_expected_actions(
        executed_actions=[
            define_expected_action(
                "update_customer",
                {"customerId": "acme", "billingContact": "jane@example.com"},
            )
        ]
    ),
)
```

When no `score_weights` are provided, each specialist test case must author at
least one of `expected_trajectory`, `final_response`, planned actions, or
executed actions. Explicit `score_weights` can opt into a scorer manually; use
that for contracts such as "this specialist should not call tools" with
`score_weights={"trajectory": 1.0}`.

## First runtime eval

Use `run_eval(...)` when the runtime should execute the case. The deterministic
testing interpreter is enough for artifact smoke tests:

```python
import asyncio

from flowai_harness import (
    AgentSpec,
    EvalArtifact,
    create_runtime,
    define_eval_config,
    define_eval_request,
    define_runtime,
    define_tenant,
    define_test_case,
)


async def run_eval():
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

    raw = await runtime.run_eval(
        define_eval_request(
            runtime,
            workspace_id="workspace-main",
            config=define_eval_config(samples_per_case=1, concurrency=1),
            test_cases=[define_test_case("tc-1", "hello")],
        )
    )
    return EvalArtifact.model_validate(raw)


artifact = asyncio.run(run_eval())
sample = artifact.test_cases[0].samples[0]
assert sample.aggregate_score >= 0.0
```

Add authored expectations or `score_weights` when you want the sample score to
assert a specific behavior contract instead of just validating artifact shape.

From synchronous code, `run_eval_sync(runtime, request)` wraps the same call
and returns a validated `EvalArtifact` directly.

`runtime.stream_eval(...)` yields event envelopes with `runId`, `sequence`,
`type`, and `data`. Use it for progress UIs or long-running evals; use
`runtime.run_eval(...)` when you only need the final artifact.

## Scripted runtime eval

Use `interpreter="scripted"` when you want deterministic runtime execution
through real agent routing, tool dispatch, plan storage, approval policy, and
action dispatchers. The LLM decisions come from JSON scripts embedded in the
test-case input.

This is a CI-friendly pattern for testing coordinator, planner, and executor
flows without provider calls. The core shape is:

Keep the role boundary explicit in scripted evals: the planner stores the plan
with `storePlan`, then the executor runs the stored plan with `executePlan`.

```python
import json

from flowai_harness import (
    define_eval_config,
    define_expected_action,
    define_expected_actions,
    define_test_case,
    define_trajectory_scorer_config,
)


plan_id = "eval-plan-1"
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
                                "body": {
                                    "rationale": "deterministic eval plan",
                                    "actions": [
                                        {
                                            "kind": "record_counter",
                                            "message": "record eval action",
                                        }
                                    ],
                                },
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

config = define_eval_config(
    samples_per_case=1,
    concurrency=1,
    score_weights={"trajectory": 0.5, "executed_actions": 0.5},
    scorer_config=define_trajectory_scorer_config(
        include_sub_agents=True,
        ignore_tools=["call_agent"],
    ),
)

case = define_test_case(
    "tc-nested-scripted-tools",
    coordinator_script,
    expected_trajectory=["storePlan", "executePlan"],
    trajectory_mode="subsequence",
    expected_actions=define_expected_actions(
        executed_actions=[
            define_expected_action(
                "record_counter",
                {"message": "record eval action"},
            )
        ],
    ),
)
```

Create the runtime with `interpreter="scripted"` and pass the case through
`define_eval_request(...)`. The artifact will include:

- `sample.actual_trajectory`, the raw top-level trajectory.
- `sample.metadata["trajectoryEvents"]`, the nested event source used for
  projection.
- `sample.planned_actions`, projected from `storePlan`.
- `sample.resolved_actions`, projected from `executePlan`.

## Verify it works

After an eval run, inspect the artifact:

```python
assert artifact.test_cases
sample = artifact.test_cases[0].samples[0]
assert sample.aggregate_score >= 0.0
assert sample.passed in {True, False}
```

For streamed evals, verify that the stream yields progress events and a final
artifact event. For CI, prefer deterministic or scripted runtimes unless the
purpose of the eval is to measure live model behavior.

## `source_thread_id`

`source_thread_id` / `sourceThreadId` is provenance for authored test cases. It
is not reused as the eval execution thread. Multi-turn eval replay is not
implemented yet, so model follow-up behavior should be represented as
single-turn eval cases for now.

## Next steps

- Use [Final-response judge evals](judge-evals.md) for judge-backed response
  scoring, verdict artifacts, and fail-closed semantics.
- Use [Test agents without provider calls](testing.md) for deterministic
  interpreters, tool context in tests, and approval flows.
- Use the [Evals reference](../reference/evals.md) for every DTO and helper.
