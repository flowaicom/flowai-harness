//! TestCaseIndex algebra for standalone test case indexing.
//!
//! Provides fast listing and lookup of test case metadata without
//! scanning all KV keys. Enables eval usage outside of Studio.
//!
//! # Laws
//!
//! - **L1 (Put-Get)**: `put(id, meta); get(id)` returns `Some(meta)`
//! - **L2 (Put-List)**: `put(id, meta); list()` contains `id`
//! - **L3 (Remove-Get)**: `put(id, meta); remove(id); get(id)` returns `None`
//! - **L4 (Remove-List)**: `put(id, meta); remove(id); list()` does not contain `id`
//! - **L5 (Idempotent Remove)**: `remove(absent_id)` succeeds without error
//! - **L6 (Overwrite)**: `put(id, m1); put(id, m2); get(id)` returns `Some(m2)`

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Lightweight metadata for a test case in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseMeta {
    /// Test case ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Tags for filtering.
    pub tags: Vec<String>,
    /// Status string (draft / active / archived).
    pub status: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 update timestamp.
    pub updated_at: String,
}

/// Error from TestCaseIndex operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TestCaseIndexError {
    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),
    /// Storage backend error.
    #[error("Storage error: {0}")]
    Storage(String),
}

/// Trait for indexing test case metadata.
///
/// Implementations may be backed by KVStore, SQLite, or any other storage.
/// The trait is object-safe for use as `Arc<dyn TestCaseIndex>`.
#[async_trait]
pub trait TestCaseIndex: Send + Sync {
    /// Insert or update a test case in the index.
    async fn put(&self, tenant: &str, meta: TestCaseMeta) -> Result<(), TestCaseIndexError>;

    /// Get a single test case's metadata by ID.
    async fn get(&self, tenant: &str, id: &str)
        -> Result<Option<TestCaseMeta>, TestCaseIndexError>;

    /// List all test case metadata for a tenant.
    async fn list(&self, tenant: &str) -> Result<Vec<TestCaseMeta>, TestCaseIndexError>;

    /// List test cases matching any of the given tags.
    async fn list_by_tags(
        &self,
        tenant: &str,
        tags: &[String],
    ) -> Result<Vec<TestCaseMeta>, TestCaseIndexError>;

    /// Remove a test case from the index.
    ///
    /// Returns `true` if the entry existed, `false` if already absent.
    async fn remove(&self, tenant: &str, id: &str) -> Result<bool, TestCaseIndexError>;

    /// Count total test cases for a tenant.
    async fn count(&self, tenant: &str) -> Result<usize, TestCaseIndexError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_meta_serde_roundtrip() {
        let meta = TestCaseMeta {
            id: "tc-1".into(),
            name: "Test pricing".into(),
            tags: vec!["pricing".into(), "regression".into()],
            status: "active".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: TestCaseMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, parsed);
    }
}
