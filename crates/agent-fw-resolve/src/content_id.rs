//! Phantom-typed content-addressed identifier.
//!
//! `ContentId<T>` is a deterministic hash of a serializable spec + tenant.
//! The phantom type parameter prevents mixing IDs of different entity types
//! at the type level.
//!
//! # Laws
//!
//! - **Determinism**: `compute(spec, tenant)` is pure — same inputs, same output.
//! - **Tenant isolation**: `compute(spec, t1) != compute(spec, t2)` when `t1 != t2`.
//! - **Collision resistance**: distinct `(spec, tenant)` pairs produce distinct IDs (SHA-256).

use agent_fw_core::TenantId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::marker::PhantomData;

/// Phantom-typed content-addressed identifier.
///
/// Uses SHA-256 (truncated to 24 hex chars) of canonical JSON for determinism.
/// The phantom type `T` makes `ContentId<ProductSet>` incompatible with
/// `ContentId<ScopeSet>` at the type level.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentId<T> {
    hash: String,
    #[serde(skip)]
    _entity: PhantomData<T>,
}

impl<T> ContentId<T> {
    /// Compute from any serializable spec + tenant.
    ///
    /// Uses canonical JSON → SHA-256 → hex (24 chars) for determinism.
    pub fn compute<S: Serialize>(spec: &S, tenant: &TenantId) -> Self {
        let payload = serde_json::json!({
            "owner": tenant.as_str(),
            "content": spec
        });
        let digest = Sha256::digest(payload.to_string().as_bytes());
        let hash = hex::encode(&digest[..12]);
        Self {
            hash,
            _entity: PhantomData,
        }
    }

    /// Get the hash as a string slice.
    pub fn as_str(&self) -> &str {
        &self.hash
    }

    /// Create from a raw hash string (for deserialization or test setup).
    pub fn new_unchecked(hash: String) -> Self {
        Self {
            hash,
            _entity: PhantomData,
        }
    }

    /// Create from a pre-computed hash string.
    ///
    /// Use when the application owns the hashing scheme (e.g., FilterHash-based IDs).
    pub fn from_raw(hash: impl Into<String>) -> Self {
        Self::new_unchecked(hash.into())
    }
}

impl<T> fmt::Display for ContentId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hash)
    }
}

impl<T> PartialEq for ContentId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl<T> Eq for ContentId<T> {}

impl<T> std::hash::Hash for ContentId<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize)]
    struct TestEntity {
        name: String,
        value: i32,
    }

    #[test]
    fn determinism() {
        let tenant = TenantId::new_unchecked("t1");
        let spec = TestEntity {
            name: "foo".into(),
            value: 42,
        };
        let id1 = ContentId::<TestEntity>::compute(&spec, &tenant);
        let id2 = ContentId::<TestEntity>::compute(&spec, &tenant);
        assert_eq!(id1, id2);
    }

    #[test]
    fn tenant_isolation() {
        let t1 = TenantId::new_unchecked("tenant-a");
        let t2 = TenantId::new_unchecked("tenant-b");
        let spec = TestEntity {
            name: "same".into(),
            value: 1,
        };
        let id1 = ContentId::<TestEntity>::compute(&spec, &t1);
        let id2 = ContentId::<TestEntity>::compute(&spec, &t2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn distinct_specs_distinct_ids() {
        let tenant = TenantId::new_unchecked("t");
        let s1 = TestEntity {
            name: "a".into(),
            value: 1,
        };
        let s2 = TestEntity {
            name: "b".into(),
            value: 2,
        };
        let id1 = ContentId::<TestEntity>::compute(&s1, &tenant);
        let id2 = ContentId::<TestEntity>::compute(&s2, &tenant);
        assert_ne!(id1, id2);
    }

    #[test]
    fn display_shows_hash() {
        let tenant = TenantId::new_unchecked("t");
        let spec = "test";
        let id = ContentId::<String>::compute(&spec, &tenant);
        let displayed = id.to_string();
        assert_eq!(displayed, id.as_str());
        assert_eq!(displayed.len(), 24); // SHA-256 truncated to 12 bytes = 24 hex
    }

    #[test]
    fn new_unchecked_preserves_hash() {
        let id = ContentId::<String>::new_unchecked("abc123".into());
        assert_eq!(id.as_str(), "abc123");
    }

    #[test]
    fn serde_roundtrip() {
        let tenant = TenantId::new_unchecked("t");
        let id = ContentId::<String>::compute(&"spec", &tenant);
        let json = serde_json::to_string(&id).unwrap();
        let parsed: ContentId<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    use hegel::generators;

    #[hegel::test]
    fn compute_is_pure(tc: hegel::TestCase) {
        let tenant: String = tc.draw(generators::from_regex("[a-zA-Z0-9]{1,20}").fullmatch(true));
        let value: String = tc.draw(generators::from_regex("[a-zA-Z0-9]{1,50}").fullmatch(true));
        let t = TenantId::new_unchecked(&tenant);
        let id1 = ContentId::<String>::compute(&value, &t);
        let id2 = ContentId::<String>::compute(&value, &t);
        assert_eq!(id1, id2);
    }

    #[hegel::test]
    fn different_tenants_different_ids(tc: hegel::TestCase) {
        let value: String = tc.draw(generators::from_regex("[a-zA-Z0-9]{1,20}").fullmatch(true));
        let t1: String = tc.draw(generators::from_regex("[a-zA-Z]{1,10}").fullmatch(true));
        let t2: String = tc.draw(generators::from_regex("[a-zA-Z]{1,10}").fullmatch(true));
        if t1 != t2 {
            let id1 = ContentId::<String>::compute(&value, &TenantId::new_unchecked(&t1));
            let id2 = ContentId::<String>::compute(&value, &TenantId::new_unchecked(&t2));
            assert_ne!(id1, id2);
        }
    }
}
