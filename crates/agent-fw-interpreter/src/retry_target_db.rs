//! RetryTargetDatabase — algebra-level retry wrapper for any TargetDatabase.
//!
//! # Design
//!
//! Retry is a **combinator**, not an implementation detail. Rather than baking
//! retry logic into each TargetDatabase interpreter (Sqlx, Mock), we compose
//! it at the algebra level:
//!
//! ```text
//! let db = RetryTargetDatabase::new(SqlxTargetDatabase::connect(url).await?, policy);
//! ```
//!
//! The same wrapper works with any `TargetDatabase` implementation,
//! including mocks (useful for testing retry behavior).

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_algebra::target_db::{DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase};
use agent_fw_core::DatabaseType;
use async_trait::async_trait;
use std::time::Duration;

/// A TargetDatabase wrapper that retries failed operations according to a policy.
///
/// Retries on `DbError::Connection` and `DbError::Timeout` errors (transient).
/// `DbError::InvalidQuery` and `DbError::Deserialization` are not retried (deterministic).
pub struct RetryTargetDatabase<T: TargetDatabase> {
    inner: T,
    policy: RetryPolicy,
}

impl<T: TargetDatabase> RetryTargetDatabase<T> {
    /// Wrap a TargetDatabase with retry behavior.
    pub fn new(inner: T, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Create with a reasonable default: 3 retries, 100ms initial delay, exponential backoff.
    pub fn with_defaults(inner: T) -> Self {
        Self::new(
            inner,
            RetryPolicy::exponential_backoff(3, Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(2)),
        )
    }

    /// Access the inner database.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Access the retry policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

fn is_retryable(e: &DbError) -> bool {
    use super::retry_defaults::TransientError;
    e.is_transient()
}

#[async_trait]
impl<T: TargetDatabase> TargetDatabase for RetryTargetDatabase<T> {
    fn database_type(&self) -> DatabaseType {
        self.inner.database_type()
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.query(query, params).await },
            is_retryable,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), DbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.health_check().await },
            is_retryable,
        )
        .await
    }

    fn timeout(&self) -> Duration {
        self.inner.timeout()
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.list_tables().await },
            is_retryable,
        )
        .await
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.get_table_columns(table_name).await },
            is_retryable,
        )
        .await
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.sample_table(table_name, limit).await },
            is_retryable,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// A TargetDatabase that fails N times then delegates to an inner impl.
    struct FailNTimes<T: TargetDatabase> {
        inner: T,
        remaining_failures: AtomicI32,
    }

    impl<T: TargetDatabase> FailNTimes<T> {
        fn new(inner: T, failures: i32) -> Self {
            Self {
                inner,
                remaining_failures: AtomicI32::new(failures),
            }
        }
    }

    impl<T: TargetDatabase> FailNTimes<T> {
        fn maybe_fail(&self) -> Result<(), DbError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(DbError::Connection("connection reset by peer".into()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl<T: TargetDatabase> TargetDatabase for FailNTimes<T> {
        async fn query(
            &self,
            query: &ReadOnlyQuery,
            params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            self.maybe_fail()?;
            self.inner.query(query, params).await
        }

        async fn health_check(&self) -> Result<(), DbError> {
            self.maybe_fail()?;
            self.inner.health_check().await
        }

        async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
            self.maybe_fail()?;
            self.inner.list_tables().await
        }

        async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
            self.maybe_fail()?;
            self.inner.get_table_columns(table_name).await
        }

        async fn sample_table(
            &self,
            table_name: &str,
            limit: usize,
        ) -> Result<Vec<serde_json::Value>, DbError> {
            self.maybe_fail()?;
            self.inner.sample_table(table_name, limit).await
        }
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_failure() {
        let inner = FailNTimes::new(crate::MockTargetDatabase::new(), 2);
        let db = RetryTargetDatabase::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        // health_check should succeed after 2 failures + 1 success
        db.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn retry_exhausts_on_persistent_failure() {
        let inner = FailNTimes::new(crate::MockTargetDatabase::new(), 100);
        let db = RetryTargetDatabase::new(inner, RetryPolicy::fixed(2, Duration::from_millis(1)));

        let result = db.health_check().await;
        assert!(result.is_err(), "Should fail after exhausting retries");
    }

    #[test]
    fn retryable_classification() {
        assert!(is_retryable(&DbError::Connection(
            "connection reset".into()
        )));
        assert!(is_retryable(&DbError::Timeout(Duration::from_secs(30))));
        assert!(!is_retryable(&DbError::InvalidQuery("bad SQL".into())));
        assert!(!is_retryable(&DbError::Deserialization("bad data".into())));
    }
}
