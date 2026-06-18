//! Test case authoring types.
//!
//! # Key Types
//!
//! - [`AuthoredTestCase`] — Rich test case with provenance (stored in KV)
//! - [`TestCaseStatus`] — Draft / Active / Archived lifecycle
//! - [`TrajectoryStep`] — Single step with tool name + source provenance
//! - [`TestCaseBuilderSession`] — Mutable KV-persisted builder state
//! - [`ToolCatalog`] — Configurable trait for valid tool name validation
//!
//! # Design
//!
//! Domain-specific tool catalogs implement [`ToolCatalog`]. The framework ships
//! [`VecToolCatalog`] as a simple default implementation.

use std::collections::HashSet;

use agent_fw_core::TestCaseId;
use serde::{Deserialize, Serialize};

use crate::ground_truth::GroundTruth;
use crate::types::TrajectoryMode;

// =============================================================================
// Constants
// =============================================================================

/// Maximum length of a test case input prompt.
pub const MAX_INPUT_LENGTH: usize = 10_000;
/// Maximum number of tags per test case.
pub const MAX_TAGS: usize = 20;
/// Maximum length of a single tag.
pub const MAX_TAG_LENGTH: usize = 50;
/// Maximum trajectory length (number of steps).
pub const MAX_TRAJECTORY_LENGTH: usize = 200;

/// Single entry point for timestamp generation.
///
/// Centralizes `chrono::Utc::now()` for all test case authoring types.
/// In test builds, the clock can be overridden via `set_test_clock` to
/// eliminate non-deterministic `thread::sleep` calls in assertions.
fn now_rfc3339() -> String {
    #[cfg(test)]
    {
        TEST_CLOCK.with(|cell| {
            let borrow = cell.borrow();
            if let Some(f) = borrow.as_ref() {
                return f();
            }
            chrono::Utc::now().to_rfc3339()
        })
    }
    #[cfg(not(test))]
    {
        chrono::Utc::now().to_rfc3339()
    }
}

#[cfg(test)]
thread_local! {
    static TEST_CLOCK: std::cell::RefCell<Option<Box<dyn Fn() -> String>>> = const { std::cell::RefCell::new(None) };
}

/// Override the clock for the current thread (test only).
///
/// Returns a guard that restores the real clock on drop.
#[cfg(test)]
fn set_test_clock(f: impl Fn() -> String + 'static) -> TestClockGuard {
    TEST_CLOCK.with(|cell| {
        *cell.borrow_mut() = Some(Box::new(f));
    });
    TestClockGuard
}

#[cfg(test)]
struct TestClockGuard;

#[cfg(test)]
impl Drop for TestClockGuard {
    fn drop(&mut self) {
        TEST_CLOCK.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

// =============================================================================
// TestCaseStatus
// =============================================================================

/// Test case lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestCaseStatus {
    /// Being authored, not yet used in eval runs.
    Draft,
    /// Ready for eval execution.
    Active,
    /// Soft-deleted, excluded from eval runs.
    Archived,
}

impl Default for TestCaseStatus {
    fn default() -> Self {
        Self::Draft
    }
}

impl TestCaseStatus {
    /// Returns the canonical camelCase string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

impl std::fmt::Display for TestCaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TestCaseStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            _ => Err(format!("unknown test case status: {s}")),
        }
    }
}

impl TryFrom<String> for TestCaseStatus {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

// =============================================================================
// TrajectoryStep
// =============================================================================

/// Provenance source for a trajectory step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrajectoryStepSource {
    /// Extracted from a conversation thread.
    #[serde(rename_all = "camelCase")]
    FromThread {
        thread_id: String,
        original_index: usize,
    },
    /// Extracted from a planner run.
    #[serde(rename_all = "camelCase")]
    FromPlanner { run_id: String },
    /// Manually authored.
    #[serde(rename_all = "camelCase")]
    Manual {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl TrajectoryStepSource {
    /// Manual provenance without an explicit reason.
    pub fn manual() -> Self {
        Self::Manual { reason: None }
    }
}

/// A single step in an expected trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryStep {
    /// The tool name expected at this position.
    pub tool_name: String,
    /// How this step was created (provenance).
    pub source: TrajectoryStepSource,
    /// Position in the trajectory (0-indexed).
    pub position: usize,
}

impl TrajectoryStep {
    /// Construct a manual trajectory step.
    pub fn manual(tool_name: impl Into<String>, position: usize) -> Self {
        Self {
            tool_name: tool_name.into(),
            source: TrajectoryStepSource::manual(),
            position,
        }
    }

    /// Construct a thread-sourced trajectory step.
    pub fn from_thread(
        tool_name: impl Into<String>,
        position: usize,
        thread_id: impl Into<String>,
        original_index: usize,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            source: TrajectoryStepSource::FromThread {
                thread_id: thread_id.into(),
                original_index,
            },
            position,
        }
    }

    /// Construct a planner-sourced trajectory step.
    pub fn from_planner(
        tool_name: impl Into<String>,
        position: usize,
        run_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            source: TrajectoryStepSource::FromPlanner {
                run_id: run_id.into(),
            },
            position,
        }
    }
}

/// Build a manual trajectory from a flat ordered list of tool names.
pub fn manual_trajectory_steps<I, S>(tool_names: I) -> Vec<TrajectoryStep>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    tool_names
        .into_iter()
        .enumerate()
        .map(|(position, tool_name)| TrajectoryStep::manual(tool_name, position))
        .collect()
}

// =============================================================================
// TrajectorySource (provenance for the entire trajectory)
// =============================================================================

/// Source provenance for the entire trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrajectorySource {
    /// Extracted from a conversation thread segment.
    #[serde(rename_all = "camelCase")]
    ThreadSegment {
        thread_id: String,
        from_index: usize,
        to_index: usize,
    },
    /// Extracted from a planner run.
    #[serde(rename_all = "camelCase")]
    PlannerRun { run_id: String },
    /// Manually authored.
    #[serde(rename_all = "camelCase")]
    Manual {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl TrajectorySource {
    /// Manual trajectory provenance without an explicit reason.
    pub fn manual() -> Self {
        Self::Manual { reason: None }
    }
}

/// Why canonical trajectory construction failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrajectoryCanonicalizationError {
    ToolNameMismatch {
        expected: Vec<String>,
        actual: Vec<String>,
    },
}

impl std::fmt::Display for TrajectoryCanonicalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolNameMismatch { expected, actual } => write!(
                f,
                "expected trajectory {:?} does not match provenance steps {:?}",
                expected, actual
            ),
        }
    }
}

impl std::error::Error for TrajectoryCanonicalizationError {}

/// Canonicalize an authored trajectory from either flat tool names or provenance-rich steps.
///
/// - When `provenance_steps` is empty, the result is a manual trajectory.
/// - When `provenance_steps` is present, it is renumbered and must describe the
///   same ordered tool names as `expected_tool_names`.
pub fn canonicalize_expected_trajectory(
    expected_tool_names: Vec<String>,
    mut provenance_steps: Vec<TrajectoryStep>,
) -> Result<Vec<TrajectoryStep>, TrajectoryCanonicalizationError> {
    if provenance_steps.is_empty() {
        return Ok(manual_trajectory_steps(expected_tool_names));
    }

    for (position, step) in provenance_steps.iter_mut().enumerate() {
        step.position = position;
    }

    let actual: Vec<String> = provenance_steps
        .iter()
        .map(|step| step.tool_name.clone())
        .collect();
    if actual != expected_tool_names {
        return Err(TrajectoryCanonicalizationError::ToolNameMismatch {
            expected: expected_tool_names,
            actual,
        });
    }

    Ok(provenance_steps)
}

// =============================================================================
// ToolCatalog trait
// =============================================================================

/// Configurable tool catalog for trajectory validation.
///
/// Domain-specific catalogs implement this trait. The framework ships
/// [`VecToolCatalog`].
pub trait ToolCatalog: Send + Sync {
    /// Check if a tool name is valid.
    fn is_valid(&self, tool_name: &str) -> bool;

    /// Return all valid tool names.
    fn all_tool_names(&self) -> Vec<&str>;

    /// Return tool entries with optional metadata.
    ///
    /// The default implementation derives entries from `all_tool_names()`
    /// and leaves metadata empty, so existing catalogs stay valid.
    fn entries(&self) -> Vec<ToolCatalogEntry> {
        self.all_tool_names()
            .into_iter()
            .map(ToolCatalogEntry::named)
            .collect()
    }
}

/// A catalog entry for an evaluatable tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

impl ToolCatalogEntry {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            category: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }
}

/// Simple tool catalog backed by a Vec of strings.
#[derive(Debug, Clone)]
pub struct VecToolCatalog {
    entries: Vec<ToolCatalogEntry>,
    lookup: HashSet<String>,
}

impl VecToolCatalog {
    /// Create a new tool catalog from a list of tool names.
    pub fn new(tools: Vec<String>) -> Self {
        Self::from_entries(tools.into_iter().map(ToolCatalogEntry::named).collect())
    }

    /// Create a new tool catalog from rich tool entries.
    pub fn from_entries(entries: Vec<ToolCatalogEntry>) -> Self {
        let lookup: HashSet<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        Self { entries, lookup }
    }
}

impl ToolCatalog for VecToolCatalog {
    fn is_valid(&self, tool_name: &str) -> bool {
        self.lookup.contains(tool_name)
    }

    fn all_tool_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    fn entries(&self) -> Vec<ToolCatalogEntry> {
        self.entries.clone()
    }
}

// =============================================================================
// Trajectory Newtypes
// =============================================================================

/// Wraps pre-remapping tool names.
///
/// A `BaselineTrajectory` holds the raw tool names before any
/// validation/remapping through a [`ToolCatalog`]. Use [`remap()`](Self::remap)
/// to produce a [`RemappedTrajectory`] with only valid tool names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineTrajectory(Vec<String>);

impl BaselineTrajectory {
    /// Create a new baseline trajectory from raw tool names.
    pub fn new(tools: Vec<String>) -> Self {
        Self(tools)
    }

    /// Remap through a tool catalog, filtering out invalid tool names.
    ///
    /// Returns `(remapped, dropped)` — no silent data loss. Every input tool
    /// is accounted for in exactly one of the two vectors.
    pub fn remap(&self, catalog: &dyn ToolCatalog) -> (RemappedTrajectory, Vec<String>) {
        let mut valid = Vec::new();
        let mut dropped = Vec::new();
        for tool in &self.0 {
            if catalog.is_valid(tool) {
                valid.push(tool.clone());
            } else {
                dropped.push(tool.clone());
            }
        }
        (RemappedTrajectory(valid), dropped)
    }

    /// Access the raw tool names.
    pub fn tools(&self) -> &[String] {
        &self.0
    }
}

/// A trajectory that has been validated against a [`ToolCatalog`].
///
/// Only constructable via [`BaselineTrajectory::remap()`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RemappedTrajectory(Vec<String>);

impl RemappedTrajectory {
    /// Consume and return the validated tool names.
    pub fn into_tools(self) -> Vec<String> {
        self.0
    }

    /// Access the validated tool names.
    pub fn tools(&self) -> &[String] {
        &self.0
    }
}

// =============================================================================
// Validation functions
// =============================================================================

/// Validate a trajectory against a tool catalog.
///
/// Returns all invalid tool names (applicative error collection).
pub fn validate_trajectory(trajectory: &[String], catalog: &dyn ToolCatalog) -> Vec<String> {
    trajectory
        .iter()
        .filter(|t| !catalog.is_valid(t))
        .cloned()
        .collect()
}

/// Diagnostic emitted when a tag is dropped or truncated during normalization.
///
/// Unlike validation errors, these are informational — normalization always succeeds
/// but callers may want to surface warnings to users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagWarning {
    /// Tag was empty or whitespace-only after trimming.
    Empty { original: String },
    /// Tag exceeded `MAX_TAG_LENGTH` after trimming.
    TooLong { tag: String, len: usize, max: usize },
    /// Tag was a case-insensitive duplicate of an earlier tag.
    Duplicate { tag: String, kept: String },
    /// Result was truncated at `MAX_TAGS`; remaining tags were dropped.
    TruncatedAtMax { kept: usize, total: usize },
}

impl std::fmt::Display for TagWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { original } => write!(f, "empty tag dropped: {:?}", original),
            Self::TooLong { tag, len, max } => {
                write!(
                    f,
                    "tag {:?} dropped: length {} exceeds max {}",
                    tag, len, max
                )
            }
            Self::Duplicate { tag, kept } => {
                write!(f, "tag {:?} dropped: duplicate of {:?}", tag, kept)
            }
            Self::TruncatedAtMax { kept, total } => {
                write!(f, "kept {} of {} tags (max {})", kept, total, MAX_TAGS)
            }
        }
    }
}

/// Normalize tags: trim, deduplicate case-insensitively (first occurrence wins),
/// preserve original casing, enforce limits.
///
/// Returns `(normalized_tags, warnings)`. Warnings describe every tag that was
/// dropped and why — no silent data loss.
///
/// Unlike the previous implementation which lowercased all tags, this preserves
/// the user-provided casing while deduplicating via a lowercase key. First
/// occurrence wins (preserves user intent).
pub fn normalize_tags(tags: Vec<String>) -> (Vec<String>, Vec<TagWarning>) {
    let total = tags.len();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut warnings = Vec::new();

    for tag in tags {
        let trimmed = tag.trim().to_string();
        let key = trimmed.to_lowercase();
        if key.is_empty() {
            warnings.push(TagWarning::Empty { original: tag });
            continue;
        }
        if key.len() > MAX_TAG_LENGTH {
            warnings.push(TagWarning::TooLong {
                tag: trimmed,
                len: key.len(),
                max: MAX_TAG_LENGTH,
            });
            continue;
        }
        if !seen.insert(key.clone()) {
            // Find the kept tag for diagnostic
            let kept = result
                .iter()
                .find(|t: &&String| t.to_lowercase() == key)
                .cloned()
                .unwrap_or_default();
            warnings.push(TagWarning::Duplicate { tag: trimmed, kept });
            continue;
        }
        if result.len() >= MAX_TAGS {
            warnings.push(TagWarning::TruncatedAtMax {
                kept: MAX_TAGS,
                total,
            });
            break;
        }
        result.push(trimmed);
    }

    (result, warnings)
}

// =============================================================================
// Test Case Validation (Applicative — collects ALL errors)
// =============================================================================

/// Validation error for test case creation requests.
///
/// Applicative: collect all errors, don't short-circuit.
#[derive(Debug, Clone, PartialEq)]
pub enum TestCaseValidationError {
    /// Input prompt exceeds maximum length.
    InputTooLong { len: usize, max: usize },
    /// Input prompt is empty or whitespace-only.
    InputEmpty,
    /// Too many tags.
    TooManyTags { count: usize, max: usize },
    /// Individual tag exceeds maximum length.
    TagTooLong { tag: String, len: usize, max: usize },
    /// A trajectory step has an empty tool name.
    TrajectoryStepEmpty { index: usize },
    /// Invalid trajectory mode string.
    InvalidTrajectoryMode(String),
}

impl std::fmt::Display for TestCaseValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputTooLong { len, max } => {
                write!(f, "input length {} exceeds maximum of {}", len, max)
            }
            Self::InputEmpty => write!(f, "input must not be empty"),
            Self::TooManyTags { count, max } => {
                write!(f, "tag count {} exceeds maximum of {}", count, max)
            }
            Self::TagTooLong { tag, len, max } => {
                write!(f, "tag {:?} length {} exceeds maximum of {}", tag, len, max)
            }
            Self::TrajectoryStepEmpty { index } => {
                write!(f, "trajectory step {} has empty tool name", index)
            }
            Self::InvalidTrajectoryMode(mode) => {
                write!(f, "invalid trajectory mode: {:?}", mode)
            }
        }
    }
}

impl std::error::Error for TestCaseValidationError {}

impl serde::Serialize for TestCaseValidationError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Validate a test case creation request (applicative — collects all errors).
///
/// Validates input, tags, and trajectory steps against framework limits.
pub fn validate_test_case_fields(
    input: &str,
    tags: &[String],
    trajectory_tool_names: &[String],
    trajectory_mode: Option<&str>,
) -> Result<(), Vec<TestCaseValidationError>> {
    let mut errors = Vec::new();

    // Validate input
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        errors.push(TestCaseValidationError::InputEmpty);
    } else if trimmed_input.len() > MAX_INPUT_LENGTH {
        errors.push(TestCaseValidationError::InputTooLong {
            len: trimmed_input.len(),
            max: MAX_INPUT_LENGTH,
        });
    }

    // Validate tags
    if tags.len() > MAX_TAGS {
        errors.push(TestCaseValidationError::TooManyTags {
            count: tags.len(),
            max: MAX_TAGS,
        });
    }
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.len() > MAX_TAG_LENGTH {
            errors.push(TestCaseValidationError::TagTooLong {
                tag: trimmed.to_string(),
                len: trimmed.len(),
                max: MAX_TAG_LENGTH,
            });
        }
    }

    // Validate trajectory steps
    for (i, tool_name) in trajectory_tool_names.iter().enumerate() {
        if tool_name.trim().is_empty() {
            errors.push(TestCaseValidationError::TrajectoryStepEmpty { index: i });
        }
    }

    // Validate trajectory mode if provided
    if let Some(mode) = trajectory_mode {
        if mode.parse::<TrajectoryMode>().is_err() {
            errors.push(TestCaseValidationError::InvalidTrajectoryMode(
                mode.to_string(),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// =============================================================================
// AuthoredTestCase
// =============================================================================

/// A fully authored test case with provenance (stored in KV).
///
/// This is the rich representation used for authoring and storage.
/// Converted to [`EvalTestCase`](crate::types::EvalTestCase) for eval
/// execution via [`to_eval_test_case`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoredTestCase {
    pub id: TestCaseId,
    pub input: String,
    pub status: TestCaseStatus,
    pub tags: Vec<String>,
    /// Expected tool trajectory (ordered list of tool names).
    #[serde(default, deserialize_with = "deserialize_expected_trajectory_steps")]
    pub expected_trajectory: Vec<TrajectoryStep>,
    /// How the trajectory was obtained (may have multiple sources for stitched segments).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trajectory_sources: Vec<TrajectorySource>,
    /// Trajectory matching mode for eval scoring.
    #[serde(default = "default_trajectory_mode")]
    pub trajectory_mode: TrajectoryMode,
    /// Structured ground truth (domain-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_ground_truth: Option<GroundTruth>,
    /// ISO-8601 timestamp.
    pub created_at: String,
    /// ISO-8601 timestamp.
    pub updated_at: String,
    /// Link to the original chat thread (if created via "Save as Test Case").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    /// Link to the builder session that created this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    /// Warnings from tag normalization (empty tags stripped, duplicates removed, etc.).
    ///
    /// Populated by `finalize()`. Absent in serialized form when empty (backward-compatible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_warnings: Vec<String>,
}

fn default_trajectory_mode() -> TrajectoryMode {
    TrajectoryMode::Unordered
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExpectedTrajectoryWire {
    Steps(Vec<TrajectoryStep>),
    ToolNames(Vec<String>),
}

fn deserialize_expected_trajectory_steps<'de, D>(
    deserializer: D,
) -> Result<Vec<TrajectoryStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let wire = ExpectedTrajectoryWire::deserialize(deserializer)?;
    Ok(match wire {
        ExpectedTrajectoryWire::Steps(steps) => steps,
        ExpectedTrajectoryWire::ToolNames(tool_names) => manual_trajectory_steps(tool_names),
    })
}

/// Errors constructing authored test cases from thread traces.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThreadTraceDraftError {
    #[error("thread trace must contain at least one tool call")]
    EmptyToolTrace,
}

/// Overrides applied when finalizing a builder session into an authored test case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeAuthoredTestCaseOptions {
    pub status: TestCaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_ground_truth: Option<GroundTruth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
}

impl Default for FinalizeAuthoredTestCaseOptions {
    fn default() -> Self {
        Self {
            status: TestCaseStatus::Draft,
            structured_ground_truth: None,
            source_thread_id: None,
            source_session_id: None,
        }
    }
}

/// Inputs for composing an authored test case from already-canonicalized parts.
///
/// Callers are responsible for input/tool validation and trajectory canonicalization.
/// This helper centralizes the framework-owned assembly concerns:
/// tag normalization, timestamp defaults, and provenance/storage fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeAuthoredTestCaseOptions {
    pub status: TestCaseStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trajectory_sources: Vec<TrajectorySource>,
    #[serde(default = "default_trajectory_mode")]
    pub trajectory_mode: TrajectoryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_ground_truth: Option<GroundTruth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Default for ComposeAuthoredTestCaseOptions {
    fn default() -> Self {
        Self {
            status: TestCaseStatus::Draft,
            tags: Vec::new(),
            trajectory_sources: Vec::new(),
            trajectory_mode: default_trajectory_mode(),
            structured_ground_truth: None,
            source_thread_id: None,
            source_session_id: None,
            created_at: None,
            updated_at: None,
        }
    }
}

impl AuthoredTestCase {
    /// Create a minimal authored test case with the given ID and input.
    pub fn new(id: TestCaseId, input: String) -> Self {
        let now = now_rfc3339();
        Self {
            id,
            input,
            status: TestCaseStatus::Draft,
            tags: Vec::new(),
            expected_trajectory: Vec::new(),
            trajectory_sources: Vec::new(),
            trajectory_mode: TrajectoryMode::Unordered,
            structured_ground_truth: None,
            created_at: now.clone(),
            updated_at: now,
            source_thread_id: None,
            source_session_id: None,
            tag_warnings: Vec::new(),
        }
    }

    /// Compose a persisted authored test case from already-validated pieces.
    pub fn from_canonicalized_parts(
        id: TestCaseId,
        input: impl Into<String>,
        expected_trajectory: Vec<TrajectoryStep>,
        options: ComposeAuthoredTestCaseOptions,
    ) -> Self {
        let now = now_rfc3339();
        let (tags, warnings) = normalize_tags(options.tags);
        let created_at = options.created_at.unwrap_or_else(|| now.clone());
        let updated_at = options.updated_at.unwrap_or_else(|| created_at.clone());

        Self {
            id,
            input: input.into(),
            status: options.status,
            tags,
            expected_trajectory,
            trajectory_sources: options.trajectory_sources,
            trajectory_mode: options.trajectory_mode,
            structured_ground_truth: options.structured_ground_truth,
            created_at,
            updated_at,
            source_thread_id: options.source_thread_id,
            source_session_id: options.source_session_id,
            tag_warnings: warnings
                .into_iter()
                .map(|warning| warning.to_string())
                .collect(),
        }
    }

    /// Convert to a lightweight eval test case (strips provenance).
    pub fn to_eval_test_case(&self) -> crate::types::EvalTestCase {
        self.to_eval_test_case_with_mapper(std::convert::identity)
    }

    /// Convert to a lightweight eval test case while mapping tool names.
    pub fn to_eval_test_case_with_mapper<F>(&self, mapper: F) -> crate::types::EvalTestCase
    where
        F: FnOnce(Vec<String>) -> Vec<String>,
    {
        let expected_trajectory = mapper(
            self.expected_trajectory
                .iter()
                .map(|s| s.tool_name.clone())
                .collect(),
        );
        crate::types::EvalTestCase {
            id: self.id.clone(),
            tags: self.tags.clone(),
            input: self.input.clone(),
            expected_trajectory,
            trajectory_mode: self.trajectory_mode,
            ground_truth: self.structured_ground_truth.clone(),
            final_response: None,
            source_thread_id: self.source_thread_id.clone(),
        }
    }

    /// Flat tool names from trajectory steps.
    pub fn trajectory_tool_names(&self) -> Vec<&str> {
        self.expected_trajectory
            .iter()
            .map(|s| s.tool_name.as_str())
            .collect()
    }

    /// Owned tool names from trajectory steps.
    pub fn trajectory_tool_names_owned(&self) -> Vec<String> {
        self.expected_trajectory
            .iter()
            .map(|s| s.tool_name.clone())
            .collect()
    }

    /// Build a draft authored test case from a single thread trace.
    ///
    /// This preserves exact trace order, provenance, and source-thread linkage
    /// while leaving any domain-specific validation to the consuming app.
    pub fn draft_from_thread_trace(
        id: TestCaseId,
        input: String,
        thread_id: impl Into<String>,
        tool_calls: &[ToolCallEntry],
    ) -> Result<Self, ThreadTraceDraftError> {
        if tool_calls.is_empty() {
            return Err(ThreadTraceDraftError::EmptyToolTrace);
        }

        let thread_id = thread_id.into();
        let now = now_rfc3339();
        let expected_trajectory = tool_calls
            .iter()
            .enumerate()
            .map(|(position, tool_call)| {
                TrajectoryStep::from_thread(
                    tool_call.tool_name.clone(),
                    position,
                    thread_id.clone(),
                    tool_call.index,
                )
            })
            .collect();

        Ok(Self {
            id,
            input,
            status: TestCaseStatus::Draft,
            tags: Vec::new(),
            expected_trajectory,
            trajectory_sources: vec![TrajectorySource::ThreadSegment {
                thread_id: thread_id.clone(),
                from_index: 0,
                to_index: tool_calls.len(),
            }],
            trajectory_mode: TrajectoryMode::Strict,
            structured_ground_truth: None,
            created_at: now.clone(),
            updated_at: now,
            source_thread_id: Some(thread_id),
            source_session_id: None,
            tag_warnings: Vec::new(),
        })
    }
}

// =============================================================================
// TestCaseBuilderSession
// =============================================================================

/// Mutable builder session for test case authoring (stored in KV with TTL).
///
/// Enables the interactive trajectory builder workflow:
/// 1. Create session from thread ID or manual input
/// 2. Add/remove/reorder trajectory steps
/// 3. Set ground truth
/// 4. Save as AuthoredTestCase
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseBuilderSession {
    pub session_id: String,
    pub input: String,
    pub tags: Vec<String>,
    pub trajectory_steps: Vec<TrajectoryStep>,
    pub trajectory_sources: Vec<TrajectorySource>,
    pub trajectory_mode: TrajectoryMode,
    pub ground_truth: Option<GroundTruth>,
    pub created_at: String,
    pub updated_at: String,
}

impl TestCaseBuilderSession {
    /// Create a new empty builder session.
    pub fn new(session_id: impl Into<String>, input: impl Into<String>) -> Self {
        let now = now_rfc3339();
        Self {
            session_id: session_id.into(),
            input: input.into(),
            tags: Vec::new(),
            trajectory_steps: Vec::new(),
            trajectory_sources: Vec::new(),
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Construct a session with an explicit trajectory mode.
    pub fn with_mode(mut self, mode: TrajectoryMode) -> Self {
        self.trajectory_mode = mode;
        self
    }

    /// Session identity.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Current authored input.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Current authored input, treating blank/whitespace-only input as absent.
    pub fn input_if_nonempty(&self) -> Option<&str> {
        let input = self.input.trim();
        if input.is_empty() {
            None
        } else {
            Some(self.input())
        }
    }

    /// Replace authored input.
    pub fn set_input(&mut self, input: impl Into<String>) {
        let input = input.into();
        if self.input != input {
            self.input = input;
            self.touch();
        }
    }

    /// Add a trajectory step at the end.
    ///
    /// Returns `Err` if the tool name is empty or the trajectory is full.
    pub fn add_step(
        &mut self,
        tool_name: impl Into<String>,
        source: TrajectoryStepSource,
    ) -> Result<(), TestCaseBuilderError> {
        let tool_name = tool_name.into();
        if tool_name.trim().is_empty() {
            return Err(TestCaseBuilderError::EmptyToolName);
        }
        if self.trajectory_steps.len() + 1 > MAX_TRAJECTORY_LENGTH {
            return Err(TestCaseBuilderError::TrajectoryTooLong {
                count: self.trajectory_steps.len() + 1,
                max: MAX_TRAJECTORY_LENGTH,
            });
        }
        let position = self.trajectory_steps.len();
        self.trajectory_steps.push(TrajectoryStep {
            tool_name,
            source,
            position,
        });
        self.touch();
        Ok(())
    }

    /// Remove a step by position, reindexing remaining steps.
    pub fn remove_step(&mut self, position: usize) -> Result<TrajectoryStep, TestCaseBuilderError> {
        if position >= self.trajectory_steps.len() {
            return Err(TestCaseBuilderError::PositionOutOfBounds {
                position,
                len: self.trajectory_steps.len(),
            });
        }
        let removed = self.trajectory_steps.remove(position);
        self.renumber_positions();
        self.touch();
        Ok(removed)
    }

    /// Finalize with validation — returns applicative errors on failure.
    ///
    /// Validates input, tags, and trajectory before producing `AuthoredTestCase`.
    /// Tags are normalized (trimmed, deduped). The trajectory mode is validated
    /// structurally (it's already a typed `TrajectoryMode`, so no string round-trip).
    pub fn finalize(
        self,
        id: TestCaseId,
    ) -> Result<AuthoredTestCase, Vec<TestCaseValidationError>> {
        let session_id = self.session_id.clone();
        let ground_truth = self.ground_truth.clone();
        self.finalize_with(
            id,
            FinalizeAuthoredTestCaseOptions {
                status: TestCaseStatus::Draft,
                structured_ground_truth: ground_truth,
                source_thread_id: None,
                source_session_id: Some(session_id),
            },
        )
    }

    /// Finalize with validation using explicit authored-test-case overrides.
    ///
    /// This keeps the builder-session invariants in one place while letting
    /// consuming applications choose final status, resolved ground truth, and
    /// provenance linkage without rebuilding the authored struct manually.
    pub fn finalize_with(
        self,
        id: TestCaseId,
        options: FinalizeAuthoredTestCaseOptions,
    ) -> Result<AuthoredTestCase, Vec<TestCaseValidationError>> {
        let tool_names: Vec<String> = self
            .trajectory_steps
            .iter()
            .map(|s| s.tool_name.clone())
            .collect();
        // TrajectoryMode is already typed — no string round-trip needed.
        // We pass None for mode since it's validated at the type level.
        validate_test_case_fields(&self.input, &self.tags, &tool_names, None)?;
        let now = now_rfc3339();
        let (tags, warnings) = normalize_tags(self.tags);
        let tag_warnings: Vec<String> = warnings.iter().map(|w| w.to_string()).collect();
        Ok(AuthoredTestCase {
            id,
            input: self.input,
            status: options.status,
            tags,
            expected_trajectory: self.trajectory_steps,
            trajectory_sources: self.trajectory_sources,
            trajectory_mode: self.trajectory_mode,
            structured_ground_truth: options.structured_ground_truth,
            created_at: self.created_at,
            updated_at: now,
            source_thread_id: options.source_thread_id,
            source_session_id: options.source_session_id.or_else(|| Some(self.session_id)),
            tag_warnings,
        })
    }

    /// Insert a step at a specific position, shifting subsequent steps.
    pub fn insert_step(
        &mut self,
        position: usize,
        tool_name: impl Into<String>,
        source: TrajectoryStepSource,
    ) -> Result<(), TestCaseBuilderError> {
        let tool_name = tool_name.into();
        if tool_name.trim().is_empty() {
            return Err(TestCaseBuilderError::EmptyToolName);
        }
        if position > self.trajectory_steps.len() {
            return Err(TestCaseBuilderError::PositionOutOfBounds {
                position,
                len: self.trajectory_steps.len(),
            });
        }
        if self.trajectory_steps.len() + 1 > MAX_TRAJECTORY_LENGTH {
            return Err(TestCaseBuilderError::TrajectoryTooLong {
                count: self.trajectory_steps.len() + 1,
                max: MAX_TRAJECTORY_LENGTH,
            });
        }
        self.trajectory_steps.insert(
            position,
            TrajectoryStep {
                tool_name,
                source,
                position: 0, // will be fixed by reindex
            },
        );
        self.renumber_positions();
        self.touch();
        Ok(())
    }

    /// Move a step from one position to another.
    pub fn move_step(&mut self, from: usize, to: usize) -> Result<(), TestCaseBuilderError> {
        let len = self.trajectory_steps.len();
        if from >= len {
            return Err(TestCaseBuilderError::PositionOutOfBounds {
                position: from,
                len,
            });
        }
        if to >= len {
            return Err(TestCaseBuilderError::PositionOutOfBounds { position: to, len });
        }
        if from == to {
            return Ok(());
        }
        let step = self.trajectory_steps.remove(from);
        self.trajectory_steps.insert(to, step);
        self.renumber_positions();
        self.touch();
        Ok(())
    }

    /// Replace the entire trajectory atomically.
    ///
    /// Returns `Err` if the new trajectory exceeds `MAX_TRAJECTORY_LENGTH`
    /// or any tool name is empty. On error the existing trajectory is unchanged.
    pub fn set_trajectory(
        &mut self,
        steps: Vec<(String, TrajectoryStepSource)>,
    ) -> Result<(), TestCaseBuilderError> {
        if steps.len() > MAX_TRAJECTORY_LENGTH {
            return Err(TestCaseBuilderError::TrajectoryTooLong {
                count: steps.len(),
                max: MAX_TRAJECTORY_LENGTH,
            });
        }
        for (name, _) in &steps {
            if name.trim().is_empty() {
                return Err(TestCaseBuilderError::EmptyToolName);
            }
        }
        let new_steps: Vec<TrajectoryStep> = steps
            .into_iter()
            .enumerate()
            .map(|(i, (tool_name, source))| TrajectoryStep {
                tool_name,
                source,
                position: i,
            })
            .collect();
        if self.trajectory_steps != new_steps {
            self.trajectory_steps = new_steps;
            self.touch();
        }
        Ok(())
    }

    /// Replace the entire trajectory with already-typed steps plus sources.
    ///
    /// The steps are renumbered contiguously and validated for empty tool names
    /// and maximum trajectory length. On error, the existing trajectory is
    /// unchanged.
    pub fn replace_trajectory_steps(
        &mut self,
        mut steps: Vec<TrajectoryStep>,
        sources: Vec<TrajectorySource>,
    ) -> Result<(), TestCaseBuilderError> {
        if steps.len() > MAX_TRAJECTORY_LENGTH {
            return Err(TestCaseBuilderError::TrajectoryTooLong {
                count: steps.len(),
                max: MAX_TRAJECTORY_LENGTH,
            });
        }
        if steps.iter().any(|step| step.tool_name.trim().is_empty()) {
            return Err(TestCaseBuilderError::EmptyToolName);
        }
        for (position, step) in steps.iter_mut().enumerate() {
            step.position = position;
        }
        if self.trajectory_steps != steps || self.trajectory_sources != sources {
            self.trajectory_steps = steps;
            self.trajectory_sources = sources;
            self.touch();
        }
        Ok(())
    }

    /// Insert multiple already-typed steps at a specific position.
    ///
    /// The inserted steps are renumbered together with the existing trajectory.
    pub fn insert_steps(
        &mut self,
        position: usize,
        steps: Vec<TrajectoryStep>,
    ) -> Result<(), TestCaseBuilderError> {
        if position > self.trajectory_steps.len() {
            return Err(TestCaseBuilderError::PositionOutOfBounds {
                position,
                len: self.trajectory_steps.len(),
            });
        }
        if self.trajectory_steps.len() + steps.len() > MAX_TRAJECTORY_LENGTH {
            return Err(TestCaseBuilderError::TrajectoryTooLong {
                count: self.trajectory_steps.len() + steps.len(),
                max: MAX_TRAJECTORY_LENGTH,
            });
        }
        if steps.iter().any(|step| step.tool_name.trim().is_empty()) {
            return Err(TestCaseBuilderError::EmptyToolName);
        }
        if steps.is_empty() {
            return Ok(());
        }
        for (offset, step) in steps.into_iter().enumerate() {
            self.trajectory_steps.insert(position + offset, step);
        }
        self.renumber_positions();
        self.touch();
        Ok(())
    }

    /// Record an additional trajectory provenance source.
    pub fn push_trajectory_source(&mut self, source: TrajectorySource) {
        self.trajectory_sources.push(source);
        self.touch();
    }

    /// Set trajectory mode without modifying steps.
    ///
    /// Idempotent: does not touch `updated_at` when the mode is unchanged.
    pub fn set_mode(&mut self, mode: TrajectoryMode) {
        if self.trajectory_mode != mode {
            self.trajectory_mode = mode;
            self.touch();
        }
    }

    /// Set structured ground truth (validated).
    pub fn set_ground_truth(&mut self, gt: GroundTruth) -> Result<(), Vec<String>> {
        gt.validate()?;
        self.ground_truth = Some(gt);
        self.touch();
        Ok(())
    }

    /// Get ground truth reference.
    pub fn ground_truth(&self) -> Option<&GroundTruth> {
        self.ground_truth.as_ref()
    }

    /// Clear trajectory and sources.
    ///
    /// Idempotent: does not touch `updated_at` when already empty.
    pub fn clear_trajectory(&mut self) {
        if !self.trajectory_steps.is_empty() || !self.trajectory_sources.is_empty() {
            self.trajectory_steps.clear();
            self.trajectory_sources.clear();
            self.touch();
        }
    }

    /// Get a display summary of the current state.
    pub fn summary(&self) -> SessionSummary {
        SessionSummary {
            session_id: self.session_id.clone(),
            step_count: self.trajectory_steps.len(),
            tool_names: self
                .trajectory_steps
                .iter()
                .map(|s| s.tool_name.clone())
                .collect(),
            trajectory_mode: self.trajectory_mode,
            has_ground_truth: self.ground_truth.is_some(),
            tag_count: self.tags.len(),
        }
    }

    /// Flat tool names from trajectory steps.
    pub fn trajectory_tool_names(&self) -> Vec<&str> {
        self.trajectory_steps
            .iter()
            .map(|s| s.tool_name.as_str())
            .collect()
    }

    /// Renumber trajectory positions contiguously from `0..N`.
    pub fn renumber_positions(&mut self) {
        for (i, step) in self.trajectory_steps.iter_mut().enumerate() {
            step.position = i;
        }
    }

    /// Refresh the session update timestamp.
    pub fn touch(&mut self) {
        self.updated_at = now_rfc3339();
    }
}

// =============================================================================
// TestCaseBuilderError
// =============================================================================

/// Error type for builder session mutations.
#[derive(Debug, Clone, PartialEq)]
pub enum TestCaseBuilderError {
    /// Position is out of bounds.
    PositionOutOfBounds { position: usize, len: usize },
    /// Tool name is empty or whitespace-only.
    EmptyToolName,
    /// Trajectory exceeds maximum allowed length.
    TrajectoryTooLong { count: usize, max: usize },
}

impl std::fmt::Display for TestCaseBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PositionOutOfBounds { position, len } => {
                write!(f, "position {} out of bounds (length {})", position, len)
            }
            Self::EmptyToolName => write!(f, "tool name must not be empty"),
            Self::TrajectoryTooLong { count, max } => {
                write!(f, "trajectory length {} exceeds maximum {}", count, max)
            }
        }
    }
}

impl std::error::Error for TestCaseBuilderError {}

// =============================================================================
// SessionSummary
// =============================================================================

/// Display summary of a builder session's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub step_count: usize,
    pub tool_names: Vec<String>,
    pub trajectory_mode: TrajectoryMode,
    pub has_ground_truth: bool,
    pub tag_count: usize,
}

// =============================================================================
// Generic trace parsing for test-case authoring
// =============================================================================

/// A single tool call extracted from a chat trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEntry {
    pub index: usize,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub invocation_id: String,
    pub message_index: usize,
}

/// A `(role, content)` pair extracted from a raw JSON message.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagePair {
    pub role: String,
    pub content: String,
}

/// Convert raw JSON messages to [`MessagePair`]s for trace parsing.
///
/// For assistant messages, only canonical `toolInteractions` contribute
/// tool-trace content. Plain-text assistant `content` is ignored here.
pub fn messages_to_pairs(messages: &[serde_json::Value]) -> Vec<MessagePair> {
    messages
        .iter()
        .map(|m| {
            let role = m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if role == "assistant" {
                if let Some(interactions) = m.get("toolInteractions").and_then(|v| v.as_array()) {
                    if !interactions.is_empty() {
                        let tool_calls: Vec<serde_json::Value> = interactions
                            .iter()
                            .map(|ti| {
                                serde_json::json!({
                                    "toolName": ti.get("toolName").and_then(|v| v.as_str()).unwrap_or(""),
                                    "invocationId": ti.get("callId").and_then(|v| v.as_str()).unwrap_or(""),
                                    "args": ti.get("arguments").cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                                    "state": {
                                        "result": ti.get("result").cloned().unwrap_or(serde_json::Value::Null)
                                    }
                                })
                            })
                            .collect();
                        return MessagePair {
                            role,
                            content: serde_json::to_string(&tool_calls).unwrap_or_default(),
                        };
                    }
                }
                return MessagePair {
                    role,
                    content: String::new(),
                };
            }

            let content = m
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MessagePair { role, content }
        })
        .collect()
}

/// Convenience: extract tool calls directly from raw JSON messages.
pub fn extract_tool_calls_from_raw_messages(messages: &[serde_json::Value]) -> Vec<ToolCallEntry> {
    let pairs = messages_to_pairs(messages);
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|p| (p.role.as_str(), p.content.as_str()))
        .collect();
    extract_tool_calls_from_messages(&refs)
}

/// Parse assistant message content for tool invocations.
///
/// Accepts `(role, content)` pairs to keep the parsing contract independent of
/// any app-specific message DTOs. Deduplicates by `invocation_id`.
pub fn extract_tool_calls_from_messages(messages: &[(&str, &str)]) -> Vec<ToolCallEntry> {
    let mut entries: Vec<ToolCallEntry> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut global_index = 0usize;

    for (msg_idx, (role, content)) in messages.iter().enumerate() {
        if *role != "assistant" {
            continue;
        }
        if !parse_json_array_tool_calls(
            content,
            msg_idx,
            &mut entries,
            &mut seen_ids,
            &mut global_index,
        ) {
            parse_sse_tool_calls(
                content,
                msg_idx,
                &mut entries,
                &mut seen_ids,
                &mut global_index,
            );
        }
    }

    entries
}

fn parse_json_array_tool_calls(
    content: &str,
    msg_idx: usize,
    entries: &mut Vec<ToolCallEntry>,
    seen_ids: &mut HashSet<String>,
    global_index: &mut usize,
) -> bool {
    let parts = match serde_json::from_str::<Vec<serde_json::Value>>(content) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let mut found = false;
    for part in &parts {
        let Some(tool_name) = part.get("toolName").and_then(|v| v.as_str()) else {
            continue;
        };
        let invocation_id = part
            .get("invocationId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("anon-{}", *global_index));

        if seen_ids.contains(&invocation_id) {
            if let Some(result) = part.get("state").and_then(|s| s.get("result")) {
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|e| e.invocation_id == invocation_id)
                {
                    entry.result = Some(result.clone());
                }
            }
            found = true;
            continue;
        }

        let args = part
            .get("args")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let result = part.get("state").and_then(|s| s.get("result")).cloned();

        seen_ids.insert(invocation_id.clone());
        entries.push(ToolCallEntry {
            index: *global_index,
            tool_name: tool_name.to_string(),
            args,
            result,
            invocation_id,
            message_index: msg_idx,
        });
        *global_index += 1;
        found = true;
    }
    found
}

fn parse_sse_tool_calls(
    content: &str,
    msg_idx: usize,
    entries: &mut Vec<ToolCallEntry>,
    seen_ids: &mut HashSet<String>,
    global_index: &mut usize,
) {
    for line in content.lines() {
        let json_str = line.strip_prefix("data: ").unwrap_or(line);

        let part = match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(p) => p,
            Err(_) => {
                if !json_str.is_empty() && json_str != "[DONE]" {
                    tracing::debug!(
                        line = &json_str[..json_str.len().min(80)],
                        msg_idx,
                        "skipped unparseable SSE line in tool call extraction"
                    );
                }
                continue;
            }
        };

        if part.get("type").and_then(|v| v.as_str()) != Some("tool-invocation") {
            continue;
        }
        let Some(tool_name) = part.get("toolName").and_then(|v| v.as_str()) else {
            continue;
        };
        let invocation_id = part
            .get("toolInvocationId")
            .or_else(|| part.get("toolCallId"))
            .or_else(|| part.get("invocationId"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("anon-{}", *global_index));
        let result = part.get("result").cloned();

        if seen_ids.contains(&invocation_id) {
            if let Some(result) = result {
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|entry| entry.invocation_id == invocation_id)
                {
                    entry.result = Some(result);
                }
            }
            continue;
        }

        let args = part
            .get("args")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        seen_ids.insert(invocation_id.clone());
        entries.push(ToolCallEntry {
            index: *global_index,
            tool_name: tool_name.to_string(),
            args,
            result,
            invocation_id,
            message_index: msg_idx,
        });
        *global_index += 1;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_status_default_is_draft() {
        assert_eq!(TestCaseStatus::default(), TestCaseStatus::Draft);
    }

    #[test]
    fn extract_tool_calls_from_messages_supports_canonical_sse_tool_invocations() {
        let messages = [(
            "assistant",
            r#"data: {"type":"tool-invocation","toolName":"draft_plan","toolInvocationId":"call-1","args":{"products":{}},"state":"call"}
data: {"type":"tool-invocation","toolName":"draft_plan","toolInvocationId":"call-1","args":{"products":{}},"state":"result","result":{"planId":"plan-1"}}
"#,
        )];

        let calls = extract_tool_calls_from_messages(&messages);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "draft_plan");
        assert_eq!(calls[0].invocation_id, "call-1");
        assert_eq!(
            calls[0].result,
            Some(serde_json::json!({"planId":"plan-1"}))
        );
    }

    #[test]
    fn test_case_status_from_str_roundtrip() {
        for status in [
            TestCaseStatus::Draft,
            TestCaseStatus::Active,
            TestCaseStatus::Archived,
        ] {
            assert_eq!(status.as_str().parse::<TestCaseStatus>(), Ok(status));
        }
    }

    #[test]
    fn test_case_status_from_str_unknown() {
        assert!("bogus".parse::<TestCaseStatus>().is_err());
    }

    #[test]
    fn normalize_tags_dedup_and_trim() {
        let tags = vec![
            " Foo ".into(),
            "bar".into(),
            "FOO".into(), // duplicate after case-insensitive dedup
            "baz".into(),
        ];
        let (result, warnings) = normalize_tags(tags);
        // Preserves original casing of first occurrence
        assert_eq!(result, vec!["Foo", "bar", "baz"]);
        assert_eq!(warnings.len(), 1); // "FOO" duplicate
        assert!(
            matches!(&warnings[0], TagWarning::Duplicate { tag, kept } if tag == "FOO" && kept == "Foo")
        );
    }

    #[test]
    fn normalize_tags_preserves_case() {
        let tags = vec!["MyTag".into(), "UPPER".into(), "lower".into()];
        let (result, warnings) = normalize_tags(tags);
        assert_eq!(result, vec!["MyTag", "UPPER", "lower"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn normalize_tags_first_occurrence_wins() {
        let tags = vec!["Forecast".into(), "forecast".into(), "FORECAST".into()];
        let (result, warnings) = normalize_tags(tags);
        assert_eq!(result, vec!["Forecast"]);
        assert_eq!(warnings.len(), 2); // "forecast" + "FORECAST" duplicates
    }

    #[test]
    fn normalize_tags_enforces_limit() {
        let tags: Vec<String> = (0..30).map(|i| format!("tag-{}", i)).collect();
        let (result, warnings) = normalize_tags(tags);
        assert_eq!(result.len(), MAX_TAGS);
        // Should have a TruncatedAtMax warning
        assert!(warnings
            .iter()
            .any(|w| matches!(w, TagWarning::TruncatedAtMax { .. })));
    }

    #[test]
    fn normalize_tags_rejects_empty_and_long() {
        let long_tag = "a".repeat(MAX_TAG_LENGTH + 1);
        let tags = vec!["".into(), "  ".into(), long_tag.clone(), "valid".into()];
        let (result, warnings) = normalize_tags(tags);
        assert_eq!(result, vec!["valid"]);
        assert_eq!(warnings.len(), 3); // 2 empty + 1 too long
        assert!(warnings
            .iter()
            .any(|w| matches!(w, TagWarning::Empty { .. })));
        assert!(warnings
            .iter()
            .any(|w| matches!(w, TagWarning::TooLong { .. })));
    }

    #[test]
    fn validate_trajectory_all_valid() {
        let catalog = VecToolCatalog::new(vec!["draft_plan".into(), "approve_plan".into()]);
        let trajectory = vec!["draft_plan".into(), "approve_plan".into()];
        assert!(validate_trajectory(&trajectory, &catalog).is_empty());
    }

    #[test]
    fn validate_trajectory_invalid_names() {
        let catalog = VecToolCatalog::new(vec!["draft_plan".into()]);
        let trajectory = vec!["draft_plan".into(), "unknownTool".into()];
        let invalid = validate_trajectory(&trajectory, &catalog);
        assert_eq!(invalid, vec!["unknownTool"]);
    }

    #[test]
    fn vec_tool_catalog_all_names() {
        let catalog = VecToolCatalog::new(vec!["a".into(), "b".into()]);
        let names = catalog.all_tool_names();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn builder_session_add_remove_steps() {
        let mut session = TestCaseBuilderSession::new("s-1", "test input");
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step(
                "approve_plan",
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        assert_eq!(session.trajectory_steps.len(), 2);
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].position, 1);

        let removed = session.remove_step(0).unwrap();
        assert_eq!(removed.tool_name, "draft_plan");
        assert_eq!(session.trajectory_steps.len(), 1);
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[0].tool_name, "approve_plan");
    }

    #[test]
    fn builder_session_remove_out_of_bounds() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        assert!(session.remove_step(0).is_err());
    }

    #[test]
    fn builder_session_finalize() {
        let mut session = TestCaseBuilderSession::new("s-1", "test input");
        session.tags = vec!["tag1".into(), "tag2".into()];
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.trajectory_mode = TrajectoryMode::Strict;

        let tc = session.finalize(TestCaseId::new_unchecked("tc-1")).unwrap();
        assert_eq!(tc.id.as_str(), "tc-1");
        assert_eq!(tc.input, "test input");
        assert_eq!(tc.status, TestCaseStatus::Draft);
        assert_eq!(tc.tags, vec!["tag1", "tag2"]);
        assert_eq!(tc.expected_trajectory.len(), 1);
        assert_eq!(tc.trajectory_mode, TrajectoryMode::Strict);
    }

    #[test]
    fn builder_session_finalize_with_overrides_status_ground_truth_and_provenance() {
        let mut session = TestCaseBuilderSession::new("s-1", "test input");
        session.tags = vec!["tag1".into(), "tag1".into(), "".into()];
        session.ground_truth = Some(GroundTruth::text("session truth").unwrap());
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();

        let tc = session
            .finalize_with(
                TestCaseId::new_unchecked("tc-1"),
                FinalizeAuthoredTestCaseOptions {
                    status: TestCaseStatus::Active,
                    structured_ground_truth: Some(GroundTruth::text("override truth").unwrap()),
                    source_thread_id: Some("thread-alpha".into()),
                    source_session_id: Some("session-override".into()),
                },
            )
            .unwrap();

        assert_eq!(tc.status, TestCaseStatus::Active);
        assert_eq!(tc.source_thread_id.as_deref(), Some("thread-alpha"));
        assert_eq!(tc.source_session_id.as_deref(), Some("session-override"));
        assert_eq!(
            tc.structured_ground_truth,
            Some(GroundTruth::text("override truth").unwrap())
        );
        assert_eq!(tc.tags, vec!["tag1"]);
        assert_eq!(tc.tag_warnings.len(), 2);
    }

    #[test]
    fn builder_session_finalize_defaults_source_session_and_ground_truth() {
        let mut session = TestCaseBuilderSession::new("session-1", "test input");
        session.ground_truth = Some(GroundTruth::text("session truth").unwrap());
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();

        let tc = session.finalize(TestCaseId::new_unchecked("tc-1")).unwrap();

        assert_eq!(tc.status, TestCaseStatus::Draft);
        assert_eq!(tc.source_thread_id, None);
        assert_eq!(tc.source_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            tc.structured_ground_truth,
            Some(GroundTruth::text("session truth").unwrap())
        );
    }

    #[test]
    fn authored_test_case_from_canonicalized_parts_normalizes_tags_and_defaults_timestamps() {
        let tc = AuthoredTestCase::from_canonicalized_parts(
            TestCaseId::new_unchecked("tc-1"),
            "test input",
            manual_trajectory_steps(["draft_plan"]),
            ComposeAuthoredTestCaseOptions {
                tags: vec!["tag1".into(), "tag1".into(), "".into()],
                ..Default::default()
            },
        );

        assert_eq!(tc.id.as_str(), "tc-1");
        assert_eq!(tc.status, TestCaseStatus::Draft);
        assert_eq!(tc.tags, vec!["tag1"]);
        assert_eq!(tc.tag_warnings.len(), 2);
        assert_eq!(tc.created_at, tc.updated_at);
        assert_eq!(tc.source_thread_id, None);
        assert_eq!(tc.source_session_id, None);
        assert_eq!(tc.trajectory_mode, TrajectoryMode::Unordered);
    }

    #[test]
    fn authored_test_case_from_canonicalized_parts_preserves_overrides() {
        let tc = AuthoredTestCase::from_canonicalized_parts(
            TestCaseId::new_unchecked("tc-1"),
            "test input",
            manual_trajectory_steps(["draft_plan"]),
            ComposeAuthoredTestCaseOptions {
                status: TestCaseStatus::Active,
                tags: vec!["tag1".into()],
                trajectory_sources: vec![TrajectorySource::ThreadSegment {
                    thread_id: "thread-alpha".into(),
                    from_index: 0,
                    to_index: 1,
                }],
                trajectory_mode: TrajectoryMode::Strict,
                structured_ground_truth: Some(GroundTruth::text("expected").unwrap()),
                source_thread_id: Some("thread-alpha".into()),
                source_session_id: Some("session-alpha".into()),
                created_at: Some("2025-01-01T00:00:00Z".into()),
                updated_at: Some("2025-01-02T00:00:00Z".into()),
            },
        );

        assert_eq!(tc.status, TestCaseStatus::Active);
        assert_eq!(tc.tags, vec!["tag1"]);
        assert!(tc.tag_warnings.is_empty());
        assert_eq!(tc.trajectory_sources.len(), 1);
        assert_eq!(tc.trajectory_mode, TrajectoryMode::Strict);
        assert_eq!(
            tc.structured_ground_truth,
            Some(GroundTruth::text("expected").unwrap())
        );
        assert_eq!(tc.source_thread_id.as_deref(), Some("thread-alpha"));
        assert_eq!(tc.source_session_id.as_deref(), Some("session-alpha"));
        assert_eq!(tc.created_at, "2025-01-01T00:00:00Z");
        assert_eq!(tc.updated_at, "2025-01-02T00:00:00Z");
    }

    #[test]
    fn authored_test_case_to_eval_test_case() {
        let tc = AuthoredTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            input: "test query".into(),
            status: TestCaseStatus::Active,
            tags: vec!["forecast".into()],
            expected_trajectory: manual_trajectory_steps(["draft_plan", "approve_plan"]),
            trajectory_sources: Vec::new(),
            trajectory_mode: TrajectoryMode::Unordered,
            structured_ground_truth: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            source_thread_id: None,
            source_session_id: None,
            tag_warnings: Vec::new(),
        };

        let eval_tc = tc.to_eval_test_case();
        assert_eq!(eval_tc.id.as_str(), "tc-1");
        assert_eq!(eval_tc.input, "test query");
        assert_eq!(
            eval_tc.expected_trajectory,
            vec!["draft_plan", "approve_plan"]
        );
        assert_eq!(eval_tc.trajectory_mode, TrajectoryMode::Unordered);
        assert!(eval_tc.ground_truth.is_none());
    }

    #[test]
    fn authored_test_case_serde_roundtrip() {
        let tc = AuthoredTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            input: "test query".into(),
            status: TestCaseStatus::Active,
            tags: vec!["forecast".into()],
            expected_trajectory: manual_trajectory_steps(["draft_plan"]),
            trajectory_sources: vec![TrajectorySource::Manual { reason: None }],
            trajectory_mode: TrajectoryMode::Unordered,
            structured_ground_truth: Some(GroundTruth::text("expected output").unwrap()),
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            source_thread_id: None,
            source_session_id: None,
            tag_warnings: Vec::new(),
        };

        let json = serde_json::to_string(&tc).unwrap();
        let parsed: AuthoredTestCase = serde_json::from_str(&json).unwrap();
        assert_eq!(tc, parsed);
    }

    #[test]
    fn authored_test_case_deserializes_flat_tool_names_into_manual_steps() {
        let json = serde_json::json!({
            "id": "tc-1",
            "input": "test query",
            "status": "active",
            "tags": ["forecast"],
            "expectedTrajectory": ["draft_plan", "approve_plan"],
            "trajectoryMode": "anyOrder",
            "createdAt": "2025-01-01T00:00:00Z",
            "updatedAt": "2025-01-01T00:00:00Z"
        });

        let parsed: AuthoredTestCase = serde_json::from_value(json).unwrap();
        assert_eq!(
            parsed.trajectory_tool_names(),
            vec!["draft_plan", "approve_plan"]
        );
        assert!(matches!(
            parsed.expected_trajectory[0].source,
            TrajectoryStepSource::Manual { reason: None }
        ));
        assert_eq!(parsed.expected_trajectory[1].position, 1);
    }

    #[test]
    fn authored_test_case_trajectory_tool_names_owned() {
        let tc = AuthoredTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            input: "test query".into(),
            status: TestCaseStatus::Active,
            tags: vec!["forecast".into()],
            expected_trajectory: manual_trajectory_steps(["draft_plan", "approve_plan"]),
            trajectory_sources: vec![TrajectorySource::Manual { reason: None }],
            trajectory_mode: TrajectoryMode::Unordered,
            structured_ground_truth: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            source_thread_id: None,
            source_session_id: None,
            tag_warnings: Vec::new(),
        };

        assert_eq!(
            tc.trajectory_tool_names_owned(),
            vec!["draft_plan".to_string(), "approve_plan".to_string()]
        );
    }

    #[test]
    fn canonicalize_expected_trajectory_builds_manual_steps_when_no_provenance() {
        let steps = canonicalize_expected_trajectory(
            vec!["draft_plan".into(), "approve_plan".into()],
            vec![],
        )
        .unwrap();
        assert_eq!(steps[0].tool_name, "draft_plan");
        assert!(matches!(
            steps[0].source,
            TrajectoryStepSource::Manual { .. }
        ));
        assert_eq!(steps[1].position, 1);
    }

    #[test]
    fn canonicalize_expected_trajectory_renumbers_and_preserves_provenance() {
        let steps = canonicalize_expected_trajectory(
            vec!["draft_plan".into(), "approve_plan".into()],
            vec![
                TrajectoryStep::from_thread("draft_plan", 7, "thread-1", 3),
                TrajectoryStep::from_planner("approve_plan", 99, "run-1"),
            ],
        )
        .unwrap();
        assert_eq!(steps[0].position, 0);
        assert_eq!(steps[1].position, 1);
        assert!(matches!(
            steps[0].source,
            TrajectoryStepSource::FromThread { .. }
        ));
        assert!(matches!(
            steps[1].source,
            TrajectoryStepSource::FromPlanner { .. }
        ));
    }

    #[test]
    fn canonicalize_expected_trajectory_rejects_name_mismatch() {
        let err = canonicalize_expected_trajectory(
            vec!["draft_plan".into()],
            vec![TrajectoryStep::manual("approve_plan", 0)],
        )
        .expect_err("mismatched provenance should be rejected");
        assert!(matches!(
            err,
            TrajectoryCanonicalizationError::ToolNameMismatch { .. }
        ));
    }

    #[test]
    fn authored_test_case_to_eval_test_case_with_mapper_applies_transform() {
        let tc = AuthoredTestCase {
            id: TestCaseId::new_unchecked("tc-1"),
            input: "test query".into(),
            status: TestCaseStatus::Active,
            tags: vec!["forecast".into()],
            expected_trajectory: manual_trajectory_steps(["oldTool", "draft_plan"]),
            trajectory_sources: Vec::new(),
            trajectory_mode: TrajectoryMode::Unordered,
            structured_ground_truth: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            source_thread_id: Some("thread-alpha".into()),
            source_session_id: None,
            tag_warnings: Vec::new(),
        };

        let eval_tc = tc.to_eval_test_case_with_mapper(|tools| {
            tools
                .into_iter()
                .map(|tool| {
                    if tool == "oldTool" {
                        "newTool".into()
                    } else {
                        tool
                    }
                })
                .collect()
        });

        assert_eq!(eval_tc.expected_trajectory, vec!["newTool", "draft_plan"]);
        assert_eq!(eval_tc.source_thread_id.as_deref(), Some("thread-alpha"));
    }

    #[test]
    fn authored_test_case_draft_from_thread_trace_preserves_thread_provenance() {
        let tc = AuthoredTestCase::draft_from_thread_trace(
            TestCaseId::new_unchecked("tc-1"),
            "Find forecast opportunities".to_string(),
            "thread-alpha",
            &[ToolCallEntry {
                index: 7,
                tool_name: "search_entities".to_string(),
                args: serde_json::json!({ "query": "forecast" }),
                result: Some(serde_json::json!({ "hits": 3 })),
                invocation_id: "call-1".to_string(),
                message_index: 1,
            }],
        )
        .expect("thread trace draft");

        assert_eq!(tc.input, "Find forecast opportunities");
        assert_eq!(tc.status, TestCaseStatus::Draft);
        assert_eq!(tc.trajectory_mode, TrajectoryMode::Strict);
        assert_eq!(tc.source_thread_id.as_deref(), Some("thread-alpha"));
        assert_eq!(tc.trajectory_sources.len(), 1);
        assert_eq!(tc.expected_trajectory.len(), 1);
        assert_eq!(tc.expected_trajectory[0].tool_name, "search_entities");
        assert!(matches!(
            tc.trajectory_sources[0],
            TrajectorySource::ThreadSegment {
                ref thread_id,
                from_index: 0,
                to_index: 1
            } if thread_id == "thread-alpha"
        ));
    }

    #[test]
    fn authored_test_case_draft_from_thread_trace_rejects_empty_tool_trace() {
        let err = AuthoredTestCase::draft_from_thread_trace(
            TestCaseId::new_unchecked("tc-1"),
            "Find forecast opportunities".to_string(),
            "thread-alpha",
            &[],
        )
        .expect_err("empty traces should be rejected");

        assert_eq!(err, ThreadTraceDraftError::EmptyToolTrace);
    }

    // =========================================================================
    // Trajectory Newtypes
    // =========================================================================

    #[test]
    fn baseline_trajectory_new_and_tools() {
        let bt = BaselineTrajectory::new(vec![
            "draft_plan".into(),
            "approve_plan".into(),
            "unknown".into(),
        ]);
        assert_eq!(bt.tools().len(), 3);
    }

    #[test]
    fn baseline_trajectory_remap_filters_invalid() {
        let catalog = VecToolCatalog::new(vec!["draft_plan".into(), "approve_plan".into()]);
        let bt = BaselineTrajectory::new(vec![
            "draft_plan".into(),
            "unknownTool".into(),
            "approve_plan".into(),
        ]);
        let (remapped, dropped) = bt.remap(&catalog);
        assert_eq!(remapped.tools(), &["draft_plan", "approve_plan"]);
        assert_eq!(dropped, vec!["unknownTool"]);
    }

    #[test]
    fn baseline_trajectory_remap_empty_catalog() {
        let catalog = VecToolCatalog::new(vec![]);
        let bt = BaselineTrajectory::new(vec!["draft_plan".into()]);
        let (remapped, dropped) = bt.remap(&catalog);
        assert!(remapped.tools().is_empty());
        assert_eq!(dropped, vec!["draft_plan"]);
    }

    #[test]
    fn remapped_trajectory_into_tools() {
        let catalog = VecToolCatalog::new(vec!["a".into(), "b".into()]);
        let bt = BaselineTrajectory::new(vec!["a".into(), "b".into()]);
        let (remapped, dropped) = bt.remap(&catalog);
        let tools = remapped.into_tools();
        assert_eq!(tools, vec!["a", "b"]);
        assert!(dropped.is_empty());
    }

    #[test]
    fn baseline_trajectory_serde_roundtrip() {
        let bt = BaselineTrajectory::new(vec!["draft_plan".into(), "approve_plan".into()]);
        let json = serde_json::to_string(&bt).unwrap();
        let parsed: BaselineTrajectory = serde_json::from_str(&json).unwrap();
        assert_eq!(bt, parsed);
    }

    #[test]
    fn remapped_trajectory_serializes() {
        let catalog = VecToolCatalog::new(vec!["draft_plan".into()]);
        let bt = BaselineTrajectory::new(vec!["draft_plan".into()]);
        let (remapped, _dropped) = bt.remap(&catalog);
        let json = serde_json::to_string(&remapped).unwrap();
        assert!(json.contains("draft_plan"));
        // RemappedTrajectory intentionally does not implement Deserialize
        // to enforce construction only via BaselineTrajectory::remap().
    }

    #[test]
    fn trajectory_step_source_serde() {
        let sources = vec![
            TrajectoryStepSource::FromThread {
                thread_id: "t-1".into(),
                original_index: 5,
            },
            TrajectoryStepSource::FromPlanner {
                run_id: "r-1".into(),
            },
            TrajectoryStepSource::Manual { reason: None },
        ];

        for source in &sources {
            let json = serde_json::to_string(source).unwrap();
            let parsed: TrajectoryStepSource = serde_json::from_str(&json).unwrap();
            assert_eq!(source, &parsed);
        }
    }

    // =========================================================================
    // Validation tests
    // =========================================================================

    #[test]
    fn validation_rejects_empty_input() {
        let result = validate_test_case_fields("", &[], &[], None);
        let errors = result.unwrap_err();
        assert!(errors.contains(&TestCaseValidationError::InputEmpty));
    }

    #[test]
    fn validation_rejects_whitespace_only_input() {
        let result = validate_test_case_fields("   \n\t  ", &[], &[], None);
        let errors = result.unwrap_err();
        assert!(errors.contains(&TestCaseValidationError::InputEmpty));
    }

    #[test]
    fn validation_rejects_over_length_input() {
        let long_input = "x".repeat(MAX_INPUT_LENGTH + 1);
        let result = validate_test_case_fields(&long_input, &[], &[], None);
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, TestCaseValidationError::InputTooLong { .. })));
    }

    #[test]
    fn validation_rejects_too_many_tags() {
        let tags: Vec<String> = (0..MAX_TAGS + 1).map(|i| format!("tag-{}", i)).collect();
        let result = validate_test_case_fields("valid input", &tags, &[], None);
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, TestCaseValidationError::TooManyTags { .. })));
    }

    #[test]
    fn validation_rejects_long_tag() {
        let long_tag = "a".repeat(MAX_TAG_LENGTH + 1);
        let tags = vec![long_tag.clone()];
        let result = validate_test_case_fields("valid input", &tags, &[], None);
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, TestCaseValidationError::TagTooLong { .. })));
    }

    #[test]
    fn validation_rejects_empty_trajectory_step() {
        let tools = vec!["draft_plan".into(), "".into(), "approve_plan".into()];
        let result = validate_test_case_fields("valid input", &[], &tools, None);
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, TestCaseValidationError::TrajectoryStepEmpty { index: 1 })));
    }

    #[test]
    fn validation_rejects_invalid_trajectory_mode() {
        let result = validate_test_case_fields("valid input", &[], &[], Some("bogusMode"));
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, TestCaseValidationError::InvalidTrajectoryMode(_))));
    }

    #[test]
    fn validation_is_applicative_collects_multiple_errors() {
        let long_tag = "a".repeat(MAX_TAG_LENGTH + 1);
        let tags = vec![long_tag];
        let tools = vec!["".into()];
        let result = validate_test_case_fields("", &tags, &tools, Some("bad"));
        let errors = result.unwrap_err();
        // Should have at least: InputEmpty, TagTooLong, TrajectoryStepEmpty, InvalidTrajectoryMode
        assert!(
            errors.len() >= 4,
            "expected >= 4 errors, got {}: {:?}",
            errors.len(),
            errors
        );
    }

    #[test]
    fn validation_accepts_valid_input() {
        let tags = vec!["forecast".into(), "demo".into()];
        let tools = vec!["draft_plan".into(), "approve_plan".into()];
        let result =
            validate_test_case_fields("Show me forecast data", &tags, &tools, Some("inOrder"));
        assert!(result.is_ok());
    }

    #[test]
    fn finalize_rejects_empty_input() {
        let session = TestCaseBuilderSession::new("s-1", "");
        let result = session.finalize(TestCaseId::new_unchecked("tc-1"));
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.contains(&TestCaseValidationError::InputEmpty));
    }

    #[test]
    fn finalize_succeeds_with_valid_session() {
        let mut session = TestCaseBuilderSession::new("s-1", "valid input");
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        let result = session.finalize(TestCaseId::new_unchecked("tc-1"));
        assert!(result.is_ok());
        let tc = result.unwrap();
        assert_eq!(tc.input, "valid input");
        assert_eq!(tc.expected_trajectory.len(), 1);
    }

    #[test]
    fn finalize_surfaces_tag_warnings() {
        let mut session = TestCaseBuilderSession::new("s-1", "valid input");
        session.tags = vec![
            "valid".into(),
            "".into(),      // empty → warning
            "VALID".into(), // duplicate of "valid" → warning
        ];
        let tc = session.finalize(TestCaseId::new_unchecked("tc-1")).unwrap();
        assert_eq!(tc.tags, vec!["valid"]);
        assert_eq!(tc.tag_warnings.len(), 2);
        assert!(tc.tag_warnings[0].contains("empty tag dropped"));
        assert!(tc.tag_warnings[1].contains("duplicate"));
    }

    #[test]
    fn finalize_no_warnings_when_tags_clean() {
        let mut session = TestCaseBuilderSession::new("s-1", "valid input");
        session.tags = vec!["forecast".into(), "demo".into()];
        let tc = session.finalize(TestCaseId::new_unchecked("tc-1")).unwrap();
        assert!(tc.tag_warnings.is_empty());
    }

    // =========================================================================
    // Phase 2: Extended builder session methods
    // =========================================================================

    #[test]
    fn insert_step_at_beginning() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .insert_step(
                0,
                "query_data",
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        assert_eq!(session.trajectory_steps.len(), 2);
        assert_eq!(session.trajectory_steps[0].tool_name, "query_data");
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].tool_name, "draft_plan");
        assert_eq!(session.trajectory_steps[1].position, 1);
    }

    #[test]
    fn insert_step_at_end() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("query_data", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .insert_step(
                1,
                "draft_plan",
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        assert_eq!(session.trajectory_steps[1].tool_name, "draft_plan");
    }

    #[test]
    fn insert_step_out_of_bounds() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let result = session.insert_step(
            5,
            "draft_plan",
            TrajectoryStepSource::Manual { reason: None },
        );
        assert!(matches!(
            result,
            Err(TestCaseBuilderError::PositionOutOfBounds {
                position: 5,
                len: 0
            })
        ));
    }

    #[test]
    fn insert_step_empty_tool_name_rejected() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let result = session.insert_step(0, "", TrajectoryStepSource::Manual { reason: None });
        assert!(matches!(result, Err(TestCaseBuilderError::EmptyToolName)));
    }

    #[test]
    fn insert_then_remove_is_identity() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step(
                "approve_plan",
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        let original_names: Vec<String> = session
            .trajectory_steps
            .iter()
            .map(|s| s.tool_name.clone())
            .collect();

        session
            .insert_step(
                1,
                "get_column_enums",
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        session.remove_step(1).unwrap();

        let names: Vec<String> = session
            .trajectory_steps
            .iter()
            .map(|s| s.tool_name.clone())
            .collect();
        assert_eq!(original_names, names);
    }

    #[test]
    fn move_step_forward() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("a", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("b", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("c", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.move_step(0, 2).unwrap();
        let names: Vec<&str> = session.trajectory_tool_names();
        assert_eq!(names, vec!["b", "c", "a"]);
        // Positions are reindexed
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].position, 1);
        assert_eq!(session.trajectory_steps[2].position, 2);
    }

    #[test]
    fn move_step_backward() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("a", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("b", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("c", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.move_step(2, 0).unwrap();
        let names: Vec<&str> = session.trajectory_tool_names();
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[test]
    fn move_step_same_position_is_noop() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("a", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("b", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        let ts_before = session.updated_at.clone();
        session.move_step(1, 1).unwrap();
        // No touch() called on noop
        assert_eq!(session.updated_at, ts_before);
    }

    #[test]
    fn move_step_roundtrip_is_identity() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("a", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("b", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("c", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.move_step(0, 2).unwrap();
        session.move_step(2, 0).unwrap();
        let names: Vec<&str> = session.trajectory_tool_names();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn move_step_out_of_bounds() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("a", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        assert!(matches!(
            session.move_step(0, 5),
            Err(TestCaseBuilderError::PositionOutOfBounds { .. })
        ));
        assert!(matches!(
            session.move_step(5, 0),
            Err(TestCaseBuilderError::PositionOutOfBounds { .. })
        ));
    }

    #[test]
    fn set_trajectory_replaces_all() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("old", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .set_trajectory(vec![
                (
                    "query_data".into(),
                    TrajectoryStepSource::Manual { reason: None },
                ),
                (
                    "draft_plan".into(),
                    TrajectoryStepSource::Manual { reason: None },
                ),
            ])
            .unwrap();
        assert_eq!(session.trajectory_steps.len(), 2);
        assert_eq!(session.trajectory_steps[0].tool_name, "query_data");
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].tool_name, "draft_plan");
        assert_eq!(session.trajectory_steps[1].position, 1);
    }

    #[test]
    fn replace_trajectory_steps_replaces_steps_and_sources() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .replace_trajectory_steps(
                vec![
                    TrajectoryStep::manual("query_data", 7),
                    TrajectoryStep::manual("draft_plan", 9),
                ],
                vec![TrajectorySource::manual()],
            )
            .unwrap();

        assert_eq!(session.trajectory_steps.len(), 2);
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].position, 1);
        assert_eq!(session.trajectory_sources, vec![TrajectorySource::manual()]);
    }

    #[test]
    fn insert_steps_inserts_and_renumbers() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("query_data", TrajectoryStepSource::manual())
            .unwrap();
        session
            .add_step("approve_plan", TrajectoryStepSource::manual())
            .unwrap();

        session
            .insert_steps(
                1,
                vec![TrajectoryStep::from_thread("draft_plan", 99, "thread-1", 3)],
            )
            .unwrap();

        assert_eq!(
            session
                .trajectory_steps
                .iter()
                .map(|step| step.tool_name.as_str())
                .collect::<Vec<_>>(),
            vec!["query_data", "draft_plan", "approve_plan"]
        );
        assert_eq!(session.trajectory_steps[1].position, 1);
    }

    #[test]
    fn push_trajectory_source_appends_source() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session.push_trajectory_source(TrajectorySource::manual());
        assert_eq!(session.trajectory_sources, vec![TrajectorySource::manual()]);
    }

    #[test]
    fn set_mode_updates_mode() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        assert_eq!(session.trajectory_mode, TrajectoryMode::Unordered);
        session.set_mode(TrajectoryMode::Strict);
        assert_eq!(session.trajectory_mode, TrajectoryMode::Strict);
    }

    #[test]
    fn with_mode_sets_mode_without_extra_ceremony() {
        let session = TestCaseBuilderSession::new("s-1", "input").with_mode(TrajectoryMode::Strict);
        assert_eq!(session.trajectory_mode, TrajectoryMode::Strict);
    }

    #[test]
    fn input_if_nonempty_ignores_blank_input() {
        let blank = TestCaseBuilderSession::new("s-1", "   ");
        assert_eq!(blank.input_if_nonempty(), None);

        let filled = TestCaseBuilderSession::new("s-1", "input");
        assert_eq!(filled.input_if_nonempty(), Some("input"));
    }

    #[test]
    fn set_ground_truth_stores_valid() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let gt = GroundTruth::text("expected output").unwrap();
        assert!(session.set_ground_truth(gt.clone()).is_ok());
        assert_eq!(session.ground_truth(), Some(&gt));
    }

    #[test]
    fn ground_truth_returns_none_initially() {
        let session = TestCaseBuilderSession::new("s-1", "input");
        assert!(session.ground_truth().is_none());
    }

    #[test]
    fn clear_trajectory_empties_steps_and_sources() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.trajectory_sources.push(TrajectorySource::Manual {
            reason: Some("test".into()),
        });
        session.clear_trajectory();
        assert!(session.trajectory_steps.is_empty());
        assert!(session.trajectory_sources.is_empty());
    }

    #[test]
    fn summary_returns_correct_state() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .add_step("query_data", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session
            .add_step("draft_plan", TrajectoryStepSource::Manual { reason: None })
            .unwrap();
        session.tags = vec!["forecast".into()];
        session.trajectory_mode = TrajectoryMode::Strict;

        let s = session.summary();
        assert_eq!(s.session_id, "s-1");
        assert_eq!(s.step_count, 2);
        assert_eq!(s.tool_names, vec!["query_data", "draft_plan"]);
        assert_eq!(s.trajectory_mode, TrajectoryMode::Strict);
        assert!(!s.has_ground_truth);
        assert_eq!(s.tag_count, 1);
    }

    #[test]
    fn summary_reflects_ground_truth() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session
            .set_ground_truth(GroundTruth::text("expected").unwrap())
            .unwrap();
        assert!(session.summary().has_ground_truth);
    }

    #[test]
    fn session_summary_serde_roundtrip() {
        let s = SessionSummary {
            session_id: "s-1".into(),
            step_count: 2,
            tool_names: vec!["query_data".into(), "draft_plan".into()],
            trajectory_mode: TrajectoryMode::Strict,
            has_ground_truth: true,
            tag_count: 1,
        };
        let json = serde_json::to_string(&s).unwrap();
        let parsed: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(s, parsed);
    }

    #[test]
    fn public_session_helpers_cover_identity_input_and_reindex() {
        let mut session = TestCaseBuilderSession::new("s-1", "");
        assert_eq!(session.session_id(), "s-1");
        assert_eq!(session.input(), "");

        session.set_input("draft prompt");
        assert_eq!(session.input(), "draft prompt");

        session.trajectory_steps = vec![
            TrajectoryStep::manual("query_data", 4),
            TrajectoryStep::manual("draft_plan", 9),
        ];
        session.renumber_positions();
        assert_eq!(session.trajectory_steps[0].position, 0);
        assert_eq!(session.trajectory_steps[1].position, 1);
    }

    // =========================================================================
    // P2: Algebraic gap closure tests
    // =========================================================================

    #[test]
    fn add_step_rejects_empty_tool_name() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        assert!(matches!(
            session.add_step("", TrajectoryStepSource::Manual { reason: None }),
            Err(TestCaseBuilderError::EmptyToolName)
        ));
        assert!(matches!(
            session.add_step("   ", TrajectoryStepSource::Manual { reason: None }),
            Err(TestCaseBuilderError::EmptyToolName)
        ));
        assert!(session.trajectory_steps.is_empty());
    }

    #[test]
    fn add_step_rejects_when_trajectory_full() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        for i in 0..MAX_TRAJECTORY_LENGTH {
            session
                .add_step(
                    format!("tool_{i}"),
                    TrajectoryStepSource::Manual { reason: None },
                )
                .unwrap();
        }
        assert!(matches!(
            session.add_step("one_more", TrajectoryStepSource::Manual { reason: None }),
            Err(TestCaseBuilderError::TrajectoryTooLong { .. })
        ));
    }

    #[test]
    fn insert_step_rejects_when_trajectory_full() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        for i in 0..MAX_TRAJECTORY_LENGTH {
            session
                .add_step(
                    format!("tool_{i}"),
                    TrajectoryStepSource::Manual { reason: None },
                )
                .unwrap();
        }
        assert!(matches!(
            session.insert_step(0, "overflow", TrajectoryStepSource::Manual { reason: None }),
            Err(TestCaseBuilderError::TrajectoryTooLong { .. })
        ));
    }

    #[test]
    fn set_trajectory_rejects_too_long() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let steps: Vec<_> = (0..MAX_TRAJECTORY_LENGTH + 1)
            .map(|i| {
                (
                    format!("tool_{i}"),
                    TrajectoryStepSource::Manual { reason: None },
                )
            })
            .collect();
        assert!(matches!(
            session.set_trajectory(steps),
            Err(TestCaseBuilderError::TrajectoryTooLong { .. })
        ));
    }

    #[test]
    fn set_trajectory_rejects_empty_tool_name() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let result = session.set_trajectory(vec![
            (
                "draft_plan".into(),
                TrajectoryStepSource::Manual { reason: None },
            ),
            ("".into(), TrajectoryStepSource::Manual { reason: None }),
        ]);
        assert!(matches!(result, Err(TestCaseBuilderError::EmptyToolName)));
        // Original trajectory unchanged (atomicity)
        assert!(session.trajectory_steps.is_empty());
    }

    #[test]
    fn remove_step_returns_error_when_empty() {
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        assert!(matches!(
            session.remove_step(0),
            Err(TestCaseBuilderError::PositionOutOfBounds {
                position: 0,
                len: 0
            })
        ));
    }

    #[test]
    fn set_mode_idempotent_does_not_touch() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let counter = std::sync::Arc::new(AtomicU64::new(0));
        let c = counter.clone();
        let _guard = set_test_clock(move || {
            let n = c.fetch_add(1, Ordering::SeqCst);
            format!("2026-01-01T00:00:{n:02}Z")
        });
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        session.set_mode(TrajectoryMode::Strict);
        let ts = session.updated_at.clone();
        session.set_mode(TrajectoryMode::Strict); // same mode — should not touch
        assert_eq!(session.updated_at, ts);
    }

    #[test]
    fn set_trajectory_idempotent_does_not_touch() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let counter = std::sync::Arc::new(AtomicU64::new(0));
        let c = counter.clone();
        let _guard = set_test_clock(move || {
            let n = c.fetch_add(1, Ordering::SeqCst);
            format!("2026-01-01T00:00:{n:02}Z")
        });
        let mut session = TestCaseBuilderSession::new("s-1", "input");
        let steps = vec![
            ("a".into(), TrajectoryStepSource::Manual { reason: None }),
            ("b".into(), TrajectoryStepSource::Manual { reason: None }),
        ];
        session.set_trajectory(steps.clone()).unwrap();
        let ts = session.updated_at.clone();
        session.set_trajectory(steps).unwrap(); // same trajectory — no touch
        assert_eq!(session.updated_at, ts);
    }

    // =========================================================================
    // P2-F: Hegel property-based move_step roundtrip law
    // =========================================================================

    fn session_with_n_steps(n: usize) -> TestCaseBuilderSession {
        let mut s = TestCaseBuilderSession::new("pbt-session", "property test input");
        for i in 0..n {
            s.add_step(
                format!("tool_{i}"),
                TrajectoryStepSource::Manual { reason: None },
            )
            .unwrap();
        }
        s
    }

    use hegel::generators;

    /// L1 (Move Roundtrip): `move(a,b); move(b,a) ≡ id`
    #[hegel::test]
    fn move_step_roundtrip_law(tc: hegel::TestCase) {
        let n = tc.draw(generators::integers::<usize>().min_value(2).max_value(19));
        let from = tc.draw(generators::integers::<usize>().min_value(0).max_value(19)) % n;
        let to = tc.draw(generators::integers::<usize>().min_value(0).max_value(19)) % n;
        let mut s = session_with_n_steps(n);
        let original: Vec<String> = s
            .trajectory_tool_names()
            .iter()
            .map(|t| t.to_string())
            .collect();
        s.move_step(from, to).unwrap();
        s.move_step(to, from).unwrap();
        let result: Vec<String> = s
            .trajectory_tool_names()
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(original, result);
    }

    /// L2 (Insert/Remove Identity): `insert(pos, tool); remove(pos) ≡ id` for tool names.
    #[hegel::test]
    fn insert_remove_identity_law(tc: hegel::TestCase) {
        let n = tc.draw(generators::integers::<usize>().min_value(1).max_value(14));
        let pos = tc.draw(generators::integers::<usize>().min_value(0).max_value(14)) % (n + 1);
        let mut s = session_with_n_steps(n);
        let original: Vec<String> = s
            .trajectory_tool_names()
            .iter()
            .map(|t| t.to_string())
            .collect();
        s.insert_step(
            pos,
            "INSERTED",
            TrajectoryStepSource::Manual { reason: None },
        )
        .unwrap();
        s.remove_step(pos).unwrap();
        let result: Vec<String> = s
            .trajectory_tool_names()
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(original, result);
    }

    /// L3 (Clear Idempotent): `clear; clear ≡ clear` — no observable difference.
    #[hegel::test]
    fn clear_trajectory_idempotent_law(tc: hegel::TestCase) {
        use std::sync::atomic::{AtomicU64, Ordering};
        let n = tc.draw(generators::integers::<usize>().min_value(0).max_value(9));
        let counter = std::sync::Arc::new(AtomicU64::new(0));
        let c = counter.clone();
        let _guard = set_test_clock(move || {
            let v = c.fetch_add(1, Ordering::SeqCst);
            format!("2026-01-01T00:00:{v:02}Z")
        });
        let mut s = session_with_n_steps(n);
        s.clear_trajectory();
        let ts_after_first = s.updated_at.clone();
        let steps_after_first = s.trajectory_steps.clone();
        s.clear_trajectory();
        // Second clear must not touch (idempotent)
        assert_eq!(&s.updated_at, &ts_after_first);
        assert_eq!(&s.trajectory_steps, &steps_after_first);
    }

    /// L4 (Normalize Idempotent): `normalize(normalize(tags).0).0 ≡ normalize(tags).0`
    #[hegel::test]
    fn normalize_tags_idempotent_law(tc: hegel::TestCase) {
        let tags: Vec<String> =
            tc.draw(generators::vecs(generators::from_regex("[a-zA-Z0-9_ ]{0,60}")).max_size(29));
        let (first_pass, _) = normalize_tags(tags);
        let (second_pass, warnings) = normalize_tags(first_pass.clone());
        assert_eq!(&first_pass, &second_pass);
        // Second pass on already-normalized tags must produce zero warnings.
        // If any appear, normalize_tags has a bug (generating spurious warnings on clean input).
        assert!(
            warnings.is_empty(),
            "Second normalize should produce no warnings: {:?}",
            warnings
        );
    }
}
