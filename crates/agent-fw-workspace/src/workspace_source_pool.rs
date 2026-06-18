//! Workspace-backed PostgreSQL source pool.
//!
//! This composes the workspace data-source registry with the encryption algebra
//! and a caller-supplied target-database factory. It removes the need for each
//! application to re-implement:
//!
//! - tenant-scoped source lookup
//! - encrypted credential decoding
//! - PostgreSQL pool caching
//! - target-db cache invalidation

use std::sync::Arc;

use agent_fw_algebra::{EncryptedPayload, EncryptionService, TargetDatabase};
use agent_fw_catalog::{DataSource, DatabaseType};
use agent_fw_core::TenantId;
use dashmap::DashMap;
use futures::future::BoxFuture;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

use crate::{DataSourceStore, WorkspaceError};

/// Decoded database credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseCredentials {
    pub username: String,
    pub password: String,
}

/// Errors from [`WorkspacePostgresSourcePool`].
#[derive(Debug, thiserror::Error)]
pub enum WorkspacePostgresSourcePoolError {
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("data source not found: {0}")]
    NotFound(String),

    #[error("unsupported database type: {0}")]
    UnsupportedType(String),

    #[error("invalid encrypted credentials: {0}")]
    InvalidEncryptedCredentials(String),

    #[error("failed to decrypt credentials: {0}")]
    DecryptFailed(String),

    #[error("invalid credential json: {0}")]
    InvalidCredentialJson(String),

    #[error("failed to connect to data source '{source_id}': {message}")]
    ConnectionFailed { source_id: String, message: String },
}

type TargetDbFactory<DB> = dyn Fn(PgPool, &DataSource) -> Arc<DB> + Send + Sync;
type PgPoolConnector = dyn Fn(
        DataSource,
        DatabaseCredentials,
    ) -> BoxFuture<'static, Result<PgPool, WorkspacePostgresSourcePoolError>>
    + Send
    + Sync;

/// Workspace-backed dynamic PostgreSQL source pool.
///
/// The local application provides the final `TargetDatabase` construction step.
/// Everything else is framework-owned:
///
/// - source lookup from `DataSourceStore`
/// - encrypted credential decoding
/// - tenant-scoped pool caching
/// - target-db object caching
pub struct WorkspacePostgresSourcePool<DB: ?Sized> {
    target_dbs: DashMap<String, Arc<DB>>,
    write_pools: DashMap<String, PgPool>,
    source_store: Arc<dyn DataSourceStore>,
    encryption: Arc<dyn EncryptionService>,
    factory: Arc<TargetDbFactory<DB>>,
    pool_connector: Arc<PgPoolConnector>,
}

impl<DB> WorkspacePostgresSourcePool<DB>
where
    DB: TargetDatabase + ?Sized + 'static,
{
    /// Create a new workspace-backed PostgreSQL source pool using the stock
    /// `sqlx` connector.
    pub fn new(
        source_store: Arc<dyn DataSourceStore>,
        encryption: Arc<dyn EncryptionService>,
        factory: Arc<TargetDbFactory<DB>>,
    ) -> Self {
        Self::new_with_pool_connector(
            source_store,
            encryption,
            factory,
            Arc::new(|source, creds| Box::pin(async move { connect_pg_pool(source, creds).await })),
        )
    }

    /// Create with a custom async pool connector.
    ///
    /// This is the low-level escape hatch for tests or custom connection
    /// policies (TLS, driver options, lazy pools, etc.).
    pub fn new_with_pool_connector(
        source_store: Arc<dyn DataSourceStore>,
        encryption: Arc<dyn EncryptionService>,
        factory: Arc<TargetDbFactory<DB>>,
        pool_connector: Arc<PgPoolConnector>,
    ) -> Self {
        Self {
            target_dbs: DashMap::new(),
            write_pools: DashMap::new(),
            source_store,
            encryption,
            factory,
            pool_connector,
        }
    }

    /// Get or create a target-database handle for a workspace data source.
    pub async fn get_or_create(
        &self,
        tenant: &TenantId,
        source_id: &str,
    ) -> Result<Arc<DB>, WorkspacePostgresSourcePoolError> {
        let key = scoped_pool_key(tenant, source_id);

        if let Some(db) = self.target_dbs.get(&key) {
            return Ok(Arc::clone(db.value()));
        }

        let source = self.load_source(tenant, source_id).await?;
        let pool = self
            .get_or_create_pool_for_source(&key, source.clone())
            .await?;
        let db = (self.factory)(pool, &source);
        self.target_dbs.insert(key, Arc::clone(&db));
        Ok(db)
    }

    /// Get or create the raw PostgreSQL pool for write-capable flows.
    pub async fn get_or_create_pool(
        &self,
        tenant: &TenantId,
        source_id: &str,
    ) -> Result<PgPool, WorkspacePostgresSourcePoolError> {
        let key = scoped_pool_key(tenant, source_id);
        if let Some(pool) = self.write_pools.get(&key) {
            return Ok(pool.value().clone());
        }
        let source = self.load_source(tenant, source_id).await?;
        self.get_or_create_pool_for_source(&key, source).await
    }

    /// Evict a cached source. The underlying pool is dropped when the last
    /// clone is released.
    pub fn evict(&self, tenant: &TenantId, source_id: &str) {
        let key = scoped_pool_key(tenant, source_id);
        self.target_dbs.remove(&key);
        self.write_pools.remove(&key);
    }

    async fn get_or_create_pool_for_source(
        &self,
        key: &str,
        source: DataSource,
    ) -> Result<PgPool, WorkspacePostgresSourcePoolError> {
        if let Some(pool) = self.write_pools.get(key) {
            return Ok(pool.value().clone());
        }

        if source.database_type != DatabaseType::PostgreSQL {
            return Err(WorkspacePostgresSourcePoolError::UnsupportedType(
                source.database_type.to_string(),
            ));
        }

        let creds = self.decrypt_credentials(&source).await?;
        let pool = (self.pool_connector)(source.clone(), creds).await?;
        self.write_pools.insert(key.to_string(), pool.clone());
        Ok(pool)
    }

    async fn load_source(
        &self,
        tenant: &TenantId,
        source_id: &str,
    ) -> Result<DataSource, WorkspacePostgresSourcePoolError> {
        self.source_store
            .get_data_source(tenant, source_id)
            .await?
            .ok_or_else(|| WorkspacePostgresSourcePoolError::NotFound(source_id.to_string()))
    }

    async fn decrypt_credentials(
        &self,
        source: &DataSource,
    ) -> Result<DatabaseCredentials, WorkspacePostgresSourcePoolError> {
        let encrypted_str = match &source.encrypted_credentials {
            Some(s) if !s.is_empty() => s.as_str(),
            _ => return Ok(DatabaseCredentials::default()),
        };

        let payload: EncryptedPayload = serde_json::from_str(encrypted_str).map_err(|e| {
            WorkspacePostgresSourcePoolError::InvalidEncryptedCredentials(e.to_string())
        })?;

        let decrypted = self
            .encryption
            .decrypt(&payload)
            .await
            .map_err(|e| WorkspacePostgresSourcePoolError::DecryptFailed(e.to_string()))?;

        #[derive(serde::Deserialize)]
        struct CredentialsWire {
            username: Option<String>,
            password: Option<String>,
        }

        let wire: CredentialsWire = serde_json::from_slice(&decrypted)
            .map_err(|e| WorkspacePostgresSourcePoolError::InvalidCredentialJson(e.to_string()))?;

        Ok(DatabaseCredentials {
            username: wire.username.unwrap_or_default(),
            password: wire.password.unwrap_or_default(),
        })
    }
}

fn scoped_pool_key(tenant: &TenantId, source_id: &str) -> String {
    format!("{}:{}", tenant.as_str(), source_id)
}

async fn connect_pg_pool(
    source: DataSource,
    creds: DatabaseCredentials,
) -> Result<PgPool, WorkspacePostgresSourcePoolError> {
    let options = PgConnectOptions::new()
        .host(&source.host)
        .port(source.port)
        .database(&source.database_name)
        .username(&creds.username)
        .password(&creds.password);

    PgPoolOptions::new()
        .connect_with(options)
        .await
        .map_err(|e| WorkspacePostgresSourcePoolError::ConnectionFailed {
            source_id: source.id,
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use agent_fw_algebra::{DbError, DbRow, QueryParam, ReadOnlyQuery};
    use agent_fw_interpreter::{DashMapKVStore, NoOpEncryptionService};

    use crate::KVWorkspaceStore;

    struct StubTargetDb {
        label: String,
    }

    #[async_trait::async_trait]
    impl TargetDatabase for StubTargetDb {
        async fn query(
            &self,
            _query: &ReadOnlyQuery,
            _params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            Err(DbError::Execution(format!("stub {}", self.label)))
        }

        async fn health_check(&self) -> Result<(), DbError> {
            Ok(())
        }

        fn timeout(&self) -> Duration {
            Duration::from_secs(30)
        }

        async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
            Ok(Vec::new())
        }

        async fn get_table_columns(&self, _table: &str) -> Result<Vec<DbRow>, DbError> {
            Ok(Vec::new())
        }

        async fn sample_table(
            &self,
            _table: &str,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, DbError> {
            Ok(Vec::new())
        }
    }

    fn lazy_pool_connector(
        source: DataSource,
        creds: DatabaseCredentials,
    ) -> BoxFuture<'static, Result<PgPool, WorkspacePostgresSourcePoolError>> {
        Box::pin(async move {
            let options = PgConnectOptions::new()
                .host(&source.host)
                .port(source.port)
                .database(&source.database_name)
                .username(&creds.username)
                .password(&creds.password);
            Ok(PgPoolOptions::new().connect_lazy_with(options))
        })
    }

    async fn seed_source(
        store: &dyn DataSourceStore,
        encryption: &dyn EncryptionService,
        tenant: &TenantId,
        id: &str,
        name: &str,
    ) {
        let encrypted = encryption
            .encrypt(br#"{"username":"user","password":"pass"}"#)
            .await
            .unwrap();
        let source = DataSource {
            id: id.to_string(),
            name: name.to_string(),
            database_type: DatabaseType::PostgreSQL,
            host: "localhost".to_string(),
            port: 5432,
            database_name: "db".to_string(),
            schema_name: "public".to_string(),
            encrypted_credentials: Some(serde_json::to_string(&encrypted).unwrap()),
            is_active: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        store.upsert_data_source(tenant, &source).await.unwrap();
    }

    #[tokio::test]
    async fn cache_hit_is_tenant_scoped() {
        let kv = Arc::new(DashMapKVStore::new());
        let store = Arc::new(KVWorkspaceStore::new(kv)) as Arc<dyn DataSourceStore>;
        let encryption = Arc::new(NoOpEncryptionService::new()) as Arc<dyn EncryptionService>;

        let tenant_a = TenantId::new_unchecked("tenant-a");
        let tenant_b = TenantId::new_unchecked("tenant-b");
        seed_source(
            store.as_ref(),
            encryption.as_ref(),
            &tenant_a,
            "source-1",
            "A",
        )
        .await;
        seed_source(
            store.as_ref(),
            encryption.as_ref(),
            &tenant_b,
            "source-1",
            "B",
        )
        .await;

        let pool = WorkspacePostgresSourcePool::<dyn TargetDatabase>::new_with_pool_connector(
            Arc::clone(&store),
            Arc::clone(&encryption),
            Arc::new(|_pool, source| {
                Arc::new(StubTargetDb {
                    label: source.name.clone(),
                }) as Arc<dyn TargetDatabase>
            }),
            Arc::new(lazy_pool_connector),
        );

        let first_a = pool.get_or_create(&tenant_a, "source-1").await.unwrap();
        let second_a = pool.get_or_create(&tenant_a, "source-1").await.unwrap();
        let first_b = pool.get_or_create(&tenant_b, "source-1").await.unwrap();

        assert!(Arc::ptr_eq(&first_a, &second_a));
        assert!(!Arc::ptr_eq(&first_a, &first_b));
    }

    #[tokio::test]
    async fn evict_forces_new_target_db_instance() {
        let kv = Arc::new(DashMapKVStore::new());
        let store = Arc::new(KVWorkspaceStore::new(kv)) as Arc<dyn DataSourceStore>;
        let encryption = Arc::new(NoOpEncryptionService::new()) as Arc<dyn EncryptionService>;

        let tenant = TenantId::new_unchecked("tenant-a");
        seed_source(
            store.as_ref(),
            encryption.as_ref(),
            &tenant,
            "source-1",
            "A",
        )
        .await;

        let pool = WorkspacePostgresSourcePool::<dyn TargetDatabase>::new_with_pool_connector(
            Arc::clone(&store),
            Arc::clone(&encryption),
            Arc::new(|_pool, source| {
                Arc::new(StubTargetDb {
                    label: source.name.clone(),
                }) as Arc<dyn TargetDatabase>
            }),
            Arc::new(lazy_pool_connector),
        );

        let first = pool.get_or_create(&tenant, "source-1").await.unwrap();
        pool.evict(&tenant, "source-1");
        let second = pool.get_or_create(&tenant, "source-1").await.unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn missing_source_is_explicit_error() {
        let kv = Arc::new(DashMapKVStore::new());
        let store = Arc::new(KVWorkspaceStore::new(kv)) as Arc<dyn DataSourceStore>;
        let encryption = Arc::new(NoOpEncryptionService::new()) as Arc<dyn EncryptionService>;

        let tenant = TenantId::new_unchecked("tenant-a");
        let pool = WorkspacePostgresSourcePool::<dyn TargetDatabase>::new_with_pool_connector(
            store,
            encryption,
            Arc::new(|_pool, _source| Arc::new(StubTargetDb { label: "x".into() })),
            Arc::new(lazy_pool_connector),
        );

        let err = match pool.get_or_create(&tenant, "missing").await {
            Ok(_) => panic!("expected missing source error"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            WorkspacePostgresSourcePoolError::NotFound(id) if id == "missing"
        ));
    }

    #[tokio::test]
    async fn rejects_non_postgres_sources() {
        let kv = Arc::new(DashMapKVStore::new());
        let store = Arc::new(KVWorkspaceStore::new(kv)) as Arc<dyn DataSourceStore>;
        let encryption = Arc::new(NoOpEncryptionService::new()) as Arc<dyn EncryptionService>;

        let tenant = TenantId::new_unchecked("tenant-a");
        let source = DataSource {
            id: "source-1".to_string(),
            name: "sqlite".to_string(),
            database_type: DatabaseType::SQLite,
            host: String::new(),
            port: 0,
            database_name: "db.sqlite".to_string(),
            schema_name: "main".to_string(),
            encrypted_credentials: None,
            is_active: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        store.upsert_data_source(&tenant, &source).await.unwrap();

        let pool = WorkspacePostgresSourcePool::<dyn TargetDatabase>::new_with_pool_connector(
            store,
            encryption,
            Arc::new(|_pool, _source| Arc::new(StubTargetDb { label: "x".into() })),
            Arc::new(lazy_pool_connector),
        );

        let err = match pool.get_or_create(&tenant, "source-1").await {
            Ok(_) => panic!("expected unsupported type error"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            WorkspacePostgresSourcePoolError::UnsupportedType(kind) if kind == "sqlite"
        ));
    }
}
