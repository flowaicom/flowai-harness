# Run your first eval

Author test cases for a tiny agent, score it, and read the results in ten to
fifteen minutes.

This walkthrough takes a minimal runtime — the same coordinator-and-specialist
shape as [the minimal runtime smoke test](../index.md#minimal-runtime) — and
puts a repeatable contract around it: you author test cases with
`define_test_case`, configure a run with `define_eval_config` and
`define_eval_request`, execute it with `run_eval_sync`, and read the resulting
`EvalArtifact`. Then you break one case on purpose and learn how a failure
reads, which is the skill you will actually use. Everything runs under the
deterministic testing interpreter, so no provider credentials are required and
every score is reproducible.

## 1. Install

From the root of a clone of the source repository:

```bash
./scripts/check-env.sh
./scripts/install.sh
./.venv/bin/flowai-harness --version
```

Python 3.12 and the Rust toolchain are required: `install.sh` builds the native
runtime from source into `.venv`. Run the scripts in this guide with
`./.venv/bin/python`. The model ids used below are illustrative: the
deterministic testing interpreter never calls a provider, so any model string
validates.

## 2. Build a runtime to evaluate

An eval needs an agent under test. This one is deliberately tiny: a
coordinator routes to a single specialist, and the deterministic testing
interpreter answers every query with a fixed mock response instead of calling
a model. That fixed text is what your test cases will score, which is exactly
what makes this tutorial's results guaranteed.

```python
from flowai_harness import (
    EvalArtifact,
    ResponseScorer,
    TestingConfig,
    create_runtime,
    define_coordinator,
    define_eval_config,
    define_eval_request,
    define_final_response_eval,
    define_runtime,
    define_specialist,
    define_tenant,
    define_test_case,
    run_eval_sync,
)

MOCK_RESPONSE = "Acme offers full refunds within 30 days of purchase."

tenant = define_tenant("acme", "v1")

specialist = define_specialist(
    name="policy_answers",
    model="claude-haiku-4-5",
    prompt="You answer Acme support policy questions.",
)
coordinator = define_coordinator(
    name="support_coordinator",
    model="claude-sonnet-4-6",
    routes=["policy_answers"],
    prompt="Route policy questions to the policy_answers specialist.",
)

runtime = create_runtime(
    define_runtime(
        tenant=tenant,
        agents=[coordinator, specialist],
        providers={"anthropic": {"apiKey": "unused"}},
    ),
    testing=TestingConfig(mock_response=MOCK_RESPONSE),
)
```

The placeholder `{"apiKey": "unused"}` satisfies provider validation without
any network access — the deterministic testing interpreter never calls the
provider.

## 3. Author test cases

A test case pairs an input prompt with expectations. `define_test_case` can
expect tool trajectories, planned or executed business actions, and final
response text; this tutorial scores only the final response, the simplest
signal to start with.

`define_final_response_eval` declares how the final user-facing text is
scored. Each `ResponseScorer` is one check, and three of the four scorer
methods are fully deterministic: `exact` (string equality), `contains`
(substring), and `regex` (pattern match). The fourth method, `judge`, asks an
LLM to grade the response — it needs real credentials, so it stays out of this
tutorial.

```python
refund_window_case = define_test_case(
    "refund-window",
    "How long do I have to return a purchase?",
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.contains(
                id="mentions_30_days",
                text="30 days",
            )
        ]
    ),
)

refund_offer_case = define_test_case(
    "refund-offer",
    "Do you offer refunds?",
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.regex(
                id="mentions_refunds",
                pattern=r"refunds?",
            )
        ]
    ),
)
```

Both cases pass against `MOCK_RESPONSE`: it contains `30 days`, and it matches
`refunds?`. The first positional argument is the test case id — it names the
case in every artifact, so make it readable.

## 4. Configure the run

`define_eval_config` controls how the run executes and how samples are scored.

```python
config = define_eval_config(
    samples_per_case=1,
    concurrency=1,
    score_weights={"final_response": 1.0},
)

request = define_eval_request(
    runtime,
    workspace_id="workspace-main",
    config=config,
    test_cases=[refund_window_case, refund_offer_case],
)
```

Three choices worth understanding:

- `samples_per_case=1`: the default is 3 because live models vary between
  runs. The deterministic testing interpreter always returns the same text,
  so one sample per case is enough here.
- `score_weights={"final_response": 1.0}`: by default the sequential preset
  blends trajectory and action scorers into the aggregate. This runtime has no
  plan or tools to score, so the whole aggregate points at the final-response
  scorer.
- `pass_threshold` is left at its default of `0.7`: a sample passes when its
  aggregate score reaches the threshold.

`define_eval_request` binds everything to the runtime: the tenant id defaults
from `runtime.resource_id` (here `"acme"`), and `workspace_id` names the
workspace the run is recorded under.

## 5. Run the eval and read the artifact

`run_eval_sync(runtime, request)` executes every test case through the runtime
and blocks until the run completes, returning a validated `EvalArtifact`. A
small helper prints the fields you will read most often:

```python
def summarize(artifact: EvalArtifact) -> None:
    summary = artifact.summary
    print(f"test cases: {summary.total_test_cases}")
    print(f"passed:     {summary.passed}")
    print(f"failed:     {summary.failed}")
    print(f"pass rate:  {summary.pass_rate}")
    print(f"aggregate:  {summary.aggregate_score}")
    for test_case in artifact.test_cases:
        for sample in test_case.samples:
            status = "PASS" if sample.passed else "FAIL"
            print(
                f"  [{status}] {test_case.test_case_id} "
                f"sample {sample.sample_index}: "
                f"aggregate_score={sample.aggregate_score}"
            )
```

Run the request and summarize the artifact:

```python
print("== First run: both cases pass ==")
passing_artifact = run_eval_sync(runtime, request)
print(f"run id: {passing_artifact.run_id}")
summarize(passing_artifact)
```

The first run prints exactly this, except for the `run id` value, which is a
fresh UUID on every run:

```text
== First run: both cases pass ==
run id: eval-34ba8485-6af6-4649-ab75-e96c5350ad57
test cases: 2
passed:     2
failed:     0
pass rate:  1.0
aggregate:  1.0
  [PASS] refund-window sample 0: aggregate_score=1.0
  [PASS] refund-offer sample 0: aggregate_score=1.0
```

Both samples pass and the aggregate is `1.0`. Reading from the top of the
artifact down:

- `artifact.summary` holds run-level totals: `passed` and `failed` count test
  cases, `pass_rate` is their ratio, and `aggregate_score` follows the
  configured aggregation strategy (`passRate` by default, so here it equals
  the pass rate).
- `artifact.test_cases` is the per-case breakdown. Each `TestCaseArtifact`
  carries its `test_case_id` and one `SampleArtifact` per sample.
- Each sample has `passed` (its `aggregate_score` measured against the
  config's `pass_threshold`), the `response_text` that was scored, and
  `component_scores` showing what each scorer contributed.

## 6. Make a case fail on purpose

Passing evals tell you the contract holds; failing evals are where you spend
your time. To learn how a failure reads, author a case that expects text the
mock response does not contain, and build a second request with it:

```python
store_credit_case = define_test_case(
    "refund-store-credit",
    "How long do I have to return a purchase?",
    final_response=define_final_response_eval(
        scorers=[
            ResponseScorer.contains(
                id="mentions_store_credit",
                text="store credit",
            )
        ]
    ),
)

failing_request = define_eval_request(
    runtime,
    workspace_id="workspace-main",
    config=config,
    test_cases=[store_credit_case, refund_offer_case],
)
```

When a sample fails, the artifact tells you why: each sample's
`final_response_eval` dict carries a `responseScorers` list (wire-shaped, so
the keys are camelCase), and every scorer entry reports its `passed` flag,
`score`, and a human-readable `reason`. A second helper walks the failed
samples:

```python
def explain_failures(artifact: EvalArtifact) -> None:
    for test_case in artifact.test_cases:
        for sample in test_case.samples:
            if sample.passed:
                continue
            print(f"why {test_case.test_case_id} failed:")
            print(f"  response_text: {sample.response_text!r}")
            for scorer in sample.final_response_eval["responseScorers"]:
                print(
                    f"  scorer {scorer['id']}: "
                    f"passed={scorer['passed']} score={scorer['score']}"
                )
                print(f"    reason: {scorer['reason']}")
```

Run the failing request and explain the failure:

```python
print()
print("== Second run: one case fails on purpose ==")
failing_artifact = run_eval_sync(runtime, failing_request)
summarize(failing_artifact)
explain_failures(failing_artifact)
```

The second run prints exactly this:

```text
== Second run: one case fails on purpose ==
test cases: 2
passed:     1
failed:     1
pass rate:  0.5
aggregate:  0.5
  [FAIL] refund-store-credit sample 0: aggregate_score=0.0
  [PASS] refund-offer sample 0: aggregate_score=1.0
why refund-store-credit failed:
  response_text: 'Received: How long do I have to return a purchase?\n\nAcme offers full refunds within 30 days of purchase.'
  scorer mentions_store_credit: passed=False score=0.0
    reason: The final response did not contain the required text.
```

One case fails, so the pass rate drops to `0.5`. The failure trail reads
bottom-up: the `reason` says the required text was missing, `response_text`
shows what the agent actually said, and the summary line shows the sample's
`aggregate_score` of `0.0` falling below the `0.7` threshold. With a live
model you would now decide whether the agent is wrong or the expectation is —
here, of course, the case demanded `store credit` from a mock that only talks
about refunds.

!!! note "The `Received:` prefix"
The deterministic testing interpreter prefixes every response with
`Received: <input>` before the mock text. Your scorers run against the
full `response_text`, which is why substring and regex checks are a better
fit for it than `exact`.

## 7. Save and run

Save the assembled sections as `first_eval.py` and run it:

```bash
./.venv/bin/python first_eval.py
```

The script prints both runs from steps 5 and 6 — first run two passes with
aggregate `1.0`, second run one pass and one fail with pass rate `0.5` — and
exits. No credentials required.

!!! tip "Next steps"
[Evals](../guides/evals.md) covers the deeper scoring semantics: trajectory
modes, action ground truth, score weighting, and offline scoring with
`score_sample(...)`. [Final-response judge evals](../guides/judge-evals.md)
adds the LLM-judge scorer method this tutorial skipped — that one does need
credentials. The [Evals reference](../reference/evals.md) documents every
DTO and helper on the eval surface.
