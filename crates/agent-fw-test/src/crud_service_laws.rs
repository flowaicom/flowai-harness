//! Proptest law verification for `CrudService<T>`.
//!
//! # Laws
//!
//! - **L1 (Roundtrip)**: `save(id, e); get(id) == Some(normalize(e))` when validation passes.
//! - **L2 (Validated gate)**: Invalid input → all errors returned, no IO side effects.
//! - **L3 (Delete idempotence)**: `delete(id); delete(id)` succeeds (no error on second delete).
//! - **L4 (Create uniqueness)**: `create(id, e); create(id, e2)` → `Err(AlreadyExists)`.
//! - **L5 (Update existence)**: `update(id, e)` when no entity → `Err(NotFound)`.

use agent_fw_algebra::validated::Validated;
use agent_fw_workspace::crud_service::{self, CrudError};
use agent_fw_workspace::indexed_entity::{EntityConfig, IndexedEntity};
use std::sync::Arc;

static TEST_CONFIG: EntityConfig = EntityConfig {
    key_prefix: "law_crud",
    index_key: "law_cruds:index",
};

fn make_kv() -> Arc<dyn agent_fw_algebra::KVStore> {
    Arc::new(agent_fw_interpreter::DashMapKVStore::new())
}

/// Predicate validation: "name" field must exist and be non-empty.
fn validate(item: &serde_json::Value) -> Validated<(), String> {
    match item.get("name").and_then(|v| v.as_str()) {
        Some(name) if !name.is_empty() => Validated::Valid(()),
        Some(_) => Validated::Invalid(vec!["name must not be empty".into()]),
        None => Validated::Invalid(vec!["name is required".into()]),
    }
}

/// L1: Roundtrip — save a valid entity, get it back.
pub async fn test_l1_roundtrip() {
    let kv = make_kv();
    let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
    let svc = crud_service::with_predicate(ie, validate);

    let entity = serde_json::json!({"name": "test"});
    svc.save("e1", &entity).await.unwrap();

    let got: Option<serde_json::Value> = svc.get("e1").await.unwrap();
    assert_eq!(got.unwrap(), entity);
}

/// L2: Validated gate — invalid input returns errors, no side effects.
pub async fn test_l2_validated_gate() {
    let kv = make_kv();
    let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
    let svc = crud_service::with_predicate(ie, validate);

    let bad = serde_json::json!({"name": ""});
    let err = svc.save("e1", &bad).await.unwrap_err();
    assert!(matches!(err, CrudError::Validation(ref v) if !v.is_empty()));

    // Should not exist
    let got: Option<serde_json::Value> = svc.get("e1").await.unwrap();
    assert!(got.is_none());
}

/// L3: Delete idempotence — double delete is safe.
pub async fn test_l3_delete_idempotence() {
    let kv = make_kv();
    let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
    let svc = crud_service::with_predicate(ie, validate);

    let entity = serde_json::json!({"name": "test"});
    svc.save("e1", &entity).await.unwrap();

    let first = svc.delete("e1").await.unwrap();
    assert!(first);

    let second = svc.delete("e1").await.unwrap();
    assert!(!second); // already gone, but no error
}

/// L4: Create uniqueness — second create on same ID fails.
pub async fn test_l4_create_uniqueness() {
    let kv = make_kv();
    let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
    let svc = crud_service::with_predicate(ie, validate);

    let entity = serde_json::json!({"name": "first"});
    svc.create("e1", &entity).await.unwrap();

    let entity2 = serde_json::json!({"name": "second"});
    let err = svc.create("e1", &entity2).await.unwrap_err();
    assert!(matches!(err, CrudError::AlreadyExists(_)));

    // Original value unchanged
    let got: serde_json::Value = svc.get("e1").await.unwrap().unwrap();
    assert_eq!(got, entity);
}

/// L5: Update existence — update on missing ID fails.
pub async fn test_l5_update_existence() {
    let kv = make_kv();
    let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
    let svc = crud_service::with_predicate(ie, validate);

    let entity = serde_json::json!({"name": "test"});
    let err = svc.update("nonexistent", &entity).await.unwrap_err();
    assert!(matches!(err, CrudError::NotFound(_)));
}

/// Run all CrudService laws.
pub async fn test_all() {
    test_l1_roundtrip().await;
    test_l2_validated_gate().await;
    test_l3_delete_idempotence().await;
    test_l4_create_uniqueness().await;
    test_l5_update_existence().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_all_laws() {
        test_all().await;
    }

    /// Multi-error accumulation: validation produces ALL errors at once.
    #[tokio::test]
    async fn multi_error_accumulation() {
        fn strict_validate(item: &serde_json::Value) -> Validated<(), String> {
            let mut errors = Vec::new();
            match item.get("name").and_then(|v| v.as_str()) {
                Some(n) if n.is_empty() => errors.push("name empty".into()),
                None => errors.push("name missing".into()),
                _ => {}
            }
            match item.get("age").and_then(|v| v.as_i64()) {
                Some(a) if a < 0 => errors.push("age negative".into()),
                None => errors.push("age missing".into()),
                _ => {}
            }
            if errors.is_empty() {
                Validated::Valid(())
            } else {
                Validated::Invalid(errors)
            }
        }

        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "law-tenant", &TEST_CONFIG);
        let svc = crud_service::with_predicate(ie, strict_validate);

        // Both fields invalid
        let bad = serde_json::json!({"name": "", "age": -1});
        let err = svc.save("e1", &bad).await.unwrap_err();
        match err {
            CrudError::Validation(errors) => {
                assert_eq!(errors.len(), 2);
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }
    }
}
