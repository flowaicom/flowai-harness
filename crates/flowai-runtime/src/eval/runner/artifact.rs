use std::collections::BTreeMap;

use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_eval::{
    EvalConfig, EvalMode, EvalSummary, EvalTestCase, PassAtKResult, RawSampleOutput, SampleResult,
    ScorerResult, TestCaseResult, TokenUsageSummary,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::eval::{extract_planned_actions, extract_resolved_actions};
use crate::eval::{ResolvedAction, SCORER_FINAL_RESPONSE};

pub const EVAL_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRequest {
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub config: EvalConfig,
    pub test_cases: Vec<EvalTestCase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_weights: Option<BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalArtifact {
    pub run_id: String,
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub mode: EvalMode,
    pub summary: EvalArtifactSummary,
    pub test_cases: Vec<TestCaseArtifact>,
    pub metadata: ArtifactMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub schema_version: u32,
    pub scorer_preset: String,
    pub score_weights: BTreeMap<String, f64>,
}

impl ArtifactMetadata {
    pub fn new(scorer_preset: impl Into<String>, score_weights: BTreeMap<String, f64>) -> Self {
        Self {
            schema_version: EVAL_ARTIFACT_SCHEMA_VERSION,
            scorer_preset: scorer_preset.into(),
            score_weights,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalArtifactSummary {
    pub total_test_cases: u32,
    pub passed: u32,
    pub failed: u32,
    #[serde(default)]
    pub skipped: u32,
    pub aggregate_score: f64,
    pub pass_rate: f64,
    #[serde(default)]
    pub pass_at_k: Vec<PassAtKResult>,
    pub total_duration_ms: u64,
    pub total_usage: TokenUsageSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<SummaryCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<SummaryLatency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseArtifact {
    pub test_case_id: String,
    #[serde(default)]
    pub input: Option<String>,
    pub samples: Vec<SampleArtifact>,
    #[serde(default)]
    pub pass_at_k: Vec<PassAtKResult>,
    pub aggregate_score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleArtifact {
    pub sample_index: u32,
    pub passed: bool,
    pub aggregate_score: f64,
    pub component_scores: Vec<ScorerResult>,
    pub actual_trajectory: Vec<String>,
    #[serde(default)]
    pub response_text: Option<String>,
    #[serde(default)]
    pub final_response_eval: Option<JsonValue>,
    #[serde(default)]
    pub planned_actions: Vec<ResolvedAction>,
    #[serde(default)]
    pub resolved_actions: Vec<ResolvedAction>,
    pub duration_ms: u64,
    #[serde(default)]
    pub model_invocations: Vec<ModelInvocation>,
    pub token_usage: TokenUsageSummary,
    #[serde(default)]
    pub cost: Option<SampleCost>,
    #[serde(default)]
    pub latency: Option<SampleLatency>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub trace: Option<EvalTraceRef>,
    #[serde(default)]
    pub metadata: Option<JsonValue>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInvocation {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_llm_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryCost {
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub per_agent: Vec<CostAgentBreakdown>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostAgentBreakdown {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: String,
    pub usage: TokenUsageSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleLatency {
    pub total_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryLatency {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p99_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTraceRef {
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub metadata: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum HarnessEvalEvent {
    EvalStarted { artifact: EvalArtifact },
    TestCaseStarted { test_case_id: String },
    SampleCompleted { sample: SampleArtifact },
    TestCaseCompleted { test_case: TestCaseArtifact },
    EvalCompleted { artifact: EvalArtifact },
    EvalFailed { error: String },
    EvalCancelled { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEvalEventEnvelope {
    pub run_id: String,
    pub sequence: u64,
    #[serde(flatten)]
    pub event: HarnessEvalEvent,
}

impl SampleArtifact {
    pub fn from_framework_result(
        sample: &SampleResult,
        model_invocations: Vec<ModelInvocation>,
    ) -> Self {
        let raw_output = RawSampleOutput {
            actual_trajectory: sample.actual_trajectory.clone(),
            response_text: sample.response_text.clone(),
            extra: sample.metadata.clone(),
        };
        let resolved_actions = extract_resolved_actions(&raw_output).unwrap_or_default();
        let planned_actions = extract_planned_actions(&raw_output).unwrap_or_default();

        Self {
            sample_index: sample.sample_index,
            passed: sample.passed,
            aggregate_score: aggregate_score_for_sample(sample),
            component_scores: sample.scores.clone(),
            actual_trajectory: sample.actual_trajectory.clone(),
            response_text: sample.response_text.clone(),
            final_response_eval: final_response_eval_from_scores(sample),
            planned_actions,
            resolved_actions,
            duration_ms: sample.duration_ms,
            cost: sample_cost_from_invocations(&model_invocations),
            model_invocations,
            token_usage: sample.token_usage.clone(),
            latency: sample.latency.as_ref().map(SampleLatency::from_core),
            thread_id: sample.thread_id.clone(),
            trace: sample.trace.as_ref().map(|trace| EvalTraceRef {
                trace_id: trace.trace_id.clone(),
                thread_id: trace
                    .scope
                    .thread_id
                    .as_ref()
                    .map(|id| id.as_str().to_string()),
                url: None,
                metadata: JsonValue::Object(Default::default()),
            }),
            metadata: sample.metadata.clone(),
            error: sample.error.clone(),
        }
    }
}

fn final_response_eval_from_scores(sample: &SampleResult) -> Option<JsonValue> {
    sample
        .scores
        .iter()
        .find(|score| score.scorer_name == SCORER_FINAL_RESPONSE)
        .and_then(|score| score.details.clone())
}

impl TestCaseArtifact {
    pub fn from_framework_result(
        result: &TestCaseResult,
        model_invocations_by_sample: impl Fn(u32) -> Vec<ModelInvocation>,
    ) -> Self {
        Self {
            test_case_id: result.test_case_id.as_str().to_string(),
            input: result.input.clone(),
            samples: result
                .samples
                .iter()
                .map(|sample| {
                    SampleArtifact::from_framework_result(
                        sample,
                        model_invocations_by_sample(sample.sample_index),
                    )
                })
                .collect(),
            pass_at_k: result.pass_at_k.clone(),
            aggregate_score: result.aggregate_score,
        }
    }
}

impl EvalArtifactSummary {
    pub fn from_framework_summary(summary: &EvalSummary) -> Self {
        let completed = summary.passed + summary.failed;
        let pass_rate = if completed == 0 {
            0.0
        } else {
            summary.passed as f64 / completed as f64
        };

        Self {
            total_test_cases: summary.total_test_cases,
            passed: summary.passed,
            failed: summary.failed,
            skipped: summary.skipped,
            aggregate_score: summary.aggregate_score,
            pass_rate,
            pass_at_k: summary.pass_at_k.clone(),
            total_duration_ms: summary.total_duration_ms,
            total_usage: summary.total_usage.clone(),
            cost: Some(SummaryCost {
                estimated_cost_usd: summary.cost.estimated_cost_usd,
                per_agent: summary
                    .cost
                    .per_agent
                    .iter()
                    .map(|agent| CostAgentBreakdown {
                        agent: agent.agent_name.clone(),
                        provider: None,
                        model: agent.model.clone(),
                        usage: agent.usage.clone(),
                        estimated_cost_usd: Some(agent.cost_usd),
                    })
                    .collect(),
            }),
            latency: summary.latency.as_ref().map(SummaryLatency::from_core),
            metadata: summary.metadata.clone(),
        }
    }
}

impl SampleLatency {
    fn from_core(latency: &agent_fw_core::LatencySummary) -> Self {
        Self {
            total_ms: latency.total_duration_ms,
            first_token_ms: latency.ttft_ms,
            model_ms: Some(latency.phases.llm_time_ms),
            tool_ms: Some(latency.phases.tool_time_ms),
        }
    }
}

impl SummaryLatency {
    fn from_core(_latency: &agent_fw_core::LatencySummary) -> Self {
        // Do not infer percentiles from a single aggregate latency value.
        // EvalRunner can fill these once it has the full per-sample latency
        // population and Studio confirms the desired percentile semantics.
        Self {
            p50_ms: None,
            p95_ms: None,
            p99_ms: None,
            min_ms: None,
            max_ms: None,
        }
    }
}

fn aggregate_score_for_sample(sample: &SampleResult) -> f64 {
    // Composite scorers currently append the authoritative aggregate result
    // last. eval runner keeps this local until the framework exposes the original
    // ScoredSample.aggregate on SampleResult.
    sample
        .scores
        .last()
        .map(|score| score.score)
        .unwrap_or(if sample.passed { 1.0 } else { 0.0 })
}

fn sample_cost_from_invocations(invocations: &[ModelInvocation]) -> Option<SampleCost> {
    let mut has_known_cost = false;
    let llm_cost_usd = invocations.iter().fold(0.0, |acc, invocation| {
        if let Some(cost) = invocation.estimated_cost_usd {
            has_known_cost = true;
            acc + cost
        } else {
            acc
        }
    });

    has_known_cost.then_some(SampleCost {
        llm_cost_usd: Some(llm_cost_usd),
        non_llm_cost_usd: None,
        total_cost_usd: Some(llm_cost_usd),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::ResolvedAction;
    use serde_json::json;

    fn token_usage(input: u64, output: u64) -> TokenUsageSummary {
        TokenUsageSummary::new(input, output, 0, 0)
    }

    #[test]
    fn eval_artifact_wire_shape_uses_contract_field_names() {
        let mut weights = BTreeMap::new();
        weights.insert("trajectory".to_string(), 0.5);
        weights.insert("fused_executor".to_string(), 0.5);

        let artifact = EvalArtifact {
            run_id: "eval-run-id".to_string(),
            tenant_id: TenantId::new_unchecked("tenant-acme"),
            workspace_id: WorkspaceId::new("workspace-main").expect("workspace id"),
            mode: EvalMode::Sequential,
            summary: EvalArtifactSummary {
                total_test_cases: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
                aggregate_score: 1.0,
                pass_rate: 1.0,
                pass_at_k: vec![],
                total_duration_ms: 1234,
                total_usage: token_usage(100, 40),
                cost: Some(SummaryCost {
                    estimated_cost_usd: 0.01,
                    per_agent: vec![],
                }),
                latency: None,
                metadata: None,
            },
            test_cases: vec![TestCaseArtifact {
                test_case_id: "tc-1".to_string(),
                input: Some("test query".to_string()),
                samples: vec![SampleArtifact {
                    sample_index: 0,
                    passed: true,
                    aggregate_score: 1.0,
                    component_scores: vec![ScorerResult::new("trajectory", 1.0)],
                    actual_trajectory: vec!["buildPlan".to_string()],
                    response_text: None,
                    final_response_eval: None,
                    planned_actions: vec![],
                    resolved_actions: vec![ResolvedAction::new(
                        "price_change",
                        serde_json::json!({
                            "changeType": "absolute",
                            "value": 10.0,
                            "productIds": ["p1"],
                        }),
                    )],
                    duration_ms: 1234,
                    model_invocations: vec![ModelInvocation {
                        agent: "planner".to_string(),
                        provider: Some("anthropic".to_string()),
                        model: "claude-sonnet-4-6".to_string(),
                        input_tokens: 100,
                        output_tokens: 40,
                        cached_tokens: 0,
                        cache_creation_tokens: 0,
                        estimated_cost_usd: Some(0.01),
                    }],
                    token_usage: token_usage(100, 40),
                    cost: Some(SampleCost {
                        llm_cost_usd: Some(0.01),
                        non_llm_cost_usd: Some(0.0),
                        total_cost_usd: Some(0.01),
                    }),
                    latency: Some(SampleLatency {
                        total_ms: 1234,
                        first_token_ms: None,
                        model_ms: None,
                        tool_ms: None,
                    }),
                    thread_id: Some("thread-1".to_string()),
                    trace: Some(EvalTraceRef {
                        trace_id: "trace-1".to_string(),
                        thread_id: Some("thread-1".to_string()),
                        url: None,
                        metadata: json!({}),
                    }),
                    metadata: None,
                    error: None,
                }],
                pass_at_k: vec![],
                aggregate_score: 1.0,
            }],
            metadata: ArtifactMetadata::new("sequential", weights),
        };

        let value = serde_json::to_value(&artifact).expect("artifact serializes");
        assert_eq!(value["runId"], "eval-run-id");
        assert_eq!(value["tenantId"], "tenant-acme");
        assert_eq!(value["workspaceId"], "workspace-main");
        assert_eq!(value["testCases"][0]["testCaseId"], "tc-1");
        assert_eq!(
            value["testCases"][0]["samples"][0]["componentScores"][0]["scorerName"],
            "trajectory"
        );
        assert_eq!(
            value["testCases"][0]["samples"][0]["resolvedActions"][0]["payload"]["productIds"][0],
            "p1"
        );
        assert_eq!(
            value["testCases"][0]["samples"][0]["modelInvocations"][0]["provider"],
            "anthropic"
        );
        assert_eq!(value["metadata"]["schemaVersion"], 1);
        assert_eq!(value["metadata"]["scoreWeights"]["fused_executor"], 0.5);
        assert!(value["testCases"][0]["samples"][0]["metadata"].is_null());
        assert!(value["testCases"][0]["samples"][0]["error"].is_null());

        let decoded: EvalArtifact = serde_json::from_value(value).expect("artifact decodes");
        assert_eq!(decoded, artifact);
    }

    #[test]
    fn sample_artifact_serializes_empty_resolved_actions_as_array() {
        let sample = SampleArtifact {
            sample_index: 0,
            passed: false,
            aggregate_score: 0.0,
            component_scores: vec![ScorerResult::new("trajectory", 0.0)],
            actual_trajectory: vec![],
            response_text: None,
            final_response_eval: None,
            planned_actions: vec![],
            resolved_actions: vec![],
            duration_ms: 0,
            model_invocations: vec![],
            token_usage: TokenUsageSummary::ZERO,
            cost: None,
            latency: None,
            thread_id: None,
            trace: None,
            metadata: None,
            error: Some("failed".to_string()),
        };

        let value = serde_json::to_value(sample).expect("sample serializes");
        assert_eq!(value["resolvedActions"], json!([]));
        assert_eq!(value["modelInvocations"], json!([]));
        assert!(value["cost"].is_null());
        assert!(value["latency"].is_null());
        assert!(value["threadId"].is_null());
        assert!(value["trace"].is_null());
        assert!(value["metadata"].is_null());
    }

    #[test]
    fn event_envelope_uses_type_and_nested_data() {
        let sample = SampleArtifact {
            sample_index: 0,
            passed: true,
            aggregate_score: 1.0,
            component_scores: vec![ScorerResult::new("trajectory", 1.0)],
            actual_trajectory: vec!["buildPlan".to_string()],
            response_text: None,
            final_response_eval: None,
            planned_actions: vec![],
            resolved_actions: vec![],
            duration_ms: 10,
            model_invocations: vec![],
            token_usage: TokenUsageSummary::ZERO,
            cost: None,
            latency: None,
            thread_id: None,
            trace: None,
            metadata: None,
            error: None,
        };

        let envelope = HarnessEvalEventEnvelope {
            run_id: "eval-run-id".to_string(),
            sequence: 3,
            event: HarnessEvalEvent::SampleCompleted { sample },
        };

        let value = serde_json::to_value(envelope).expect("event serializes");
        assert_eq!(
            value,
            json!({
                "runId": "eval-run-id",
                "sequence": 3,
                "type": "sampleCompleted",
                "data": {
                    "sample": {
                        "sampleIndex": 0,
                        "passed": true,
                        "aggregateScore": 1.0,
                        "componentScores": [{
                            "scorerName": "trajectory",
                            "score": 1.0
                        }],
                        "actualTrajectory": ["buildPlan"],
                        "responseText": null,
                        "finalResponseEval": null,
                        "plannedActions": [],
                        "resolvedActions": [],
                        "durationMs": 10,
                        "modelInvocations": [],
                        "tokenUsage": {
                            "inputTokens": 0,
                            "outputTokens": 0,
                            "cachedTokens": 0,
                            "cacheCreationTokens": 0
                        },
                        "cost": null,
                        "latency": null,
                        "threadId": null,
                        "trace": null,
                        "metadata": null,
                        "error": null
                    }
                }
            })
        );
    }

    #[test]
    fn sample_artifact_conversion_projects_metadata_and_cost() {
        let sample = SampleResult {
            sample_index: 0,
            passed: true,
            scores: vec![ScorerResult::new("fused_executor", 1.0)],
            actual_trajectory: vec!["executePlan".to_string()],
            response_text: Some("Done.".to_string()),
            duration_ms: 25,
            token_usage: token_usage(11, 7),
            error: None,
            retry_count: 0,
            thread_id: Some("thread-1".to_string()),
            trace: None,
            metadata: Some(json!({
                "plannedActions": [{
                    "type": "price_change",
                    "payload": { "scopeId": "s1" }
                }],
                "resolvedActions": [{
                    "type": "price_change",
                    "payload": {
                        "changeType": "absolute",
                        "value": 10.0
                    }
                }]
            })),
            latency: None,
        };
        let model_invocations = vec![ModelInvocation {
            agent: "executor".to_string(),
            provider: Some("anthropic".to_string()),
            model: "claude-haiku-4-5".to_string(),
            input_tokens: 11,
            output_tokens: 7,
            cached_tokens: 1,
            cache_creation_tokens: 0,
            estimated_cost_usd: Some(0.25),
        }];

        let artifact = SampleArtifact::from_framework_result(&sample, model_invocations);

        assert_eq!(artifact.aggregate_score, 1.0);
        assert_eq!(artifact.response_text.as_deref(), Some("Done."));
        assert_eq!(artifact.resolved_actions.len(), 1);
        assert_eq!(artifact.resolved_actions[0].action_type, "price_change");
        assert_eq!(
            artifact.resolved_actions[0].payload["value"],
            serde_json::json!(10.0)
        );
        assert_eq!(artifact.planned_actions.len(), 1);
        assert_eq!(artifact.planned_actions[0].action_type, "price_change");
        assert_eq!(
            artifact.planned_actions[0].payload["scopeId"],
            serde_json::json!("s1")
        );
        assert_eq!(
            artifact.cost.as_ref().and_then(|cost| cost.llm_cost_usd),
            Some(0.25)
        );
        assert_eq!(
            artifact.cost.as_ref().and_then(|cost| cost.total_cost_usd),
            Some(0.25)
        );
        assert_eq!(artifact.thread_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn summary_latency_does_not_fabricate_percentiles() {
        let latency = agent_fw_core::LatencySummary::zero();
        let summary_latency = SummaryLatency::from_core(&latency);

        assert_eq!(
            summary_latency,
            SummaryLatency {
                p50_ms: None,
                p95_ms: None,
                p99_ms: None,
                min_ms: None,
                max_ms: None,
            }
        );
    }
}
