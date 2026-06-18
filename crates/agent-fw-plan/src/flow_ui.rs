//! Generic plan UI projection placeholders.
//!
//! This module reserves the plan-level UI projection boundary without carrying
//! the retired product/pricing card model. Future Studio or harness-owned UI
//! work should generalize from these neutral data shapes instead of rebuilding
//! the old vertical-specific `flowGenUI` surface.

use serde::{Deserialize, Serialize};

/// A display fact for a plan approval or inspection surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUiFact {
    pub label: String,
    pub value: String,
}

impl PlanUiFact {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// Minimal, domain-neutral projection for rendering a plan to a UI layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUiProjection {
    pub plan_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub action_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<PlanUiFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl PlanUiProjection {
    pub fn new(plan_id: impl Into<String>, action_count: usize) -> Self {
        Self {
            plan_id: plan_id.into(),
            action_count,
            ..Self::default()
        }
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_fact(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.facts.push(PlanUiFact::new(label, value));
        self
    }

    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_serializes_without_domain_fields() {
        let projection = PlanUiProjection::new("plan-1", 2)
            .with_summary("Adjust allocation")
            .with_fact("Entities", "12")
            .with_warning("Requires approval");

        let value = serde_json::to_value(projection).unwrap();
        assert_eq!(value["planId"], "plan-1");
        assert_eq!(value["actionCount"], 2);
        assert!(value.get("products").is_none());
        assert!(value.get("pricing").is_none());
    }
}
