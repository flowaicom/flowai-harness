# Judge final responses

Use `define_final_response_eval(...)` when the runtime's final user-facing
text is part of the product outcome. This is common for action-taking agents:
action scorers verify what the agent planned or executed, and final-response
scorers verify what the coordinator told the user.

Deterministic response scorers (`ResponseScorer.exact`, `.contains`, `.regex`)
cover exact IDs, emails, status strings, and constrained outputs.
`ResponseScorer.judge(...)` adds one binary LLM-as-judge call per judge scorer
for qualities that resist string matching, such as "does the response report
success?" or "is it similar to this reference answer?".

Judge response scorers are runtime-backed. They run during
`runtime.run_eval(...)`, `run_eval_sync(...)`, or `runtime.stream_eval(...)`;
`score_sample(...)` stays deterministic and does not call a judge model.

## When to use this guide

Use final-response judge evals when the quality of the final natural-language
answer matters, not just the trajectory or tool calls.

Use [Evals](evals.md) first when you are scoring tool trajectories or action
payloads. Use [Test agents without provider calls](testing.md) when you need
deterministic correctness checks without model calls.

## Configure judge scorers

The example below scores an action-taking flow on three signals: the executed
action, the tool trajectory, and the final response. It needs a runtime with a
real provider, since both the agents and the judge make model calls:

```python
import os

from flowai_harness import (
    AgentSpec,
    ResponseScorer,
    create_runtime,
    define_eval_config,
    define_eval_request,
    define_expected_action,
    define_expected_actions,
    define_final_response_eval,
    define_runtime,
    define_tenant,
    define_test_case,
    run_eval_sync,
)


coordinator = AgentSpec(
    name="coordinator",
    role="coordinator",
    model="claude-sonnet-4-6",
    system_prompt="Update customer records and report the outcome to the user.",
    routes=["planner", "executor"],
)
planner = AgentSpec(
    name="planner",
    role="planner",
    model="claude-sonnet-4-6",
    system_prompt="Plan the requested change.",
)
executor = AgentSpec(
    name="executor",
    role="executor",
    model="claude-haiku-4-5",
    system_prompt="Execute the approved plan.",
)

runtime = create_runtime(
    define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, planner, executor],
        providers={"anthropic": {"apiKey": os.environ["ANTHROPIC_API_KEY"]}},
    )
)

case = define_test_case(
    "billing-contact-update",
    "Update Acme Corp's billing contact to jane@example.com and tell me what changed.",
    expected_trajectory=["executePlan"],
    expected_actions=define_expected_actions(
        executed_actions=[
            define_expected_action(
                "update_customer",
                {
                    "customerId": "acme",
                    "billingContact": "jane@example.com",
                },
            )
        ],
    ),
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.judge(
                id="reports_success",
                instructions=(
                    "The final response states that the billing contact update "
                    "succeeded."
                ),
                reference_response=(
                    "Acme Corp's billing contact was updated to jane@example.com."
                ),
                required=True,
                weight=2,
            ),
            ResponseScorer.judge(
                id="does_not_claim_refund",
                instructions="The final response does not say that a refund was issued.",
                required=True,
            ),
            ResponseScorer.regex(
                id="mentions_email",
                pattern=r"\bjane@example\.com\b",
                weight=1,
            ),
        ],
        pass_threshold=0.8,
    ),
)

request = define_eval_request(
    runtime,
    workspace_id="workspace-main",
    config=define_eval_config(
        mode="sequential",
        provider="anthropic",
        model="claude-sonnet-4-6",
        samples_per_case=3,
        concurrency=1,
        score_weights={
            "executed_actions": 0.45,
            "final_response": 0.40,
            "trajectory": 0.15,
        },
    ),
    test_cases=[case],
)

artifact = run_eval_sync(runtime, request)
sample = artifact.test_cases[0].samples[0]

assert sample.final_response_eval is not None
assert sample.final_response_eval["passed"] is True
```

Keep each judge scorer narrow and binary. Good scorer instructions usually ask
one question, such as "does the response report success?", "does it include the
required limitation?", or "is it similar to this reference response?". The judge
returns only `passed`, `selected_rubric_score` (`0` or `1`), and `reason`; the
harness owns all weighting and aggregation. (See
[Score semantics](evals.md#score-semantics) for how weights, `required`, and
the two `pass_threshold` layers combine.)

If the pass/fail boundary needs to be explicit, add a binary rubric:

```python
ResponseScorer.judge(
    id="matches_reference",
    instructions="Compare the final response to the reference response.",
    reference_response="The billing contact was updated to jane@example.com.",
    rubric={
        0: "The response is not similar, misses key facts, or changes the meaning.",
        1: "The response is similar and contains the key facts, even if wording differs.",
    },
    required=True,
)
```

## Judge model selection

Judge scorers use the eval run `provider` / `model` when configured on
`define_eval_config(...)`. If those are omitted, the runtime falls back to the
coordinator model, then the first registered agent model.

## Unit-test weighting without a judge model

For unit tests, do not call a live judge model just to verify weighting,
`required`, or `pass_threshold` behavior. Precompute the judge verdicts and pass
them through `RawSampleOutput.with_judge_verdicts(...)`:

```python
from flowai_harness import (
    JudgeVerdict,
    RawSampleOutput,
    ResponseScorer,
    define_final_response_eval,
    define_test_case,
    score_sample,
)


case = define_test_case(
    "billing-response-unit",
    "Update Acme Corp's billing contact.",
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.judge(
                id="reports_success",
                instructions="Pass when the response reports the update.",
                weight=2,
            ),
            ResponseScorer.judge(
                id="does_not_claim_refund",
                instructions="Pass when the response does not claim a refund.",
                required=True,
            ),
        ],
        pass_threshold=0.75,
    ),
)

output = RawSampleOutput(
    response_text="Acme Corp's billing contact was updated."
).with_judge_verdicts(
    {
        "reports_success": JudgeVerdict(
            passed=True,
            selected_rubric_score=1,
            reason="The response reports the successful update.",
        ),
        "does_not_claim_refund": JudgeVerdict(
            passed=True,
            selected_rubric_score=1,
            reason="The response does not mention a refund.",
        ),
    }
)

scored = score_sample(case, output, score_weights={"final_response": 1})

assert scored.aggregate == 1.0
```

## Read verdicts in artifacts

Judge-backed artifacts include the final response text, per-scorer verdicts,
judge run metadata, and judge model invocations:

```json
{
  "responseText": "Acme Corp's billing contact was updated to jane@example.com.",
  "finalResponseEval": {
    "passed": true,
    "score": 1.0,
    "effectiveScore": 1.0,
    "passThreshold": 0.8,
    "requiredFailed": [],
    "responseScorers": [
      {
        "id": "reports_success",
        "method": "judge",
        "passed": true,
        "score": 1.0,
        "required": true,
        "details": {
          "verdict": {
            "passed": true,
            "selected_rubric_score": 1,
            "reason": "The response states that the update succeeded."
          },
          "judgeRun": {
            "schemaVersion": 1,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "promptSha256": "64-character SHA-256 hex digest",
            "contextSha256": "64-character SHA-256 hex digest"
          }
        }
      }
    ]
  },
  "modelInvocations": [
    {
      "agent": "judge",
      "provider": "anthropic",
      "model": "claude-sonnet-4-6"
    }
  ]
}
```

`contextSha256` is a stable hash of the judge context, used for review and
deduplication. It covers the scorer id, instructions, reference response,
rubric, authored context, runtime context, and final response text.
`promptSha256` hashes the rendered judge prompt. Both are inspectability
metadata, not part of the score math.

## Judge trace

To inspect the rendered judge prompt and raw judge model response, opt into
judge trace output:

```python
from flowai_harness import define_final_response_scorer_config

request = define_eval_request(
    runtime,
    workspace_id="workspace-main",
    config=define_eval_config(
        mode="sequential",
        provider="anthropic",
        model="claude-sonnet-4-6",
        scorer_config=define_final_response_scorer_config(
            include_judge_trace=True,
        ),
    ),
    test_cases=[case],
)
```

Judge trace is off by default because it can contain final responses,
reference answers, rubric text, authored context, runtime context, and other
test data. When enabled, each judge scorer detail includes:

```json
{
  "judgeTrace": {
    "prompt": "Rendered judge prompt...",
    "response": "{\"passed\": true, \"selected_rubric_score\": 1, \"reason\": \"...\"}"
  }
}
```

## Fail-closed semantics

Judge scorer execution errors fail closed in v1. If no judge provider is
available, the judge produces no final text, or the judge output cannot be
parsed as the verdict schema, the affected scorer is recorded as
`passed=false` with `score=0.0` and an explicit `errorKind` in its details.

Errored judge scorers are not excluded or renormalized out of the
final-response aggregate; they count as failed scorers in the weighting, and a
required errored judge scorer fails the final-response eval. A judge outage
therefore reads as failing scores, never as silently passing ones.

## Verify it works

After a run, inspect the first sample and confirm:

- `sample.final_response_eval` is present
- each judge scorer has a verdict
- required scorers passed
- the artifact records the judge model invocation

For unit tests of weighting and thresholds, use precomputed
`JudgeVerdict` values so the test does not call a live judge model.

## See also

- [Evals](evals.md) for score semantics, trajectory modes, action payload
  matching, and runtime-backed eval mechanics.
- [Test agents without provider calls](testing.md) for deterministic
  interpreters and approval flows.
- [Evals reference](../reference/evals.md) for `ResponseScorer`,
  `FinalResponseEval`, `JudgeVerdict`, and artifact models.
