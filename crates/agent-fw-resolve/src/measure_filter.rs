//! Generic measure-filter placeholders.
//!
//! This module keeps the data-model boundary for aggregate filters while
//! avoiding SQL generation and product-specific semantics. Database-specific
//! interpreters should translate these specs in catalog or warehouse layers.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Aggregate operation for a measure filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregateOp {
    #[default]
    Avg,
    Min,
    Max,
    Sum,
    Count,
    Any,
}

/// Comparison operation for a measure filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComparisonOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Between,
}

/// Data-only aggregate filter specification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureFilterSpec {
    pub measure: String,
    #[serde(default)]
    pub aggregate: AggregateOp,
    pub operator: ComparisonOp,
    pub value: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_to: Option<JsonValue>,
}

impl MeasureFilterSpec {
    pub fn new(
        measure: impl Into<String>,
        aggregate: AggregateOp,
        operator: ComparisonOp,
        value: JsonValue,
    ) -> Self {
        Self {
            measure: measure.into(),
            aggregate,
            operator,
            value,
            value_to: None,
        }
    }

    pub fn between(
        measure: impl Into<String>,
        aggregate: AggregateOp,
        value: JsonValue,
        value_to: JsonValue,
    ) -> Self {
        Self {
            measure: measure.into(),
            aggregate,
            operator: ComparisonOp::Between,
            value,
            value_to: Some(value_to),
        }
    }

    pub fn validate(&self) -> Result<(), MeasureFilterError> {
        if self.measure.trim().is_empty() {
            return Err(MeasureFilterError::EmptyMeasure);
        }
        if self.operator == ComparisonOp::Between && self.value_to.is_none() {
            return Err(MeasureFilterError::MissingUpperBound);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeasureFilterError {
    #[error("measure name must not be empty")]
    EmptyMeasure,
    #[error("between filters require value_to")]
    MissingUpperBound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_requires_upper_bound() {
        let filter = MeasureFilterSpec::new(
            "conversion_rate",
            AggregateOp::Avg,
            ComparisonOp::Between,
            serde_json::json!(0.1),
        );

        assert_eq!(
            filter.validate(),
            Err(MeasureFilterError::MissingUpperBound)
        );
    }

    #[test]
    fn data_model_is_domain_neutral() {
        let filter = MeasureFilterSpec::between(
            "conversion_rate",
            AggregateOp::Avg,
            serde_json::json!(0.1),
            serde_json::json!(0.2),
        );

        let value = serde_json::to_value(filter).unwrap();
        assert_eq!(value["measure"], "conversion_rate");
        assert!(value.get("product").is_none());
    }
}
