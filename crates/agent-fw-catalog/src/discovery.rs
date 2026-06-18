//! Schema discovery types — physical database structure.
//!
//! These types represent the raw schema metadata discovered from a target
//! database through introspection queries.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The kind of database table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableType {
    BaseTable,
    View,
    MaterializedView,
    Foreign,
}

/// Summary information about a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub schema_name: String,
    pub table_name: String,
    pub table_type: TableType,
    pub row_count: Option<i64>,
    pub column_count: Option<i64>,
    pub description: Option<String>,
}

/// Metadata for a single column.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub column_name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub column_default: Option<String>,
    pub ordinal_position: i32,
    pub is_primary_key: bool,
    pub foreign_key: Option<ForeignKeyRef>,
}

/// A foreign key reference from a column to another table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyRef {
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_column: String,
    pub constraint_name: String,
}

/// Kind of table constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    #[serde(rename = "PRIMARY KEY")]
    PrimaryKey,
    #[serde(rename = "FOREIGN KEY")]
    ForeignKey,
    #[serde(rename = "UNIQUE")]
    Unique,
    #[serde(rename = "CHECK")]
    Check,
}

impl ConstraintKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrimaryKey => "PRIMARY KEY",
            Self::ForeignKey => "FOREIGN KEY",
            Self::Unique => "UNIQUE",
            Self::Check => "CHECK",
        }
    }
}

impl fmt::Display for ConstraintKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConstraintKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "PRIMARY KEY" | "PRIMARY_KEY" | "P" => Ok(Self::PrimaryKey),
            "FOREIGN KEY" | "FOREIGN_KEY" | "F" => Ok(Self::ForeignKey),
            "UNIQUE" | "U" => Ok(Self::Unique),
            "CHECK" | "C" => Ok(Self::Check),
            _ => Err(format!("Unknown constraint kind: {s}")),
        }
    }
}

/// A table constraint (PK, FK, UNIQUE, CHECK).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintInfo {
    pub name: String,
    pub constraint_type: ConstraintKind,
    pub columns: Vec<String>,
}

/// An index on a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
}

/// Complete physical table metadata (columns + constraints + indexes).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalTable {
    pub schema_name: String,
    pub table_name: String,
    pub columns: Vec<ColumnInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub indexes: Vec<IndexInfo>,
    pub row_count: i64,
}

/// A foreign key edge between two tables.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyEdge {
    pub source_table: String,
    pub source_column: String,
    pub target_table: String,
    pub target_column: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_kind_roundtrip() {
        for kind in [
            ConstraintKind::PrimaryKey,
            ConstraintKind::ForeignKey,
            ConstraintKind::Unique,
            ConstraintKind::Check,
        ] {
            let s = kind.as_str();
            let parsed: ConstraintKind = s.parse().unwrap();
            assert_eq!(kind, parsed);

            let json = serde_json::to_string(&kind).unwrap();
            let deser: ConstraintKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, deser);
        }
    }

    #[test]
    fn physical_table_serde() {
        let table = PhysicalTable {
            schema_name: "public".into(),
            table_name: "users".into(),
            columns: vec![ColumnInfo {
                column_name: "id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: Some("nextval('users_id_seq')".into()),
                ordinal_position: 1,
                is_primary_key: true,
                foreign_key: None,
            }],
            constraints: vec![ConstraintInfo {
                name: "users_pkey".into(),
                constraint_type: ConstraintKind::PrimaryKey,
                columns: vec!["id".into()],
            }],
            indexes: vec![],
            row_count: 5000,
        };

        let json = serde_json::to_string(&table).unwrap();
        let parsed: PhysicalTable = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.table_name, "users");
        assert_eq!(parsed.columns.len(), 1);
        assert!(parsed.columns[0].is_primary_key);
    }
}
