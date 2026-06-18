//! Data source application service — encrypted CRUD plus connection testing.
//!
//! This is the canonical workflow for workspace-managed database connections:
//!
//! - create/update/list/get/delete [`DataSource`] via [`DataSourceStore`]
//! - encrypt persisted credentials via [`EncryptionService`]
//! - probe database connectivity via [`TargetDatabase`]
//!
//! Environment-driven seeding or app-specific URL/config policy stays in the
//! consuming application. This module owns the reusable mechanics.

use std::sync::Arc;
use std::time::Instant;

use agent_fw_algebra::{
    DbError, EncryptionError, EncryptionService, ReadOnlyQuery, TargetDatabase,
};
use agent_fw_core::TenantId;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data_source::{
    ConnectionTestResult, CreateDataSourceRequest, DataSource, DatabaseType,
    UpdateDataSourceRequest,
};
use crate::store::{DataSourceStore, WorkspaceError};

/// Errors from generic data-source application flows.
#[derive(Debug, thiserror::Error)]
pub enum DataSourceServiceError {
    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Data source not found: {0}")]
    NotFound(String),

    #[error("Workspace store error: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("Encryption error: {0}")]
    Encryption(#[from] EncryptionError),

    #[error("Database error: {0}")]
    Database(#[from] DbError),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceCredentials {
    username: Option<String>,
    password: Option<String>,
}

impl DataSourceCredentials {
    pub fn new(username: Option<String>, password: Option<String>) -> Self {
        Self { username, password }
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    fn is_empty(&self) -> bool {
        self.username.is_none() && self.password.is_none()
    }
}

/// Encrypt optional data-source credentials for persistence.
///
/// Returns `None` when both username and password are absent.
pub async fn encrypt_data_source_credentials(
    encryption: &dyn EncryptionService,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<String>, DataSourceServiceError> {
    if username.is_none() && password.is_none() {
        return Ok(None);
    }

    let credentials = DataSourceCredentials {
        username: username.map(str::to_owned),
        password: password.map(str::to_owned),
    };
    let plaintext = serde_json::to_vec(&credentials).map_err(|error| {
        DataSourceServiceError::Validation(format!("Failed to serialize credentials: {error}"))
    })?;
    let payload = encryption.encrypt(&plaintext).await?;
    serde_json::to_string(&payload).map(Some).map_err(|error| {
        DataSourceServiceError::Validation(format!(
            "Failed to serialize encrypted credentials: {error}"
        ))
    })
}

pub async fn decrypt_data_source_credentials(
    encryption: &dyn EncryptionService,
    payload_str: &str,
) -> Result<DataSourceCredentials, DataSourceServiceError> {
    let payload = serde_json::from_str(payload_str).map_err(|error| {
        DataSourceServiceError::Validation(format!(
            "Failed to deserialize encrypted credentials: {error}"
        ))
    })?;
    let plaintext = encryption.decrypt(&payload).await?;
    serde_json::from_slice(&plaintext).map_err(|error| {
        DataSourceServiceError::Validation(format!(
            "Failed to deserialize credential JSON: {error}"
        ))
    })
}

/// Decrypt optional credentials, returning an empty credential set when absent.
pub async fn resolve_data_source_credentials(
    encryption: &dyn EncryptionService,
    encrypted_credentials: Option<&str>,
) -> Result<DataSourceCredentials, DataSourceServiceError> {
    match encrypted_credentials {
        Some(payload) => decrypt_data_source_credentials(encryption, payload).await,
        None => Ok(DataSourceCredentials::default()),
    }
}

async fn merged_encrypted_credentials(
    encryption: &dyn EncryptionService,
    current: Option<&str>,
    request: &UpdateDataSourceRequest,
) -> Result<Option<String>, DataSourceServiceError> {
    if request.username.is_none() && request.password.is_none() {
        return Ok(current.map(str::to_owned));
    }

    let existing = match current {
        Some(payload) => decrypt_data_source_credentials(encryption, payload).await?,
        None => DataSourceCredentials::default(),
    };
    let merged = DataSourceCredentials {
        username: request.username.clone().or(existing.username),
        password: request.password.clone().or(existing.password),
    };

    if merged.is_empty() {
        Ok(None)
    } else {
        encrypt_data_source_credentials(
            encryption,
            merged.username.as_deref(),
            merged.password.as_deref(),
        )
        .await
    }
}

fn version_query(database_type: DatabaseType) -> Option<ReadOnlyQuery> {
    ReadOnlyQuery::parse(database_type.version_query()).ok()
}

fn extract_server_version(rows: &[agent_fw_algebra::DbRow]) -> Option<String> {
    rows.first()
        .and_then(|row| row.get("version"))
        .and_then(|value| value.as_str().map(str::to_owned))
}

/// Generic data-source CRUD and connectivity service.
pub struct DataSourceService {
    encryption: Arc<dyn EncryptionService>,
    store: Arc<dyn DataSourceStore>,
}

impl DataSourceService {
    pub fn new(encryption: Arc<dyn EncryptionService>, store: Arc<dyn DataSourceStore>) -> Self {
        Self { encryption, store }
    }

    pub async fn list(&self, tenant: &TenantId) -> Result<Vec<DataSource>, DataSourceServiceError> {
        Ok(self.store.list_data_sources(tenant).await?)
    }

    pub async fn create(
        &self,
        tenant: &TenantId,
        request: CreateDataSourceRequest,
    ) -> Result<DataSource, DataSourceServiceError> {
        let now = Utc::now().to_rfc3339();
        let encrypted_credentials = encrypt_data_source_credentials(
            self.encryption.as_ref(),
            request.username.as_deref(),
            request.password.as_deref(),
        )
        .await?;

        let database_type = request.database_type;
        let database_name = request.database_name;
        let schema_name = request
            .schema_name
            .unwrap_or_else(|| database_type.default_schema_name(&database_name));

        let source = DataSource {
            id: Uuid::new_v4().to_string(),
            name: request.name,
            database_type,
            host: request.host,
            port: request.port,
            database_name,
            schema_name,
            encrypted_credentials,
            is_active: true,
            created_at: now.clone(),
            updated_at: now,
        };

        self.store.upsert_data_source(tenant, &source).await?;
        Ok(source)
    }

    pub async fn get(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<DataSource, DataSourceServiceError> {
        self.store
            .get_data_source(tenant, id)
            .await?
            .ok_or_else(|| DataSourceServiceError::NotFound(id.to_string()))
    }

    pub async fn update(
        &self,
        tenant: &TenantId,
        source_id: &str,
        request: UpdateDataSourceRequest,
    ) -> Result<DataSource, DataSourceServiceError> {
        let mut source = self.get(tenant, source_id).await?;

        if let Some(name) = request.name.clone() {
            source.name = name;
        }
        if let Some(host) = request.host.clone() {
            source.host = host;
        }
        if let Some(port) = request.port {
            source.port = port;
        }
        if let Some(database_name) = request.database_name.clone() {
            source.database_name = database_name;
        }
        if let Some(schema_name) = request.schema_name.clone() {
            source.schema_name = schema_name;
        }
        if let Some(is_active) = request.is_active {
            source.is_active = is_active;
        }

        source.encrypted_credentials = merged_encrypted_credentials(
            self.encryption.as_ref(),
            source.encrypted_credentials.as_deref(),
            &request,
        )
        .await?;
        source.updated_at = Utc::now().to_rfc3339();

        self.store.upsert_data_source(tenant, &source).await?;
        Ok(source)
    }

    pub async fn delete(
        &self,
        tenant: &TenantId,
        source_id: &str,
    ) -> Result<(), DataSourceServiceError> {
        self.store.delete_data_source(tenant, source_id).await?;
        Ok(())
    }

    pub async fn test_connection(
        &self,
        tenant: &TenantId,
        source_id: &str,
        db: Arc<dyn TargetDatabase>,
    ) -> Result<ConnectionTestResult, DataSourceServiceError> {
        let source = self.get(tenant, source_id).await?;
        let started = Instant::now();

        match db.health_check().await {
            Ok(()) => {
                let server_version = match version_query(source.database_type) {
                    Some(query) => match db.query(&query, &[]).await {
                        Ok(rows) => extract_server_version(&rows)
                            .or_else(|| Some(source.database_type.display_name().to_string())),
                        Err(_) => Some(source.database_type.display_name().to_string()),
                    },
                    None => Some(source.database_type.display_name().to_string()),
                };

                Ok(ConnectionTestResult::connected(
                    started.elapsed().as_millis() as u64,
                    server_version,
                ))
            }
            Err(error) => Ok(ConnectionTestResult::failed(
                started.elapsed().as_millis() as u64,
                error.to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_fw_algebra::{DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase};
    use agent_fw_core::TenantId;
    use agent_fw_interpreter::{DashMapKVStore, NoOpEncryptionService};
    use async_trait::async_trait;

    use super::*;
    use crate::KVWorkspaceStore;

    struct StubDb {
        version_sql: &'static str,
        health_error: Option<DbError>,
    }

    #[async_trait]
    impl TargetDatabase for StubDb {
        async fn query(
            &self,
            query: &ReadOnlyQuery,
            _params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            if query.sql() == self.version_sql {
                Ok(vec![DbRow::new(
                    vec!["version".into()],
                    vec![serde_json::json!("engine 1.0")],
                )])
            } else {
                Ok(vec![])
            }
        }

        async fn health_check(&self) -> Result<(), DbError> {
            match &self.health_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![])
        }

        async fn get_table_columns(&self, _table_name: &str) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![])
        }

        async fn sample_table(
            &self,
            _table_name: &str,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, DbError> {
            Ok(vec![])
        }
    }

    fn service() -> DataSourceService {
        let store: Arc<dyn DataSourceStore> =
            Arc::new(KVWorkspaceStore::new(Arc::new(DashMapKVStore::new())));
        let encryption: Arc<dyn EncryptionService> = Arc::new(NoOpEncryptionService::new());
        DataSourceService::new(encryption, store)
    }

    fn tenant() -> TenantId {
        TenantId::new_unchecked("tenant-1")
    }

    #[tokio::test]
    async fn create_defaults_schema_from_database_type() {
        let service = service();
        let source = service
            .create(
                &tenant(),
                CreateDataSourceRequest {
                    name: "Warehouse".into(),
                    database_type: DatabaseType::PostgreSQL,
                    host: "localhost".into(),
                    port: 5432,
                    database_name: "warehouse".into(),
                    schema_name: None,
                    username: None,
                    password: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(source.schema_name, "public");
    }

    #[tokio::test]
    async fn update_merges_new_credentials_with_existing_payload() {
        let service = service();
        let created = service
            .create(
                &tenant(),
                CreateDataSourceRequest {
                    name: "Warehouse".into(),
                    database_type: DatabaseType::PostgreSQL,
                    host: "localhost".into(),
                    port: 5432,
                    database_name: "warehouse".into(),
                    schema_name: Some("public".into()),
                    username: Some("alice".into()),
                    password: Some("secret".into()),
                },
            )
            .await
            .unwrap();

        let updated = service
            .update(
                &tenant(),
                &created.id,
                UpdateDataSourceRequest {
                    name: None,
                    host: None,
                    port: None,
                    database_name: None,
                    schema_name: None,
                    username: None,
                    password: Some("new-secret".into()),
                    is_active: None,
                },
            )
            .await
            .unwrap();

        let payload = updated.encrypted_credentials.expect("credentials present");
        let decrypted = decrypt_data_source_credentials(service.encryption.as_ref(), &payload)
            .await
            .unwrap();
        assert_eq!(decrypted.username.as_deref(), Some("alice"));
        assert_eq!(decrypted.password.as_deref(), Some("new-secret"));
    }

    #[tokio::test]
    async fn test_connection_uses_database_specific_version_query() {
        let service = service();
        let source = service
            .create(
                &tenant(),
                CreateDataSourceRequest {
                    name: "Local SQLite".into(),
                    database_type: DatabaseType::SQLite,
                    host: "local".into(),
                    port: 0,
                    database_name: "main".into(),
                    schema_name: None,
                    username: None,
                    password: None,
                },
            )
            .await
            .unwrap();

        let result = service
            .test_connection(
                &tenant(),
                &source.id,
                Arc::new(StubDb {
                    version_sql: DatabaseType::SQLite.version_query(),
                    health_error: None,
                }),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.server_version.as_deref(), Some("engine 1.0"));
    }

    #[tokio::test]
    async fn test_connection_returns_failed_result_for_health_errors() {
        let service = service();
        let source = service
            .create(
                &tenant(),
                CreateDataSourceRequest {
                    name: "Warehouse".into(),
                    database_type: DatabaseType::PostgreSQL,
                    host: "localhost".into(),
                    port: 5432,
                    database_name: "warehouse".into(),
                    schema_name: Some("public".into()),
                    username: None,
                    password: None,
                },
            )
            .await
            .unwrap();

        let result = service
            .test_connection(
                &tenant(),
                &source.id,
                Arc::new(StubDb {
                    version_sql: DatabaseType::PostgreSQL.version_query(),
                    health_error: Some(DbError::Connection("refused".into())),
                }),
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("Connection error: refused"));
    }
}
