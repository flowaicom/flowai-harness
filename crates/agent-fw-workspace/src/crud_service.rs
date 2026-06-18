//! `CrudService<T>` — generic CRUD combinator over `IndexedEntity` + `Validated`.
//!
//! Composes `IndexedEntity` (persistence) with applicative validation
//! (`Validated<T, E>`). The validator may normalize/refine the entity —
//! `save` persists the **validated** output, not the raw input.
//!
//! # Laws
//!
//! - **L1 (Roundtrip)**: `save(id, e); get(id) == Some(normalize(e))` when validation passes.
//! - **L2 (Validated gate)**: If `validate(e)` returns `Invalid(errors)`,
//!   `save(id, e)` returns `Err(Validation(errors))` and `get(id)` remains unchanged.
//!   All errors are accumulated (not just the first).
//! - **L3 (Delete idempotence)**: `delete(id); delete(id)` succeeds (no error on second delete).
//! - **L4 (Create uniqueness)**: `create(id, e); create(id, e2)` → `Err(AlreadyExists)`.
//! - **L5 (Update existence)**: `update(id, e)` when no entity → `Err(NotFound)`.

use agent_fw_algebra::validated::Validated;
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;

use crate::indexed_entity::{IndexedEntity, IndexedEntityError};

/// Error type for CRUD operations.
#[derive(Debug, thiserror::Error)]
pub enum CrudError<E: std::fmt::Debug + std::fmt::Display> {
    /// One or more validation errors. No IO was performed.
    #[error("Validation failed: {0:?}")]
    Validation(Vec<E>),
    /// Storage-level error from IndexedEntity.
    #[error("Storage error: {0}")]
    Storage(#[from] IndexedEntityError),
    /// Entity already exists (create refused).
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    /// Entity not found (update refused).
    #[error("Not found: {0}")]
    NotFound(String),
}

/// Generic CRUD service composing `IndexedEntity` with applicative validation.
///
/// `V` is the validation function `Fn(&T) -> Validated<T, E>` — it may
/// normalize/refine the entity (e.g. trim whitespace, default fields).
/// `T` is the entity type (must be `Serialize + DeserializeOwned`).
/// `E` is the validation error type.
///
/// Validation runs before any IO on `save`, `create`, and `update`.
/// If validation returns `Invalid(errors)`, all errors are returned and
/// no side effects occur.
pub struct CrudService<'a, T, V, E> {
    entity: IndexedEntity<'a>,
    validate: V,
    _phantom: PhantomData<(T, E)>,
}

impl<'a, T, V, E> CrudService<'a, T, V, E>
where
    T: Serialize + DeserializeOwned,
    V: Fn(&T) -> Validated<T, E>,
    E: std::fmt::Debug + std::fmt::Display,
{
    /// Create a new CRUD service with a normalizing validator.
    ///
    /// The validator `Fn(&T) -> Validated<T, E>` may refine/normalize the
    /// entity. `save` persists the validated output.
    pub fn new(entity: IndexedEntity<'a>, validate: V) -> Self {
        Self {
            entity,
            validate,
            _phantom: PhantomData,
        }
    }

    /// List all entity IDs.
    pub async fn list_ids(&self) -> Result<Vec<String>, CrudError<E>> {
        Ok(self.entity.list_ids().await?)
    }

    /// List all entities.
    pub async fn list(&self) -> Result<Vec<T>, CrudError<E>> {
        Ok(self.entity.list().await?)
    }

    /// Get a single entity by ID.
    pub async fn get(&self, id: &str) -> Result<Option<T>, CrudError<E>> {
        Ok(self.entity.get(id).await?)
    }

    /// Honest upsert — always succeeds (validation permitting).
    ///
    /// Validates and normalizes the entity, then persists the result.
    /// No existence check — creates or overwrites as needed.
    pub async fn save(&self, id: &str, entity: &T) -> Result<(), CrudError<E>> {
        let normalized = match (self.validate)(entity) {
            Validated::Valid(t) => t,
            Validated::Invalid(errors) => return Err(CrudError::Validation(errors)),
        };
        Ok(self.entity.put(id, &normalized).await?)
    }

    /// Create a new entity. Fails with `AlreadyExists` if entity exists.
    ///
    /// Validates first (L2), then checks existence. The TOCTOU on the
    /// existence check is documented — consistent with `IndexedEntity`'s
    /// single-writer discipline.
    pub async fn create(&self, id: &str, entity: &T) -> Result<(), CrudError<E>> {
        // Check existence BEFORE validation to give the cheapest failure path,
        // but we validate before writing to maintain L2.
        let existing: Option<T> = self.entity.get(id).await?;
        if existing.is_some() {
            return Err(CrudError::AlreadyExists(id.to_string()));
        }
        self.save(id, entity).await
    }

    /// Update an existing entity. Fails with `NotFound` if entity does not exist.
    ///
    /// Validates first (L2), then checks existence.
    pub async fn update(&self, id: &str, entity: &T) -> Result<(), CrudError<E>> {
        let existing: Option<T> = self.entity.get(id).await?;
        if existing.is_none() {
            return Err(CrudError::NotFound(id.to_string()));
        }
        self.save(id, entity).await
    }

    /// Delete an entity by ID.
    ///
    /// Idempotent: deleting a non-existent entity returns `Ok(false)` (L3).
    pub async fn delete(&self, id: &str) -> Result<bool, CrudError<E>> {
        Ok(self.entity.delete(id).await?)
    }
}

/// Construct a `CrudService` from a predicate-only validator.
///
/// Migration path for callers that only check invariants (returning
/// `Validated<(), E>`) and don't need normalization. The entity is
/// cloned through on `Valid(())`.
pub fn with_predicate<'a, T, P, E>(
    entity: IndexedEntity<'a>,
    predicate: P,
) -> CrudService<'a, T, impl Fn(&T) -> Validated<T, E>, E>
where
    T: Serialize + DeserializeOwned + Clone + 'static,
    P: Fn(&T) -> Validated<(), E> + 'static,
    E: std::fmt::Debug + std::fmt::Display,
{
    let validate = move |t: &T| match predicate(t) {
        Validated::Valid(()) => Validated::Valid(t.clone()),
        Validated::Invalid(es) => Validated::Invalid(es),
    };
    CrudService::new(entity, validate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::validated::Validated;
    use std::sync::Arc;

    static TEST_CONFIG: crate::indexed_entity::EntityConfig = crate::indexed_entity::EntityConfig {
        key_prefix: "crud_test",
        index_key: "crud_tests:index",
    };

    fn make_kv() -> Arc<dyn agent_fw_algebra::KVStore> {
        Arc::new(agent_fw_interpreter::DashMapKVStore::new())
    }

    /// Predicate-style validation: name must not be empty, value must be > 0.
    fn validate_item(item: &serde_json::Value) -> Validated<(), String> {
        let mut errors = Vec::new();
        if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
            if name.is_empty() {
                errors.push("name must not be empty".into());
            }
        } else {
            errors.push("name is required".into());
        }
        if let Some(value) = item.get("value").and_then(|v| v.as_i64()) {
            if value <= 0 {
                errors.push("value must be positive".into());
            }
        }
        if errors.is_empty() {
            Validated::Valid(())
        } else {
            Validated::Invalid(errors)
        }
    }

    // =========================================================================
    // L1: Roundtrip
    // =========================================================================

    #[tokio::test]
    async fn l1_save_get_roundtrip() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        let entity = serde_json::json!({"name": "test", "value": 42});
        svc.save("e1", &entity).await.unwrap();

        let got: Option<serde_json::Value> = svc.get("e1").await.unwrap();
        assert_eq!(got.unwrap(), entity);
    }

    // =========================================================================
    // L2: Validated gate
    // =========================================================================

    #[tokio::test]
    async fn l2_invalid_input_returns_all_errors_no_side_effects() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        // Both name empty AND value non-positive
        let bad_entity = serde_json::json!({"name": "", "value": -1});
        let err = svc.save("e1", &bad_entity).await.unwrap_err();

        match err {
            CrudError::Validation(errors) => {
                assert_eq!(errors.len(), 2);
                assert!(errors.iter().any(|e| e.contains("name")));
                assert!(errors.iter().any(|e| e.contains("value")));
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }

        // Entity should not exist (no IO side effects)
        let got: Option<serde_json::Value> = svc.get("e1").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn l2_update_also_validates() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        // Create a valid entity
        let good = serde_json::json!({"name": "test", "value": 1});
        svc.save("e1", &good).await.unwrap();

        // Try to update with invalid data
        let bad = serde_json::json!({"name": "", "value": 1});
        assert!(svc.update("e1", &bad).await.is_err());

        // Original value should be unchanged
        let got: serde_json::Value = svc.get("e1").await.unwrap().unwrap();
        assert_eq!(got, good);
    }

    // =========================================================================
    // L3: Delete idempotence
    // =========================================================================

    #[tokio::test]
    async fn l3_delete_idempotent() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        let entity = serde_json::json!({"name": "test", "value": 1});
        svc.save("e1", &entity).await.unwrap();

        let first = svc.delete("e1").await.unwrap();
        assert!(first); // existed

        let second = svc.delete("e1").await.unwrap();
        assert!(!second); // already gone, but no error
    }

    // =========================================================================
    // L4: Create uniqueness
    // =========================================================================

    #[tokio::test]
    async fn l4_create_rejects_duplicate() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        let entity = serde_json::json!({"name": "test", "value": 1});
        svc.create("e1", &entity).await.unwrap();

        let entity2 = serde_json::json!({"name": "other", "value": 2});
        let err = svc.create("e1", &entity2).await.unwrap_err();
        assert!(matches!(err, CrudError::AlreadyExists(_)));

        // Original value should be unchanged
        let got: serde_json::Value = svc.get("e1").await.unwrap().unwrap();
        assert_eq!(got, entity);
    }

    // =========================================================================
    // L5: Update existence
    // =========================================================================

    #[tokio::test]
    async fn l5_update_rejects_missing() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        let entity = serde_json::json!({"name": "test", "value": 1});
        let err = svc.update("e1", &entity).await.unwrap_err();
        assert!(matches!(err, CrudError::NotFound(_)));
    }

    // =========================================================================
    // List
    // =========================================================================

    #[tokio::test]
    async fn list_returns_all() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        svc.save("e1", &serde_json::json!({"name": "a", "value": 1}))
            .await
            .unwrap();
        svc.save("e2", &serde_json::json!({"name": "b", "value": 2}))
            .await
            .unwrap();

        let ids = svc.list_ids().await.unwrap();
        assert_eq!(ids.len(), 2);

        let entities: Vec<serde_json::Value> = svc.list().await.unwrap();
        assert_eq!(entities.len(), 2);
    }

    // =========================================================================
    // Multi-error accumulation
    // =========================================================================

    #[tokio::test]
    async fn multi_error_accumulation() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);
        let svc = with_predicate(ie, validate_item);

        // Missing name AND non-positive value → 2 errors
        let bad = serde_json::json!({"value": 0});
        let err = svc.save("e1", &bad).await.unwrap_err();
        match err {
            CrudError::Validation(errors) => assert!(errors.len() >= 2),
            other => panic!("Expected Validation, got: {:?}", other),
        }
    }

    // =========================================================================
    // Normalizing validator
    // =========================================================================

    #[tokio::test]
    async fn normalizing_validator_persists_refined_entity() {
        let kv = make_kv();
        let ie = IndexedEntity::new(kv.as_ref(), "tenant-1", &TEST_CONFIG);

        // Validator that trims whitespace from the name
        let svc = CrudService::new(ie, |item: &serde_json::Value| {
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.trim().is_empty() {
                return Validated::Invalid(vec!["name must not be empty".to_string()]);
            }
            let mut normalized = item.clone();
            normalized["name"] = serde_json::Value::String(name.trim().to_string());
            Validated::Valid(normalized)
        });

        let entity = serde_json::json!({"name": "  hello  ", "value": 1});
        svc.save("e1", &entity).await.unwrap();

        let got: serde_json::Value = svc.get("e1").await.unwrap().unwrap();
        assert_eq!(got["name"], "hello"); // trimmed
    }
}
