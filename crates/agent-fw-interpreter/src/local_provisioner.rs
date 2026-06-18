//! LocalPostgresProvisioner — PostgreSQL environment provisioning via admin pool.
//!
//! Implements the `DatabaseProvisioner` algebra using a direct PostgreSQL connection
//! for `CREATE DATABASE` / `DROP DATABASE` operations.
//!
//! # Feature Gate
//!
//! Requires the `postgres` feature.
//!
//! # Laws Satisfied
//!
//! - L1 (Provision-List): After provision, list includes the new environment
//! - L2 (Deprovision-Removes): After deprovision, list excludes it
//! - L3 (Name-Uniqueness): Two provisions with the same name fail on the second
//! - L4 (Connection-Valid): Provisioned connection can connect (integration-only)
//! - L5 (Deprovision-NotFound): Deprovisioning non-existent returns NotFound

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::postgres::PgPool;
use tokio::sync::RwLock;

use agent_fw_catalog::{
    DatabaseProvisioner, EnvironmentId, EnvironmentSummary, ProvisionRequest,
    ProvisionedConnection, ProvisionedEnvironment, ProvisioningError,
};

/// Tracked environment: the provisioned metadata plus database names created within it.
struct TrackedEnvironment {
    env: ProvisionedEnvironment,
    /// Database names created via `create_database` — dropped on deprovision.
    databases: Vec<String>,
}

/// PostgreSQL-backed database provisioner using a privileged admin connection.
///
/// Tracks provisioned environments in memory and executes DDL (`CREATE DATABASE`,
/// `DROP DATABASE`) against the admin pool.
pub struct LocalPostgresProvisioner {
    admin_pool: PgPool,
    host: String,
    environments: Arc<RwLock<HashMap<EnvironmentId, TrackedEnvironment>>>,
    next_id: Arc<RwLock<u64>>,
}

impl LocalPostgresProvisioner {
    /// Create a new provisioner with an admin connection pool.
    ///
    /// The `admin_url` is used to derive the host for provisioned connections.
    pub fn new(admin_pool: PgPool, admin_url: &str) -> Self {
        let host = extract_host(admin_url);
        Self {
            admin_pool,
            host,
            environments: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Connect with an admin URL and create the provisioner.
    pub async fn connect(admin_url: &str) -> Result<Self, ProvisioningError> {
        let pool = PgPool::connect(admin_url)
            .await
            .map_err(|e| ProvisioningError::Network(e.to_string()))?;
        Ok(Self::new(pool, admin_url))
    }

    /// Close the admin pool.
    pub async fn close(&self) {
        self.admin_pool.close().await;
    }
}

/// Sanitize a name to a valid database identifier (alphanumeric + underscore).
fn db_prefix(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Extract host from a PostgreSQL URL.
///
/// E.g., `postgresql://user:pass@myhost:5432/db` -> `myhost`
fn extract_host(url: &str) -> String {
    // Strip scheme
    let after_scheme = url
        .strip_prefix("postgresql://")
        .or_else(|| url.strip_prefix("postgres://"))
        .unwrap_or(url);

    // Strip userinfo (user:pass@)
    let after_userinfo = if let Some(at_pos) = after_scheme.find('@') {
        &after_scheme[at_pos + 1..]
    } else {
        after_scheme
    };

    // Extract host (before : or /)
    let host = after_userinfo
        .split(&[':', '/'][..])
        .next()
        .unwrap_or("localhost");

    host.to_string()
}

#[async_trait]
impl DatabaseProvisioner for LocalPostgresProvisioner {
    async fn provision(
        &self,
        req: ProvisionRequest,
    ) -> Result<ProvisionedEnvironment, ProvisioningError> {
        // Single write lock for the entire provision — prevents TOCTOU race
        // where two concurrent provisions with the same name both pass the
        // uniqueness check.
        let mut envs = self.environments.write().await;

        if envs.values().any(|e| e.env.name == req.name) {
            return Err(ProvisioningError::Conflict(req.name.to_string()));
        }

        let mut id_counter = self.next_id.write().await;
        let id = EnvironmentId::new(format!("local-env-{}", *id_counter));
        *id_counter += 1;
        drop(id_counter);

        let env = ProvisionedEnvironment {
            id: id.clone(),
            name: req.name,
            parent_id: req.parent_id,
            host: self.host.clone(),
            current_state: "ready".into(),
            created_at: {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("{now}")
            },
        };

        envs.insert(
            id,
            TrackedEnvironment {
                env: env.clone(),
                databases: Vec::new(),
            },
        );

        Ok(env)
    }

    async fn deprovision(&self, env_id: &EnvironmentId) -> Result<(), ProvisioningError> {
        let removed = self.environments.write().await.remove(env_id);
        match removed {
            Some(tracked) => {
                // Best-effort: drop databases created for this environment.
                for db_name in &tracked.databases {
                    let safe_db = db_prefix(db_name);
                    let sql = format!("DROP DATABASE IF EXISTS \"{safe_db}\"");
                    if let Err(e) = sqlx::query(&sql).execute(&self.admin_pool).await {
                        tracing::warn!(
                            env_id = %env_id,
                            database = %db_name,
                            error = %e,
                            "Best-effort DROP DATABASE failed during deprovision"
                        );
                    }
                }
                Ok(())
            }
            None => Err(ProvisioningError::NotFound(env_id.to_string())),
        }
    }

    async fn list_environments(&self) -> Result<Vec<EnvironmentSummary>, ProvisioningError> {
        let envs = self.environments.read().await;
        Ok(envs
            .values()
            .map(|t| EnvironmentSummary {
                id: t.env.id.clone(),
                name: t.env.name.clone(),
                current_state: t.env.current_state.clone(),
                created_at: t.env.created_at.clone(),
            })
            .collect())
    }

    async fn get_connection(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        role_name: &str,
    ) -> Result<ProvisionedConnection, ProvisioningError> {
        let envs = self.environments.read().await;
        let tracked = envs
            .get(env_id)
            .ok_or_else(|| ProvisioningError::NotFound(env_id.to_string()))?;

        Ok(ProvisionedConnection {
            host: tracked.env.host.clone(),
            connection_uri: Some(format!(
                "postgresql://{}@{}/{}",
                role_name, tracked.env.host, database_name
            )),
            role_name: role_name.into(),
            role_password: String::new(), // Caller must supply credentials
        })
    }

    async fn create_database(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        owner_name: &str,
    ) -> Result<(), ProvisioningError> {
        // Verify environment exists and track database name
        {
            let mut envs = self.environments.write().await;
            let tracked = envs
                .get_mut(env_id)
                .ok_or_else(|| ProvisioningError::NotFound(env_id.to_string()))?;
            tracked.databases.push(database_name.to_string());
        }

        // Sanitize names for SQL
        let safe_db = db_prefix(database_name);
        let safe_owner = db_prefix(owner_name);
        let sql = format!("CREATE DATABASE \"{safe_db}\" OWNER \"{safe_owner}\"");

        sqlx::query(&sql)
            .execute(&self.admin_pool)
            .await
            .map_err(|e| ProvisioningError::Api {
                status: 500,
                message: e.to_string(),
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_prefix_sanitizes() {
        assert_eq!(db_prefix("my-branch"), "my_branch");
        assert_eq!(db_prefix("feature/123"), "feature_123");
        assert_eq!(db_prefix("clean_name"), "clean_name");
        assert_eq!(db_prefix(""), "");
    }

    #[test]
    fn extract_host_from_url() {
        assert_eq!(
            extract_host("postgresql://user:pass@myhost:5432/db"),
            "myhost"
        );
        assert_eq!(
            extract_host("postgres://admin@localhost:5432/admin_db"),
            "localhost"
        );
        assert_eq!(
            extract_host("postgresql://user@db.example.com/mydb"),
            "db.example.com"
        );
    }

    #[test]
    fn extract_host_no_scheme() {
        assert_eq!(extract_host("user:pass@myhost:5432/db"), "myhost");
    }
}
