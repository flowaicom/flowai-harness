//! Opaque domain context for plans.
//!
//! `PlanContext` is a typed wrapper over a JSON object map, allowing plans
//! to carry domain-specific metadata (entity set IDs, scope references,
//! partition keys, etc.) without parameterizing the `Plan<A>` type further.
//!
//! # Design Rationale
//!
//! Instead of making `Plan<A, E>` doubly generic over entity type, we store
//! entity references as structured JSON. This keeps the state machine generic
//! while allowing any domain to attach whatever context it needs.
//!
//! The framework's state machine logic never inspects the context — it is
//! purely pass-through data owned by the consuming application.
//!
//! # Typed Accessors
//!
//! In addition to raw `get`/`set`, `PlanContext` provides typed accessors:
//!
//! - [`extract`](PlanContext::extract) — deserialize JSON at key into `T`
//! - [`insert`](PlanContext::insert) — serialize `T` to JSON at key
//! - [`try_extract`](PlanContext::try_extract) — lenient extract (`None` for absent keys)
//!
//! These eliminate `.and_then(|v| v.as_u64())` chains and distinguish
//! "key absent" from "wrong type" via [`ContextError`].

use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::plan::Plan;

// ─── ContextKey<T> ──────────────────────────────────────────────────

/// A phantom-typed key for `PlanContext` entries.
///
/// Associates a static string name with a Rust type `T` that the value
/// must serialize to/deserialize from. The type parameter is phantom —
/// it exists only at compile time for type-guided access.
///
/// # Example
///
/// ```
/// use agent_fw_plan::context::ContextKey;
/// use agent_fw_plan::PlanContext;
///
/// static PRODUCT_COUNT: ContextKey<usize> = ContextKey::new("product_count");
///
/// let mut ctx = PlanContext::new();
/// ctx.set_typed(&PRODUCT_COUNT, &42).unwrap();
/// assert_eq!(ctx.get_typed(&PRODUCT_COUNT), Some(42));
/// ```
///
/// # Laws
///
/// - **Roundtrip**: `set_typed(k, &v); get_typed(k) == Some(v)` for `T: Serialize + DeserializeOwned + PartialEq`
/// - **Independence**: distinct keys don't interfere
/// - **Backward compat**: `set_typed(k, &v)` then `get(k.name())` returns the same JSON as `serde_json::to_value(&v)`
/// - **Graceful degradation**: `get_typed` on wrong type returns `None`, never panics
pub struct ContextKey<T> {
    name: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> ContextKey<T> {
    /// Create a new typed key.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _marker: PhantomData,
        }
    }

    /// The string key name used in the underlying JSON map.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

// ─── ContextError ─────────────────────────────────────────────────────

/// Error type for typed context access.
///
/// Distinguishes three failure modes that raw `Option<&Value>` conflates:
/// - Key not present (`MissingKey`)
/// - Key present but wrong JSON shape (`TypeMismatch`)
/// - Serialization failure on insert (`SerializationError`)
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("missing context key: {key}")]
    MissingKey { key: String },
    #[error("type mismatch for key '{key}': {detail}")]
    TypeMismatch { key: String, detail: String },
    #[error("serialization error for key '{key}': {detail}")]
    SerializationError { key: String, detail: String },
}

// ─── PlanContext ──────────────────────────────────────────────────────

/// Opaque domain-specific context attached to a plan.
///
/// Stores arbitrary key-value pairs as JSON. The framework never
/// inspects these values — they are pass-through for the domain layer.
///
/// # Example
///
/// ```
/// use agent_fw_plan::PlanContext;
///
/// let mut ctx = PlanContext::new();
/// ctx.set("entity_set_id", serde_json::json!("set-abc123"));
/// ctx.set("scope_id", serde_json::json!("scope-xyz"));
/// assert_eq!(ctx.get("entity_set_id").and_then(|v| v.as_str()), Some("set-abc123"));
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanContext(serde_json::Map<String, serde_json::Value>);

impl PlanContext {
    pub fn new() -> Self {
        Self(serde_json::Map::new())
    }

    /// Insert or update a key-value pair.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.0.insert(key.into(), value);
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.0.get(key)
    }

    /// Remove a key, returning its value if it existed.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.0.remove(key)
    }

    /// Check if the context is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over entries.
    pub fn iter(&self) -> serde_json::map::Iter<'_> {
        self.0.iter()
    }

    /// Get the underlying map.
    pub fn into_inner(self) -> serde_json::Map<String, serde_json::Value> {
        self.0
    }

    // ─── Typed accessors ─────────────────────────────────────────────

    /// Typed extract: deserialize JSON at key into `T`.
    ///
    /// # Laws
    ///
    /// - **R1 (Roundtrip)**: `insert(k, &v); extract::<T>(k) == Ok(v)` for `T: Serialize + DeserializeOwned`
    /// - **R2 (MissingKey)**: `extract::<T>(absent_key) == Err(MissingKey)`
    /// - **R3 (TypeMismatch)**: `set(k, json!("str")); extract::<i64>(k) == Err(TypeMismatch)`
    pub fn extract<T: DeserializeOwned>(&self, key: &str) -> Result<T, ContextError> {
        let value = self.0.get(key).ok_or_else(|| ContextError::MissingKey {
            key: key.to_string(),
        })?;
        serde_json::from_value(value.clone()).map_err(|e| ContextError::TypeMismatch {
            key: key.to_string(),
            detail: e.to_string(),
        })
    }

    /// Insert typed value: serialize `T` to JSON at key.
    ///
    /// Symmetric with `extract` — R1 roundtrip guarantee.
    pub fn insert<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), ContextError> {
        let json_value =
            serde_json::to_value(value).map_err(|e| ContextError::SerializationError {
                key: key.to_string(),
                detail: e.to_string(),
            })?;
        self.0.insert(key.to_string(), json_value);
        Ok(())
    }

    // ─── Phantom-typed accessors (via ContextKey<T>) ──────────────────

    /// Get a typed value using a phantom-typed key.
    ///
    /// Returns `None` if the key is absent OR the stored value doesn't
    /// deserialize to `T`. Never panics — gracefully degrades to `None`.
    pub fn get_typed<T: DeserializeOwned>(&self, key: &ContextKey<T>) -> Option<T> {
        self.0
            .get(key.name())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Set a typed value using a phantom-typed key.
    ///
    /// Returns `Err` if serialization fails. This should never happen
    /// for well-formed `Serialize` types, but the return type encodes
    /// that possibility rather than silently discarding it.
    ///
    /// # Law
    ///
    /// - **TC1 (Totality)**: All outcomes are visible in the return type.
    ///   No silent loss.
    pub fn set_typed<T: Serialize>(
        &mut self,
        key: &ContextKey<T>,
        value: &T,
    ) -> Result<(), ContextError> {
        let json_value =
            serde_json::to_value(value).map_err(|e| ContextError::SerializationError {
                key: key.name().to_string(),
                detail: e.to_string(),
            })?;
        self.0.insert(key.name().to_string(), json_value);
        Ok(())
    }

    /// Check if a typed key is present and deserializable to `T`.
    pub fn has_typed<T: DeserializeOwned>(&self, key: &ContextKey<T>) -> bool {
        self.get_typed(key).is_some()
    }

    /// Lenient extract: `None` for absent keys, `Err` only for type mismatch.
    ///
    /// Use when the key is optional but a wrong type is still a bug.
    pub fn try_extract<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, ContextError> {
        match self.0.get(key) {
            None => Ok(None),
            Some(value) => {
                let t = serde_json::from_value(value.clone()).map_err(|e| {
                    ContextError::TypeMismatch {
                        key: key.to_string(),
                        detail: e.to_string(),
                    }
                })?;
                Ok(Some(t))
            }
        }
    }
}

impl PartialEq for PlanContext {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for PlanContext {}

impl From<serde_json::Map<String, serde_json::Value>> for PlanContext {
    fn from(map: serde_json::Map<String, serde_json::Value>) -> Self {
        Self(map)
    }
}

// ─── PlanContextProjection ───────────────────────────────────────────

/// Typed projection from `Plan` context into a domain snapshot.
///
/// Implementors define how to extract a typed domain value from the
/// plan's opaque JSON context. The framework provides the trait;
/// the domain provides the implementation.
///
/// # Laws
///
/// - **E1 (Totality)**: `extract` never panics — all failures are `Err`
/// - **E2 (Purity)**: same plan → same result (deterministic)
pub trait PlanContextProjection<A>: Sized {
    fn extract(plan: &Plan<A>) -> Result<Self, ContextError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_context() {
        let ctx = PlanContext::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);
    }

    #[test]
    fn set_and_get() {
        let mut ctx = PlanContext::new();
        ctx.set("key", serde_json::json!("value"));
        assert_eq!(ctx.get("key"), Some(&serde_json::json!("value")));
        assert_eq!(ctx.len(), 1);
    }

    #[test]
    fn remove_returns_value() {
        let mut ctx = PlanContext::new();
        ctx.set("a", serde_json::json!(42));
        assert_eq!(ctx.remove("a"), Some(serde_json::json!(42)));
        assert!(ctx.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let mut ctx = PlanContext::new();
        ctx.set("entity_set_id", serde_json::json!("set-123"));
        ctx.set("scope_id", serde_json::json!("scope-456"));

        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: PlanContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, parsed);
    }

    #[test]
    fn transparent_serialization() {
        let mut ctx = PlanContext::new();
        ctx.set("x", serde_json::json!(1));

        let json = serde_json::to_string(&ctx).unwrap();
        // Should serialize as a plain JSON object, not wrapped
        assert_eq!(json, r#"{"x":1}"#);
    }

    // ─── Typed accessor tests ─────────────────────────────────────────

    #[test]
    fn extract_string_roundtrip() {
        let mut ctx = PlanContext::new();
        ctx.insert("name", &"hello".to_string()).unwrap();
        let val: String = ctx.extract("name").unwrap();
        assert_eq!(val, "hello");
    }

    #[test]
    fn extract_i64_roundtrip() {
        let mut ctx = PlanContext::new();
        ctx.insert("count", &42i64).unwrap();
        let val: i64 = ctx.extract("count").unwrap();
        assert_eq!(val, 42);
    }

    #[test]
    fn extract_vec_roundtrip() {
        let mut ctx = PlanContext::new();
        let original = vec!["a".to_string(), "b".to_string()];
        ctx.insert("tags", &original).unwrap();
        let val: Vec<String> = ctx.extract("tags").unwrap();
        assert_eq!(val, original);
    }

    #[test]
    fn extract_missing_key() {
        let ctx = PlanContext::new();
        let result = ctx.extract::<String>("absent");
        assert!(matches!(result, Err(ContextError::MissingKey { .. })));
    }

    #[test]
    fn extract_type_mismatch() {
        let mut ctx = PlanContext::new();
        ctx.set("name", serde_json::json!("hello"));
        let result = ctx.extract::<i64>("name");
        assert!(matches!(result, Err(ContextError::TypeMismatch { .. })));
    }

    #[test]
    fn try_extract_absent_returns_none() {
        let ctx = PlanContext::new();
        let result = ctx.try_extract::<i64>("absent").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn try_extract_present_returns_some() {
        let mut ctx = PlanContext::new();
        ctx.insert("count", &42i64).unwrap();
        let result = ctx.try_extract::<i64>("count").unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn try_extract_type_mismatch_is_err() {
        let mut ctx = PlanContext::new();
        ctx.set("name", serde_json::json!("hello"));
        let result = ctx.try_extract::<i64>("name");
        assert!(matches!(result, Err(ContextError::TypeMismatch { .. })));
    }

    // ─── ContextKey<T> typed accessor tests ────────────────────────────

    static TEST_COUNT: ContextKey<usize> = ContextKey::new("count");
    static TEST_NAME: ContextKey<String> = ContextKey::new("name");
    static TEST_TAGS: ContextKey<Vec<String>> = ContextKey::new("tags");

    #[test]
    fn typed_roundtrip_usize() {
        let mut ctx = PlanContext::new();
        ctx.set_typed(&TEST_COUNT, &42).unwrap();
        assert_eq!(ctx.get_typed(&TEST_COUNT), Some(42));
    }

    #[test]
    fn typed_roundtrip_string() {
        let mut ctx = PlanContext::new();
        ctx.set_typed(&TEST_NAME, &"hello".to_string()).unwrap();
        assert_eq!(ctx.get_typed(&TEST_NAME), Some("hello".to_string()));
    }

    #[test]
    fn typed_roundtrip_vec() {
        let mut ctx = PlanContext::new();
        let tags = vec!["a".to_string(), "b".to_string()];
        ctx.set_typed(&TEST_TAGS, &tags).unwrap();
        assert_eq!(ctx.get_typed(&TEST_TAGS), Some(tags));
    }

    #[test]
    fn typed_get_missing_returns_none() {
        let ctx = PlanContext::new();
        assert_eq!(ctx.get_typed(&TEST_COUNT), None);
    }

    #[test]
    fn typed_graceful_degradation_wrong_type() {
        let mut ctx = PlanContext::new();
        ctx.set("count", serde_json::json!("not a number"));
        // get_typed returns None, not panic
        assert_eq!(ctx.get_typed(&TEST_COUNT), None);
    }

    #[test]
    fn typed_has_typed() {
        let mut ctx = PlanContext::new();
        assert!(!ctx.has_typed(&TEST_COUNT));
        ctx.set_typed(&TEST_COUNT, &42).unwrap();
        assert!(ctx.has_typed(&TEST_COUNT));
    }

    #[test]
    fn typed_backward_compat_with_raw_get() {
        let mut ctx = PlanContext::new();
        ctx.set_typed(&TEST_COUNT, &42).unwrap();
        // Raw get sees the same JSON
        let raw = ctx.get("count").unwrap();
        assert_eq!(raw, &serde_json::json!(42));
    }

    #[test]
    fn typed_independence() {
        let mut ctx = PlanContext::new();
        ctx.set_typed(&TEST_COUNT, &10).unwrap();
        ctx.set_typed(&TEST_NAME, &"hello".to_string()).unwrap();
        // Setting name doesn't affect count
        assert_eq!(ctx.get_typed(&TEST_COUNT), Some(10));
        assert_eq!(ctx.get_typed(&TEST_NAME), Some("hello".to_string()));
    }

    #[test]
    fn typed_overwrite() {
        let mut ctx = PlanContext::new();
        ctx.set_typed(&TEST_COUNT, &1).unwrap();
        ctx.set_typed(&TEST_COUNT, &2).unwrap();
        assert_eq!(ctx.get_typed(&TEST_COUNT), Some(2));
    }

    #[test]
    fn insert_overwrite() {
        let mut ctx = PlanContext::new();
        ctx.insert("key", &1i64).unwrap();
        ctx.insert("key", &2i64).unwrap();
        let val: i64 = ctx.extract("key").unwrap();
        assert_eq!(val, 2);
    }
}
