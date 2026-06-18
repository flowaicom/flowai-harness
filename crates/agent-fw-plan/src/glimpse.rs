//! Generic plan glimpse placeholders.
//!
//! A plan glimpse is a compact, serializable preview of a plan for prompts,
//! approval surfaces, and trace summaries. It is intentionally separate from
//! entity/reference glimpses in `agent-fw-resolve` and `flowai-runtime`.

use serde::{Deserialize, Serialize};

/// Lightweight reference metadata included in a plan glimpse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanGlimpseReference {
    pub kind: String,
    pub id: String,
}

impl PlanGlimpseReference {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

/// Compact preview of a plan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanGlimpse {
    pub plan_id: String,
    pub action_count: usize,
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<PlanGlimpseReference>,
}

impl PlanGlimpse {
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

    pub fn with_reference(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.references.push(PlanGlimpseReference::new(kind, id));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glimpse_is_domain_neutral() {
        let glimpse = PlanGlimpse::new("plan-1", 1)
            .with_summary("One action")
            .with_reference("AudienceSet", "ref-1");

        let value = serde_json::to_value(glimpse).unwrap();
        assert_eq!(value["planId"], "plan-1");
        assert_eq!(value["references"][0]["kind"], "AudienceSet");
        assert!(value.get("productCount").is_none());
    }
}
