//! Database provisioner — environment lifecycle management.
//!
//! The `DatabaseProvisioner` trait abstracts environment provisioning
//! (create/destroy database branches, get connection credentials).
//!
//! # Laws
//!
//! L1 (Provision-List): After provision, list includes new environment
//! L2 (Deprovision-Removes): After deprovision, list excludes it
//! L3 (Name-Uniqueness): Two provisions with same name fail on second
//! L4 (Connection-Valid): Provisioned connection can connect
//! L5 (Deprovision-NotFound): Deprovisioning non-existent returns NotFound

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use thiserror::Error;

/// A provisioned environment identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnvironmentId(String);

impl EnvironmentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvironmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for EnvironmentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for EnvironmentId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A human-readable environment name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for EnvironmentName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Provisioning error.
#[derive(Debug, Error)]
pub enum ProvisioningError {
    #[error("Environment not found: {0}")]
    NotFound(String),

    #[error("Environment name conflict: {0}")]
    Conflict(String),

    #[error("API error: {status} {message}")]
    Api { status: u16, message: String },

    #[error("Network error: {0}")]
    Network(String),

    #[error("Database provisioner not configured")]
    NotConfigured,
}

/// Request to provision a new environment.
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionRequest {
    pub name: EnvironmentName,
    pub parent_id: Option<EnvironmentId>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// A successfully provisioned environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedEnvironment {
    pub id: EnvironmentId,
    pub name: EnvironmentName,
    pub parent_id: Option<EnvironmentId>,
    pub host: String,
    pub current_state: String,
    pub created_at: String,
}

/// Summary of an environment (for listing).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSummary {
    pub id: EnvironmentId,
    pub name: EnvironmentName,
    pub current_state: String,
    pub created_at: String,
}

/// Connection credentials for a provisioned environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedConnection {
    pub host: String,
    pub connection_uri: Option<String>,
    pub role_name: String,
    pub role_password: String,
}

/// Async database environment provisioner.
#[async_trait]
pub trait DatabaseProvisioner: Send + Sync {
    /// Provision a new database environment.
    async fn provision(
        &self,
        req: ProvisionRequest,
    ) -> Result<ProvisionedEnvironment, ProvisioningError>;

    /// Destroy a provisioned environment.
    async fn deprovision(&self, env_id: &EnvironmentId) -> Result<(), ProvisioningError>;

    /// List all provisioned environments.
    async fn list_environments(&self) -> Result<Vec<EnvironmentSummary>, ProvisioningError>;

    /// Get connection credentials for an environment.
    async fn get_connection(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        role_name: &str,
    ) -> Result<ProvisionedConnection, ProvisioningError>;

    /// Create a database within an environment.
    async fn create_database(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        owner_name: &str,
    ) -> Result<(), ProvisioningError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_id_display() {
        let id = EnvironmentId::new("env-123");
        assert_eq!(id.to_string(), "env-123");
        assert_eq!(id.as_str(), "env-123");
        assert_eq!(&*id, "env-123"); // Deref
    }

    #[test]
    fn environment_name_serde() {
        let name = EnvironmentName::new("my-branch");
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"my-branch\"");
        let parsed: EnvironmentName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.as_str(), "my-branch");
    }

    #[test]
    fn provisioned_environment_serde() {
        let env = ProvisionedEnvironment {
            id: EnvironmentId::new("env-1"),
            name: EnvironmentName::new("test-branch"),
            parent_id: Some(EnvironmentId::new("env-0")),
            host: "db.example.com".into(),
            current_state: "ready".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: ProvisionedEnvironment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id.as_str(), "env-1");
        assert_eq!(parsed.parent_id.unwrap().as_str(), "env-0");
    }
}
