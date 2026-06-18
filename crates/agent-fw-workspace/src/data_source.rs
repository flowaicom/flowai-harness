//! Workspace data-source compatibility re-exports.
//!
//! The canonical datasource contract is owned by `agent-fw-catalog`.
//! `agent-fw-workspace` re-exports those types so workspace CRUD and lifecycle
//! code share the same datasource vocabulary instead of carrying a parallel
//! model.

pub use agent_fw_catalog::datasource::{
    ConnectionTestResult, CreateDataSourceRequest, DataSource, DataSourceStatus, DatabaseType,
    UpdateDataSourceRequest,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(id: &str) -> DataSource {
        DataSource {
            id: id.to_string(),
            name: format!("Test DB {}", id),
            database_type: DatabaseType::PostgreSQL,
            host: "localhost".to_string(),
            port: 5432,
            database_name: "testdb".to_string(),
            schema_name: "public".to_string(),
            encrypted_credentials: None,
            is_active: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn data_source_serde_roundtrip() {
        let ds = fixture("ds-1");
        let json = serde_json::to_string(&ds).unwrap();
        let parsed: DataSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, ds.id);
        assert_eq!(parsed.name, ds.name);
        assert_eq!(parsed.database_type, DatabaseType::PostgreSQL);
        assert_eq!(parsed.port, 5432);
        assert!(parsed.is_active);
    }

    #[test]
    fn data_source_camel_case_serialization() {
        let ds = fixture("ds-1");
        let json = serde_json::to_string(&ds).unwrap();
        assert!(json.contains("\"databaseType\""));
        assert!(json.contains("\"databaseName\""));
        assert!(json.contains("\"schemaName\""));
        assert!(json.contains("\"isActive\""));
        assert!(json.contains("\"createdAt\""));
        assert!(json.contains("\"updatedAt\""));
    }
}
