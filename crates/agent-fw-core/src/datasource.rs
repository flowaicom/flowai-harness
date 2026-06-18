//! Shared datasource kernel types.
//!
//! These are the smallest pure-data concepts needed across the framework's
//! catalog, workspace, and connection-pool layers.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Type of database engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    #[serde(rename = "postgresql")]
    PostgreSQL,
    #[serde(rename = "mysql")]
    MySQL,
    #[serde(rename = "sqlite")]
    SQLite,
}

impl fmt::Display for DatabaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PostgreSQL => f.write_str("postgresql"),
            Self::MySQL => f.write_str("mysql"),
            Self::SQLite => f.write_str("sqlite"),
        }
    }
}

impl DatabaseType {
    /// Infer the database engine from a connection URL or config-backed location.
    ///
    /// PostgreSQL and MySQL use explicit URI schemes. Everything else is treated
    /// as SQLite so relative paths and `sqlite:` URLs share one low-ceremony path.
    pub fn infer_from_connection_url(url: &str) -> Self {
        let trimmed = url.trim().to_ascii_lowercase();
        if trimmed.starts_with("postgresql://") || trimmed.starts_with("postgres://") {
            Self::PostgreSQL
        } else if trimmed.starts_with("mysql://") {
            Self::MySQL
        } else {
            Self::SQLite
        }
    }

    /// Human-readable engine label for logs and UI fallbacks.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::PostgreSQL => "PostgreSQL",
            Self::MySQL => "MySQL",
            Self::SQLite => "SQLite",
        }
    }

    /// Default schema/database namespace for new data sources when callers omit it.
    pub fn default_schema_name(&self, database_name: &str) -> String {
        match self {
            Self::PostgreSQL => "public".to_string(),
            Self::MySQL => database_name.to_string(),
            Self::SQLite => "main".to_string(),
        }
    }

    /// Read-only version query for the database engine.
    pub fn version_query(&self) -> &'static str {
        match self {
            Self::PostgreSQL | Self::MySQL => "SELECT version() AS version",
            Self::SQLite => "SELECT sqlite_version() AS version",
        }
    }
}

impl FromStr for DatabaseType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgresql" | "postgres" | "pg" => Ok(Self::PostgreSQL),
            "mysql" => Ok(Self::MySQL),
            "sqlite" => Ok(Self::SQLite),
            _ => Err(format!("Unknown database type: {s}")),
        }
    }
}

impl TryFrom<String> for DatabaseType {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_type_parsing() {
        assert_eq!(
            "postgresql".parse::<DatabaseType>().unwrap(),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            "postgres".parse::<DatabaseType>().unwrap(),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            "pg".parse::<DatabaseType>().unwrap(),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            "mysql".parse::<DatabaseType>().unwrap(),
            DatabaseType::MySQL
        );
        assert_eq!(
            "sqlite".parse::<DatabaseType>().unwrap(),
            DatabaseType::SQLite
        );
        assert!("oracle".parse::<DatabaseType>().is_err());
    }

    #[test]
    fn database_type_serde() {
        let json = serde_json::to_string(&DatabaseType::PostgreSQL).unwrap();
        assert_eq!(json, "\"postgresql\"");
        let parsed: DatabaseType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DatabaseType::PostgreSQL);

        let json = serde_json::to_string(&DatabaseType::MySQL).unwrap();
        assert_eq!(json, "\"mysql\"");
        let parsed: DatabaseType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DatabaseType::MySQL);
    }

    #[test]
    fn database_type_helpers_cover_default_schema_and_version_query() {
        assert_eq!(DatabaseType::PostgreSQL.display_name(), "PostgreSQL");
        assert_eq!(
            DatabaseType::PostgreSQL.default_schema_name("warehouse"),
            "public"
        );
        assert_eq!(
            DatabaseType::PostgreSQL.version_query(),
            "SELECT version() AS version"
        );

        assert_eq!(
            DatabaseType::MySQL.default_schema_name("warehouse"),
            "warehouse"
        );
        assert_eq!(
            DatabaseType::SQLite.default_schema_name("warehouse"),
            "main"
        );
        assert_eq!(
            DatabaseType::SQLite.version_query(),
            "SELECT sqlite_version() AS version"
        );
    }

    #[test]
    fn database_type_infers_from_connection_urls() {
        assert_eq!(
            DatabaseType::infer_from_connection_url("postgresql://localhost/demo"),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            DatabaseType::infer_from_connection_url("postgres://localhost/demo"),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            DatabaseType::infer_from_connection_url("mysql://localhost/demo"),
            DatabaseType::MySQL
        );
        assert_eq!(
            DatabaseType::infer_from_connection_url("sqlite:data/demo.db"),
            DatabaseType::SQLite
        );
        assert_eq!(
            DatabaseType::infer_from_connection_url("relative/path/demo.db"),
            DatabaseType::SQLite
        );
    }
}
