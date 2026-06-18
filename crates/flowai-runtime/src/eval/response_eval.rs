//! Harness-owned final-response scoring.
//!
//! The generic `agent-fw-eval` crate transports the response text and scorer
//! payload. This module owns Flow AI's interpretation of that payload.

use std::collections::{BTreeMap, HashSet};

use agent_fw_eval::{EvalScorer, EvalTestCase, RawSampleOutput, ScoredSample};
use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

/// Canonical final-response scorer name emitted in result artifacts.
pub const SCORER_FINAL_RESPONSE: &str = "final_response";

/// Default top-level weight used when a case authors `finalResponse` and the
/// eval run did not provide explicit score weights.
pub const DEFAULT_FINAL_RESPONSE_WEIGHT: f64 = 0.5;

/// Extra payload key used to pass async judge verdicts into the pure scorer.
pub const FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY: &str = "finalResponseJudgeVerdicts";

const VERDICT_SCHEMA_VERSION: u32 = 1;
const JUDGE_RUN_SCHEMA_VERSION: u32 = 1;

fn default_weight() -> f64 {
    1.0
}

fn default_pass_threshold() -> f64 {
    1.0
}

fn default_case_sensitive() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum FinalResponseEvalError {
    #[error("final_response eval spec must include at least one scorer")]
    Empty,
    #[error("final_response pass_threshold must be finite and between 0 and 1")]
    InvalidPassThreshold,
    #[error("response scorer id must be non-empty")]
    EmptyId,
    #[error("response scorer id '{0}' is duplicated")]
    DuplicateId(String),
    #[error("response scorer '{id}' weight must be positive and finite")]
    InvalidWeight { id: String },
    #[error("judge response scorer '{id}' instructions must be non-empty")]
    MissingJudgeInstructions { id: String },
    #[error("judge response scorer '{id}' rubric must define exactly 0 and 1")]
    InvalidJudgeRubric { id: String },
    #[error("judge response scorer '{id}' rubric descriptions must be non-empty")]
    EmptyJudgeRubricDescription { id: String },
    #[error("exact response scorer '{id}' expected text must be provided")]
    MissingExactExpected { id: String },
    #[error("contains response scorer '{id}' text must be non-empty")]
    MissingContainsText { id: String },
    #[error("regex response scorer '{id}' pattern must be non-empty")]
    MissingRegexPattern { id: String },
    #[error("invalid regex response scorer pattern for '{id}': {source}")]
    InvalidRegex {
        id: String,
        #[source]
        source: regex_lite::Error,
    },
    #[error(
        "judge prompt can only be built for judge response scorers, got {method:?} for '{id}'"
    )]
    NonJudgeScorer {
        id: String,
        method: ResponseScorerMethod,
    },
    #[error("invalid final_response eval spec: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalResponseEvalSpec {
    pub scorers: Vec<ResponseScorerSpec>,
    #[serde(default = "default_pass_threshold")]
    pub pass_threshold: f64,
}

impl FinalResponseEvalSpec {
    pub fn from_value(value: &JsonValue) -> Result<Self, FinalResponseEvalError> {
        let spec: Self = serde_json::from_value(value.clone())?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), FinalResponseEvalError> {
        if self.scorers.is_empty() {
            return Err(FinalResponseEvalError::Empty);
        }
        if !self.pass_threshold.is_finite()
            || self.pass_threshold < 0.0
            || self.pass_threshold > 1.0
        {
            return Err(FinalResponseEvalError::InvalidPassThreshold);
        }

        let mut ids = HashSet::new();
        for scorer in &self.scorers {
            scorer.validate()?;
            if !ids.insert(scorer.id.clone()) {
                return Err(FinalResponseEvalError::DuplicateId(scorer.id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseScorerSpec {
    pub id: String,
    pub method: ResponseScorerMethod,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub required: bool,
    pub instructions: Option<String>,
    pub reference_response: Option<String>,
    pub rubric: Option<BTreeMap<i32, String>>,
    pub context: Option<JsonValue>,
    pub expected: Option<String>,
    pub text: Option<String>,
    pub pattern: Option<String>,
    #[serde(default = "default_case_sensitive")]
    pub case_sensitive: bool,
}

impl ResponseScorerSpec {
    fn validate(&self) -> Result<(), FinalResponseEvalError> {
        if self.id.trim().is_empty() {
            return Err(FinalResponseEvalError::EmptyId);
        }
        if self.weight <= 0.0 || !self.weight.is_finite() {
            return Err(FinalResponseEvalError::InvalidWeight {
                id: self.id.clone(),
            });
        }

        match self.method {
            ResponseScorerMethod::Judge => {
                if self
                    .instructions
                    .as_deref()
                    .is_none_or(|instructions| instructions.trim().is_empty())
                {
                    return Err(FinalResponseEvalError::MissingJudgeInstructions {
                        id: self.id.clone(),
                    });
                }
                if let Some(rubric) = &self.rubric {
                    if rubric.keys().copied().collect::<HashSet<_>>() != HashSet::from([0, 1]) {
                        return Err(FinalResponseEvalError::InvalidJudgeRubric {
                            id: self.id.clone(),
                        });
                    }
                    if rubric.values().any(|value| value.trim().is_empty()) {
                        return Err(FinalResponseEvalError::EmptyJudgeRubricDescription {
                            id: self.id.clone(),
                        });
                    }
                }
            }
            ResponseScorerMethod::Exact => {
                if self.expected.is_none() {
                    return Err(FinalResponseEvalError::MissingExactExpected {
                        id: self.id.clone(),
                    });
                }
            }
            ResponseScorerMethod::Contains => {
                if self.text.as_deref().is_none_or(str::is_empty) {
                    return Err(FinalResponseEvalError::MissingContainsText {
                        id: self.id.clone(),
                    });
                }
            }
            ResponseScorerMethod::Regex => {
                let Some(pattern) = self
                    .pattern
                    .as_deref()
                    .filter(|pattern| !pattern.is_empty())
                else {
                    return Err(FinalResponseEvalError::MissingRegexPattern {
                        id: self.id.clone(),
                    });
                };
                Regex::new(pattern).map_err(|source| FinalResponseEvalError::InvalidRegex {
                    id: self.id.clone(),
                    source,
                })?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseScorerMethod {
    Judge,
    Exact,
    Contains,
    Regex,
}

#[derive(Debug, thiserror::Error)]
pub enum JudgeResponseVerdictError {
    #[error("judge verdict must be valid JSON matching the expected schema: {0}")]
    Decode(#[from] serde_json::Error),
    #[error(
        "final response judge verdicts extra field must be an object keyed by response scorer id"
    )]
    InvalidVerdictContainer,
    #[error("invalid judge verdict for response scorer '{id}': {source}")]
    InvalidVerdict {
        id: String,
        #[source]
        source: Box<JudgeResponseVerdictError>,
    },
    #[error("judge verdict selected_rubric_score must be 0 or 1")]
    InvalidRubricScore,
    #[error("judge verdict passed must be true exactly when selected_rubric_score is 1")]
    InconsistentPassed,
    #[error("judge verdict reason must be non-empty")]
    EmptyReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeResponseVerdict {
    pub passed: bool,
    pub selected_rubric_score: u8,
    pub reason: String,
}

impl JudgeResponseVerdict {
    pub fn from_json_str(raw: &str) -> Result<Self, JudgeResponseVerdictError> {
        let verdict: Self = serde_json::from_str(raw)?;
        verdict.validate()
    }

    pub fn from_json_value(value: JsonValue) -> Result<Self, JudgeResponseVerdictError> {
        let verdict: Self = serde_json::from_value(value)?;
        verdict.validate()
    }

    fn validate(mut self) -> Result<Self, JudgeResponseVerdictError> {
        if self.selected_rubric_score > 1 {
            return Err(JudgeResponseVerdictError::InvalidRubricScore);
        }
        if self.passed != (self.selected_rubric_score == 1) {
            return Err(JudgeResponseVerdictError::InconsistentPassed);
        }
        self.reason = self.reason.trim().to_string();
        if self.reason.is_empty() {
            return Err(JudgeResponseVerdictError::EmptyReason);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgeRunMetadata {
    pub schema_version: u32,
    pub provider: String,
    pub model: String,
    pub prompt_sha256: String,
    pub context_sha256: String,
}

impl JudgeRunMetadata {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        prompt: &str,
        context: &JsonValue,
    ) -> Self {
        Self {
            schema_version: JUDGE_RUN_SCHEMA_VERSION,
            provider: provider.into(),
            model: model.into(),
            prompt_sha256: sha256_hex(prompt.as_bytes()),
            context_sha256: stable_json_sha256(context),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgeTrace {
    pub prompt: String,
    pub response: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgeResponseScoringData {
    pub verdict: JudgeResponseVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_run: Option<JudgeRunMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<JudgeResponseErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_trace: Option<JudgeTrace>,
}

impl JudgeResponseScoringData {
    pub fn new(verdict: JudgeResponseVerdict) -> Self {
        Self {
            verdict,
            judge_run: None,
            error_kind: None,
            judge_trace: None,
        }
    }

    pub fn with_judge_run(mut self, judge_run: JudgeRunMetadata) -> Self {
        self.judge_run = Some(judge_run);
        self
    }

    pub fn with_error_kind(mut self, error_kind: JudgeResponseErrorKind) -> Self {
        self.error_kind = Some(error_kind);
        self
    }

    pub fn with_judge_trace(mut self, judge_trace: JudgeTrace) -> Self {
        self.judge_trace = Some(judge_trace);
        self
    }

    fn from_json_value(value: JsonValue) -> Result<Self, JudgeResponseVerdictError> {
        let looks_like_wrapped = value
            .as_object()
            .is_some_and(|object| object.contains_key("verdict"));
        if looks_like_wrapped {
            let mut data: Self = serde_json::from_value(value)?;
            data.verdict = data.verdict.validate()?;
            Ok(data)
        } else {
            JudgeResponseVerdict::from_json_value(value).map(Self::new)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeResponseErrorKind {
    JudgeNoText,
    JudgeInvalidJson,
    JudgeInvalidSchema,
    JudgeProviderUnavailable,
    JudgeExecutionFailed,
    JudgePromptFailed,
}

pub fn judge_context_for_hash(
    scorer: &ResponseScorerSpec,
    response_text: &str,
    run_context: Option<&JsonValue>,
) -> JsonValue {
    serde_json::json!({
        "scorerId": scorer.id,
        "instructions": scorer.instructions,
        "referenceResponse": scorer.reference_response,
        "rubric": scorer.rubric.clone().unwrap_or_else(default_judge_rubric),
        "context": scorer.context,
        "runtimeContext": run_context,
        "finalResponse": response_text,
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn stable_json_sha256(value: &JsonValue) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

fn canonical_json(value: &JsonValue) -> String {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            serde_json::to_string(value).expect("serde_json value serializes")
        }
        JsonValue::Array(items) => {
            let items = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{items}]")
        }
        JsonValue::Object(object) => {
            let items = object
                .iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).expect("JSON object key serializes");
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{items}}}")
        }
    }
}

pub fn default_judge_rubric() -> BTreeMap<i32, String> {
    BTreeMap::from([
        (
            0,
            "The final response does not satisfy this scorer's instructions.".to_string(),
        ),
        (
            1,
            "The final response satisfies this scorer's instructions.".to_string(),
        ),
    ])
}

pub fn final_response_judge_verdicts_extra(
    verdicts: &BTreeMap<String, JudgeResponseVerdict>,
) -> JsonValue {
    let results = verdicts
        .iter()
        .map(|(id, verdict)| (id.clone(), JudgeResponseScoringData::new(verdict.clone())))
        .collect();
    final_response_judge_results_extra(&results)
}

pub fn final_response_judge_results_extra(
    results: &BTreeMap<String, JudgeResponseScoringData>,
) -> JsonValue {
    serde_json::json!({
        FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY: results,
    })
}

pub fn final_response_judge_verdicts_from_extra(
    extra: Option<&JsonValue>,
) -> Result<BTreeMap<String, JudgeResponseVerdict>, JudgeResponseVerdictError> {
    final_response_judge_results_from_extra(extra).map(|results| {
        results
            .into_iter()
            .map(|(id, result)| (id, result.verdict))
            .collect()
    })
}

pub fn final_response_judge_results_from_extra(
    extra: Option<&JsonValue>,
) -> Result<BTreeMap<String, JudgeResponseScoringData>, JudgeResponseVerdictError> {
    let Some(JsonValue::Object(extra)) = extra else {
        return Ok(BTreeMap::new());
    };
    let Some(value) = extra.get(FINAL_RESPONSE_JUDGE_VERDICTS_EXTRA_KEY) else {
        return Ok(BTreeMap::new());
    };
    let JsonValue::Object(values) = value else {
        return Err(JudgeResponseVerdictError::InvalidVerdictContainer);
    };

    values
        .iter()
        .map(|(id, value)| {
            JudgeResponseScoringData::from_json_value(value.clone())
                .map(|result| (id.clone(), result))
                .map_err(|source| JudgeResponseVerdictError::InvalidVerdict {
                    id: id.clone(),
                    source: Box::new(source),
                })
        })
        .collect()
}

pub fn build_judge_prompt(
    scorer: &ResponseScorerSpec,
    response_text: &str,
    run_context: Option<&JsonValue>,
) -> Result<String, FinalResponseEvalError> {
    if scorer.method != ResponseScorerMethod::Judge {
        return Err(FinalResponseEvalError::NonJudgeScorer {
            id: scorer.id.clone(),
            method: scorer.method,
        });
    }

    let instructions = scorer.instructions.as_deref().unwrap_or_default().trim();
    let rubric = scorer.rubric.clone().unwrap_or_else(default_judge_rubric);

    let mut prompt = String::new();
    prompt.push_str("You are evaluating one final response from a coordinator agent.\n");
    prompt.push_str("Judge only the scorer described below. Return only JSON.\n\n");
    prompt.push_str("Scorer id:\n");
    prompt.push_str(&scorer.id);
    prompt.push_str("\n\nInstructions:\n");
    prompt.push_str(instructions);
    prompt.push_str("\n\nRubric:\n");
    for (score, description) in rubric {
        prompt.push_str("Score ");
        prompt.push_str(&score.to_string());
        prompt.push_str(": ");
        prompt.push_str(description.trim());
        prompt.push('\n');
    }

    if let Some(reference_response) = scorer.reference_response.as_deref() {
        prompt.push_str("\nReference response:\n");
        prompt.push_str(reference_response.trim());
        prompt.push('\n');
    }

    if let Some(context) = &scorer.context {
        prompt.push_str("\nUser-provided eval context:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string()),
        );
        prompt.push('\n');
    }

    if let Some(context) = run_context {
        prompt.push_str("\nRuntime context:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string()),
        );
        prompt.push('\n');
    }

    prompt.push_str("\nFinal response:\n");
    prompt.push_str(response_text);
    prompt.push_str("\n\nReturn exactly this JSON shape:\n");
    prompt.push_str(
        r#"{
  "passed": true,
  "selected_rubric_score": 1,
  "reason": "Short explanation grounded in the final response."
}"#,
    );
    Ok(prompt)
}

#[derive(Debug, Clone, Default)]
pub struct FinalResponseScorer {
    spec: Option<FinalResponseEvalSpec>,
}

impl FinalResponseScorer {
    pub fn with_spec(spec: FinalResponseEvalSpec) -> Self {
        Self { spec: Some(spec) }
    }
}

impl EvalScorer for FinalResponseScorer {
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
        let spec = match self.spec.as_ref() {
            Some(spec) => spec.clone(),
            None => match test_case.final_response.as_ref() {
                Some(value) => match FinalResponseEvalSpec::from_value(value) {
                    Ok(spec) => spec,
                    Err(error) => {
                        return invalid_final_response_score(error.to_string());
                    }
                },
                None => {
                    return ScoredSample::leaf_with_details(
                        SCORER_FINAL_RESPONSE,
                        1.0,
                        serde_json::json!({
                            "configured": false,
                            "reason": "No finalResponse eval was configured for this test case.",
                        }),
                    );
                }
            },
        };

        let Some(response_text) = output.response_text.as_deref() else {
            return ScoredSample::leaf_with_details(
                SCORER_FINAL_RESPONSE,
                0.0,
                serde_json::json!({
                    "configured": true,
                    "passed": false,
                    "reason": "No final response text was captured for this sample.",
                }),
            );
        };

        let judge_results = match final_response_judge_results_from_extra(output.extra.as_ref()) {
            Ok(verdicts) => verdicts,
            Err(error) => {
                return invalid_final_response_score(format!(
                    "Invalid final response judge verdicts: {error}"
                ));
            }
        };

        let result = evaluate_final_response(&spec, response_text, &judge_results);
        ScoredSample::leaf_with_details(
            SCORER_FINAL_RESPONSE,
            result.effective_score,
            serde_json::to_value(result).unwrap_or(JsonValue::Null),
        )
    }

    fn name(&self) -> &str {
        SCORER_FINAL_RESPONSE
    }
}

fn invalid_final_response_score(reason: String) -> ScoredSample {
    ScoredSample::leaf_with_details(
        SCORER_FINAL_RESPONSE,
        0.0,
        serde_json::json!({
            "configured": true,
            "passed": false,
            "reason": reason,
        }),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalResponseEvalResult {
    pub verdict_schema_version: u32,
    pub passed: bool,
    pub score: f64,
    pub effective_score: f64,
    pub pass_threshold: f64,
    pub passed_weight: f64,
    pub total_weight: f64,
    pub required_failed: Vec<String>,
    pub response_scorers: Vec<ResponseScorerResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseScorerResult {
    pub id: String,
    pub method: ResponseScorerMethod,
    pub passed: bool,
    pub score: f64,
    pub required: bool,
    pub weight: f64,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

fn evaluate_final_response(
    spec: &FinalResponseEvalSpec,
    response_text: &str,
    judge_results: &BTreeMap<String, JudgeResponseScoringData>,
) -> FinalResponseEvalResult {
    let mut total_weight = 0.0;
    let mut passed_weight = 0.0;
    let mut required_failed = Vec::new();
    let mut response_scorers = Vec::with_capacity(spec.scorers.len());

    for scorer in &spec.scorers {
        let result = score_response_scorer(scorer, response_text, judge_results);
        total_weight += scorer.weight;
        if result.passed {
            passed_weight += scorer.weight;
        } else if scorer.required {
            required_failed.push(scorer.id.clone());
        }
        response_scorers.push(result);
    }

    let score = if total_weight > 0.0 {
        passed_weight / total_weight
    } else {
        0.0
    };
    let passed = score >= spec.pass_threshold && required_failed.is_empty();
    let effective_score = if passed { score } else { 0.0 };

    FinalResponseEvalResult {
        verdict_schema_version: VERDICT_SCHEMA_VERSION,
        passed,
        score,
        effective_score,
        pass_threshold: spec.pass_threshold,
        passed_weight,
        total_weight,
        required_failed,
        response_scorers,
    }
}

fn score_response_scorer(
    scorer: &ResponseScorerSpec,
    response_text: &str,
    judge_results: &BTreeMap<String, JudgeResponseScoringData>,
) -> ResponseScorerResult {
    match scorer.method {
        ResponseScorerMethod::Judge => match judge_results.get(&scorer.id) {
            Some(result) => {
                let mut details = serde_json::Map::new();
                details.insert(
                    "verdict".to_string(),
                    serde_json::to_value(&result.verdict).unwrap_or(JsonValue::Null),
                );
                if let Some(judge_run) = &result.judge_run {
                    details.insert(
                        "judgeRun".to_string(),
                        serde_json::to_value(judge_run).unwrap_or(JsonValue::Null),
                    );
                }
                if let Some(error_kind) = result.error_kind {
                    details.insert(
                        "errorKind".to_string(),
                        serde_json::to_value(error_kind).unwrap_or(JsonValue::Null),
                    );
                }
                if let Some(judge_trace) = &result.judge_trace {
                    details.insert(
                        "judgeTrace".to_string(),
                        serde_json::to_value(judge_trace).unwrap_or(JsonValue::Null),
                    );
                }
                ResponseScorerResult {
                    id: scorer.id.clone(),
                    method: scorer.method,
                    passed: result.verdict.passed,
                    score: if result.verdict.passed { 1.0 } else { 0.0 },
                    required: scorer.required,
                    weight: scorer.weight,
                    reason: result.verdict.reason.clone(),
                    details: Some(JsonValue::Object(details)),
                }
            }
            None => ResponseScorerResult {
                id: scorer.id.clone(),
                method: scorer.method,
                passed: false,
                score: 0.0,
                required: scorer.required,
                weight: scorer.weight,
                reason: "Judge response scorer did not have a precomputed judge verdict."
                    .to_string(),
                details: Some(serde_json::json!({
                    "missingVerdict": true,
                })),
            },
        },
        ResponseScorerMethod::Exact => {
            let expected = scorer.expected.as_deref().unwrap_or_default();
            let passed = response_text == expected;
            binary_response_result(
                scorer,
                passed,
                if passed {
                    "The final response exactly matched the expected text."
                } else {
                    "The final response did not exactly match the expected text."
                },
                serde_json::json!({
                    "expected": expected,
                }),
            )
        }
        ResponseScorerMethod::Contains => {
            let text = scorer.text.as_deref().unwrap_or_default();
            let passed = if scorer.case_sensitive {
                response_text.contains(text)
            } else {
                response_text.to_lowercase().contains(&text.to_lowercase())
            };
            binary_response_result(
                scorer,
                passed,
                if passed {
                    "The final response contained the required text."
                } else {
                    "The final response did not contain the required text."
                },
                serde_json::json!({
                    "text": text,
                    "caseSensitive": scorer.case_sensitive,
                }),
            )
        }
        ResponseScorerMethod::Regex => {
            let pattern = scorer.pattern.as_deref().unwrap_or_default();
            let regex = match Regex::new(pattern) {
                Ok(regex) => regex,
                Err(error) => {
                    return ResponseScorerResult {
                        id: scorer.id.clone(),
                        method: scorer.method,
                        passed: false,
                        score: 0.0,
                        required: scorer.required,
                        weight: scorer.weight,
                        reason: format!("Invalid regex pattern: {error}"),
                        details: Some(serde_json::json!({
                            "pattern": pattern,
                        })),
                    };
                }
            };
            let passed = regex.is_match(response_text);
            binary_response_result(
                scorer,
                passed,
                if passed {
                    "The final response matched the required regex pattern."
                } else {
                    "The final response did not match the required regex pattern."
                },
                serde_json::json!({
                    "pattern": pattern,
                }),
            )
        }
    }
}

fn binary_response_result(
    scorer: &ResponseScorerSpec,
    passed: bool,
    reason: &str,
    details: JsonValue,
) -> ResponseScorerResult {
    ResponseScorerResult {
        id: scorer.id.clone(),
        method: scorer.method,
        passed,
        score: if passed { 1.0 } else { 0.0 },
        required: scorer.required,
        weight: scorer.weight,
        reason: reason.to_string(),
        details: Some(details),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::TestCaseId;
    use agent_fw_eval::TrajectoryMode;

    fn test_case(final_response: JsonValue) -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked("tc-response"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec![],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: Some(final_response),
            source_thread_id: None,
        }
    }

    #[test]
    fn final_response_scores_weighted_deterministic_scorers() {
        let scorer = FinalResponseScorer::default();
        let tc = test_case(serde_json::json!({
            "scorers": [
                {
                    "id": "mentions_email",
                    "method": "contains",
                    "text": "jane@example.com",
                    "weight": 2.0
                },
                {
                    "id": "mentions_ticket",
                    "method": "regex",
                    "pattern": "TICKET-[0-9]+",
                    "weight": 1.0
                }
            ],
            "passThreshold": 0.7
        }));
        let output = RawSampleOutput::new(vec![])
            .with_response_text("Updated billing contact to jane@example.com.");

        let scored = scorer.score(&tc, &output);

        assert_eq!(scored.aggregate, 0.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("final response details");
        assert_eq!(detail["score"], serde_json::json!(2.0 / 3.0));
        assert_eq!(detail["passed"], serde_json::json!(false));
    }

    #[test]
    fn final_response_required_failure_gates_effective_score() {
        let scorer = FinalResponseScorer::default();
        let tc = test_case(serde_json::json!({
            "scorers": [
                {
                    "id": "mentions_success",
                    "method": "contains",
                    "text": "updated",
                    "weight": 2.0
                },
                {
                    "id": "no_refund_claim",
                    "method": "exact",
                    "expected": "Updated without issuing a refund.",
                    "required": true,
                    "weight": 1.0
                }
            ],
            "passThreshold": 0.5
        }));
        let output =
            RawSampleOutput::new(vec![]).with_response_text("The billing contact was updated.");

        let scored = scorer.score(&tc, &output);

        assert_eq!(scored.aggregate, 0.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("final response details");
        assert_eq!(detail["score"], serde_json::json!(2.0 / 3.0));
        assert_eq!(
            detail["requiredFailed"],
            serde_json::json!(["no_refund_claim"])
        );
    }

    #[test]
    fn final_response_scores_precomputed_judge_verdict() {
        let scorer = FinalResponseScorer::default();
        let tc = test_case(serde_json::json!({
            "scorers": [
                {
                    "id": "similar_to_reference",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "Billing was updated for Acme.",
                    "weight": 1.0
                }
            ],
            "passThreshold": 1.0
        }));
        let judge_run = JudgeRunMetadata {
            schema_version: 1,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            prompt_sha256: "a".repeat(64),
            context_sha256: "b".repeat(64),
        };
        let extra = final_response_judge_results_extra(&BTreeMap::from([(
            "similar_to_reference".to_string(),
            JudgeResponseScoringData::new(JudgeResponseVerdict {
                passed: true,
                selected_rubric_score: 1,
                reason: "The response states the billing update for Acme.".to_string(),
            })
            .with_judge_run(judge_run),
        )]));
        let output = RawSampleOutput::with_extra(vec![], extra)
            .with_response_text("I updated billing for Acme.");

        let scored = scorer.score(&tc, &output);

        assert_eq!(scored.aggregate, 1.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("final response details");
        assert_eq!(detail["passed"], serde_json::json!(true));
        assert_eq!(
            detail["responseScorers"][0]["score"],
            serde_json::json!(1.0)
        );
        assert_eq!(
            detail["responseScorers"][0]["details"]["verdict"]["selected_rubric_score"],
            serde_json::json!(1)
        );
        assert_eq!(
            detail["responseScorers"][0]["details"]["judgeRun"]["provider"],
            serde_json::json!("anthropic")
        );
        assert_eq!(
            detail["responseScorers"][0]["details"]["judgeRun"]["promptSha256"],
            serde_json::json!("a".repeat(64))
        );
    }

    #[test]
    fn final_response_surfaces_judge_trace_when_present() {
        let scorer = FinalResponseScorer::default();
        let tc = test_case(serde_json::json!({
            "scorers": [
                {
                    "id": "similar_to_reference",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "Billing was updated for Acme."
                }
            ]
        }));
        let extra = final_response_judge_results_extra(&BTreeMap::from([(
            "similar_to_reference".to_string(),
            JudgeResponseScoringData::new(JudgeResponseVerdict {
                passed: true,
                selected_rubric_score: 1,
                reason: "The response states the billing update for Acme.".to_string(),
            })
            .with_judge_trace(JudgeTrace {
                prompt: "rendered judge prompt".to_string(),
                response: r#"{"passed":true}"#.to_string(),
            }),
        )]));
        let output = RawSampleOutput::with_extra(vec![], extra)
            .with_response_text("I updated billing for Acme.");

        let scored = scorer.score(&tc, &output);

        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("final response details");
        assert_eq!(
            detail["responseScorers"][0]["details"]["judgeTrace"],
            serde_json::json!({
                "prompt": "rendered judge prompt",
                "response": r#"{"passed":true}"#,
            })
        );
    }

    #[test]
    fn final_response_surfaces_judge_error_kind_in_details() {
        let scorer = FinalResponseScorer::default();
        let tc = test_case(serde_json::json!({
            "scorers": [
                {
                    "id": "similar_to_reference",
                    "method": "judge",
                    "instructions": "Pass when the response is similar to the reference.",
                    "referenceResponse": "Billing was updated for Acme.",
                    "weight": 1.0
                }
            ],
            "passThreshold": 1.0
        }));
        let extra = final_response_judge_results_extra(&BTreeMap::from([(
            "similar_to_reference".to_string(),
            JudgeResponseScoringData::new(JudgeResponseVerdict {
                passed: false,
                selected_rubric_score: 0,
                reason: "Judge returned invalid verdict JSON: expected value at line 1 column 1"
                    .to_string(),
            })
            .with_error_kind(JudgeResponseErrorKind::JudgeInvalidJson),
        )]));
        let output =
            RawSampleOutput::with_extra(vec![], extra).with_response_text("I updated billing.");

        let scored = scorer.score(&tc, &output);

        assert_eq!(scored.aggregate, 0.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("final response details");
        assert_eq!(
            detail["responseScorers"][0]["details"]["errorKind"],
            serde_json::json!("judge_invalid_json")
        );
    }

    #[test]
    fn judge_run_hashes_are_stable_and_full_sha256_hex() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let left = serde_json::json!({
            "b": [2, 1],
            "a": {
                "z": true,
                "y": null
            }
        });
        let right = serde_json::json!({
            "a": {
                "y": null,
                "z": true
            },
            "b": [2, 1]
        });

        let hash = stable_json_sha256(&left);
        assert_eq!(hash, stable_json_sha256(&right));
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn judge_prompt_includes_single_scorer_contract() {
        let spec = FinalResponseEvalSpec::from_value(&serde_json::json!({
            "scorers": [
                {
                    "id": "similar_to_reference",
                    "method": "judge",
                    "instructions": "Pass when the response is semantically similar to the reference answer.",
                    "referenceResponse": "Billing was updated for Acme.",
                    "rubric": {
                        "0": "The response misses the billing update.",
                        "1": "The response includes the billing update."
                    },
                    "context": {
                        "customer": "Acme"
                    }
                }
            ]
        }))
        .expect("valid spec");

        let prompt = build_judge_prompt(
            &spec.scorers[0],
            "I updated billing for Acme.",
            Some(&serde_json::json!({"sampleId": "tc-1"})),
        )
        .expect("prompt");

        assert!(prompt.contains("Scorer id:\nsimilar_to_reference"));
        assert!(prompt.contains("Score 0: The response misses the billing update."));
        assert!(prompt.contains("Score 1: The response includes the billing update."));
        assert!(prompt.contains("Reference response:\nBilling was updated for Acme."));
        assert!(prompt.contains("\"customer\": \"Acme\""));
        assert!(prompt.contains("\"sampleId\": \"tc-1\""));
        assert!(prompt.contains("\"selected_rubric_score\": 1"));
    }

    #[test]
    fn judge_verdict_accepts_only_consistent_binary_json() {
        let verdict = JudgeResponseVerdict::from_json_str(
            r#"{"passed":true,"selected_rubric_score":1,"reason":"It matches."}"#,
        )
        .expect("valid verdict");

        assert!(verdict.passed);
        assert_eq!(verdict.selected_rubric_score, 1);
        assert_eq!(verdict.reason, "It matches.");

        assert!(matches!(
            JudgeResponseVerdict::from_json_str(
                r#"{"passed":false,"selected_rubric_score":1,"reason":"Mismatch."}"#
            ),
            Err(JudgeResponseVerdictError::InconsistentPassed)
        ));
        assert!(matches!(
            JudgeResponseVerdict::from_json_str(
                r#"{"passed":true,"selected_rubric_score":2,"reason":"Too high."}"#
            ),
            Err(JudgeResponseVerdictError::InvalidRubricScore)
        ));
        assert!(matches!(
            JudgeResponseVerdict::from_json_str(
                r#"{"passed":false,"selected_rubric_score":0,"reason":"   "}"#
            ),
            Err(JudgeResponseVerdictError::EmptyReason)
        ));
        assert!(matches!(
            JudgeResponseVerdict::from_json_str(
                r#"{"passed":true,"selected_rubric_score":1,"reason":"Ok","extra":[]}"#
            ),
            Err(JudgeResponseVerdictError::Decode(_))
        ));
    }
}
