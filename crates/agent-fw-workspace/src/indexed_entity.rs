//! IndexedEntity — generic KV CRUD with secondary index management.
//!
//! Factors out the repetitive pattern of storing entities in KV with a
//! separate index key tracking all IDs per tenant.
//!
//! # Laws
//!
//! - **L1 (Roundtrip)**: `put(id, e); get(id) == Some(e)`
//! - **L2 (Index consistency)**: `put(e) => list_ids() contains e.id`
//! - **L3 (Delete removes)**: `delete(id); get(id) == None AND list_ids() !contains id`
//! - **L4 (Upsert idempotence)**: `put(e); put(e) => list_ids() has exactly one e.id`

use agent_fw_algebra::{KVError, KVStore};
use serde::{de::DeserializeOwned, Serialize};

/// Configuration for a KV-backed entity with index.
///
/// Defines the key prefix and index key for a particular entity type.
#[derive(Debug, Clone)]
pub struct EntityConfig {
    /// Prefix for entity keys.
    ///
    /// If `key_prefix` already ends with `:`, it is treated as a complete
    /// prefix and used verbatim (`"data:document:"` → `"data:document:{id}"`).
    /// Otherwise the canonical `:` separator is inserted
    /// (`"thread"` → `"thread:{id}"`).
    pub key_prefix: &'static str,
    /// Key for the ID index (e.g., `"threads:index"`).
    pub index_key: &'static str,
}

impl EntityConfig {
    /// Generate the entity key for a given ID.
    pub fn entity_key(&self, id: &str) -> String {
        if self.key_prefix.ends_with(':') {
            format!("{}{}", self.key_prefix, id)
        } else {
            format!("{}:{}", self.key_prefix, id)
        }
    }
}

/// Typed index tracking entity IDs per tenant.
///
/// Unified structure shared across all entity types. Serde aliases
/// handle legacy field names from prior index formats.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "T: Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
pub struct IdIndex<T> {
    #[serde(
        default,
        alias = "thread_ids",
        alias = "case_ids",
        alias = "caseIds",
        alias = "run_ids",
        alias = "source_ids",
        alias = "workspace_ids"
    )]
    pub ids: Vec<T>,
}

impl<T> Default for IdIndex<T> {
    fn default() -> Self {
        Self { ids: Vec::new() }
    }
}

/// Backward-compatible string ID index used by generic KV CRUD helpers.
pub type EntityIndex = IdIndex<String>;

/// Generic CRUD + index operations over KV.
///
/// Wraps a `KVStore` reference and provides typed entity access
/// with automatic index management.
///
/// # Concurrency
///
/// `IndexedEntity` is NOT safe for concurrent writers to the same
/// `(tenant, config)` pair. The index update is a non-atomic
/// read-modify-write. Callers must ensure single-writer discipline
/// per `(tenant, config)` — typically enforced by the pipeline or
/// orchestrator that owns the entity.
pub struct IndexedEntity<'a, K: KVStore + ?Sized = dyn KVStore> {
    kv: &'a K,
    tenant: &'a str,
    config: &'a EntityConfig,
}

impl<'a, K: KVStore + ?Sized> IndexedEntity<'a, K> {
    /// Create a new indexed entity accessor.
    pub fn new(kv: &'a K, tenant: &'a str, config: &'a EntityConfig) -> Self {
        Self { kv, tenant, config }
    }

    /// Store an entity and add its ID to the index.
    ///
    /// # Law L4 (Upsert idempotence)
    /// If the ID already exists in the index, it is not duplicated.
    pub async fn put<T: Serialize>(&self, id: &str, entity: &T) -> Result<(), IndexedEntityError> {
        let key = self.config.entity_key(id);
        let value = serde_json::to_value(entity).map_err(IndexedEntityError::serde)?;
        self.kv
            .put_json(self.tenant, &key, value, None)
            .await
            .map_err(IndexedEntityError::kv)?;

        let mut idx = self.load_index().await?;
        if !idx.ids.contains(&id.to_string()) {
            idx.ids.insert(0, id.to_string());
            self.save_index(&idx).await?;
        }
        Ok(())
    }

    /// Retrieve an entity by ID.
    pub async fn get<T: DeserializeOwned>(
        &self,
        id: &str,
    ) -> Result<Option<T>, IndexedEntityError> {
        let key = self.config.entity_key(id);
        let val = self
            .kv
            .get_json(self.tenant, &key)
            .await
            .map_err(IndexedEntityError::kv)?;
        match val {
            Some(v) => {
                let entity = serde_json::from_value(v).map_err(IndexedEntityError::serde)?;
                Ok(Some(entity))
            }
            None => Ok(None),
        }
    }

    /// Delete an entity by ID, removing it from both storage and index.
    ///
    /// Returns `true` if the entity existed.
    pub async fn delete(&self, id: &str) -> Result<bool, IndexedEntityError> {
        let key = self.config.entity_key(id);
        let existed = self
            .kv
            .delete(self.tenant, &key)
            .await
            .map_err(IndexedEntityError::kv)?;

        let mut idx = self.load_index().await?;
        idx.ids.retain(|existing| existing != id);
        self.save_index(&idx).await?;

        Ok(existed)
    }

    /// List all entity IDs in the index.
    pub async fn list_ids(&self) -> Result<Vec<String>, IndexedEntityError> {
        let idx = self.load_index().await?;
        Ok(idx.ids)
    }

    /// List all entities (loads each by ID from the index).
    pub async fn list<T: DeserializeOwned>(&self) -> Result<Vec<T>, IndexedEntityError> {
        let idx = self.load_index().await?;
        let mut entities = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(entity) = self.get::<T>(id).await? {
                entities.push(entity);
            }
        }
        Ok(entities)
    }

    /// Delete multiple entities by ID.
    ///
    /// Returns the count of entities that actually existed.
    pub async fn batch_delete(&self, ids: &[String]) -> Result<u64, IndexedEntityError> {
        let mut count = 0u64;
        for id in ids {
            let key = self.config.entity_key(id);
            if self
                .kv
                .delete(self.tenant, &key)
                .await
                .map_err(IndexedEntityError::kv)?
            {
                count += 1;
            }
        }

        let mut idx = self.load_index().await?;
        idx.ids.retain(|existing| !ids.contains(existing));
        self.save_index(&idx).await?;

        Ok(count)
    }

    // =============================================================================
    // Internal helpers
    // =============================================================================

    async fn load_index(&self) -> Result<EntityIndex, IndexedEntityError> {
        let val = self
            .kv
            .get_json(self.tenant, self.config.index_key)
            .await
            .map_err(IndexedEntityError::kv)?;
        match val {
            Some(v) => serde_json::from_value(v).map_err(IndexedEntityError::serde),
            None => Ok(EntityIndex::default()),
        }
    }

    async fn save_index(&self, index: &EntityIndex) -> Result<(), IndexedEntityError> {
        let v = serde_json::to_value(index).map_err(IndexedEntityError::serde)?;
        self.kv
            .put_json(self.tenant, self.config.index_key, v, None)
            .await
            .map_err(IndexedEntityError::kv)
    }
}

/// Errors from indexed entity operations.
#[derive(Debug, thiserror::Error)]
pub enum IndexedEntityError {
    #[error("KV error: {0}")]
    KV(#[from] KVError),
    #[error("Serialization error: {0}")]
    Serde(String),
}

impl IndexedEntityError {
    fn kv(e: KVError) -> Self {
        Self::KV(e)
    }

    fn serde(e: serde_json::Error) -> Self {
        Self::Serde(e.to_string())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    static TEST_CONFIG: EntityConfig = EntityConfig {
        key_prefix: "test_entity",
        index_key: "test_entities:index",
    };

    fn make_kv() -> Arc<dyn KVStore> {
        Arc::new(agent_fw_interpreter::DashMapKVStore::new())
    }

    // =========================================================================
    // L1: Roundtrip
    // =========================================================================

    #[tokio::test]
    async fn l1_roundtrip() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        let value = serde_json::json!({"name": "test", "version": 1});
        ie.put("e1", &value).await.unwrap();

        let got: Option<serde_json::Value> = ie.get("e1").await.unwrap();
        assert_eq!(got.unwrap(), value);
    }

    // =========================================================================
    // L2: Index consistency
    // =========================================================================

    #[tokio::test]
    async fn l2_put_adds_to_index() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        ie.put("e1", &serde_json::json!({"id": "e1"}))
            .await
            .unwrap();
        ie.put("e2", &serde_json::json!({"id": "e2"}))
            .await
            .unwrap();

        let ids = ie.list_ids().await.unwrap();
        assert!(ids.contains(&"e1".to_string()));
        assert!(ids.contains(&"e2".to_string()));
    }

    // =========================================================================
    // L3: Delete removes
    // =========================================================================

    #[tokio::test]
    async fn l3_delete_removes_entity_and_index() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        ie.put("e1", &serde_json::json!({"id": "e1"}))
            .await
            .unwrap();
        ie.delete("e1").await.unwrap();

        let got: Option<serde_json::Value> = ie.get("e1").await.unwrap();
        assert!(got.is_none());

        let ids = ie.list_ids().await.unwrap();
        assert!(!ids.contains(&"e1".to_string()));
    }

    // =========================================================================
    // L4: Upsert idempotence
    // =========================================================================

    #[tokio::test]
    async fn l4_upsert_does_not_duplicate_index() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        ie.put("e1", &serde_json::json!({"v": 1})).await.unwrap();
        ie.put("e1", &serde_json::json!({"v": 2})).await.unwrap();

        let ids = ie.list_ids().await.unwrap();
        assert_eq!(ids.iter().filter(|id| *id == "e1").count(), 1);

        // Latest value is stored
        let got: serde_json::Value = ie.get("e1").await.unwrap().unwrap();
        assert_eq!(got["v"], 2);
    }

    // =========================================================================
    // list
    // =========================================================================

    #[tokio::test]
    async fn list_returns_all_entities() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        ie.put("e1", &serde_json::json!({"id": "e1"}))
            .await
            .unwrap();
        ie.put("e2", &serde_json::json!({"id": "e2"}))
            .await
            .unwrap();

        let entities: Vec<serde_json::Value> = ie.list().await.unwrap();
        assert_eq!(entities.len(), 2);
    }

    // =========================================================================
    // batch_delete
    // =========================================================================

    #[tokio::test]
    async fn batch_delete_removes_multiple() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        for i in 0..4 {
            ie.put(
                &format!("e{}", i),
                &serde_json::json!({"id": format!("e{}", i)}),
            )
            .await
            .unwrap();
        }

        let count = ie
            .batch_delete(&["e0".into(), "e1".into(), "e99".into()])
            .await
            .unwrap();
        assert_eq!(count, 2); // e99 doesn't exist

        let ids = ie.list_ids().await.unwrap();
        assert!(!ids.contains(&"e0".to_string()));
        assert!(!ids.contains(&"e1".to_string()));
        assert!(ids.contains(&"e2".to_string()));
        assert!(ids.contains(&"e3".to_string()));
    }

    // =========================================================================
    // Tenant isolation
    // =========================================================================

    #[tokio::test]
    async fn tenant_isolation() {
        let kv = make_kv();
        let ie1 = IndexedEntity::new(kv.as_ref(), "tenant-a", &TEST_CONFIG);
        let ie2 = IndexedEntity::new(kv.as_ref(), "tenant-b", &TEST_CONFIG);

        ie1.put("shared-id", &serde_json::json!({"owner": "a"}))
            .await
            .unwrap();

        let got: Option<serde_json::Value> = ie2.get("shared-id").await.unwrap();
        assert!(got.is_none());
    }

    // =========================================================================
    // get missing entity
    // =========================================================================

    #[tokio::test]
    async fn get_missing_returns_none() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        let got: Option<serde_json::Value> = ie.get("nonexistent").await.unwrap();
        assert!(got.is_none());
    }

    // =========================================================================
    // EntityConfig key generation
    // =========================================================================

    #[test]
    fn entity_key_format() {
        assert_eq!(TEST_CONFIG.entity_key("abc"), "test_entity:abc");
    }

    #[test]
    fn entity_key_preserves_complete_prefixes() {
        let cfg = EntityConfig {
            key_prefix: "data:document:",
            index_key: "data:documents:index",
        };

        assert_eq!(cfg.entity_key("doc-1"), "data:document:doc-1");
    }
}
