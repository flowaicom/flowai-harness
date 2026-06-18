//! Harness-owned eval scorer presets and action matching.
//!
//! `agent-fw-eval` owns the generic scorer algebra. This module owns the
//! Flow AI harness composition on top of that substrate:
//!
//! - mode-to-scorer preset wiring
//! - action-oriented expected/actual matching over two sources (planned vs
//!   executed), with canonical component names and no compatibility aliases

use std::sync::Arc;

use agent_fw_core::NonEmpty;
use agent_fw_eval::scoring::{compute_f_beta, score_trajectory, trajectory_score_details};
use agent_fw_eval::{
    CompositeError, CompositeScorer, EvalMode, EvalScorer, EvalTestCase, RawSampleOutput,
    ScoreWeights, ScoreWeightsError, ScoredSample, WeightedChild,
};
use serde::{Deserialize, Serialize};

use agent_fw_eval::GroundTruth as FrameworkGroundTruth;

use super::action_ground_truth::{
    normalize_ground_truth, ActionGroundTruth, ActionPayloadMatchMode, ExpectedAction,
    GroundTruthNormalizationError,
};
use super::action_projection::{extract_planned_actions, extract_resolved_actions, ResolvedAction};
pub use super::response_eval::{
    FinalResponseEvalError, FinalResponseEvalSpec, FinalResponseScorer,
    DEFAULT_FINAL_RESPONSE_WEIGHT, SCORER_FINAL_RESPONSE,
};
use super::scorer_config::{HarnessScorerConfig, TrajectoryScorerConfig};
use super::trajectory_projection::project_trajectory;

const DEFAULT_BETA: f64 = 2.0;

/// Canonical trajectory scorer name emitted in result artifacts.
pub const SCORER_TRAJECTORY: &str = "trajectory";
/// Canonical planned-action scorer name (expected vs stored-plan actions).
pub const SCORER_PLANNED_ACTIONS: &str = "planned_actions";
/// Canonical executed-action scorer name (expected vs executed actions).
pub const SCORER_EXECUTED_ACTIONS: &str = "executed_actions";

/// Default trajectory weight for planner and sequential composite presets.
pub const DEFAULT_TRAJECTORY_WEIGHT: f64 = 0.5;
/// Default planned-action weight for the planner composite preset.
pub const DEFAULT_PLANNED_ACTION_WEIGHT: f64 = 0.5;
/// Default per-source action weight when sequential splits the action weight
/// across planned and executed components (half of `1 - trajectory`).
pub const DEFAULT_SEQUENTIAL_ACTION_WEIGHT: f64 = 0.25;
/// Default trajectory weight when executor evals author an expected trajectory.
pub const DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT: f64 = 0.2;
/// Default executed-action weight when executor evals also score trajectory.
pub const DEFAULT_EXECUTOR_ACTION_WEIGHT: f64 = 0.8;

/// Phase A helper preset for scoring trajectories only.
pub const PRESET_TRAJECTORY_ONLY: &str = "trajectory_only";
/// Planner preset: trajectory plus planned actions.
pub const PRESET_PLANNER: &str = "planner";
/// Executor preset: executed actions.
pub const PRESET_EXECUTOR: &str = "executor";
/// Sequential preset: trajectory plus planned and executed actions.
pub const PRESET_SEQUENTIAL: &str = "sequential";
/// Specialist preset: scorer components are inferred from authored expectations.
pub const PRESET_SPECIALIST: &str = "specialist";
/// Test-case-builder preset; currently sequential-like.
pub const PRESET_TEST_CASE_BUILDER: &str = "test_case_builder";

#[derive(Debug, thiserror::Error)]
pub enum PresetScorerError {
    #[error("weight validation failed: {0}")]
    Weights(#[from] ScoreWeightsError),
    #[error("invalid final_response eval: {0}")]
    FinalResponse(#[from] FinalResponseEvalError),
    #[error("invalid scorer_config: {0}")]
    ScorerConfig(#[from] serde_json::Error),
    #[error("composite scorer construction failed: {0}")]
    Composite(#[from] CompositeError),
    #[error("scorer '{scorer}' is not valid for preset '{preset}'")]
    ScorerNotAllowedForPreset {
        preset: &'static str,
        scorer: String,
    },
    #[error("specialist eval scoring requires at least one test case with scoreable expectations")]
    SpecialistNoScoreableExpectations,
    #[error(
        "specialist eval test case '{test_case_id}' has no scoreable expectations; author expectedTrajectory, finalResponse, plannedActions, or executedActions, or configure explicit scoreWeights"
    )]
    SpecialistTestCaseNoScoreableExpectations { test_case_id: String },
    #[error(
        "specialist scorer '{scorer}' requires test case '{test_case_id}' to author {requirement}"
    )]
    SpecialistScorerMissingExpectation {
        scorer: &'static str,
        test_case_id: String,
        requirement: &'static str,
    },
    #[error("scorer preset '{preset}' is not valid for eval mode '{mode}'")]
    PresetNotAllowedForMode { preset: String, mode: &'static str },
    #[error("invalid specialist action ground truth: {0}")]
    SpecialistGroundTruth(#[from] GroundTruthNormalizationError),
    #[error("unknown eval scorer preset '{0}'")]
    UnknownPreset(String),
}

fn canonical_scorer_name(name: &str) -> Option<&'static str> {
    match name {
        SCORER_TRAJECTORY => Some(SCORER_TRAJECTORY),
        SCORER_PLANNED_ACTIONS => Some(SCORER_PLANNED_ACTIONS),
        SCORER_EXECUTED_ACTIONS => Some(SCORER_EXECUTED_ACTIONS),
        SCORER_FINAL_RESPONSE => Some(SCORER_FINAL_RESPONSE),
        _ => None,
    }
}

/// Return canonical, normalized scorer weights for artifact metadata.
///
/// The harness is new, so there are no compatibility aliases: weight keys must
/// be one of the canonical scorer names. Duplicate keys are summed before
/// validation.
pub fn materialize_score_weights(weights: ScoreWeights) -> Result<ScoreWeights, PresetScorerError> {
    let mut trajectory = 0.0;
    let mut planned = 0.0;
    let mut executed = 0.0;
    let mut final_response = 0.0;

    for (name, weight) in weights.iter() {
        match canonical_scorer_name(name.as_str()) {
            Some(SCORER_TRAJECTORY) => trajectory += *weight,
            Some(SCORER_PLANNED_ACTIONS) => planned += *weight,
            Some(SCORER_EXECUTED_ACTIONS) => executed += *weight,
            Some(SCORER_FINAL_RESPONSE) => final_response += *weight,
            Some(_) => unreachable!("all canonical scorer names are handled"),
            None => return Err(ScoreWeightsError::UnknownScorer(name.clone()).into()),
        };
    }

    let mut canonical = Vec::new();
    if trajectory > 0.0 {
        canonical.push((SCORER_TRAJECTORY.to_string(), trajectory));
    }
    if planned > 0.0 {
        canonical.push((SCORER_PLANNED_ACTIONS.to_string(), planned));
    }
    if executed > 0.0 {
        canonical.push((SCORER_EXECUTED_ACTIONS.to_string(), executed));
    }
    if final_response > 0.0 {
        canonical.push((SCORER_FINAL_RESPONSE.to_string(), final_response));
    }

    let validated = ScoreWeights::new(canonical)?;
    Ok(ScoreWeights::new(validated.normalized())?)
}

fn materialize_score_weights_for_preset(
    preset: &'static str,
    weights: ScoreWeights,
) -> Result<ScoreWeights, PresetScorerError> {
    let materialized = materialize_score_weights(weights)?;
    for (name, _) in materialized.iter() {
        if !scorer_allowed_for_preset(preset, name) {
            return Err(PresetScorerError::ScorerNotAllowedForPreset {
                preset,
                scorer: name.clone(),
            });
        }
    }
    Ok(materialized)
}

fn scorer_allowed_for_preset(preset: &str, scorer_name: &str) -> bool {
    match preset {
        PRESET_TRAJECTORY_ONLY => scorer_name == SCORER_TRAJECTORY,
        PRESET_PLANNER => {
            matches!(
                scorer_name,
                SCORER_TRAJECTORY | SCORER_PLANNED_ACTIONS | SCORER_FINAL_RESPONSE
            )
        }
        PRESET_EXECUTOR => {
            matches!(
                scorer_name,
                SCORER_TRAJECTORY | SCORER_EXECUTED_ACTIONS | SCORER_FINAL_RESPONSE
            )
        }
        PRESET_SEQUENTIAL | PRESET_SPECIALIST | PRESET_TEST_CASE_BUILDER => {
            matches!(
                scorer_name,
                SCORER_TRAJECTORY
                    | SCORER_PLANNED_ACTIONS
                    | SCORER_EXECUTED_ACTIONS
                    | SCORER_FINAL_RESPONSE
            )
        }
        _ => false,
    }
}

/// Return the canonical default weights for a stable harness preset.
pub fn default_score_weights_for_preset(
    preset: &str,
) -> Result<Option<ScoreWeights>, PresetScorerError> {
    match preset {
        PRESET_TRAJECTORY_ONLY => Ok(Some(ScoreWeights::new(vec![(
            SCORER_TRAJECTORY.to_string(),
            1.0,
        )])?)),
        PRESET_PLANNER => Ok(Some(ScoreWeights::new(vec![
            (SCORER_TRAJECTORY.to_string(), DEFAULT_TRAJECTORY_WEIGHT),
            (
                SCORER_PLANNED_ACTIONS.to_string(),
                DEFAULT_PLANNED_ACTION_WEIGHT,
            ),
        ])?)),
        PRESET_EXECUTOR => Ok(Some(ScoreWeights::new(vec![(
            SCORER_EXECUTED_ACTIONS.to_string(),
            1.0,
        )])?)),
        PRESET_SEQUENTIAL | PRESET_TEST_CASE_BUILDER => Ok(Some(ScoreWeights::new(vec![
            (SCORER_TRAJECTORY.to_string(), DEFAULT_TRAJECTORY_WEIGHT),
            (
                SCORER_PLANNED_ACTIONS.to_string(),
                DEFAULT_SEQUENTIAL_ACTION_WEIGHT,
            ),
            (
                SCORER_EXECUTED_ACTIONS.to_string(),
                DEFAULT_SEQUENTIAL_ACTION_WEIGHT,
            ),
        ])?)),
        PRESET_SPECIALIST => Ok(None),
        unknown => Err(PresetScorerError::UnknownPreset(unknown.to_string())),
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct SpecialistExpectationSet {
    trajectory: bool,
    planned_actions: bool,
    executed_actions: bool,
    final_response: bool,
}

impl SpecialistExpectationSet {
    fn is_empty(self) -> bool {
        !self.trajectory && !self.planned_actions && !self.executed_actions && !self.final_response
    }

    fn merge(&mut self, other: SpecialistExpectationSet) {
        self.trajectory |= other.trajectory;
        self.planned_actions |= other.planned_actions;
        self.executed_actions |= other.executed_actions;
        self.final_response |= other.final_response;
    }

    fn score_weight_entries(self) -> Vec<(String, f64)> {
        let mut entries = Vec::new();
        if self.trajectory {
            entries.push((SCORER_TRAJECTORY.to_string(), 1.0));
        }
        if self.planned_actions {
            entries.push((SCORER_PLANNED_ACTIONS.to_string(), 1.0));
        }
        if self.executed_actions {
            entries.push((SCORER_EXECUTED_ACTIONS.to_string(), 1.0));
        }
        if self.final_response {
            entries.push((SCORER_FINAL_RESPONSE.to_string(), 1.0));
        }
        entries
    }
}

fn specialist_expectations_for_test_case(
    test_case: &EvalTestCase,
) -> Result<SpecialistExpectationSet, PresetScorerError> {
    let mut expectations = SpecialistExpectationSet {
        trajectory: !test_case.expected_trajectory.is_empty(),
        final_response: test_case.final_response.is_some(),
        ..SpecialistExpectationSet::default()
    };

    if let Some(ground_truth) = normalize_ground_truth(test_case.ground_truth.as_ref())? {
        expectations.planned_actions = !ground_truth.planned.is_empty();
        expectations.executed_actions = !ground_truth.executed.is_empty();
    }

    Ok(expectations)
}

fn specialist_default_score_weights_for_test_cases(
    test_cases: &[EvalTestCase],
) -> Result<ScoreWeights, PresetScorerError> {
    if test_cases.is_empty() {
        return Err(PresetScorerError::SpecialistNoScoreableExpectations);
    }

    let mut combined = SpecialistExpectationSet::default();
    for test_case in test_cases {
        let expectations = specialist_expectations_for_test_case(test_case)?;
        if expectations.is_empty() {
            return Err(
                PresetScorerError::SpecialistTestCaseNoScoreableExpectations {
                    test_case_id: test_case.id.as_str().to_string(),
                },
            );
        }
        combined.merge(expectations);
    }

    if combined.is_empty() {
        return Err(PresetScorerError::SpecialistNoScoreableExpectations);
    }

    Ok(ScoreWeights::new(combined.score_weight_entries())?)
}

/// Validate explicit specialist scorer weights against authored expectations.
///
/// Explicit trajectory/planned/executed weights are treated as user intent and
/// may score an empty expected set. `final_response` still requires a concrete
/// finalResponse spec because there is no meaningful empty-response scorer.
pub fn validate_specialist_explicit_score_weights(
    weights: &ScoreWeights,
    test_cases: &[EvalTestCase],
) -> Result<(), PresetScorerError> {
    let weights = materialize_score_weights_for_preset(PRESET_SPECIALIST, weights.clone())?;
    let requires_final_response = weights
        .iter()
        .any(|(name, _)| name.as_str() == SCORER_FINAL_RESPONSE);

    if !requires_final_response {
        return Ok(());
    }

    for test_case in test_cases {
        if test_case.final_response.is_none() {
            return Err(PresetScorerError::SpecialistScorerMissingExpectation {
                scorer: SCORER_FINAL_RESPONSE,
                test_case_id: test_case.id.as_str().to_string(),
                requirement: "finalResponse",
            });
        }
    }

    Ok(())
}

/// Return default weights for a preset in the context of concrete test cases.
///
/// Most presets have static defaults. Executor evals are the exception: action
/// correctness remains the default signal, but authored executor trajectories
/// opt the default scorer into a small trajectory component.
pub fn default_score_weights_for_preset_and_test_cases(
    preset: &str,
    test_cases: &[EvalTestCase],
) -> Result<Option<ScoreWeights>, PresetScorerError> {
    if preset == PRESET_SPECIALIST {
        return Ok(Some(specialist_default_score_weights_for_test_cases(
            test_cases,
        )?));
    }

    if preset == PRESET_EXECUTOR
        && test_cases
            .iter()
            .any(|test_case| !test_case.expected_trajectory.is_empty())
    {
        return Ok(Some(ScoreWeights::new(vec![
            (
                SCORER_EXECUTED_ACTIONS.to_string(),
                DEFAULT_EXECUTOR_ACTION_WEIGHT,
            ),
            (
                SCORER_TRAJECTORY.to_string(),
                DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT,
            ),
        ])?));
    }

    default_score_weights_for_preset(preset)
}

pub fn add_default_final_response_weight(
    weights: ScoreWeights,
) -> Result<ScoreWeights, PresetScorerError> {
    let mut entries: Vec<(String, f64)> = weights.iter().map(|(n, w)| (n.clone(), *w)).collect();
    if !entries
        .iter()
        .any(|(name, _)| name.as_str() == SCORER_FINAL_RESPONSE)
    {
        entries.push((
            SCORER_FINAL_RESPONSE.to_string(),
            DEFAULT_FINAL_RESPONSE_WEIGHT,
        ));
    }
    Ok(ScoreWeights::new(entries)?)
}

fn resolve_scorer_names(
    weights: &ScoreWeights,
) -> Result<NonEmpty<Arc<dyn EvalScorer>>, PresetScorerError> {
    let scorers: Vec<Arc<dyn EvalScorer>> = weights
        .iter()
        .map(
            |(name, _)| -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
                match canonical_scorer_name(name.as_str()) {
                    Some(SCORER_TRAJECTORY) => {
                        Ok(Arc::new(HarnessTrajectoryScorer::default()) as Arc<dyn EvalScorer>)
                    }
                    Some(SCORER_PLANNED_ACTIONS) => {
                        Ok(Arc::new(ActionScorer::planned()) as Arc<dyn EvalScorer>)
                    }
                    Some(SCORER_EXECUTED_ACTIONS) => {
                        Ok(Arc::new(ActionScorer::executed()) as Arc<dyn EvalScorer>)
                    }
                    Some(SCORER_FINAL_RESPONSE) => {
                        Ok(Arc::new(FinalResponseScorer::default()) as Arc<dyn EvalScorer>)
                    }
                    Some(_) => unreachable!("all canonical scorer names are handled"),
                    None => Err(ScoreWeightsError::UnknownScorer(name.clone()).into()),
                }
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NonEmpty::from_vec(scorers)
        .expect("ScoreWeights is NonEmpty, so resolved scorers are non-empty"))
}

pub fn scorer_for_mode(
    mode: EvalMode,
    weights: Option<ScoreWeights>,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    match mode {
        EvalMode::Planner => match weights {
            Some(weights) => {
                let weights = materialize_score_weights_for_preset(PRESET_PLANNER, weights)?;
                let scorers = resolve_scorer_names(&weights)?;
                Ok(Arc::new(weights.into_composite(scorers)?))
            }
            None => Ok(Arc::new(planner_default_scorer()?)),
        },
        EvalMode::Executor => match weights {
            Some(weights) => {
                let weights = materialize_score_weights_for_preset(PRESET_EXECUTOR, weights)?;
                let scorers = resolve_scorer_names(&weights)?;
                Ok(Arc::new(weights.into_composite(scorers)?))
            }
            None => Ok(Arc::new(ActionScorer::executed())),
        },
        EvalMode::Sequential | EvalMode::Specialist | EvalMode::TestCaseBuilder => match weights {
            Some(weights) => {
                let preset = match mode {
                    EvalMode::Specialist => PRESET_SPECIALIST,
                    EvalMode::TestCaseBuilder => PRESET_TEST_CASE_BUILDER,
                    _ => PRESET_SEQUENTIAL,
                };
                let weights = materialize_score_weights_for_preset(preset, weights)?;
                let scorers = resolve_scorer_names(&weights)?;
                Ok(Arc::new(weights.into_composite(scorers)?))
            }
            None => match mode {
                EvalMode::Specialist => Err(PresetScorerError::SpecialistNoScoreableExpectations),
                _ => Ok(Arc::new(sequential_default_scorer()?)),
            },
        },
    }
}

/// Build a scorer from a stable harness preset name.
///
/// This is the string-based factory intended for language facades. Compatibility
/// scorer weights use canonical names only; preset names are intentionally
/// stable and explicit.
pub fn scorer_for_preset(
    preset: &str,
    weights: Option<ScoreWeights>,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    match preset {
        PRESET_TRAJECTORY_ONLY => {
            if let Some(weights) = weights {
                let weights =
                    materialize_score_weights_for_preset(PRESET_TRAJECTORY_ONLY, weights)?;
                let scorers = resolve_scorer_names(&weights)?;
                Ok(Arc::new(weights.into_composite(scorers)?))
            } else {
                Ok(Arc::new(HarnessTrajectoryScorer::default()))
            }
        }
        PRESET_PLANNER => scorer_for_mode(EvalMode::Planner, weights),
        PRESET_EXECUTOR => scorer_for_mode(EvalMode::Executor, weights),
        PRESET_SEQUENTIAL => scorer_for_mode(EvalMode::Sequential, weights),
        PRESET_SPECIALIST => scorer_for_mode(EvalMode::Specialist, weights),
        PRESET_TEST_CASE_BUILDER => scorer_for_mode(EvalMode::TestCaseBuilder, weights),
        unknown => Err(PresetScorerError::UnknownPreset(unknown.to_string())),
    }
}

/// Map a preset name string to its canonical `'static` constant.
fn preset_static_name(preset: &str) -> Result<&'static str, PresetScorerError> {
    match preset {
        PRESET_TRAJECTORY_ONLY => Ok(PRESET_TRAJECTORY_ONLY),
        PRESET_PLANNER => Ok(PRESET_PLANNER),
        PRESET_EXECUTOR => Ok(PRESET_EXECUTOR),
        PRESET_SEQUENTIAL => Ok(PRESET_SEQUENTIAL),
        PRESET_SPECIALIST => Ok(PRESET_SPECIALIST),
        PRESET_TEST_CASE_BUILDER => Ok(PRESET_TEST_CASE_BUILDER),
        unknown => Err(PresetScorerError::UnknownPreset(unknown.to_string())),
    }
}

/// Build a leaf scorer for a canonical scorer name. When `captured` is `Some`,
/// action leaves are bound to those expected actions (normalized once for the
/// whole test case) instead of re-normalizing per sample.
fn leaf_for_name(
    name: &str,
    captured: Option<&ActionGroundTruth>,
    final_response: Option<&FinalResponseEvalSpec>,
    scorer_config: &HarnessScorerConfig,
) -> Arc<dyn EvalScorer> {
    match name {
        SCORER_PLANNED_ACTIONS => {
            let scorer = ActionScorer::planned();
            match captured {
                Some(gt) => Arc::new(scorer.with_expected(gt.planned.clone(), gt.payload_match)),
                None => Arc::new(scorer),
            }
        }
        SCORER_EXECUTED_ACTIONS => {
            let scorer = ActionScorer::executed();
            match captured {
                Some(gt) => Arc::new(scorer.with_expected(gt.executed.clone(), gt.payload_match)),
                None => Arc::new(scorer),
            }
        }
        SCORER_FINAL_RESPONSE => match final_response {
            Some(spec) => Arc::new(FinalResponseScorer::with_spec(spec.clone())),
            None => Arc::new(FinalResponseScorer::default()),
        },
        // SCORER_TRAJECTORY (only remaining canonical name).
        _ => Arc::new(HarnessTrajectoryScorer::with_config(
            scorer_config.trajectory.clone(),
        )),
    }
}

/// Build a scorer for a single test case, including only the action components
/// whose ground-truth bucket is populated.
///
/// The ground truth is normalized once here (not per sample): the resulting
/// expected actions are captured into the action leaves. Trajectory is always
/// kept (when the preset uses it). An action component is dropped when its
/// bucket is empty or absent, and the remaining weights are re-normalized — an
/// unpopulated bucket contributes no component rather than a freebie `1.0`. If
/// filtering removes every component (e.g. the action-only `executor` preset
/// whose ground truth is absent), the full preset weights are kept so the
/// composite is never empty; those leaves then score vacuously (`1.0`). That
/// only happens when an action preset is run without matching ground truth,
/// which is a misconfiguration rather than a normal path.
pub fn scorer_for_test_case(
    preset: &str,
    weights: ScoreWeights,
    ground_truth: Option<&FrameworkGroundTruth>,
    final_response: Option<&serde_json::Value>,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    scorer_for_test_case_with_config(preset, weights, ground_truth, final_response, None)
}

pub fn scorer_for_test_case_with_config(
    preset: &str,
    weights: ScoreWeights,
    ground_truth: Option<&FrameworkGroundTruth>,
    final_response: Option<&serde_json::Value>,
    scorer_config: Option<&serde_json::Value>,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    scorer_for_test_case_inner(
        preset,
        weights,
        true,
        ground_truth,
        final_response,
        scorer_config,
    )
}

pub fn scorer_for_eval_test_case_with_config(
    preset: &str,
    weights: ScoreWeights,
    test_case: &EvalTestCase,
    scorer_config: Option<&serde_json::Value>,
    keep_empty_trajectory: bool,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    scorer_for_test_case_inner(
        preset,
        weights,
        keep_empty_trajectory || !test_case.expected_trajectory.is_empty(),
        test_case.ground_truth.as_ref(),
        test_case.final_response.as_ref(),
        scorer_config,
    )
}

fn scorer_for_test_case_inner(
    preset: &str,
    weights: ScoreWeights,
    trajectory_present: bool,
    ground_truth: Option<&FrameworkGroundTruth>,
    final_response: Option<&serde_json::Value>,
    scorer_config: Option<&serde_json::Value>,
) -> Result<Arc<dyn EvalScorer>, PresetScorerError> {
    let weights = materialize_score_weights_for_preset(preset_static_name(preset)?, weights)?;
    let scorer_config = HarnessScorerConfig::from_value(scorer_config)?;
    let final_response = final_response
        .map(FinalResponseEvalSpec::from_value)
        .transpose()?;

    // Normalize once. On a normalization error, keep both action components and
    // leave them uncaptured so a leaf re-reads the ground truth and surfaces the
    // error rather than silently dropping the component.
    let (captured, planned_present, executed_present) = match normalize_ground_truth(ground_truth) {
        Ok(Some(gt)) => {
            let planned = !gt.planned.is_empty();
            let executed = !gt.executed.is_empty();
            (Some(gt), planned, executed)
        }
        Ok(None) => (None, false, false),
        Err(_) => (None, true, true),
    };

    let kept: Vec<(String, f64)> = weights
        .iter()
        .filter(|(name, _)| match name.as_str() {
            SCORER_TRAJECTORY => trajectory_present,
            SCORER_PLANNED_ACTIONS => planned_present,
            SCORER_EXECUTED_ACTIONS => executed_present,
            SCORER_FINAL_RESPONSE => final_response.is_some(),
            _ => true,
        })
        .map(|(name, weight)| (name.clone(), *weight))
        .collect();

    let kept = if kept.is_empty() {
        let fallback: Vec<(String, f64)> = weights
            .iter()
            .filter(|(name, _)| name.as_str() != SCORER_TRAJECTORY || trajectory_present)
            .map(|(n, w)| (n.clone(), *w))
            .collect();
        if fallback.is_empty() {
            weights.iter().map(|(n, w)| (n.clone(), *w)).collect()
        } else {
            fallback
        }
    } else {
        kept
    };

    let weights = ScoreWeights::new(kept)?;
    let weights = ScoreWeights::new(weights.normalized())?;
    let leaves: Vec<Arc<dyn EvalScorer>> = weights
        .iter()
        .map(|(name, _)| {
            leaf_for_name(
                name,
                captured.as_ref(),
                final_response.as_ref(),
                &scorer_config,
            )
        })
        .collect();
    let leaves =
        NonEmpty::from_vec(leaves).expect("kept weights are NonEmpty, so leaves are non-empty");
    Ok(Arc::new(weights.into_composite(leaves)?))
}

fn planner_default_scorer() -> Result<CompositeScorer, CompositeError> {
    let trajectory = WeightedChild::new(
        Arc::new(HarnessTrajectoryScorer::default()),
        DEFAULT_TRAJECTORY_WEIGHT,
    )?;
    let planned = WeightedChild::new(
        Arc::new(ActionScorer::planned()),
        DEFAULT_PLANNED_ACTION_WEIGHT,
    )?;
    CompositeScorer::new(NonEmpty::new(trajectory, vec![planned]))
}

fn sequential_default_scorer() -> Result<CompositeScorer, CompositeError> {
    let trajectory = WeightedChild::new(
        Arc::new(HarnessTrajectoryScorer::default()),
        DEFAULT_TRAJECTORY_WEIGHT,
    )?;
    let planned = WeightedChild::new(
        Arc::new(ActionScorer::planned()),
        DEFAULT_SEQUENTIAL_ACTION_WEIGHT,
    )?;
    let executed = WeightedChild::new(
        Arc::new(ActionScorer::executed()),
        DEFAULT_SEQUENTIAL_ACTION_WEIGHT,
    )?;
    CompositeScorer::new(NonEmpty::new(trajectory, vec![planned, executed]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Exact,
    Missing,
    Extra,
}

impl ActionStatus {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Exact)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionComparisonDetail {
    pub signature: String,
    pub status: ActionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonSummary {
    pub total_expected: usize,
    pub total_actual: usize,
    pub exact_count: usize,
    pub missing_count: usize,
    pub extra_count: usize,
    pub pass_rate: f64,
}

impl ComparisonSummary {
    pub fn all_exact(&self) -> bool {
        self.missing_count == 0 && self.extra_count == 0
    }

    pub fn all_signatures_matched(&self) -> bool {
        self.missing_count == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMatchResult {
    pub pass: bool,
    pub payload_match: ActionPayloadMatchMode,
    pub actions: Vec<ActionComparisonDetail>,
    pub summary: ComparisonSummary,
    pub issues: Vec<String>,
}

pub fn compare_actions(
    expected: &[ExpectedAction],
    actual: &[ResolvedAction],
) -> ActionMatchResult {
    compare_actions_with_payload_match(expected, actual, ActionPayloadMatchMode::Subset)
}

pub fn compare_actions_with_payload_match(
    expected: &[ExpectedAction],
    actual: &[ResolvedAction],
    payload_match: ActionPayloadMatchMode,
) -> ActionMatchResult {
    let mut details = Vec::new();
    let mut issues = Vec::new();

    let mut matched_expected = vec![false; expected.len()];
    let mut matched_actual = vec![false; actual.len()];

    let mut exact_count = 0;
    let mut missing_count = 0;
    let mut extra_count = 0;

    // Assign expected actions to actual actions with maximum bipartite matching.
    // A greedy first-match pass lets a broad expected payload (e.g. an empty
    // subset) consume an actual that only a more specific expected action could
    // match, undercounting valid assignments. Kuhn's augmenting-path algorithm
    // finds a maximum matching instead.
    let candidate_actuals: Vec<Vec<usize>> = expected
        .iter()
        .map(|expected_action| {
            actual
                .iter()
                .enumerate()
                .filter(|(_, actual_action)| {
                    expected_action.action_type == actual_action.action_type
                        && payload_matches(
                            &expected_action.payload,
                            &actual_action.payload,
                            payload_match,
                        )
                })
                .map(|(actual_index, _)| actual_index)
                .collect()
        })
        .collect();

    let mut actual_assignment: Vec<Option<usize>> = vec![None; actual.len()];
    for (expected_index, matched) in matched_expected.iter_mut().enumerate() {
        let mut visited = vec![false; actual.len()];
        *matched = assign_expected_action(
            expected_index,
            &candidate_actuals,
            &mut actual_assignment,
            &mut visited,
        );
    }
    for (actual_index, assignment) in actual_assignment.iter().enumerate() {
        if assignment.is_some() {
            matched_actual[actual_index] = true;
        }
    }

    for (expected_index, expected_action) in expected.iter().enumerate() {
        if matched_expected[expected_index] {
            exact_count += 1;
            details.push(ActionComparisonDetail {
                signature: action_signature(&expected_action.action_type, &expected_action.payload),
                status: ActionStatus::Exact,
            });
        }
    }

    for (expected_index, expected_action) in expected.iter().enumerate() {
        if !matched_expected[expected_index] {
            let signature =
                action_signature(&expected_action.action_type, &expected_action.payload);
            missing_count += 1;
            details.push(ActionComparisonDetail {
                signature: signature.clone(),
                status: ActionStatus::Missing,
            });
            issues.push(format!("Missing action: {}", signature));
        }
    }

    for (actual_index, actual_action) in actual.iter().enumerate() {
        if !matched_actual[actual_index] {
            let signature = action_signature(&actual_action.action_type, &actual_action.payload);
            extra_count += 1;
            details.push(ActionComparisonDetail {
                signature: signature.clone(),
                status: ActionStatus::Extra,
            });
            issues.push(format!("Extra action: {}", signature));
        }
    }

    let total_expected = expected.len();
    let total_actual = actual.len();
    let pass_rate = if total_expected == 0 {
        if total_actual == 0 {
            1.0
        } else {
            0.0
        }
    } else {
        exact_count as f64 / total_expected as f64
    };

    let pass = missing_count == 0 && extra_count == 0;

    ActionMatchResult {
        pass,
        payload_match,
        actions: details,
        summary: ComparisonSummary {
            total_expected,
            total_actual,
            exact_count,
            missing_count,
            extra_count,
            pass_rate,
        },
        issues,
    }
}

/// Kuhn's augmenting-path step for maximum bipartite matching between expected
/// and actual actions. Tries to assign `expected_index` to one of its candidate
/// actuals, displacing an existing assignment if that earlier expected action
/// can be rerouted to another candidate.
fn assign_expected_action(
    expected_index: usize,
    candidate_actuals: &[Vec<usize>],
    actual_assignment: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &actual_index in &candidate_actuals[expected_index] {
        if visited[actual_index] {
            continue;
        }
        visited[actual_index] = true;
        let can_take = match actual_assignment[actual_index] {
            None => true,
            Some(other_expected) => assign_expected_action(
                other_expected,
                candidate_actuals,
                actual_assignment,
                visited,
            ),
        };
        if can_take {
            actual_assignment[actual_index] = Some(expected_index);
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq)]
struct ExpectedActionBucket {
    actions: Vec<ExpectedAction>,
    payload_match: ActionPayloadMatchMode,
}

pub fn f_beta_from_summary(summary: &ComparisonSummary, beta: f64) -> (f64, f64, f64) {
    let tp = summary.exact_count as f64;
    let fp = summary.extra_count as f64;
    let fn_count = summary.missing_count as f64;

    if tp == 0.0 && fp == 0.0 && fn_count == 0.0 {
        return (1.0, 1.0, 1.0);
    }

    let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
    let recall = if tp + fn_count > 0.0 {
        tp / (tp + fn_count)
    } else {
        0.0
    };
    (compute_f_beta(precision, recall, beta), precision, recall)
}

/// Harness trajectory scorer with explicit projection controls.
///
/// The raw artifact keeps the observed tool trajectory. This scorer optionally
/// projects stream-derived trajectory events before applying the generic
/// F-beta trajectory score.
#[derive(Debug, Clone)]
pub struct HarnessTrajectoryScorer {
    pub beta: f64,
    pub config: TrajectoryScorerConfig,
}

impl Default for HarnessTrajectoryScorer {
    fn default() -> Self {
        Self {
            beta: DEFAULT_BETA,
            config: TrajectoryScorerConfig::default(),
        }
    }
}

impl HarnessTrajectoryScorer {
    pub fn with_config(config: TrajectoryScorerConfig) -> Self {
        Self {
            beta: DEFAULT_BETA,
            config,
        }
    }
}

impl EvalScorer for HarnessTrajectoryScorer {
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
        let projection = project_trajectory(
            &output.actual_trajectory,
            output.extra.as_ref(),
            &self.config,
        );
        let f_beta = score_trajectory(
            &test_case.expected_trajectory,
            &projection.scored_trajectory,
            test_case.trajectory_mode,
            self.beta,
        );

        let mut details = serde_json::to_value(trajectory_score_details(
            &test_case.expected_trajectory,
            &projection.scored_trajectory,
            test_case.trajectory_mode,
            f_beta.f_score == 1.0,
        ))
        .expect("trajectory score details serialize");
        if let serde_json::Value::Object(ref mut object) = details {
            object.insert(
                "projection".to_string(),
                serde_json::to_value(projection).expect("trajectory projection serializes"),
            );
        }

        ScoredSample::leaf_with_details(SCORER_TRAJECTORY, f_beta.f_score, details)
    }

    fn name(&self) -> &str {
        SCORER_TRAJECTORY
    }
}

/// The action source a scorer compares expected actions against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionSource {
    /// Business actions projected from the stored plan (`storePlan`).
    Planned,
    /// Business actions resolved by execution (`executePlan`).
    Executed,
}

/// Scores expected actions against actual actions from one source using an
/// F-beta aggregate.
///
/// The leaf score is the F-beta value computed from exact/missing/extra action
/// counts. `details.summary.passRate` is also emitted for diagnostics, but it
/// is exact matches divided by expected actions and is not the canonical score.
///
/// An empty expected bucket is vacuous (score `1.0`): "presence of a bucket =
/// intent to score it". The per-test-case composite normally omits a scorer
/// whose bucket is empty, so this is a defensive fallback.
#[derive(Debug, Clone)]
pub struct ActionScorer {
    source: ActionSource,
    scorer_name: &'static str,
    pub beta: f64,
    /// Expected actions captured at construction. When set, the leaf scores
    /// against these instead of re-normalizing the ground truth per sample;
    /// `None` means "read and normalize from the test case at score time".
    expected: Option<ExpectedActionBucket>,
}

impl ActionScorer {
    /// Scorer for the planned (stored-plan) action source.
    pub fn planned() -> Self {
        Self {
            source: ActionSource::Planned,
            scorer_name: SCORER_PLANNED_ACTIONS,
            beta: DEFAULT_BETA,
            expected: None,
        }
    }

    /// Scorer for the executed action source.
    pub fn executed() -> Self {
        Self {
            source: ActionSource::Executed,
            scorer_name: SCORER_EXECUTED_ACTIONS,
            beta: DEFAULT_BETA,
            expected: None,
        }
    }

    pub fn with_beta(mut self, beta: f64) -> Self {
        self.beta = beta;
        self
    }

    /// Capture the expected actions once, avoiding per-sample ground-truth
    /// normalization. Used by the per-test-case composite builder.
    pub fn with_expected(
        mut self,
        expected: Vec<ExpectedAction>,
        payload_match: ActionPayloadMatchMode,
    ) -> Self {
        self.expected = Some(ExpectedActionBucket {
            actions: expected,
            payload_match,
        });
        self
    }

    pub fn source(&self) -> ActionSource {
        self.source
    }

    fn expected_actions(
        &self,
        ground_truth: Option<&FrameworkGroundTruth>,
    ) -> Result<Option<ExpectedActionBucket>, GroundTruthNormalizationError> {
        let Some(normalized) = normalize_ground_truth(ground_truth)? else {
            return Ok(None);
        };
        let actions = match self.source {
            ActionSource::Planned => normalized.planned,
            ActionSource::Executed => normalized.executed,
        };
        Ok(Some(ExpectedActionBucket {
            actions,
            payload_match: normalized.payload_match,
        }))
    }

    fn actual_actions(
        &self,
        output: &RawSampleOutput,
    ) -> Result<Vec<ResolvedAction>, serde_json::Error> {
        match self.source {
            ActionSource::Planned => extract_planned_actions(output),
            ActionSource::Executed => extract_resolved_actions(output),
        }
    }
}

impl EvalScorer for ActionScorer {
    fn score(&self, test_case: &EvalTestCase, output: &RawSampleOutput) -> ScoredSample {
        let expected_actions = match &self.expected {
            // Captured at construction (per-test-case composite): no re-normalization.
            Some(captured) if !captured.actions.is_empty() => captured.clone(),
            Some(_) => return ScoredSample::leaf(self.scorer_name, 1.0),
            // Generic scorer: read and normalize from the test case at score time.
            None => match self.expected_actions(test_case.ground_truth.as_ref()) {
                Ok(Some(bucket)) if !bucket.actions.is_empty() => bucket,
                Ok(_) => return ScoredSample::leaf(self.scorer_name, 1.0),
                Err(error) => return normalization_error_score(self.scorer_name, &error),
            },
        };

        let actual_actions = match self.actual_actions(output) {
            Ok(actions) => actions,
            Err(error) => {
                return ScoredSample::leaf_with_details(
                    self.scorer_name,
                    0.0,
                    serde_json::json!({
                        "error": format!("Failed to deserialize actions: {error}"),
                        "rawValue": output.extra,
                    }),
                );
            }
        };

        let result = compare_actions_with_payload_match(
            &expected_actions.actions,
            &actual_actions,
            expected_actions.payload_match,
        );
        let (f_score, _precision, _recall) = f_beta_from_summary(&result.summary, self.beta);

        ScoredSample::leaf_with_details(
            self.scorer_name,
            f_score,
            serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
        )
    }

    fn name(&self) -> &str {
        self.scorer_name
    }
}

fn invalid_expectation_score(scorer_name: &str, message: &str) -> ScoredSample {
    ScoredSample::leaf_with_details(
        scorer_name,
        0.0,
        serde_json::json!({
            "error": message,
        }),
    )
}

fn normalization_error_score(
    scorer_name: &str,
    error: &GroundTruthNormalizationError,
) -> ScoredSample {
    invalid_expectation_score(
        scorer_name,
        &format!("Failed to normalize harness action ground truth: {error}"),
    )
}

fn action_signature(action_type: &str, payload: &serde_json::Value) -> String {
    let canonical_payload = canonical_json(payload);
    let rendered = serde_json::to_string(&canonical_payload).unwrap_or_else(|_| "null".into());
    format!("{action_type}:{rendered}")
}

fn payload_matches(
    expected: &serde_json::Value,
    actual: &serde_json::Value,
    mode: ActionPayloadMatchMode,
) -> bool {
    match mode {
        ActionPayloadMatchMode::Subset => payload_matches_subset(expected, actual),
        ActionPayloadMatchMode::Exact => semantic_json_eq(expected, actual),
    }
}

fn payload_matches_subset(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Object(expected_map), serde_json::Value::Object(actual_map)) => {
            expected_map.iter().all(|(key, expected_value)| {
                actual_map.get(key).is_some_and(|actual_value| {
                    payload_matches_subset(expected_value, actual_value)
                })
            })
        }
        (serde_json::Value::Array(expected_values), serde_json::Value::Array(actual_values)) => {
            if expected_values.iter().all(is_scalar_json)
                && actual_values.iter().all(is_scalar_json)
            {
                scalar_arrays_match_unordered(expected_values, actual_values)
            } else {
                expected_values.len() == actual_values.len()
                    && expected_values.iter().zip(actual_values).all(
                        |(expected_value, actual_value)| {
                            payload_matches_subset(expected_value, actual_value)
                        },
                    )
            }
        }
        _ => semantic_json_eq(expected, actual),
    }
}

fn semantic_json_eq(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Number(expected), serde_json::Value::Number(actual)) => {
            json_numbers_equal(expected, actual)
        }
        (serde_json::Value::Array(expected_values), serde_json::Value::Array(actual_values)) => {
            expected_values.len() == actual_values.len()
                && expected_values.iter().zip(actual_values).all(
                    |(expected_value, actual_value)| semantic_json_eq(expected_value, actual_value),
                )
        }
        (serde_json::Value::Object(expected_map), serde_json::Value::Object(actual_map)) => {
            expected_map.len() == actual_map.len()
                && expected_map.iter().all(|(key, expected_value)| {
                    actual_map
                        .get(key)
                        .is_some_and(|actual_value| semantic_json_eq(expected_value, actual_value))
                })
        }
        _ => expected == actual,
    }
}

fn json_numbers_equal(expected: &serde_json::Number, actual: &serde_json::Number) -> bool {
    if expected == actual {
        return true;
    }

    match (
        json_number_as_integer(expected),
        json_number_as_integer(actual),
    ) {
        // Both integral: compare exactly, with full u64/i64 range preserved.
        (Some(expected), Some(actual)) => expected == actual,
        // Exactly one integral: the float side must be a whole number that
        // equals the integer exactly. Going through `as_f64()` here would lose
        // precision for integers beyond 2^53 (e.g. 9007199254740993 would
        // collapse onto 9007199254740992.0), so compare in integer space.
        (Some(integer), None) => float_equals_integer(actual, integer),
        (None, Some(integer)) => float_equals_integer(expected, integer),
        // Both non-integral: compare as f64.
        (None, None) => match (expected.as_f64(), actual.as_f64()) {
            (Some(expected), Some(actual)) => expected == actual,
            _ => false,
        },
    }
}

/// Exact integer value of a JSON number, or `None` if it is a floating-point
/// value. serde_json stores integers as `i64` or `u64`, so `i128` holds either.
fn json_number_as_integer(number: &serde_json::Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

/// Whether a floating-point JSON number is a whole number exactly equal to
/// `integer`. Lossless: the float is compared in integer space.
fn float_equals_integer(float: &serde_json::Number, integer: i128) -> bool {
    match float.as_f64() {
        Some(value) if value.is_finite() && value.fract() == 0.0 => (value as i128) == integer,
        _ => false,
    }
}

fn scalar_arrays_match_unordered(
    expected_values: &[serde_json::Value],
    actual_values: &[serde_json::Value],
) -> bool {
    if expected_values.len() != actual_values.len() {
        return false;
    }

    let mut matched_actual = vec![false; actual_values.len()];
    expected_values.iter().all(|expected_value| {
        let matched_index = actual_values
            .iter()
            .enumerate()
            .find(|(actual_index, actual_value)| {
                !matched_actual[*actual_index] && semantic_json_eq(expected_value, actual_value)
            })
            .map(|(actual_index, _)| actual_index);
        if let Some(actual_index) = matched_index {
            matched_actual[actual_index] = true;
            true
        } else {
            false
        }
    })
}

fn is_scalar_json(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_)
    )
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::TestCaseId;
    use agent_fw_eval::{GroundTruth, TrajectoryMode};

    fn test_case_with_ground_truth(ground_truth: GroundTruth) -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["draft_plan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: Some(ground_truth),
            final_response: None,
            source_thread_id: None,
        }
    }

    fn test_case_with_expected_trajectory(expected_trajectory: Vec<&str>) -> EvalTestCase {
        EvalTestCase {
            id: TestCaseId::new_unchecked("tc-trajectory"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: expected_trajectory
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        }
    }

    fn expected_action(action_type: &str, payload: serde_json::Value) -> ExpectedAction {
        ExpectedAction {
            action_type: action_type.to_string(),
            payload,
        }
    }

    #[test]
    fn compare_actions_matches_exact_generic_payload() {
        let expected = expected_action(
            "send_email",
            serde_json::json!({
                "template": "stockout_warning",
                "recipient": "ops@example.com",
            }),
        );
        let actual = ResolvedAction::new(
            "send_email",
            serde_json::json!({
                "recipient": "ops@example.com",
                "template": "stockout_warning",
            }),
        );

        let result = compare_actions(&[expected], &[actual]);
        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 1);
    }

    #[test]
    fn compare_actions_matches_expected_payload_subset() {
        let expected = expected_action(
            "price_change",
            serde_json::json!({
                "changeType": "absolute",
                "value": 10.0,
                "context": { "channels": ["ONLINE"] }
            }),
        );
        let actual = ResolvedAction::new(
            "price_change",
            serde_json::json!({
                "changeType": "absolute",
                "value": 10.0,
                "context": {
                    "channels": ["ONLINE"],
                    "region": "EU"
                },
                "scopeId": "generated-scope",
                "productSetId": "generated-product-set",
                "name": "Model-authored plan name"
            }),
        );

        let result = compare_actions(&[expected], &[actual]);
        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 1);
        assert_eq!(result.summary.extra_count, 0);
    }

    #[test]
    fn compare_actions_exact_payload_rejects_extra_fields() {
        let expected = expected_action(
            "price_change",
            serde_json::json!({
                "changeType": "absolute",
                "value": 10.0
            }),
        );
        let actual = ResolvedAction::new(
            "price_change",
            serde_json::json!({
                "changeType": "absolute",
                "value": 10.0,
                "duration": "forever"
            }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );
        assert!(!result.pass);
        assert_eq!(result.payload_match, ActionPayloadMatchMode::Exact);
        assert_eq!(result.summary.exact_count, 0);
        assert_eq!(result.summary.missing_count, 1);
        assert_eq!(result.summary.extra_count, 1);
    }

    #[test]
    fn compare_actions_exact_payload_ignores_object_key_order() {
        let expected = expected_action(
            "send_email",
            serde_json::json!({
                "template": "stockout_warning",
                "recipient": "ops@example.com",
            }),
        );
        let actual = ResolvedAction::new(
            "send_email",
            serde_json::json!({
                "recipient": "ops@example.com",
                "template": "stockout_warning",
            }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );
        assert!(result.pass, "{result:?}");
        assert_eq!(result.payload_match, ActionPayloadMatchMode::Exact);
        assert_eq!(result.summary.exact_count, 1);
    }

    #[test]
    fn compare_actions_exact_payload_matches_semantic_numbers() {
        let expected = expected_action(
            "availability_change",
            serde_json::json!({
                "availabilityChanges": [
                    { "type": "SetAbsolute", "value": 1 }
                ]
            }),
        );
        let actual = ResolvedAction::new(
            "availability_change",
            serde_json::json!({
                "availabilityChanges": [
                    { "type": "SetAbsolute", "value": 1.0 }
                ]
            }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );
        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 1);
    }

    #[test]
    fn compare_actions_exact_payload_keeps_array_order_significant() {
        let expected = expected_action(
            "delist_products",
            serde_json::json!({ "productIds": ["a", "b", "c"] }),
        );
        let actual = ResolvedAction::new(
            "delist_products",
            serde_json::json!({ "productIds": ["c", "a", "b"] }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );
        assert!(!result.pass);
        assert_eq!(result.summary.exact_count, 0);
        assert_eq!(result.summary.missing_count, 1);
        assert_eq!(result.summary.extra_count, 1);
    }

    #[test]
    fn compare_actions_subset_payload_matches_unordered_scalar_arrays() {
        let expected = expected_action(
            "delist_products",
            serde_json::json!({ "productIds": ["a", "b", "c"] }),
        );
        let actual = ResolvedAction::new(
            "delist_products",
            serde_json::json!({
                "productIds": ["c", "a", "b"],
                "reason": "seasonal cleanup"
            }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Subset,
        );
        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 1);
    }

    #[test]
    fn compare_actions_subset_payload_rejects_scalar_array_with_extra_item() {
        let expected = expected_action(
            "delist_products",
            serde_json::json!({ "productIds": ["a", "b", "c"] }),
        );
        let actual = ResolvedAction::new(
            "delist_products",
            serde_json::json!({ "productIds": ["c", "a", "b", "d"] }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Subset,
        );
        assert!(!result.pass);
        assert_eq!(result.summary.exact_count, 0);
        assert_eq!(result.summary.missing_count, 1);
        assert_eq!(result.summary.extra_count, 1);
    }

    #[test]
    fn compare_actions_matches_duplicate_expected_and_actual_actions() {
        let expected_one = expected_action(
            "availability_change",
            serde_json::json!({ "value": 1, "scope": "online" }),
        );
        let expected_two = expected_action(
            "availability_change",
            serde_json::json!({ "value": 1, "scope": "online" }),
        );
        let actual_one = ResolvedAction::new(
            "availability_change",
            serde_json::json!({ "value": 1.0, "scope": "online" }),
        );
        let actual_two = ResolvedAction::new(
            "availability_change",
            serde_json::json!({ "value": 1.0, "scope": "online" }),
        );

        let result = compare_actions_with_payload_match(
            &[expected_one, expected_two],
            &[actual_one, actual_two],
            ActionPayloadMatchMode::Exact,
        );
        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 2);
        assert_eq!(result.summary.pass_rate, 1.0);
        assert!(result.issues.is_empty(), "{result:?}");
    }

    #[test]
    fn compare_actions_rejects_subset_payload_mismatch() {
        let expected = expected_action(
            "price_change",
            serde_json::json!({
                "changeType": "absolute",
                "value": 10.0
            }),
        );
        let actual = ResolvedAction::new(
            "price_change",
            serde_json::json!({
                "changeType": "relative",
                "value": 10.0,
                "scopeId": "generated-scope"
            }),
        );

        let result = compare_actions(&[expected], &[actual]);
        assert!(!result.pass);
        assert_eq!(result.summary.exact_count, 0);
        assert_eq!(result.summary.missing_count, 1);
        assert_eq!(result.summary.extra_count, 1);
    }

    #[test]
    fn compare_actions_subset_finds_maximum_assignment_not_greedy() {
        // A broad expected payload (empty subset) and a specific one. Greedy
        // first-match would let the broad expected consume the only actual the
        // specific expected can match. Maximum matching assigns both.
        let broad = expected_action("update", serde_json::json!({}));
        let specific = expected_action("update", serde_json::json!({ "id": 1 }));
        let actual_specific = ResolvedAction::new("update", serde_json::json!({ "id": 1 }));
        let actual_other = ResolvedAction::new("update", serde_json::json!({ "other": 2 }));

        let result = compare_actions_with_payload_match(
            &[broad, specific],
            &[actual_specific, actual_other],
            ActionPayloadMatchMode::Subset,
        );

        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 2);
        assert_eq!(result.summary.missing_count, 0);
        assert_eq!(result.summary.extra_count, 0);
    }

    #[test]
    fn compare_actions_exact_distinguishes_large_integers_from_lossy_floats() {
        // 9007199254740993 cannot be represented exactly as f64; an `as_f64()`
        // comparison would collapse it onto 9007199254740992.0.
        let expected = expected_action(
            "set_quantity",
            serde_json::json!({ "value": 9007199254740993i64 }),
        );
        let actual = ResolvedAction::new(
            "set_quantity",
            serde_json::json!({ "value": 9007199254740992.0 }),
        );

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );

        assert!(!result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 0);
        assert_eq!(result.summary.missing_count, 1);
        assert_eq!(result.summary.extra_count, 1);
    }

    #[test]
    fn compare_actions_exact_matches_whole_number_int_and_float() {
        // The semantic 1 == 1.0 behavior must still hold after the precision fix.
        let expected = expected_action("set_quantity", serde_json::json!({ "value": 7i64 }));
        let actual = ResolvedAction::new("set_quantity", serde_json::json!({ "value": 7.0 }));

        let result = compare_actions_with_payload_match(
            &[expected],
            &[actual],
            ActionPayloadMatchMode::Exact,
        );

        assert!(result.pass, "{result:?}");
        assert_eq!(result.summary.exact_count, 1);
    }

    #[test]
    fn action_executor_scores_exact_match() {
        let scorer = ActionScorer::executed();
        let ground_truth = GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "executedActions": [
                {
                    "type": "create_ticket",
                    "payload": { "priority": "high", "team": "supply" }
                }
            ]
        }));
        let output = RawSampleOutput::with_extra(
            vec!["executePlan".into()],
            serde_json::json!({
                "resolvedActions": [
                    {
                        "type": "create_ticket",
                        "payload": { "team": "supply", "priority": "high" }
                    }
                ]
            }),
        );

        let scored = scorer.score(&test_case_with_ground_truth(ground_truth), &output);
        assert_eq!(scored.aggregate, 1.0);
        assert_eq!(
            scored.component_scores[0].scorer_name,
            SCORER_EXECUTED_ACTIONS
        );
    }

    #[test]
    fn action_executor_exact_payload_match_rejects_extra_fields() {
        let scorer = ActionScorer::executed();
        let ground_truth = GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "payloadMatch": "exact",
            "executedActions": [
                {
                    "type": "create_ticket",
                    "payload": { "priority": "high", "team": "supply" }
                }
            ]
        }));
        let output = RawSampleOutput::with_extra(
            vec!["executePlan".into()],
            serde_json::json!({
                "resolvedActions": [
                    {
                        "type": "create_ticket",
                        "payload": {
                            "team": "supply",
                            "priority": "high",
                            "notify": true
                        }
                    }
                ]
            }),
        );

        let scored = scorer.score(&test_case_with_ground_truth(ground_truth), &output);
        assert_eq!(scored.aggregate, 0.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("action scorer should emit details");
        assert_eq!(detail["payloadMatch"], serde_json::json!("exact"));
        assert_eq!(detail["summary"]["missingCount"], serde_json::json!(1));
        assert_eq!(detail["summary"]["extraCount"], serde_json::json!(1));
    }

    #[test]
    fn action_planner_exact_payload_match_scores_equal_payload() {
        let scorer = ActionScorer::planned();
        let ground_truth = GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "payloadMatch": "exact",
            "plannedActions": [
                {
                    "type": "create_ticket",
                    "payload": { "priority": "high", "team": "supply" }
                }
            ]
        }));
        let output = RawSampleOutput::with_extra(
            vec!["storePlan".into()],
            serde_json::json!({
                "plannedActions": [
                    {
                        "type": "create_ticket",
                        "payload": { "team": "supply", "priority": "high" }
                    }
                ]
            }),
        );

        let scored = scorer.score(&test_case_with_ground_truth(ground_truth), &output);
        assert_eq!(scored.aggregate, 1.0);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("action scorer should emit details");
        assert_eq!(detail["payloadMatch"], serde_json::json!("exact"));
        assert_eq!(detail["summary"]["exactCount"], serde_json::json!(1));
    }

    #[test]
    fn planner_preset_builds_composite_graph() {
        let scorer = scorer_for_mode(EvalMode::Planner, None).expect("planner scorer should build");
        assert_eq!(scorer.name(), "composite");
    }

    #[test]
    fn planner_preset_emits_canonical_component_names() {
        let scorer = scorer_for_preset(PRESET_PLANNER, None).expect("planner preset should build");
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-planner"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["buildPlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };

        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec!["buildPlan".into()]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![SCORER_TRAJECTORY, SCORER_PLANNED_ACTIONS, "composite"]
        );
    }

    #[test]
    fn scorer_for_preset_builds_trajectory_only_helper() {
        let scorer = scorer_for_preset(PRESET_TRAJECTORY_ONLY, None)
            .expect("trajectory-only preset should build");
        assert_eq!(scorer.name(), SCORER_TRAJECTORY);

        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-trajectory"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["buildPlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };

        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec!["buildPlan".into()]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(names, vec![SCORER_TRAJECTORY]);
        assert_eq!(scored.aggregate, 1.0);
    }

    #[test]
    fn test_case_builder_preset_uses_sequential_components() {
        let scorer = scorer_for_preset(PRESET_TEST_CASE_BUILDER, None)
            .expect("test-case-builder preset should build");
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-builder"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["buildPlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };

        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec!["buildPlan".into()]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                SCORER_TRAJECTORY,
                SCORER_PLANNED_ACTIONS,
                SCORER_EXECUTED_ACTIONS,
                "composite"
            ]
        );
    }

    #[test]
    fn scorer_for_preset_rejects_unknown_preset() {
        let error = match scorer_for_preset("unknown", None) {
            Ok(_) => panic!("preset should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, PresetScorerError::UnknownPreset(name) if name == "unknown"));
    }

    #[test]
    fn sequential_default_contains_trajectory_planned_and_executed_scores() {
        let scorer =
            scorer_for_mode(EvalMode::Sequential, None).expect("sequential scorer should build");
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-2"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["draft_plan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };
        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec!["draft_plan".into()]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                SCORER_TRAJECTORY,
                SCORER_PLANNED_ACTIONS,
                SCORER_EXECUTED_ACTIONS,
                "composite"
            ]
        );
    }

    #[test]
    fn per_test_case_scorer_captures_expected_and_drops_empty_bucket() {
        // Only the executed bucket is populated.
        let ground_truth = GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "executedActions": [
                { "type": "price_change", "payload": { "value": 10.0 } }
            ]
        }));
        let weights = default_score_weights_for_preset(PRESET_SEQUENTIAL)
            .expect("preset resolves")
            .expect("sequential has weights");
        let scorer = scorer_for_test_case(PRESET_SEQUENTIAL, weights, Some(&ground_truth), None)
            .expect("per-test-case scorer builds");

        // The test case passed at score time carries NO ground truth: a correct
        // score proves the expected actions were captured at build time, not
        // re-read per sample.
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-capture"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["executePlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };
        let output = RawSampleOutput::with_extra(
            vec!["executePlan".into()],
            serde_json::json!({
                "resolvedActions": [
                    { "type": "price_change", "payload": { "value": 10.0 } }
                ]
            }),
        );

        let scored = scorer.score(&test_case, &output);
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        // The empty planned bucket is dropped; executed is captured and matches.
        assert_eq!(
            names,
            vec![SCORER_TRAJECTORY, SCORER_EXECUTED_ACTIONS, "composite"]
        );
        assert_eq!(scored.aggregate, 1.0, "{scored:?}");
    }

    #[test]
    fn canonical_weight_keys_build_sequential_components() {
        let weights = ScoreWeights::new(vec![
            (SCORER_TRAJECTORY.into(), 0.2),
            (SCORER_PLANNED_ACTIONS.into(), 0.3),
            (SCORER_EXECUTED_ACTIONS.into(), 0.5),
        ])
        .expect("weights should validate");
        let scorer = scorer_for_preset(PRESET_SEQUENTIAL, Some(weights))
            .expect("weighted sequential preset should build");
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-weighted"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["buildPlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };

        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec!["buildPlan".into()]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                SCORER_TRAJECTORY,
                SCORER_PLANNED_ACTIONS,
                SCORER_EXECUTED_ACTIONS,
                "composite"
            ]
        );
    }

    #[test]
    fn legacy_weight_aliases_are_rejected() {
        // The harness is new: there are no `fusedExecutor` / `executor` aliases.
        let weights = ScoreWeights::new(vec![("fusedExecutor".into(), 1.0)])
            .expect("shape-valid weights build before scorer resolution");
        let error = match scorer_for_preset(PRESET_SEQUENTIAL, Some(weights)) {
            Ok(_) => panic!("legacy alias should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            PresetScorerError::Weights(ScoreWeightsError::UnknownScorer(name))
                if name == "fusedExecutor"
        ));
    }

    #[test]
    fn materialized_weights_are_canonical_and_normalized() {
        let weights = ScoreWeights::new(vec![
            (SCORER_EXECUTED_ACTIONS.into(), 1.0),
            (SCORER_PLANNED_ACTIONS.into(), 1.0),
            (SCORER_TRAJECTORY.into(), 2.0),
        ])
        .expect("weights should validate");
        let materialized = materialize_score_weights(weights).expect("weights should materialize");
        let entries = materialized.normalized();

        assert_eq!(
            entries,
            vec![
                (SCORER_TRAJECTORY.to_string(), 0.5),
                (SCORER_PLANNED_ACTIONS.to_string(), 0.25),
                (SCORER_EXECUTED_ACTIONS.to_string(), 0.25),
            ]
        );
    }

    #[test]
    fn default_weights_are_exposed_for_artifact_metadata() {
        let sequential = default_score_weights_for_preset(PRESET_SEQUENTIAL)
            .expect("preset should resolve")
            .expect("sequential has materialized weights");
        assert_eq!(
            sequential.normalized(),
            vec![
                (SCORER_TRAJECTORY.to_string(), 0.5),
                (SCORER_PLANNED_ACTIONS.to_string(), 0.25),
                (SCORER_EXECUTED_ACTIONS.to_string(), 0.25),
            ]
        );

        let planner = default_score_weights_for_preset(PRESET_PLANNER)
            .expect("preset should resolve")
            .expect("planner has materialized weights");
        assert_eq!(
            planner.normalized(),
            vec![
                (SCORER_TRAJECTORY.to_string(), 0.5),
                (SCORER_PLANNED_ACTIONS.to_string(), 0.5),
            ]
        );
    }

    #[test]
    fn custom_weight_unknown_scorer_returns_weight_error() {
        let weights = ScoreWeights::new(vec![("not_a_scorer".into(), 1.0)])
            .expect("shape-valid weights should build before scorer resolution");
        let error = match scorer_for_preset(PRESET_SEQUENTIAL, Some(weights)) {
            Ok(_) => panic!("scorer should fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::Weights(ScoreWeightsError::UnknownScorer(name))
                if name == "not_a_scorer"
        ));
    }

    #[test]
    fn planner_rejects_executor_only_weights() {
        let weights = ScoreWeights::new(vec![(SCORER_EXECUTED_ACTIONS.to_string(), 1.0)])
            .expect("shape-valid weights should build before preset validation");
        let error = match scorer_for_preset(PRESET_PLANNER, Some(weights)) {
            Ok(_) => panic!("planner should reject executor-only scorer weights"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::ScorerNotAllowedForPreset { preset, scorer }
                if preset == PRESET_PLANNER && scorer == SCORER_EXECUTED_ACTIONS
        ));
    }

    #[test]
    fn executor_accepts_trajectory_weights() {
        let weights = ScoreWeights::new(vec![
            (SCORER_TRAJECTORY.to_string(), 0.25),
            (SCORER_EXECUTED_ACTIONS.to_string(), 0.75),
        ])
        .expect("shape-valid weights should build before preset validation");
        scorer_for_preset(PRESET_EXECUTOR, Some(weights))
            .expect("executor should allow trajectory scorer weights");
    }

    #[test]
    fn executor_default_weights_add_trajectory_when_case_authors_expected_trajectory() {
        let test_case =
            test_case_with_expected_trajectory(vec!["lookupCustomer", "updateCustomer"]);
        let weights =
            default_score_weights_for_preset_and_test_cases(PRESET_EXECUTOR, &[test_case])
                .expect("preset should resolve")
                .expect("executor has default weights");

        assert_eq!(
            weights.normalized(),
            vec![
                (
                    SCORER_EXECUTED_ACTIONS.to_string(),
                    DEFAULT_EXECUTOR_ACTION_WEIGHT,
                ),
                (
                    SCORER_TRAJECTORY.to_string(),
                    DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT,
                ),
            ]
        );
    }

    #[test]
    fn executor_default_weights_remain_action_only_without_expected_trajectory() {
        let test_case = test_case_with_expected_trajectory(vec![]);
        let weights =
            default_score_weights_for_preset_and_test_cases(PRESET_EXECUTOR, &[test_case])
                .expect("preset should resolve")
                .expect("executor has default weights");

        assert_eq!(
            weights.normalized(),
            vec![(SCORER_EXECUTED_ACTIONS.to_string(), 1.0)]
        );
    }

    #[test]
    fn specialist_default_weights_follow_authored_expectations() {
        let trajectory_case = test_case_with_expected_trajectory(vec!["lookupCustomer"]);
        let mut response_case = test_case_with_expected_trajectory(vec![]);
        response_case.id = TestCaseId::new_unchecked("tc-response");
        response_case.final_response = Some(serde_json::json!({
            "scorers": [
                {
                    "id": "mentions_customer",
                    "method": "contains",
                    "text": "customer"
                }
            ]
        }));
        let mut action_case = test_case_with_expected_trajectory(vec![]);
        action_case.id = TestCaseId::new_unchecked("tc-action");
        action_case.ground_truth = Some(GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "executedActions": [
                {
                    "type": "update_customer",
                    "payload": { "customerId": "cust_123" }
                }
            ]
        })));

        let weights = default_score_weights_for_preset_and_test_cases(
            PRESET_SPECIALIST,
            &[trajectory_case, response_case, action_case],
        )
        .expect("specialist preset should resolve")
        .expect("specialist authored expectations produce weights");

        assert_eq!(
            weights.normalized(),
            vec![
                (SCORER_TRAJECTORY.to_string(), 1.0 / 3.0),
                (SCORER_EXECUTED_ACTIONS.to_string(), 1.0 / 3.0),
                (SCORER_FINAL_RESPONSE.to_string(), 1.0 / 3.0),
            ]
        );
    }

    #[test]
    fn specialist_default_weights_reject_empty_expectations() {
        let error = match default_score_weights_for_preset_and_test_cases(
            PRESET_SPECIALIST,
            &[test_case_with_expected_trajectory(vec![])],
        ) {
            Ok(_) => panic!("empty specialist expectations should be rejected"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::SpecialistTestCaseNoScoreableExpectations { test_case_id }
                if test_case_id == "tc-trajectory"
        ));
    }

    #[test]
    fn specialist_preset_rejects_implicit_scorer_without_test_case_context() {
        let error = match scorer_for_preset(PRESET_SPECIALIST, None) {
            Ok(_) => panic!("specialist should not have context-free default scorer"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::SpecialistNoScoreableExpectations
        ));
    }

    #[test]
    fn specialist_explicit_trajectory_weight_can_score_empty_expected_trajectory() {
        let weights = ScoreWeights::new(vec![(SCORER_TRAJECTORY.to_string(), 1.0)])
            .expect("weights should build");
        let scorer = scorer_for_preset(PRESET_SPECIALIST, Some(weights))
            .expect("explicit specialist trajectory scorer should build");
        let test_case = test_case_with_expected_trajectory(vec![]);

        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec![]));

        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();
        assert_eq!(names, vec![SCORER_TRAJECTORY, "composite"]);
        assert_eq!(scored.aggregate, 1.0);
    }

    #[test]
    fn specialist_explicit_final_response_weight_requires_final_response_spec() {
        let weights = ScoreWeights::new(vec![(SCORER_FINAL_RESPONSE.to_string(), 1.0)])
            .expect("weights should build");
        let error = match validate_specialist_explicit_score_weights(
            &weights,
            &[test_case_with_expected_trajectory(vec![])],
        ) {
            Ok(_) => panic!("final_response scorer should require finalResponse"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::SpecialistScorerMissingExpectation {
                scorer,
                test_case_id,
                requirement
            } if scorer == SCORER_FINAL_RESPONSE
                && test_case_id == "tc-trajectory"
                && requirement == "finalResponse"
        ));
    }

    #[test]
    fn executor_per_case_scorer_drops_default_added_trajectory_when_case_has_no_expectation() {
        let empty_case = test_case_with_expected_trajectory(vec![]);
        let weights = ScoreWeights::new(vec![
            (
                SCORER_EXECUTED_ACTIONS.to_string(),
                DEFAULT_EXECUTOR_ACTION_WEIGHT,
            ),
            (
                SCORER_TRAJECTORY.to_string(),
                DEFAULT_EXECUTOR_TRAJECTORY_WEIGHT,
            ),
        ])
        .expect("weights should build");
        let scorer = scorer_for_eval_test_case_with_config(
            PRESET_EXECUTOR,
            weights,
            &empty_case,
            None,
            false,
        )
        .expect("executor scorer should build");

        let scored = scorer.score(&empty_case, &RawSampleOutput::new(vec![]));
        let names: Vec<&str> = scored
            .component_scores
            .iter()
            .map(|result| result.scorer_name.as_str())
            .collect();

        assert_eq!(names, vec![SCORER_EXECUTED_ACTIONS, "composite"]);
    }

    #[test]
    fn trajectory_only_rejects_composite_weights() {
        let weights = ScoreWeights::new(vec![(SCORER_EXECUTED_ACTIONS.to_string(), 1.0)])
            .expect("shape-valid weights should build before preset validation");
        let error = match scorer_for_preset(PRESET_TRAJECTORY_ONLY, Some(weights)) {
            Ok(_) => panic!("trajectory-only should reject executor scorer weights"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PresetScorerError::ScorerNotAllowedForPreset { preset, scorer }
                if preset == PRESET_TRAJECTORY_ONLY && scorer == SCORER_EXECUTED_ACTIONS
        ));
    }

    #[test]
    fn fused_executor_detail_shape_is_stable() {
        let scorer = ActionScorer::executed();
        let ground_truth = GroundTruth::structured(serde_json::json!({
            "kind": "flat",
            "executedActions": [
                {
                    "type": "availability_change",
                    "payload": {
                        "targetAvailability": 100,
                        "products": ["p1"],
                        "context": { "channels": ["ONLINE"] }
                    }
                }
            ]
        }));
        let output = RawSampleOutput::with_extra(
            vec!["executePlan".into()],
            serde_json::json!({
                "resolvedActions": [
                    {
                        "type": "availability_change",
                        "payload": {
                            "context": { "channels": ["ONLINE"] },
                            "products": ["p1"],
                            "targetAvailability": 100
                        }
                    }
                ]
            }),
        );

        let scored = scorer.score(&test_case_with_ground_truth(ground_truth), &output);
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("fused executor should emit details");

        assert_eq!(detail["pass"], serde_json::json!(true));
        assert_eq!(detail["summary"]["totalExpected"], serde_json::json!(1));
        assert_eq!(detail["summary"]["totalActual"], serde_json::json!(1));
        assert_eq!(detail["summary"]["exactCount"], serde_json::json!(1));
        assert_eq!(detail["summary"]["missingCount"], serde_json::json!(0));
        assert_eq!(detail["summary"]["extraCount"], serde_json::json!(0));
        assert_eq!(detail["summary"]["passRate"], serde_json::json!(1.0));
        assert_eq!(detail["actions"][0]["status"], serde_json::json!("exact"));
        assert!(detail["actions"][0].get("products").is_none());
        assert!(detail["actions"][0].get("scope").is_none());
    }

    #[test]
    fn trajectory_detail_shape_is_stable() {
        let scorer = scorer_for_preset(PRESET_TRAJECTORY_ONLY, None)
            .expect("trajectory-only preset should build");
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-trajectory-details"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["buildPlan".into(), "executePlan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };

        let scored = scorer.score(
            &test_case,
            &RawSampleOutput::new(vec!["buildPlan".into(), "executePlan".into()]),
        );
        let detail = scored.component_scores[0]
            .details
            .as_ref()
            .expect("trajectory scorer should emit details");

        assert_eq!(scored.component_scores[0].scorer_name, SCORER_TRAJECTORY);
        assert_eq!(scored.component_scores[0].score, 1.0);
        assert_eq!(detail["mode"], serde_json::json!("unordered"));
        assert_eq!(detail["passed"], serde_json::json!(true));
        assert_eq!(
            detail["expected"],
            serde_json::json!(["buildPlan", "executePlan"])
        );
        assert_eq!(
            detail["actual"],
            serde_json::json!(["buildPlan", "executePlan"])
        );
        assert_eq!(
            detail["matched"],
            serde_json::json!(["buildPlan", "executePlan"])
        );
        assert_eq!(detail["missing"], serde_json::json!([]));
        assert_eq!(detail["unexpected"], serde_json::json!([]));
        assert_eq!(detail["diagnostics"]["precision"], serde_json::json!(1.0));
        assert_eq!(detail["diagnostics"]["recall"], serde_json::json!(1.0));
        assert_eq!(detail["diagnostics"]["f1"], serde_json::json!(1.0));
        assert_eq!(detail["diagnostics"]["f2"], serde_json::json!(1.0));
        assert_eq!(
            detail["projection"]["source"],
            serde_json::json!("actualTrajectory")
        );
        assert!(detail.get("fScore").is_none());
    }

    #[test]
    fn comparison_summary_predicates_cover_exact_and_partial_matches() {
        let exact = ComparisonSummary {
            total_expected: 2,
            total_actual: 2,
            exact_count: 2,
            missing_count: 0,
            extra_count: 0,
            pass_rate: 1.0,
        };
        assert!(exact.all_exact());
        assert!(exact.all_signatures_matched());

        let missing = ComparisonSummary {
            total_expected: 2,
            total_actual: 1,
            exact_count: 1,
            missing_count: 1,
            extra_count: 0,
            pass_rate: 0.5,
        };
        assert!(!missing.all_exact());
        assert!(!missing.all_signatures_matched());
    }

    #[test]
    fn f_beta_from_summary_handles_vacuous_and_missing_cases() {
        let vacuous = ComparisonSummary {
            total_expected: 0,
            total_actual: 0,
            exact_count: 0,
            missing_count: 0,
            extra_count: 0,
            pass_rate: 1.0,
        };
        assert_eq!(f_beta_from_summary(&vacuous, 2.0), (1.0, 1.0, 1.0));

        let missing = ComparisonSummary {
            total_expected: 2,
            total_actual: 1,
            exact_count: 1,
            missing_count: 1,
            extra_count: 0,
            pass_rate: 0.5,
        };
        let (f, p, r) = f_beta_from_summary(&missing, 2.0);
        assert_eq!(p, 1.0);
        assert_eq!(r, 0.5);
        assert!(f > 0.0 && f < 1.0);
    }

    #[test]
    fn duplicate_expected_actions_require_matching_actual_actions() {
        let first = expected_action("create_ticket", serde_json::json!({ "priority": "high" }));
        let second = expected_action("create_ticket", serde_json::json!({ "priority": "high" }));

        let actual =
            ResolvedAction::new("create_ticket", serde_json::json!({ "priority": "high" }));

        let result = compare_actions(&[first, second], &[actual]);
        assert!(!result.pass);
        assert_eq!(result.summary.exact_count, 1);
        assert_eq!(result.summary.missing_count, 1);
        assert!(result
            .issues
            .iter()
            .all(|issue| !issue.contains("Duplicate")));
    }

    #[test]
    fn duplicate_actual_actions_count_unmatched_actual_as_extra() {
        let expected = expected_action("create_ticket", serde_json::json!({ "priority": "high" }));

        let first = ResolvedAction::new("create_ticket", serde_json::json!({ "priority": "high" }));
        let second =
            ResolvedAction::new("create_ticket", serde_json::json!({ "priority": "high" }));

        let result = compare_actions(&[expected], &[first, second]);
        assert!(!result.pass);
        assert_eq!(result.summary.exact_count, 1);
        assert_eq!(result.summary.extra_count, 1);
        assert!(result
            .issues
            .iter()
            .all(|issue| !issue.contains("Duplicate")));
    }

    #[test]
    fn action_comparison_scorer_is_vacuously_true_without_ground_truth() {
        let scorer = ActionScorer::planned();
        let test_case = EvalTestCase {
            id: TestCaseId::new_unchecked("tc-3"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec![],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        };
        let scored = scorer.score(&test_case, &RawSampleOutput::new(vec![]));
        assert_eq!(scored.aggregate, 1.0);
        assert_eq!(scored.component_scores.len().get(), 1);
    }
}
