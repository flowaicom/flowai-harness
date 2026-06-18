//! RetryCatalog — algebra-level retry wrapper for DataCatalog + CatalogWriter.
//!
//! # Design
//!
//! Retry is a **combinator**, not an implementation detail. Compose it
//! at the algebra level:
//!
//! ```text
//! let catalog = RetryCatalog::new(PostgresCatalog::connect(url).await?, policy);
//! ```

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, CatalogWriter, DataCatalog, JoinPath,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// A DataCatalog + CatalogWriter wrapper that retries on transient failures.
pub struct RetryCatalog<C: DataCatalog> {
    inner: C,
    policy: RetryPolicy,
}

impl<C: DataCatalog> RetryCatalog<C> {
    /// Wrap a catalog with retry behavior.
    pub fn new(inner: C, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Create with a reasonable default: 3 retries, 100ms initial delay, exponential backoff.
    pub fn with_defaults(inner: C) -> Self {
        Self::new(
            inner,
            RetryPolicy::exponential_backoff(3, Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(2)),
        )
    }

    /// Access the inner catalog.
    pub fn inner(&self) -> &C {
        &self.inner
    }

    /// Access the retry policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

fn is_retryable(e: &CatalogError) -> bool {
    use super::retry_defaults::TransientError;
    e.is_transient()
}

#[async_trait]
impl<C: DataCatalog> DataCatalog for RetryCatalog<C> {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_by_id(id).await },
            is_retryable,
        )
        .await
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_by_ids(ids).await },
            is_retryable,
        )
        .await
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_by_qualified_name(kind, qualified_name).await },
            is_retryable,
        )
        .await
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_by_name(kind, name).await },
            is_retryable,
        )
        .await
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.list_by_type(kind, limit).await },
            is_retryable,
        )
        .await
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_related(id, relation_type).await },
            is_retryable,
        )
        .await
    }

    async fn find_join_path(
        &self,
        from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.find_join_path(from_table, to_table).await },
            is_retryable,
        )
        .await
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.list_tables().await },
            is_retryable,
        )
        .await
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_columns(table_name).await },
            is_retryable,
        )
        .await
    }

    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_enum_values(column_id).await },
            is_retryable,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.health_check().await },
            is_retryable,
        )
        .await
    }
}

#[async_trait]
impl<C: DataCatalog + CatalogWriter> CatalogWriter for RetryCatalog<C> {
    /// Save items with retry. Uses `Arc` clone-on-retry for owned `Vec<CatalogEntry>`.
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let items = Arc::new(items);
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || {
                let items = items.clone();
                async move { self.inner.save_items((*items).clone()).await }
            },
            is_retryable,
        )
        .await
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.delete_items(ids).await },
            is_retryable,
        )
        .await
    }

    /// Save in transaction with retry. Uses `Arc` clone-on-retry for owned `Vec<CatalogEntry>`.
    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        let items = Arc::new(items);
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || {
                let items = items.clone();
                async move { self.inner.save_in_transaction((*items).clone()).await }
            },
            is_retryable,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// A DataCatalog + CatalogWriter that fails N times then delegates.
    struct FailNTimes<C: DataCatalog> {
        inner: C,
        remaining_failures: AtomicI32,
    }

    impl<C: DataCatalog> FailNTimes<C> {
        fn new(inner: C, failures: i32) -> Self {
            Self {
                inner,
                remaining_failures: AtomicI32::new(failures),
            }
        }

        fn maybe_fail(&self) -> Result<(), CatalogError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(CatalogError::Unavailable("service unavailable".into()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl<C: DataCatalog> DataCatalog for FailNTimes<C> {
        async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_by_id(id).await
        }

        async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_by_ids(ids).await
        }

        async fn get_by_qualified_name(
            &self,
            kind: CatalogKind,
            qualified_name: &str,
        ) -> Result<Option<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_by_qualified_name(kind, qualified_name).await
        }

        async fn get_by_name(
            &self,
            kind: CatalogKind,
            name: &str,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_by_name(kind, name).await
        }

        async fn list_by_type(
            &self,
            kind: CatalogKind,
            limit: usize,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.list_by_type(kind, limit).await
        }

        async fn get_related(
            &self,
            id: &str,
            relation_type: Option<&str>,
        ) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_related(id, relation_type).await
        }

        async fn find_join_path(
            &self,
            from_table: &str,
            to_table: &str,
        ) -> Result<Option<JoinPath>, CatalogError> {
            self.maybe_fail()?;
            self.inner.find_join_path(from_table, to_table).await
        }

        async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.list_tables().await
        }

        async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_columns(table_name).await
        }

        async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
            self.maybe_fail()?;
            self.inner.get_enum_values(column_id).await
        }
    }

    #[async_trait]
    impl<C: DataCatalog + CatalogWriter> CatalogWriter for FailNTimes<C> {
        async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
            self.maybe_fail()?;
            self.inner.save_items(items).await
        }

        async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
            self.maybe_fail()?;
            self.inner.delete_items(ids).await
        }

        async fn save_in_transaction(
            &self,
            items: Vec<CatalogEntry>,
        ) -> Result<Vec<String>, CatalogError> {
            self.maybe_fail()?;
            self.inner.save_in_transaction(items).await
        }
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_failure() {
        let inner = FailNTimes::new(crate::MockCatalog::new(), 2);
        let catalog = RetryCatalog::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        // list_tables should succeed after 2 failures + 1 success
        let tables = catalog.list_tables().await.unwrap();
        assert!(tables.is_empty()); // MockCatalog starts empty
    }

    #[tokio::test]
    async fn retry_exhausts_on_persistent_failure() {
        let inner = FailNTimes::new(crate::MockCatalog::new(), 100);
        let catalog = RetryCatalog::new(inner, RetryPolicy::fixed(2, Duration::from_millis(1)));

        let result = catalog.list_tables().await;
        assert!(result.is_err(), "Should fail after exhausting retries");
    }

    /// Exercises the Arc clone-on-retry path for save_items (owned Vec<CatalogEntry>).
    #[tokio::test]
    async fn retry_save_items_recovers_from_transient_failure() {
        let inner = FailNTimes::new(crate::MockCatalog::new(), 2);
        let catalog = RetryCatalog::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        let entry = CatalogEntry {
            id: "test-1".into(),
            name: "Test Entry".into(),
            kind: CatalogKind::Table,
            qualified_name: Some("public.test".into()),
            content: "A test table".into(),
            tags: vec![],
            links: vec![],
            metadata: Default::default(),
        };
        let ids = catalog.save_items(vec![entry]).await.unwrap();
        assert_eq!(ids, vec!["test-1"]);
    }

    #[test]
    fn retryable_classification() {
        assert!(is_retryable(&CatalogError::Unavailable(
            "service unavailable".into()
        )));
        assert!(!is_retryable(&CatalogError::NotFound("missing".into())));
        assert!(!is_retryable(&CatalogError::InvalidQuery("bad".into())));
    }
}
