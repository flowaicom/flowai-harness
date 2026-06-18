//! SQL identifier newtypes with validation.
//!
//! Provides type-safe wrappers for SQL identifiers (table names, column names,
//! schema names). These enforce basic validity rules at construction time:
//!
//! - Non-empty
//! - No null bytes
//! - Maximum length (128 characters)
//!
//! # Usage
//!
//! ```
//! use agent_fw_catalog::identifier::{TableName, ColumnName, SchemaName};
//!
//! let table = TableName::new("users").unwrap();
//! assert_eq!(table.as_str(), "users");
//!
//! // Invalid identifiers are rejected:
//! assert!(TableName::new("").is_err());
//! ```
//!
//! The newtypes implement transparent serde, `Deref<Target=str>`, `Display`,
//! `AsRef<str>`, and `From<T> for String`, making them drop-in replacements
//! for `String` in most contexts.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum length for SQL identifiers.
const MAX_IDENTIFIER_LEN: usize = 128;

/// Error from invalid SQL identifier construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentifierError {
    #[error("identifier must not be empty")]
    Empty,
    #[error("identifier must not contain null bytes")]
    NullByte,
    #[error("identifier exceeds maximum length of {MAX_IDENTIFIER_LEN} characters")]
    TooLong,
}

macro_rules! define_identifier {
    (
        $(#[$meta:meta])*
        $name:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Create a new identifier, validating the input.
            pub fn new(s: impl Into<String>) -> Result<Self, IdentifierError> {
                let s = s.into();
                validate_identifier(&s)?;
                Ok(Self(s))
            }

            /// Create without validation. Use only for trusted sources
            /// (e.g., values read from a database catalog).
            pub fn new_unchecked(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            /// Get the inner string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume and return the inner string.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                self.0 == *other
            }
        }
    };
}

define_identifier!(
    /// A validated SQL table name.
    TableName
);

define_identifier!(
    /// A validated SQL column name.
    ColumnName
);

define_identifier!(
    /// A validated SQL schema name.
    SchemaName
);

/// Qualified table reference: `schema.table`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualifiedTable {
    pub schema: SchemaName,
    pub table: TableName,
}

impl QualifiedTable {
    /// Create a new qualified table reference.
    pub fn new(schema: SchemaName, table: TableName) -> Self {
        Self { schema, table }
    }

    /// Parse from "schema.table" format.
    pub fn parse(s: &str) -> Result<Self, IdentifierError> {
        if let Some((schema, table)) = s.split_once('.') {
            Ok(Self {
                schema: SchemaName::new(schema)?,
                table: TableName::new(table)?,
            })
        } else {
            // No schema prefix — assume "public"
            Ok(Self {
                schema: SchemaName::new_unchecked("public"),
                table: TableName::new(s)?,
            })
        }
    }

    /// Format as "schema.table".
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.table)
    }
}

impl fmt::Display for QualifiedTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.schema, self.table)
    }
}

/// Validate a raw identifier string.
fn validate_identifier(s: &str) -> Result<(), IdentifierError> {
    if s.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if s.contains('\0') {
        return Err(IdentifierError::NullByte);
    }
    if s.len() > MAX_IDENTIFIER_LEN {
        return Err(IdentifierError::TooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_identifiers() {
        assert!(TableName::new("users").is_ok());
        assert!(ColumnName::new("created_at").is_ok());
        assert!(SchemaName::new("public").is_ok());
    }

    #[test]
    fn empty_rejected() {
        assert_eq!(TableName::new(""), Err(IdentifierError::Empty));
        assert_eq!(ColumnName::new(""), Err(IdentifierError::Empty));
    }

    #[test]
    fn null_byte_rejected() {
        assert_eq!(TableName::new("users\0"), Err(IdentifierError::NullByte));
    }

    #[test]
    fn too_long_rejected() {
        let long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
        assert_eq!(TableName::new(long), Err(IdentifierError::TooLong));
    }

    #[test]
    fn max_length_accepted() {
        let max = "a".repeat(MAX_IDENTIFIER_LEN);
        assert!(TableName::new(max).is_ok());
    }

    #[test]
    fn deref_and_display() {
        let t = TableName::new("orders").unwrap();
        assert_eq!(t.as_str(), "orders");
        assert_eq!(format!("{t}"), "orders");
        assert_eq!(&*t, "orders");
    }

    #[test]
    fn into_string() {
        let t = TableName::new("orders").unwrap();
        let s: String = t.into();
        assert_eq!(s, "orders");
    }

    #[test]
    fn serde_roundtrip() {
        let t = TableName::new("users").unwrap();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "\"users\"");
        let parsed: TableName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn qualified_table_parse() {
        let qt = QualifiedTable::parse("public.orders").unwrap();
        assert_eq!(qt.schema.as_str(), "public");
        assert_eq!(qt.table.as_str(), "orders");
        assert_eq!(qt.qualified_name(), "public.orders");
    }

    #[test]
    fn qualified_table_default_schema() {
        let qt = QualifiedTable::parse("orders").unwrap();
        assert_eq!(qt.schema.as_str(), "public");
        assert_eq!(qt.table.as_str(), "orders");
    }

    #[test]
    fn partial_eq_str() {
        let t = TableName::new("users").unwrap();
        assert_eq!(t, *"users");
        assert_eq!(t, "users".to_string());
    }

    #[test]
    fn new_unchecked() {
        let t = TableName::new_unchecked("");
        assert_eq!(t.as_str(), "");
    }
}
