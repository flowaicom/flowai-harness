//! Data source configuration — database connection metadata.
//!
//! These types describe external database connections that users configure
//! in the workspace.

use serde::{Deserialize, Serialize};

pub use agent_fw_core::DatabaseType;

/// A configured external data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub database_type: DatabaseType,
    pub host: String,
    pub port: u16,
    pub database_name: String,
    pub schema_name: String,
    pub encrypted_credentials: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Connection status of a data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DataSourceStatus {
    Connected,
    Disconnected,
    #[serde(rename_all = "camelCase")]
    Error {
        message: String,
    },
}

/// Result of testing a data source connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub success: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

impl ConnectionTestResult {
    pub fn connected(latency_ms: u64, server_version: Option<String>) -> Self {
        Self {
            success: true,
            latency_ms,
            error: None,
            server_version,
        }
    }

    pub fn failed(latency_ms: u64, error: String) -> Self {
        Self {
            success: false,
            latency_ms,
            error: Some(error),
            server_version: None,
        }
    }
}

/// Request to create a data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDataSourceRequest {
    pub name: String,
    pub database_type: DatabaseType,
    pub host: String,
    pub port: u16,
    pub database_name: String,
    pub schema_name: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Request to update a data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDataSourceRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database_name: Option<String>,
    pub schema_name: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub is_active: Option<bool>,
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
    fn connection_test_result_constructors() {
        let ok = ConnectionTestResult::connected(5, Some("PostgreSQL 16.1".into()));
        assert!(ok.success);
        assert!(ok.error.is_none());

        let fail = ConnectionTestResult::failed(100, "Connection refused".into());
        assert!(!fail.success);
        assert_eq!(fail.error.as_deref(), Some("Connection refused"));
    }

    #[test]
    fn data_source_status_serde() {
        let connected = DataSourceStatus::Connected;
        let json = serde_json::to_string(&connected).unwrap();
        assert!(json.contains("\"status\":\"connected\""));

        let error = DataSourceStatus::Error {
            message: "timeout".into(),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"status\":\"error\""));
        let parsed: DataSourceStatus = serde_json::from_str(&json).unwrap();
        match parsed {
            DataSourceStatus::Error { message } => assert_eq!(message, "timeout"),
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn data_source_request_dtos_reject_unknown_fields() {
        let create_err = serde_json::from_value::<CreateDataSourceRequest>(serde_json::json!({
            "name": "Warehouse",
            "databaseType": "postgresql",
            "host": "localhost",
            "port": 5432,
            "databaseName": "warehouse",
            "unexpected": true
        }))
        .unwrap_err();
        assert!(create_err.to_string().contains("unknown field"));

        let update_err = serde_json::from_value::<UpdateDataSourceRequest>(serde_json::json!({
            "name": "Warehouse",
            "unexpected": true
        }))
        .unwrap_err();
        assert!(update_err.to_string().contains("unknown field"));
    }
}
