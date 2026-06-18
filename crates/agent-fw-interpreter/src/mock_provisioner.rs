//! Mock DatabaseProvisioner — in-memory environment tracking for testing.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use agent_fw_catalog::{
    DatabaseProvisioner, EnvironmentId, EnvironmentSummary, ProvisionRequest,
    ProvisionedConnection, ProvisionedEnvironment, ProvisioningError,
};

#[cfg(test)]
use agent_fw_catalog::EnvironmentName;

struct MockEnvironment {
    env: ProvisionedEnvironment,
}

/// In-memory provisioner that tracks environments in a Vec.
pub struct MockProvisioner {
    envs: Arc<RwLock<Vec<MockEnvironment>>>,
    next_id: Arc<RwLock<u64>>,
}

impl MockProvisioner {
    pub fn new() -> Self {
        Self {
            envs: Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(RwLock::new(1)),
        }
    }
}

impl Default for MockProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DatabaseProvisioner for MockProvisioner {
    async fn provision(
        &self,
        req: ProvisionRequest,
    ) -> Result<ProvisionedEnvironment, ProvisioningError> {
        // Single write lock for the entire provision — prevents TOCTOU race.
        let mut envs = self.envs.write().await;

        if envs.iter().any(|e| e.env.name == req.name) {
            return Err(ProvisioningError::Conflict(req.name.to_string()));
        }

        let mut id_counter = self.next_id.write().await;
        let id = EnvironmentId::new(format!("mock-env-{}", *id_counter));
        *id_counter += 1;
        drop(id_counter);

        let env = ProvisionedEnvironment {
            id,
            name: req.name,
            parent_id: req.parent_id,
            host: "localhost".into(),
            current_state: "ready".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
        };

        envs.push(MockEnvironment { env: env.clone() });
        Ok(env)
    }

    async fn deprovision(&self, env_id: &EnvironmentId) -> Result<(), ProvisioningError> {
        let mut envs = self.envs.write().await;
        let before = envs.len();
        envs.retain(|e| e.env.id != *env_id);
        if envs.len() == before {
            return Err(ProvisioningError::NotFound(env_id.to_string()));
        }
        Ok(())
    }

    async fn list_environments(&self) -> Result<Vec<EnvironmentSummary>, ProvisioningError> {
        let envs = self.envs.read().await;
        Ok(envs
            .iter()
            .map(|e| EnvironmentSummary {
                id: e.env.id.clone(),
                name: e.env.name.clone(),
                current_state: e.env.current_state.clone(),
                created_at: e.env.created_at.clone(),
            })
            .collect())
    }

    async fn get_connection(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        role_name: &str,
    ) -> Result<ProvisionedConnection, ProvisioningError> {
        let envs = self.envs.read().await;
        let env = envs
            .iter()
            .find(|e| e.env.id == *env_id)
            .ok_or_else(|| ProvisioningError::NotFound(env_id.to_string()))?;

        Ok(ProvisionedConnection {
            host: env.env.host.clone(),
            connection_uri: Some(format!(
                "postgresql://{}:mock-pw@{}/{}",
                role_name, env.env.host, database_name
            )),
            role_name: role_name.into(),
            role_password: "mock-password".into(),
        })
    }

    async fn create_database(
        &self,
        env_id: &EnvironmentId,
        _database_name: &str,
        _owner_name: &str,
    ) -> Result<(), ProvisioningError> {
        let envs = self.envs.read().await;
        if !envs.iter().any(|e| e.env.id == *env_id) {
            return Err(ProvisioningError::NotFound(env_id.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn provision_and_list() {
        let prov = MockProvisioner::new();
        let env = prov
            .provision(ProvisionRequest {
                name: EnvironmentName::new("test-branch"),
                parent_id: None,
                expires_at: None,
            })
            .await
            .unwrap();

        assert_eq!(env.current_state, "ready");

        let envs = prov.list_environments().await.unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name.as_str(), "test-branch");
    }

    #[tokio::test]
    async fn name_conflict() {
        let prov = MockProvisioner::new();
        prov.provision(ProvisionRequest {
            name: EnvironmentName::new("branch-1"),
            parent_id: None,
            expires_at: None,
        })
        .await
        .unwrap();

        let result = prov
            .provision(ProvisionRequest {
                name: EnvironmentName::new("branch-1"),
                parent_id: None,
                expires_at: None,
            })
            .await;
        assert!(matches!(result, Err(ProvisioningError::Conflict(_))));
    }

    #[tokio::test]
    async fn deprovision_removes() {
        let prov = MockProvisioner::new();
        let env = prov
            .provision(ProvisionRequest {
                name: EnvironmentName::new("temp"),
                parent_id: None,
                expires_at: None,
            })
            .await
            .unwrap();

        prov.deprovision(&env.id).await.unwrap();

        let envs = prov.list_environments().await.unwrap();
        assert!(envs.is_empty());
    }

    #[tokio::test]
    async fn deprovision_not_found() {
        let prov = MockProvisioner::new();
        let result = prov.deprovision(&EnvironmentId::new("nonexistent")).await;
        assert!(matches!(result, Err(ProvisioningError::NotFound(_))));
    }

    #[tokio::test]
    async fn get_connection_for_provisioned() {
        let prov = MockProvisioner::new();
        let env = prov
            .provision(ProvisionRequest {
                name: EnvironmentName::new("conn-test"),
                parent_id: None,
                expires_at: None,
            })
            .await
            .unwrap();

        let conn = prov
            .get_connection(&env.id, "mydb", "myuser")
            .await
            .unwrap();
        assert!(conn.connection_uri.is_some());
        assert_eq!(conn.role_name, "myuser");
    }
}
