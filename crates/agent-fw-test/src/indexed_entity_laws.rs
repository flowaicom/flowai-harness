//! IndexedEntity algebraic law test harnesses.
//!
//! Verifies that `IndexedEntity` backed by a `KVStore` satisfies
//! the documented CRUD + index consistency laws.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn indexed_entity_satisfies_laws() {
//!     let store = DashMapKVStore::new();
//!     agent_fw_test::indexed_entity_laws::test_all(&store).await;
//! }
//! ```

use agent_fw_algebra::KVStore;
use agent_fw_workspace::{EntityConfig, IndexedEntity};
use serde::{Deserialize, Serialize};

/// Test entity for law harnesses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestEntity {
    pub id: String,
    pub name: String,
    pub value: u32,
}

/// Config for test entities.
const TEST_CONFIG: EntityConfig = EntityConfig {
    key_prefix: "test:entity",
    index_key: "test:entity:__index",
};

/// Run all deterministic IndexedEntity laws against the given KV store.
pub async fn test_all(store: &dyn KVStore) {
    law_roundtrip(store).await;
    law_index_consistency(store).await;
    law_delete_removes(store).await;
    law_upsert_idempotence(store).await;
    law_tenant_isolation(store).await;
    law_batch_delete(store).await;
    law_list_returns_all(store).await;
}

/// L1 (Roundtrip): put(e); get(e.id) == Some(e).
pub async fn law_roundtrip(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_l1", &TEST_CONFIG);
    let item = TestEntity {
        id: "item-1".into(),
        name: "Alpha".into(),
        value: 42,
    };

    entity.put(&item.id, &item).await.unwrap();
    let retrieved: Option<TestEntity> = entity.get(&item.id).await.unwrap();
    assert_eq!(
        retrieved,
        Some(item),
        "L1: get after put must return stored entity"
    );
}

/// L2 (Index consistency): put(e) => list_ids() contains e.id.
pub async fn law_index_consistency(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_l2", &TEST_CONFIG);
    let item = TestEntity {
        id: "item-idx".into(),
        name: "Beta".into(),
        value: 7,
    };

    entity.put(&item.id, &item).await.unwrap();
    let ids = entity.list_ids().await.unwrap();
    assert!(
        ids.contains(&"item-idx".to_string()),
        "L2: list_ids must contain put entity's id"
    );
}

/// L3 (Delete removes): delete(e.id) => get(e.id) == None AND list_ids() !contains e.id.
pub async fn law_delete_removes(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_l3", &TEST_CONFIG);
    let item = TestEntity {
        id: "item-del".into(),
        name: "Gamma".into(),
        value: 99,
    };

    entity.put(&item.id, &item).await.unwrap();
    let deleted = entity.delete(&item.id).await.unwrap();
    assert!(deleted, "L3: delete of existing entity must return true");

    let retrieved: Option<TestEntity> = entity.get(&item.id).await.unwrap();
    assert_eq!(retrieved, None, "L3: get after delete must return None");

    let ids = entity.list_ids().await.unwrap();
    assert!(
        !ids.contains(&"item-del".to_string()),
        "L3: list_ids must not contain deleted entity's id"
    );
}

/// L4 (Upsert idempotence): put(e); put(e) => list_ids() has exactly one e.id.
pub async fn law_upsert_idempotence(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_l4", &TEST_CONFIG);
    let item = TestEntity {
        id: "item-upsert".into(),
        name: "Delta".into(),
        value: 1,
    };

    entity.put(&item.id, &item).await.unwrap();
    // Update with different data
    let updated = TestEntity {
        id: "item-upsert".into(),
        name: "Delta Updated".into(),
        value: 2,
    };
    entity.put(&updated.id, &updated).await.unwrap();

    let ids = entity.list_ids().await.unwrap();
    let count = ids.iter().filter(|id| id.as_str() == "item-upsert").count();
    assert_eq!(count, 1, "L4: upsert must not duplicate index entry");

    let retrieved: Option<TestEntity> = entity.get("item-upsert").await.unwrap();
    assert_eq!(
        retrieved,
        Some(updated),
        "L4: upsert must store latest value"
    );
}

/// Tenant isolation: entities in one tenant are invisible to another.
pub async fn law_tenant_isolation(store: &dyn KVStore) {
    let entity_a = IndexedEntity::new(store, "tenant-a", &TEST_CONFIG);
    let entity_b = IndexedEntity::new(store, "tenant-b", &TEST_CONFIG);

    let item = TestEntity {
        id: "shared-id".into(),
        name: "Isolated".into(),
        value: 42,
    };

    entity_a.put(&item.id, &item).await.unwrap();

    // Tenant B should not see tenant A's entity
    let retrieved: Option<TestEntity> = entity_b.get(&item.id).await.unwrap();
    assert_eq!(
        retrieved, None,
        "Tenant isolation: tenant B must not see tenant A's entity"
    );

    let ids_b = entity_b.list_ids().await.unwrap();
    assert!(
        !ids_b.contains(&"shared-id".to_string()),
        "Tenant isolation: tenant B's index must not contain tenant A's entity"
    );
}

/// Batch delete removes multiple entities at once.
pub async fn law_batch_delete(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_batch", &TEST_CONFIG);

    for i in 0..5 {
        let item = TestEntity {
            id: format!("batch-{i}"),
            name: format!("Item {i}"),
            value: i,
        };
        entity.put(&item.id, &item).await.unwrap();
    }

    let to_delete: Vec<String> = (0..3).map(|i| format!("batch-{i}")).collect();
    let deleted = entity.batch_delete(&to_delete).await.unwrap();
    assert_eq!(
        deleted, 3,
        "batch_delete must report count of deleted entities"
    );

    let ids = entity.list_ids().await.unwrap();
    assert_eq!(ids.len(), 2, "batch_delete must leave remaining entities");
    assert!(ids.contains(&"batch-3".to_string()));
    assert!(ids.contains(&"batch-4".to_string()));
}

/// list() returns all entities (not just IDs).
pub async fn law_list_returns_all(store: &dyn KVStore) {
    let entity = IndexedEntity::new(store, "t_list", &TEST_CONFIG);

    let items: Vec<TestEntity> = (0..3)
        .map(|i| TestEntity {
            id: format!("list-{i}"),
            name: format!("Item {i}"),
            value: i,
        })
        .collect();

    for item in &items {
        entity.put(&item.id, item).await.unwrap();
    }

    let listed: Vec<TestEntity> = entity.list().await.unwrap();
    assert_eq!(listed.len(), 3, "list must return all entities");
    for item in &items {
        assert!(listed.contains(item), "list must contain all put entities");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_indexed_entity_laws_with_dashmap() {
        // Use DashMapKVStore via agent-fw-interpreter (dev-dependency)
        let store = agent_fw_interpreter::DashMapKVStore::new();
        test_all(&store).await;
    }
}
