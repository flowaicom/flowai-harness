//! RetryWritableDatabase — algebra-level retry wrapper for any WritableDatabase.
//!
//! # Design
//!
//! Retry is a **combinator**, not an implementation detail. Compose it
//! at the algebra level:
//!
//! ```text
//! let db = RetryWritableDatabase::new(SqlxWritableDatabase::connect(url).await?, policy);
//! ```
//!
//! The same wrapper works with any `WritableDatabase` implementation,
//! including mocks (useful for testing retry behavior).

use agent_fw_algebra::retry::RetryPolicy;
use agent_fw_algebra::target_db::QueryParam;
use agent_fw_algebra::writable_db::{
    DdlStatement, DmlStatement, InsertBatch, TableName, WritableDatabase, WriteDbError,
};
use agent_fw_core::DatabaseType;
use async_trait::async_trait;
use std::time::Duration;

/// A WritableDatabase wrapper that retries failed operations according to a policy.
///
/// Retries on transient errors (connection, timeout, deadlock).
/// `InvalidSql` is not retried (deterministic).
pub struct RetryWritableDatabase<W: WritableDatabase> {
    inner: W,
    policy: RetryPolicy,
}

impl<W: WritableDatabase> RetryWritableDatabase<W> {
    /// Wrap a WritableDatabase with retry behavior.
    pub fn new(inner: W, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Create with a reasonable default: 3 retries, 100ms initial delay, exponential backoff.
    pub fn with_defaults(inner: W) -> Self {
        Self::new(
            inner,
            RetryPolicy::exponential_backoff(3, Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(2)),
        )
    }

    /// Access the inner database.
    pub fn inner(&self) -> &W {
        &self.inner
    }

    /// Access the retry policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

fn is_retryable(e: &WriteDbError) -> bool {
    use super::retry_defaults::TransientError;
    e.is_transient()
}

#[async_trait]
impl<W: WritableDatabase> WritableDatabase for RetryWritableDatabase<W> {
    fn database_type(&self) -> DatabaseType {
        self.inner.database_type()
    }

    async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.execute_ddl(stmt).await },
            is_retryable,
        )
        .await
    }

    async fn execute_dml(
        &self,
        stmt: &DmlStatement,
        params: &[QueryParam],
    ) -> Result<u64, WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.execute_dml(stmt, params).await },
            is_retryable,
        )
        .await
    }

    async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.insert_batch(batch).await },
            is_retryable,
        )
        .await
    }

    async fn insert_batch_returning(&self, batch: &InsertBatch) -> Result<Vec<i64>, WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.insert_batch_returning(batch).await },
            is_retryable,
        )
        .await
    }

    async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.drop_table_if_exists(table).await },
            is_retryable,
        )
        .await
    }

    async fn execute_in_transaction(
        &self,
        statements: &[DmlStatement],
    ) -> Result<(), WriteDbError> {
        agent_fw_algebra::retry::retry_when(
            &self.policy,
            || async { self.inner.execute_in_transaction(statements).await },
            is_retryable,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), WriteDbError> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// A WritableDatabase that fails N times then delegates.
    struct FailNTimes<W: WritableDatabase> {
        inner: W,
        remaining_failures: AtomicI32,
    }

    impl<W: WritableDatabase> FailNTimes<W> {
        fn new(inner: W, failures: i32) -> Self {
            Self {
                inner,
                remaining_failures: AtomicI32::new(failures),
            }
        }

        fn maybe_fail(&self) -> Result<(), WriteDbError> {
            if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(WriteDbError::Connection("connection reset by peer".into()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl<W: WritableDatabase> WritableDatabase for FailNTimes<W> {
        async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError> {
            self.maybe_fail()?;
            self.inner.execute_ddl(stmt).await
        }

        async fn execute_dml(
            &self,
            stmt: &DmlStatement,
            params: &[QueryParam],
        ) -> Result<u64, WriteDbError> {
            self.maybe_fail()?;
            self.inner.execute_dml(stmt, params).await
        }

        async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
            self.maybe_fail()?;
            self.inner.insert_batch(batch).await
        }

        async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError> {
            self.maybe_fail()?;
            self.inner.drop_table_if_exists(table).await
        }

        async fn execute_in_transaction(
            &self,
            statements: &[DmlStatement],
        ) -> Result<(), WriteDbError> {
            self.maybe_fail()?;
            self.inner.execute_in_transaction(statements).await
        }

        async fn health_check(&self) -> Result<(), WriteDbError> {
            self.maybe_fail()?;
            self.inner.health_check().await
        }
    }

    #[tokio::test]
    async fn retry_recovers_from_transient_failure() {
        let inner = FailNTimes::new(crate::MockWritableDatabase::new(), 2);
        let db = RetryWritableDatabase::new(inner, RetryPolicy::fixed(3, Duration::from_millis(1)));

        // health_check should succeed after 2 failures + 1 success
        db.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn retry_exhausts_on_persistent_failure() {
        let inner = FailNTimes::new(crate::MockWritableDatabase::new(), 100);
        let db = RetryWritableDatabase::new(inner, RetryPolicy::fixed(2, Duration::from_millis(1)));

        let result = db.health_check().await;
        assert!(result.is_err(), "Should fail after exhausting retries");
    }

    #[test]
    fn retryable_classification() {
        assert!(is_retryable(&WriteDbError::Connection(
            "connection reset".into()
        )));
        assert!(is_retryable(&WriteDbError::Timeout(Duration::from_secs(
            30
        ))));
        assert!(is_retryable(&WriteDbError::Transaction("deadlock".into())));
        assert!(!is_retryable(&WriteDbError::InvalidSql("bad SQL".into())));
        // DDL/DML without transient keywords are not retried
        assert!(!is_retryable(&WriteDbError::Ddl(
            "syntax error near 'FOO'".into()
        )));
    }
}
