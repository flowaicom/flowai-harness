//! Table role classification — pure, deterministic classifier.
//!
//! Classifies tables into dimensional modelling roles (Fact, Dimension, Bridge)
//! based on naming conventions, FK topology, and structural heuristics.
//!
//! # Laws
//!
//! - **L1 (Totality)**: Never panics for any input
//! - **L2 (Determinism)**: Same input always produces the same role
//! - **L3 (Naming precedence)**: Naming conventions override structural heuristics
//!   - `fact_*` or `fct_*` → Fact
//!   - `dim_*` → Dimension
//!   - `bridge_*` or `xref_*` → Bridge

use serde::{Deserialize, Serialize};

/// Dimensional modelling role for a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableRole {
    Fact,
    Dimension,
    Bridge,
    Unknown,
}

impl std::fmt::Display for TableRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fact => write!(f, "Fact"),
            Self::Dimension => write!(f, "Dimension"),
            Self::Bridge => write!(f, "Bridge"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Input data for table classification.
///
/// All fields are structural metadata — no DB access required.
#[derive(Clone, Debug)]
pub struct TableClassificationInput {
    pub table_name: String,
    pub row_count: usize,
    pub column_count: usize,
    pub fk_outbound_count: usize,
    pub fk_inbound_count: usize,
    pub has_measures: bool,
}

/// Pure classifier. Priority: naming > FK topology > fallback.
///
/// # Laws
/// - L1 (Totality): Never panics
/// - L2 (Determinism): Same input → same role
/// - L3 (Naming precedence): Name prefixes override all heuristics
pub fn classify_table_role(input: &TableClassificationInput) -> TableRole {
    let name_lower = input.table_name.to_lowercase();

    // L3: Naming conventions take precedence
    if name_lower.starts_with("fact_") || name_lower.starts_with("fct_") {
        return TableRole::Fact;
    }
    if name_lower.starts_with("dim_") {
        return TableRole::Dimension;
    }
    if name_lower.starts_with("bridge_") || name_lower.starts_with("xref_") {
        return TableRole::Bridge;
    }

    // FK topology heuristics
    // Bridge tables: many outbound FKs, few/no inbound, few columns
    if input.fk_outbound_count >= 2 && input.fk_inbound_count == 0 && input.column_count <= 5 {
        return TableRole::Bridge;
    }

    // Fact tables: have measures, many rows, some outbound FKs
    if input.has_measures && input.fk_outbound_count >= 1 {
        return TableRole::Fact;
    }

    // Dimension tables: many inbound FKs (other tables reference them), no measures
    if input.fk_inbound_count >= 2 && !input.has_measures {
        return TableRole::Dimension;
    }

    // Dimension tables: few columns, no measures (lookup tables)
    if input.column_count <= 10 && !input.has_measures && input.row_count < 10_000 {
        return TableRole::Dimension;
    }

    TableRole::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> TableClassificationInput {
        TableClassificationInput {
            table_name: name.to_string(),
            row_count: 1000,
            column_count: 10,
            fk_outbound_count: 0,
            fk_inbound_count: 0,
            has_measures: false,
        }
    }

    // ── L3: Naming precedence ──

    #[test]
    fn fact_prefix() {
        assert_eq!(classify_table_role(&input("fact_sales")), TableRole::Fact);
        assert_eq!(classify_table_role(&input("fct_orders")), TableRole::Fact);
        assert_eq!(classify_table_role(&input("FACT_REVENUE")), TableRole::Fact);
    }

    #[test]
    fn dim_prefix() {
        assert_eq!(
            classify_table_role(&input("dim_products")),
            TableRole::Dimension
        );
        assert_eq!(
            classify_table_role(&input("DIM_CUSTOMERS")),
            TableRole::Dimension
        );
    }

    #[test]
    fn bridge_prefix() {
        assert_eq!(
            classify_table_role(&input("bridge_order_product")),
            TableRole::Bridge
        );
        assert_eq!(
            classify_table_role(&input("xref_user_role")),
            TableRole::Bridge
        );
    }

    // ── FK topology ──

    #[test]
    fn bridge_by_topology() {
        let mut i = input("order_products");
        i.fk_outbound_count = 3;
        i.fk_inbound_count = 0;
        i.column_count = 4;
        assert_eq!(classify_table_role(&i), TableRole::Bridge);
    }

    #[test]
    fn fact_by_measures() {
        let mut i = input("sales_data");
        i.has_measures = true;
        i.fk_outbound_count = 2;
        assert_eq!(classify_table_role(&i), TableRole::Fact);
    }

    #[test]
    fn dimension_by_inbound_fks() {
        let mut i = input("products");
        i.fk_inbound_count = 5;
        i.has_measures = false;
        assert_eq!(classify_table_role(&i), TableRole::Dimension);
    }

    #[test]
    fn unknown_fallback() {
        let mut i = input("staging_temp");
        i.row_count = 100_000;
        i.column_count = 50;
        assert_eq!(classify_table_role(&i), TableRole::Unknown);
    }

    // ── L2: Determinism ──

    #[test]
    fn determinism() {
        let i = input("fact_sales");
        let a = classify_table_role(&i);
        let b = classify_table_role(&i);
        assert_eq!(a, b, "L2: same input must produce same result");
    }

    // ── Serde roundtrip ──

    #[test]
    fn serde_roundtrip() {
        let role = TableRole::Fact;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"fact\"");
        let parsed: TableRole = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, TableRole::Fact);
    }
}
