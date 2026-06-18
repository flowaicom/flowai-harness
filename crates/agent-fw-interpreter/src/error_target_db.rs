//! Error-returning TargetDatabase — used when no target database is configured.

use std::time::Duration;

use agent_fw_algebra::target_db::{DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase};
use async_trait::async_trait;

/// A TargetDatabase implementation that always returns errors.
///
/// Used as a sentinel when no target database is configured,
/// providing clear error messages instead of panics.
pub struct ErrorTargetDatabase {
    message: String,
    timeout: Duration,
}

const NOT_CONFIGURED: &str = "target database not configured";

impl Default for ErrorTargetDatabase {
    fn default() -> Self {
        Self {
            message: NOT_CONFIGURED.to_string(),
            timeout: Duration::from_secs(0),
        }
    }
}

impl ErrorTargetDatabase {
    /// Create an error database with a caller-supplied message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    /// Override the reported timeout for this sentinel database.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Human-readable error prefix.
    pub fn message(&self) -> &str {
        &self.message
    }

    fn err(&self, context: &str) -> DbError {
        DbError::Connection(format!("{} ({})", self.message, context))
    }
}

#[async_trait]
impl TargetDatabase for ErrorTargetDatabase {
    async fn query(
        &self,
        query: &ReadOnlyQuery,
        _params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        let sql = query.sql();
        let snippet = &sql[..sql.len().min(100)];
        Err(self.err(&format!("attempted query: {}", snippet)))
    }

    async fn health_check(&self) -> Result<(), DbError> {
        Err(self.err("health_check"))
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        Err(self.err("list_tables"))
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        Err(self.err(&format!("get_table_columns({})", table_name)))
    }

    async fn sample_table(
        &self,
        table_name: &str,
        _limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        Err(self.err(&format!("sample_table({})", table_name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn query_returns_error() {
        let db = ErrorTargetDatabase::default();
        let q = ReadOnlyQuery::parse("SELECT 1").unwrap();
        let result = db.query(&q, &[]).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("target database not configured"));
        assert!(err.to_string().contains("SELECT 1"));
    }

    #[tokio::test]
    async fn health_check_returns_error() {
        let db = ErrorTargetDatabase::new("DB unavailable");
        assert!(db.health_check().await.is_err());
    }

    #[tokio::test]
    async fn list_tables_returns_error() {
        let db = ErrorTargetDatabase::default();
        assert!(db.list_tables().await.is_err());
    }

    #[test]
    fn timeout_is_configurable() {
        let db = ErrorTargetDatabase::new("DB unavailable").with_timeout(Duration::from_secs(1));
        assert_eq!(db.timeout(), Duration::from_secs(1));
    }
}
