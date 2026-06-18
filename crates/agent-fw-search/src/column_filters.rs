//! ColumnFilters — arbitrary column filters (column → values).
//!
//! A domain-agnostic wrapper around `HashMap<String, Vec<String>>` used for:
//! - Tool input (e.g. matchedFilters.columnFilters)
//! - Eval ground truth (e.g. ExpectedFilters.matched_filters)
//!
//! # Laws
//!
//! L1 (Transparent serialization): JSON wire format is identical to raw HashMap.
//! L2 (From<HashMap>): Can be constructed from raw HashMap for backwards compatibility.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Arbitrary column filters: column name → list of values.
///
/// Wraps `HashMap<String, Vec<String>>` with a domain-specific API.
/// `#[serde(transparent)]` preserves the JSON wire format (critical for LLM tool calls).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ColumnFilters(HashMap<String, Vec<String>>);

impl ColumnFilters {
    /// Create an empty filter set.
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Add a column filter with its values.
    pub fn add_column_filter(&mut self, column: String, values: Vec<String>) {
        self.0.insert(column, values);
    }

    /// Add a single filter value for a column (appends if column exists).
    pub fn add_filter_value(&mut self, column: String, value: String) {
        self.0.entry(column).or_default().push(value);
    }

    /// Extend values for a column (appends if column exists).
    pub fn extend_values(&mut self, column: String, values: impl IntoIterator<Item = String>) {
        self.0.entry(column).or_default().extend(values);
    }

    /// Remove a column and return its values.
    pub fn remove(&mut self, column: &str) -> Option<Vec<String>> {
        self.0.remove(column)
    }

    /// Get the filter values for a column.
    pub fn get(&self, column: &str) -> Option<&Vec<String>> {
        self.0.get(column)
    }

    /// Iterate over all (column, values) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.0.iter()
    }

    /// Iterate over all value sets.
    pub fn values(&self) -> impl Iterator<Item = &Vec<String>> {
        self.0.values()
    }

    /// Check if no filters are set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of columns with filters.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over column names.
    pub fn columns(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    /// Convert to inner HashMap.
    pub fn into_inner(self) -> HashMap<String, Vec<String>> {
        self.0
    }
}

/// Allow construction from HashMap for backwards compatibility.
impl From<HashMap<String, Vec<String>>> for ColumnFilters {
    fn from(map: HashMap<String, Vec<String>>) -> Self {
        Self(map)
    }
}

/// Allow conversion to HashMap when needed.
impl From<ColumnFilters> for HashMap<String, Vec<String>> {
    fn from(filters: ColumnFilters) -> Self {
        filters.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_serialization() {
        let mut filters = ColumnFilters::new();
        filters.add_column_filter(
            "brand".to_string(),
            vec!["Brand A".to_string(), "Brand B".to_string()],
        );
        filters.add_column_filter("category".to_string(), vec!["Electronics".to_string()]);

        let json = serde_json::to_string(&filters).unwrap();
        // Should serialize as a plain object (transparent)
        assert!(json.contains("\"brand\""));
        assert!(json.contains("\"Brand A\""));

        let parsed: ColumnFilters = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.get("brand"),
            Some(&vec!["Brand A".to_string(), "Brand B".to_string()])
        );
    }

    #[test]
    fn from_hashmap() {
        let mut map = HashMap::new();
        map.insert("brand".to_string(), vec!["X".to_string()]);
        let filters: ColumnFilters = map.into();
        assert_eq!(filters.get("brand"), Some(&vec!["X".to_string()]));
    }

    #[test]
    fn into_hashmap() {
        let mut filters = ColumnFilters::new();
        filters.add_column_filter("brand".to_string(), vec!["Y".to_string()]);
        let map: HashMap<String, Vec<String>> = filters.into();
        assert_eq!(map.get("brand"), Some(&vec!["Y".to_string()]));
    }
}
