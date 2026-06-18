//! Default eval builder tools — the 6 interactive tools for test case authoring.
//!
//! These implement [`ToolCatalog`] and provide the reference builder workflow:
//!
//! 1. `mergeTraceSegment` — Splice tool-trace fragments into a trajectory
//! 2. `addStep`           — Add a trajectory step at a position
//! 3. `removeStep`        — Remove a trajectory step by position
//! 4. `reorderSteps`      — Reorder trajectory steps
//! 5. `setExpectedOutput`  — Set or update the ground truth
//! 6. `tagTestCase`       — Add/remove tags
//!
//! # Design
//!
//! `DefaultBuilderCatalog` implements [`ToolCatalog`] so it can be plugged
//! directly into [`TestCaseBuilderSession`] workflows. Domain-specific
//! applications can compose this with their own tools via [`ComposedCatalog`].

use std::collections::HashSet;

use crate::test_case::ToolCatalog;

// =============================================================================
// Tool Names (constants)
// =============================================================================

/// Splice tool-trace fragment(s) into the trajectory.
pub const MERGE_TRACE_SEGMENT: &str = "mergeTraceSegment";
/// Add a single step at a given position.
pub const ADD_STEP: &str = "addStep";
/// Remove a step by position index.
pub const REMOVE_STEP: &str = "removeStep";
/// Reorder steps by providing the new position sequence.
pub const REORDER_STEPS: &str = "reorderSteps";
/// Set or update the expected output / ground truth.
pub const SET_EXPECTED_OUTPUT: &str = "setExpectedOutput";
/// Add or remove tags on the test case.
pub const TAG_TEST_CASE: &str = "tagTestCase";

/// All builder tool names in a static slice.
pub const BUILDER_TOOLS: &[&str] = &[
    MERGE_TRACE_SEGMENT,
    ADD_STEP,
    REMOVE_STEP,
    REORDER_STEPS,
    SET_EXPECTED_OUTPUT,
    TAG_TEST_CASE,
];

// =============================================================================
// DefaultBuilderCatalog
// =============================================================================

/// Default tool catalog shipping the 6 builder tools.
///
/// Default builder subset that domain-specific catalogs can compose with.
#[derive(Debug, Clone)]
pub struct DefaultBuilderCatalog {
    lookup: HashSet<&'static str>,
}

impl DefaultBuilderCatalog {
    /// Create the default builder catalog with all 6 tools.
    pub fn new() -> Self {
        Self {
            lookup: BUILDER_TOOLS.iter().copied().collect(),
        }
    }
}

impl Default for DefaultBuilderCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolCatalog for DefaultBuilderCatalog {
    fn is_valid(&self, tool_name: &str) -> bool {
        self.lookup.contains(tool_name)
    }

    fn all_tool_names(&self) -> Vec<&str> {
        BUILDER_TOOLS.to_vec()
    }
}

// =============================================================================
// ComposedCatalog
// =============================================================================

/// Composition of two tool catalogs (union semantics).
///
/// `is_valid` returns `true` if either catalog accepts the name.
/// `all_tool_names` returns the merged, deduplicated list.
///
/// # Law: Union
///
/// ```text
/// ComposedCatalog(A, B).is_valid(t) ↔ A.is_valid(t) ∨ B.is_valid(t)
/// ```
pub struct ComposedCatalog<A: ToolCatalog, B: ToolCatalog> {
    left: A,
    right: B,
}

impl<A: ToolCatalog, B: ToolCatalog> ComposedCatalog<A, B> {
    /// Compose two catalogs.
    pub fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A: ToolCatalog, B: ToolCatalog> ToolCatalog for ComposedCatalog<A, B> {
    fn is_valid(&self, tool_name: &str) -> bool {
        self.left.is_valid(tool_name) || self.right.is_valid(tool_name)
    }

    fn all_tool_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.left.all_tool_names();
        let left_set: HashSet<&str> = names.iter().copied().collect();
        for name in self.right.all_tool_names() {
            if !left_set.contains(name) {
                names.push(name);
            }
        }
        names
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_case::VecToolCatalog;

    #[test]
    fn default_catalog_has_all_6_tools() {
        let catalog = DefaultBuilderCatalog::new();
        assert_eq!(catalog.all_tool_names().len(), 6);
    }

    #[test]
    fn default_catalog_validates_builder_tools() {
        let catalog = DefaultBuilderCatalog::new();
        for tool in BUILDER_TOOLS {
            assert!(catalog.is_valid(tool), "Expected {} to be valid", tool);
        }
    }

    #[test]
    fn default_catalog_rejects_unknown() {
        let catalog = DefaultBuilderCatalog::new();
        assert!(!catalog.is_valid("draft_plan"));
        assert!(!catalog.is_valid("queryData"));
    }

    #[test]
    fn composed_catalog_union() {
        let builder = DefaultBuilderCatalog::new();
        let domain = VecToolCatalog::new(vec!["draft_plan".into(), "queryData".into()]);
        let composed = ComposedCatalog::new(builder, domain);

        // Builder tools valid
        assert!(composed.is_valid(MERGE_TRACE_SEGMENT));
        assert!(composed.is_valid(ADD_STEP));

        // Domain tools valid
        assert!(composed.is_valid("draft_plan"));
        assert!(composed.is_valid("queryData"));

        // Unknown still rejected
        assert!(!composed.is_valid("unknownTool"));
    }

    #[test]
    fn composed_catalog_dedup_names() {
        let a = VecToolCatalog::new(vec!["foo".into(), "bar".into()]);
        let b = VecToolCatalog::new(vec!["bar".into(), "baz".into()]);
        let composed = ComposedCatalog::new(a, b);

        let names = composed.all_tool_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));
    }
}
