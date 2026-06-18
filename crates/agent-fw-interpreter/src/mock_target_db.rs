//! Mock TargetDatabase — in-memory implementation for testing.
//!
//! Stores pre-configured table data, exact query expectations, default results,
//! and executed-query history for deterministic tests.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;

use agent_fw_algebra::target_db::{
    validate_read_only_for, DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase,
};
use agent_fw_core::DatabaseType;

/// A mock exact-query result.
#[derive(Debug, Clone, Default)]
pub struct MockQueryResult {
    /// Column names.
    pub columns: Vec<String>,
    /// Rows of values.
    pub rows: Vec<Vec<serde_json::Value>>,
}

impl MockQueryResult {
    /// Create an empty result.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create from columns and rows.
    pub fn new(columns: Vec<&str>, rows: Vec<Vec<serde_json::Value>>) -> Self {
        Self {
            columns: columns.into_iter().map(String::from).collect(),
            rows,
        }
    }

    fn to_db_rows(&self) -> Vec<DbRow> {
        self.rows
            .iter()
            .map(|row| DbRow::new(self.columns.clone(), row.clone()))
            .collect()
    }
}

/// In-memory mock database for testing.
///
/// Pre-load tables and rows; `query()` returns rows from the first table
/// whose name appears in the SQL string.
pub struct MockTargetDatabase {
    tables: Arc<RwLock<HashMap<String, Vec<DbRow>>>>,
    columns: Arc<RwLock<HashMap<String, Vec<DbRow>>>>,
    expectations: Arc<RwLock<HashMap<String, MockQueryResult>>>,
    default_result: Arc<RwLock<Option<MockQueryResult>>>,
    executed_queries: Arc<RwLock<Vec<(String, Vec<QueryParam>)>>>,
    validate_read_only: bool,
    timeout: Duration,
}

impl MockTargetDatabase {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(RwLock::new(HashMap::new())),
            columns: Arc::new(RwLock::new(HashMap::new())),
            expectations: Arc::new(RwLock::new(HashMap::new())),
            default_result: Arc::new(RwLock::new(None)),
            executed_queries: Arc::new(RwLock::new(Vec::new())),
            validate_read_only: true,
            timeout: Duration::from_secs(30),
        }
    }

    /// Add rows for a table (used by `query` when the SQL mentions the table name).
    pub fn add_table_rows(&self, table_name: &str, rows: Vec<DbRow>) {
        self.tables
            .write()
            .unwrap()
            .insert(table_name.to_string(), rows);
    }

    /// Add column metadata for a table (returned by `get_table_columns`).
    pub fn add_column_metadata(&self, table_name: &str, columns: Vec<DbRow>) {
        self.columns
            .write()
            .unwrap()
            .insert(table_name.to_string(), columns);
    }

    /// Expect a specific query and return the given result.
    pub fn expect_query(&self, query: &str, result: MockQueryResult) {
        self.expectations
            .write()
            .unwrap()
            .insert(query.trim().to_string(), result);
    }

    /// Set a default result for unmatched queries.
    pub fn set_default_result(&self, result: MockQueryResult) {
        *self.default_result.write().unwrap() = Some(result);
    }

    /// Clear the default unmatched-query result.
    pub fn clear_default_result(&self) {
        *self.default_result.write().unwrap() = None;
    }

    /// Return the current default rows, if configured.
    pub fn default_result_rows(&self) -> Option<Vec<DbRow>> {
        self.default_result
            .read()
            .unwrap()
            .as_ref()
            .map(MockQueryResult::to_db_rows)
    }

    /// Disable read-only validation for compatibility with legacy tests.
    pub fn disable_read_only_validation(&mut self) {
        self.validate_read_only = false;
    }

    /// Set the mock timeout surface.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Executed query history.
    pub fn executed_queries(&self) -> Vec<(String, Vec<QueryParam>)> {
        self.executed_queries.read().unwrap().clone()
    }

    /// Clear query history.
    pub fn clear_history(&self) {
        self.executed_queries.write().unwrap().clear();
    }

    /// Did a query containing this fragment execute?
    pub fn was_executed(&self, query_fragment: &str) -> bool {
        self.executed_queries
            .read()
            .unwrap()
            .iter()
            .any(|(q, _)| q.contains(query_fragment))
    }
}

impl Default for MockTargetDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TargetDatabase for MockTargetDatabase {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        if self.validate_read_only {
            validate_read_only_for(query.sql(), self.database_type())?;
        }

        self.executed_queries
            .write()
            .unwrap()
            .push((query.sql().to_string(), params.to_vec()));

        let trimmed = query.sql().trim().to_string();
        if let Some(result) = self.expectations.read().unwrap().get(&trimmed) {
            return Ok(result.to_db_rows());
        }

        if let Some(result) = self.default_result_rows() {
            return Ok(result);
        }

        let tables = self.tables.read().unwrap();
        let sql_lower = query.sql().to_lowercase();

        for (name, rows) in tables.iter() {
            if sql_lower.contains(&name.to_lowercase()) {
                return Ok(rows.clone());
            }
        }

        Ok(vec![])
    }

    async fn health_check(&self) -> Result<(), DbError> {
        Ok(())
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        let tables = self.tables.read().unwrap();
        let rows: Vec<DbRow> = tables
            .keys()
            .map(|name| DbRow::new(vec!["table_name".into()], vec![serde_json::json!(name)]))
            .collect();
        Ok(rows)
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        let columns = self.columns.read().unwrap();
        Ok(columns.get(table_name).cloned().unwrap_or_default())
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let tables = self.tables.read().unwrap();
        match tables.get(table_name) {
            Some(rows) => Ok(rows
                .iter()
                .take(limit)
                .map(|r| serde_json::to_value(r.as_map()).unwrap_or_default())
                .collect()),
            None => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_db_basic_operations() {
        let db = MockTargetDatabase::new();

        db.add_table_rows(
            "users",
            vec![DbRow::new(
                vec!["id".into(), "name".into()],
                vec![serde_json::json!(1), serde_json::json!("Alice")],
            )],
        );

        // Query matches by table name
        let q = ReadOnlyQuery::parse("SELECT * FROM users").unwrap();
        let rows = db.query(&q, &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name"), Some(&serde_json::json!("Alice")));

        // Health check
        db.health_check().await.unwrap();

        // List tables
        let tables = db.list_tables().await.unwrap();
        assert_eq!(tables.len(), 1);

        // Sample
        let samples = db.sample_table("users", 10).await.unwrap();
        assert_eq!(samples.len(), 1);
    }

    #[tokio::test]
    async fn mock_db_no_match_returns_empty() {
        let db = MockTargetDatabase::new();
        let q = ReadOnlyQuery::parse("SELECT * FROM nonexistent").unwrap();
        let rows = db.query(&q, &[]).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn mock_db_exact_expectation_wins() {
        let db = MockTargetDatabase::new();
        db.expect_query(
            "SELECT * FROM users",
            MockQueryResult::new(vec!["id"], vec![vec![serde_json::json!(1)]]),
        );

        let q = ReadOnlyQuery::parse("SELECT * FROM users").unwrap();
        let rows = db.query(&q, &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&serde_json::json!(1)));
    }

    #[tokio::test]
    async fn mock_db_default_result_is_used() {
        let db = MockTargetDatabase::new();
        db.set_default_result(MockQueryResult::new(
            vec!["count"],
            vec![vec![serde_json::json!(42)]],
        ));

        let q = ReadOnlyQuery::parse("SELECT COUNT(*) FROM anything").unwrap();
        let rows = db.query(&q, &[]).await.unwrap();
        assert_eq!(rows[0].get("count"), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn mock_db_tracks_executed_queries() {
        let db = MockTargetDatabase::new();
        db.set_default_result(MockQueryResult::empty());

        let q1 = ReadOnlyQuery::parse("SELECT * FROM users").unwrap();
        let q2 = ReadOnlyQuery::parse("SELECT * FROM products").unwrap();
        let _: Vec<DbRow> = db.query(&q1, &[]).await.unwrap();
        let _: Vec<DbRow> = db.query(&q2, &[]).await.unwrap();

        assert_eq!(db.executed_queries().len(), 2);
        assert!(db.was_executed("users"));
        assert!(db.was_executed("products"));
    }
}
