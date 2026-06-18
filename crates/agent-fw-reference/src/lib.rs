//! KV-backed typed references and cached glimpses.
//!
//! This crate owns the framework-level reference primitive: a stable
//! [`ArtifactRef`] handle, the stored reference envelope, and a registry trait
//! with a KV-backed interpreter. Harnesses layer schema validation and
//! language-specific glimpse callbacks on top.

use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};
use agent_fw_core::id::tenant_scoped_hash;
use agent_fw_core::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const KV_PREFIX: &str = "reference";

/// Typed artifact reference used by plans, tools, and reference lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    /// Artifact kind, usually matching a harness reference declaration name.
    pub kind: String,
    /// Content-addressed artifact identifier.
    pub id: String,
}

/// Persisted body for a reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredReference {
    pub kind: String,
    pub resource_id: TenantId,
    pub value: JsonValue,
    pub glimpse: JsonValue,
    pub created_at: DateTime<Utc>,
}

/// Errors surfaced by reference registries.
#[derive(Debug, thiserror::Error)]
pub enum ReferenceError {
    /// No body exists for this `(tenant, kind, id)` triple.
    #[error("reference not found: kind={kind} id={id}")]
    NotFound { kind: String, id: String },
    /// Underlying KV store error.
    #[error("kv error: {0}")]
    Storage(String),
}

impl From<KVError> for ReferenceError {
    fn from(value: KVError) -> Self {
        Self::Storage(value.to_string())
    }
}

/// Framework reference registry algebra.
#[async_trait]
pub trait ReferenceRegistry: Send + Sync {
    /// Create a tenant-scoped reference, optionally expiring after `ttl`.
    async fn create(
        &self,
        kind: &str,
        value: JsonValue,
        glimpse: JsonValue,
        tenant: &TenantId,
        ttl: Option<Duration>,
    ) -> Result<ArtifactRef, ReferenceError>;

    /// Resolve the full stored reference envelope.
    async fn resolve(
        &self,
        artifact: &ArtifactRef,
        tenant: &TenantId,
    ) -> Result<StoredReference, ReferenceError>;

    /// Resolve only the cached glimpse.
    async fn glimpse(
        &self,
        artifact: &ArtifactRef,
        tenant: &TenantId,
    ) -> Result<JsonValue, ReferenceError> {
        Ok(self.resolve(artifact, tenant).await?.glimpse)
    }
}

/// KV-backed reference registry.
pub struct KvReferenceRegistry {
    kv: Arc<dyn KVStore>,
}

impl KvReferenceRegistry {
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self { kv }
    }
}

impl std::fmt::Debug for KvReferenceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KvReferenceRegistry")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ReferenceRegistry for KvReferenceRegistry {
    async fn create(
        &self,
        kind: &str,
        value: JsonValue,
        glimpse: JsonValue,
        tenant: &TenantId,
        ttl: Option<Duration>,
    ) -> Result<ArtifactRef, ReferenceError> {
        let artifact = ArtifactRef {
            kind: kind.to_string(),
            id: tenant_scoped_hash(tenant, &value),
        };
        let body = StoredReference {
            kind: kind.to_string(),
            resource_id: tenant.clone(),
            value,
            glimpse,
            created_at: Utc::now(),
        };
        self.kv
            .put(tenant.as_str(), &kv_key(&artifact), &body, ttl)
            .await?;
        Ok(artifact)
    }

    async fn resolve(
        &self,
        artifact: &ArtifactRef,
        tenant: &TenantId,
    ) -> Result<StoredReference, ReferenceError> {
        let body: Option<StoredReference> = self
            .kv
            .get::<StoredReference>(tenant.as_str(), &kv_key(artifact))
            .await?;
        let body = body.ok_or_else(|| ReferenceError::NotFound {
            kind: artifact.kind.clone(),
            id: artifact.id.clone(),
        })?;
        if body.resource_id != *tenant {
            return Err(ReferenceError::NotFound {
                kind: artifact.kind.clone(),
                id: artifact.id.clone(),
            });
        }
        Ok(body)
    }
}

fn kv_key(artifact: &ArtifactRef) -> String {
    format!("{KV_PREFIX}:{}:{}", artifact.kind, artifact.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::DashMapKVStore;
    use serde_json::json;

    #[tokio::test]
    async fn create_resolve_and_glimpse_round_trip() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let registry = KvReferenceRegistry::new(kv);
        let tenant = TenantId::new_unchecked("tenant-a");
        let value = json!({"ids": ["a", "b"]});
        let glimpse = json!({"count": 2});

        let artifact = registry
            .create("Selection", value.clone(), glimpse.clone(), &tenant, None)
            .await
            .unwrap();

        let stored = registry.resolve(&artifact, &tenant).await.unwrap();
        assert_eq!(stored.value, value);
        assert_eq!(registry.glimpse(&artifact, &tenant).await.unwrap(), glimpse);
    }
}
