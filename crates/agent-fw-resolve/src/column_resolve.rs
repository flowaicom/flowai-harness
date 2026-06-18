//! Column resolution placeholders.
//!
//! The retired vertical used this filename for product-specific term
//! resolution. The generic version should describe the contract between a
//! column-value search step and downstream filter/reference creation without
//! depending on a particular catalog, database, or vector implementation.

use serde::{Deserialize, Serialize};

/// Strategy hint for resolving user terms against a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnResolutionStrategy {
    /// Prefer one canonical categorical value per term.
    #[default]
    Categorical,
    /// Allow multiple display/free-text matches.
    FreeText,
    /// Treat values as stable identifiers.
    Identifier,
}

/// Domain-neutral request to resolve terms against a named column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnResolutionSpec {
    pub column: String,
    #[serde(default)]
    pub terms: Vec<String>,
    #[serde(default)]
    pub strategy: ColumnResolutionStrategy,
}

impl ColumnResolutionSpec {
    pub fn new(
        column: impl Into<String>,
        terms: Vec<String>,
        strategy: ColumnResolutionStrategy,
    ) -> Self {
        Self {
            column: column.into(),
            terms,
            strategy,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// One matched value produced by a resolver implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMatch {
    pub value: String,
    pub score: u16,
}

impl ColumnMatch {
    pub fn new(value: impl Into<String>, score: u16) -> Self {
        Self {
            value: value.into(),
            score,
        }
    }
}

/// Result envelope for column resolution implementations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnResolution {
    pub column: String,
    #[serde(default)]
    pub matches: Vec<ColumnMatch>,
    #[serde(default)]
    pub strategy: ColumnResolutionStrategy,
}

impl ColumnResolution {
    pub fn empty(spec: &ColumnResolutionSpec) -> Self {
        Self {
            column: spec.column.clone(),
            matches: Vec::new(),
            strategy: spec.strategy,
        }
    }

    pub fn matched_values(&self) -> Vec<&str> {
        self.matches
            .iter()
            .map(|matched| matched.value.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_resolution_preserves_spec_shape() {
        let spec =
            ColumnResolutionSpec::new("region", Vec::new(), ColumnResolutionStrategy::Categorical);
        let resolution = ColumnResolution::empty(&spec);

        assert_eq!(resolution.column, "region");
        assert!(resolution.matches.is_empty());
        assert_eq!(resolution.strategy, ColumnResolutionStrategy::Categorical);
    }
}
