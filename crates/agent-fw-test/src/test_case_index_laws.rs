//! TestCaseIndex algebraic law test harnesses.
//!
//! Verifies Laws L1–L6 documented in `agent_fw_algebra::test_case_index`.
//!
//! Each invocation of [`test_all`] uses a unique tenant name so the harness
//! is safe to call multiple times on the same persistent index.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_index_satisfies_laws() {
//!     let index = MyTestCaseIndex::new();
//!     agent_fw_test::test_case_index_laws::test_all(&index).await;
//! }
//! ```

use agent_fw_algebra::test_case_index::{TestCaseIndex, TestCaseMeta};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter for generating unique tenant names per invocation.
static TENANT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique tenant name for test isolation.
fn unique_tenant(prefix: &str) -> String {
    let n = TENANT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{n}")
}

fn make_meta(id: &str, name: &str, tags: &[&str]) -> TestCaseMeta {
    TestCaseMeta {
        id: id.to_string(),
        name: name.to_string(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        status: "active".to_string(),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

/// Run all deterministic TestCaseIndex laws against the given index.
///
/// Each invocation uses a unique tenant to avoid cross-contamination
/// across repeated calls on the same persistent index.
pub async fn test_all(index: &dyn TestCaseIndex) {
    let tenant = unique_tenant("tci_law");
    law_put_get(index, &tenant).await;
    law_put_list(index, &tenant).await;
    law_remove_get(index, &tenant).await;
    law_remove_list(index, &tenant).await;
    law_idempotent_remove(index, &tenant).await;
    law_overwrite(index, &tenant).await;
}

/// L1 (Put-Get): `put(meta); get(meta.id)` returns `Some(meta)`.
pub async fn law_put_get(index: &dyn TestCaseIndex, tenant: &str) {
    let meta = make_meta("tci-l1-001", "L1 test case", &["regression"]);
    index.put(tenant, meta.clone()).await.unwrap();

    let retrieved = index.get(tenant, "tci-l1-001").await.unwrap();
    assert_eq!(
        retrieved,
        Some(meta),
        "L1 (Put-Get): get after put must return stored meta"
    );
}

/// L2 (Put-List): After put, `list()` contains the id.
pub async fn law_put_list(index: &dyn TestCaseIndex, tenant: &str) {
    let meta = make_meta("tci-l2-001", "L2 test case", &["smoke"]);
    index.put(tenant, meta).await.unwrap();

    let all = index.list(tenant).await.unwrap();
    assert!(
        all.iter().any(|m| m.id == "tci-l2-001"),
        "L2 (Put-List): list must contain the id after put"
    );
}

/// L3 (Remove-Get): `put(meta); remove(id); get(id)` returns `None`.
pub async fn law_remove_get(index: &dyn TestCaseIndex, tenant: &str) {
    let meta = make_meta("tci-l3-001", "L3 test case", &[]);
    index.put(tenant, meta).await.unwrap();
    index.remove(tenant, "tci-l3-001").await.unwrap();

    let retrieved = index.get(tenant, "tci-l3-001").await.unwrap();
    assert_eq!(
        retrieved, None,
        "L3 (Remove-Get): get after remove must return None"
    );
}

/// L4 (Remove-List): `put(meta); remove(id); list()` does not contain `id`.
pub async fn law_remove_list(index: &dyn TestCaseIndex, tenant: &str) {
    let meta = make_meta("tci-l4-001", "L4 test case", &[]);
    index.put(tenant, meta).await.unwrap();
    index.remove(tenant, "tci-l4-001").await.unwrap();

    let all = index.list(tenant).await.unwrap();
    assert!(
        !all.iter().any(|m| m.id == "tci-l4-001"),
        "L4 (Remove-List): list must not contain id after remove"
    );
}

/// L5 (Idempotent Remove): Removing a non-existent id succeeds without error.
pub async fn law_idempotent_remove(index: &dyn TestCaseIndex, tenant: &str) {
    // Remove something that was never inserted
    let result = index.remove(tenant, "tci-l5-nonexistent").await;
    assert!(
        result.is_ok(),
        "L5 (Idempotent Remove): removing absent id must not error"
    );
    // The return value should be false (nothing was removed)
    assert_eq!(
        result.unwrap(),
        false,
        "L5 (Idempotent Remove): removing absent id must return false"
    );
}

/// L6 (Overwrite): `put(id, m1); put(id, m2); get(id)` returns `Some(m2)`.
pub async fn law_overwrite(index: &dyn TestCaseIndex, tenant: &str) {
    let m1 = make_meta("tci-l6-001", "L6 original", &["v1"]);
    let m2 = make_meta("tci-l6-001", "L6 updated", &["v2"]);

    index.put(tenant, m1).await.unwrap();
    index.put(tenant, m2.clone()).await.unwrap();

    let retrieved = index.get(tenant, "tci-l6-001").await.unwrap();
    assert_eq!(
        retrieved,
        Some(m2),
        "L6 (Overwrite): second put must overwrite first"
    );
}

/// Additional law: `count()` is consistent with `list().len()`.
///
/// Uses its own unique tenant to avoid counting entries from other laws.
pub async fn law_count_consistent(index: &dyn TestCaseIndex) {
    let tenant = unique_tenant("tci_count");
    let meta = make_meta("tci-count-001", "Count test", &[]);
    index.put(&tenant, meta).await.unwrap();

    let list = index.list(&tenant).await.unwrap();
    let count = index.count(&tenant).await.unwrap();
    assert_eq!(count, list.len(), "count() must equal list().len()");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal in-memory implementation for self-testing the harness.
    struct InMemoryTestCaseIndex {
        data: tokio::sync::Mutex<std::collections::HashMap<(String, String), TestCaseMeta>>,
    }

    impl InMemoryTestCaseIndex {
        fn new() -> Self {
            Self {
                data: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl TestCaseIndex for InMemoryTestCaseIndex {
        async fn put(
            &self,
            tenant: &str,
            meta: TestCaseMeta,
        ) -> Result<(), agent_fw_algebra::TestCaseIndexError> {
            let key = (tenant.to_string(), meta.id.clone());
            self.data.lock().await.insert(key, meta);
            Ok(())
        }

        async fn get(
            &self,
            tenant: &str,
            id: &str,
        ) -> Result<Option<TestCaseMeta>, agent_fw_algebra::TestCaseIndexError> {
            let key = (tenant.to_string(), id.to_string());
            Ok(self.data.lock().await.get(&key).cloned())
        }

        async fn list(
            &self,
            tenant: &str,
        ) -> Result<Vec<TestCaseMeta>, agent_fw_algebra::TestCaseIndexError> {
            let data = self.data.lock().await;
            Ok(data
                .iter()
                .filter(|((t, _), _)| t == tenant)
                .map(|(_, v)| v.clone())
                .collect())
        }

        async fn list_by_tags(
            &self,
            tenant: &str,
            tags: &[String],
        ) -> Result<Vec<TestCaseMeta>, agent_fw_algebra::TestCaseIndexError> {
            let data = self.data.lock().await;
            Ok(data
                .iter()
                .filter(|((t, _), v)| t == tenant && v.tags.iter().any(|tag| tags.contains(tag)))
                .map(|(_, v)| v.clone())
                .collect())
        }

        async fn remove(
            &self,
            tenant: &str,
            id: &str,
        ) -> Result<bool, agent_fw_algebra::TestCaseIndexError> {
            let key = (tenant.to_string(), id.to_string());
            Ok(self.data.lock().await.remove(&key).is_some())
        }

        async fn count(&self, tenant: &str) -> Result<usize, agent_fw_algebra::TestCaseIndexError> {
            let data = self.data.lock().await;
            Ok(data.keys().filter(|(t, _)| t == tenant).count())
        }
    }

    #[tokio::test]
    async fn in_memory_index_satisfies_all_laws() {
        let index = InMemoryTestCaseIndex::new();
        test_all(&index).await;
        law_count_consistent(&index).await;
    }

    /// Verify reentrant safety: calling test_all twice on the same index works.
    #[tokio::test]
    async fn reentrant_test_all() {
        let index = InMemoryTestCaseIndex::new();
        test_all(&index).await;
        test_all(&index).await; // second call uses a different tenant
    }
}
