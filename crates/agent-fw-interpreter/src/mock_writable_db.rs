//! Mock WritableDatabase — in-memory implementation for testing.
//!
//! Stores tables as `HashMap<String, Vec<Vec<Value>>>` behind a `RwLock`.
//! Satisfies all WritableDatabase laws (L1-L4).
//!
//! # Companion
//!
//! Use with `MockTargetDatabase` to verify L2 (Insert-Readable):
//! after `insert_batch`, rows can be read back via `MockTargetDatabase::query`.

use agent_fw_algebra::target_db::QueryParam;
use agent_fw_algebra::writable_db::{
    DdlKind, DdlStatement, DmlStatement, InsertBatch, TableName, WritableDatabase, WriteDbError,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Schema for a mock table: ordered column names.
#[derive(Debug, Clone)]
struct MockTableSchema {
    columns: Vec<String>,
}

/// A mock table: schema + rows.
#[derive(Debug, Clone)]
struct MockTable {
    schema: MockTableSchema,
    rows: Vec<Vec<serde_json::Value>>,
}

/// In-memory writable database for testing ETL pipelines.
///
/// Satisfies WritableDatabase laws L1-L4.
pub struct MockWritableDatabase {
    tables: Arc<RwLock<HashMap<String, MockTable>>>,
    /// If true, the next `execute_in_transaction` call will fail
    /// (for testing L4: transaction atomicity / rollback).
    fail_next_transaction: Arc<std::sync::atomic::AtomicBool>,
}

impl MockWritableDatabase {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(RwLock::new(HashMap::new())),
            fail_next_transaction: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Arm the next transaction to fail (for testing rollback behavior).
    pub fn set_fail_next_transaction(&self, fail: bool) {
        self.fail_next_transaction
            .store(fail, std::sync::atomic::Ordering::Release);
    }

    /// Read rows for a table (for test assertions / L2 verification).
    pub async fn read_rows(
        &self,
        table: &str,
    ) -> Option<(Vec<String>, Vec<Vec<serde_json::Value>>)> {
        let tables = self.tables.read().await;
        tables
            .get(table)
            .map(|t| (t.schema.columns.clone(), t.rows.clone()))
    }

    /// Check if a table exists (for test assertions).
    pub async fn table_exists(&self, table: &str) -> bool {
        self.tables.read().await.contains_key(table)
    }

    /// Get table count (for test assertions).
    pub async fn table_count(&self) -> usize {
        self.tables.read().await.len()
    }
}

impl Default for MockWritableDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WritableDatabase for MockWritableDatabase {
    async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError> {
        // Use AST metadata extracted by the smart constructor — no re-parsing.
        match stmt.kind() {
            DdlKind::CreateTable => {
                let table_name = stmt.table_name();

                let mut tables = self.tables.write().await;
                if tables.contains_key(table_name) {
                    return Err(WriteDbError::Ddl(format!(
                        "Table '{table_name}' already exists"
                    )));
                }
                tables.insert(
                    table_name.to_string(),
                    MockTable {
                        schema: MockTableSchema {
                            columns: stmt.columns().to_vec(),
                        },
                        rows: Vec::new(),
                    },
                );
            }
            DdlKind::CreateIndex => {
                // Indexes are no-ops in mock
            }
            DdlKind::CreateView | DdlKind::AlterTable | DdlKind::AlterIndex => {
                // No-ops for mock
            }
        }

        Ok(())
    }

    async fn execute_dml(
        &self,
        _stmt: &DmlStatement,
        _params: &[QueryParam],
    ) -> Result<u64, WriteDbError> {
        // Simple mock: DML is validated by smart constructor, we just return 1
        Ok(1)
    }

    async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
        let mut tables = self.tables.write().await;

        let table = tables.get_mut(batch.table()).ok_or_else(|| {
            WriteDbError::Dml(format!("Table '{}' does not exist", batch.table()))
        })?;

        // Verify columns match schema
        if table.schema.columns != batch.columns() {
            return Err(WriteDbError::Dml(format!(
                "Column mismatch: table has {:?}, batch has {:?}",
                table.schema.columns,
                batch.columns()
            )));
        }

        let count = batch.row_count() as u64;
        for row in batch.rows() {
            table.rows.push(row.clone());
        }

        Ok(count)
    }

    async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError> {
        self.tables.write().await.remove(table.as_str());
        Ok(()) // Idempotent (L3)
    }

    async fn execute_in_transaction(
        &self,
        statements: &[DmlStatement],
    ) -> Result<(), WriteDbError> {
        if self
            .fail_next_transaction
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(WriteDbError::Transaction(
                "Simulated transaction failure".into(),
            ));
        }

        // Execute each DML statement in order. On failure, we don't partially commit.
        for stmt in statements {
            // In mock, DML is just validated — no actual row changes from bare DML
            let _sql = stmt.sql();
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<(), WriteDbError> {
        Ok(())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify MockWritableDatabase satisfies all WritableDatabase algebraic laws
    /// using the reusable harness from agent-fw-test.
    #[tokio::test]
    async fn satisfies_writable_db_laws() {
        let db = MockWritableDatabase::new();
        agent_fw_test::writable_db_laws::test_all(&db).await;
    }

    /// L2 (Insert-Readable) with the mock's read_rows method.
    #[tokio::test]
    async fn satisfies_insert_readable_law() {
        let db = MockWritableDatabase::new();
        agent_fw_test::writable_db_laws::law_insert_readable(&db, |table| {
            let tables = db.tables.clone();
            let table = table.to_string();
            async move {
                let guard = tables.read().await;
                guard
                    .get(&table)
                    .map(|t| t.rows.clone())
                    .unwrap_or_default()
            }
        })
        .await;
    }

    #[tokio::test]
    async fn law_execute_ddl() {
        let db = MockWritableDatabase::new();
        let ddl =
            DdlStatement::parse("CREATE TABLE users (id INT PRIMARY KEY, name TEXT)").unwrap();
        db.execute_ddl(&ddl).await.unwrap();
        assert!(db.table_exists("users").await);
    }

    #[tokio::test]
    async fn law_execute_ddl_duplicate_fails() {
        let db = MockWritableDatabase::new();
        let ddl =
            DdlStatement::parse("CREATE TABLE users (id INT PRIMARY KEY, name TEXT)").unwrap();
        db.execute_ddl(&ddl).await.unwrap();
        let result = db.execute_ddl(&ddl).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn law_insert_readable() {
        let db = MockWritableDatabase::new();

        // Create table
        let ddl = DdlStatement::parse("CREATE TABLE products (id INT, name TEXT)").unwrap();
        db.execute_ddl(&ddl).await.unwrap();

        // Insert batch
        let batch = InsertBatch::new(
            "products",
            vec!["id".into(), "name".into()],
            vec![
                vec![serde_json::json!(1), serde_json::json!("Widget")],
                vec![serde_json::json!(2), serde_json::json!("Gadget")],
            ],
        )
        .unwrap();

        let count = db.insert_batch(&batch).await.unwrap();
        assert_eq!(count, 2);

        // Read back
        let (cols, rows) = db.read_rows("products").await.unwrap();
        assert_eq!(cols, vec!["id", "name"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], serde_json::json!("Widget"));
    }

    #[tokio::test]
    async fn law_idempotent_drop() {
        let db = MockWritableDatabase::new();
        let table = TableName::parse("nonexistent").unwrap();

        // Drop non-existent table succeeds (L3)
        db.drop_table_if_exists(&table).await.unwrap();

        // Create then drop
        let ddl = DdlStatement::parse("CREATE TABLE nonexistent (id INT)").unwrap();
        db.execute_ddl(&ddl).await.unwrap();
        assert!(db.table_exists("nonexistent").await);

        db.drop_table_if_exists(&table).await.unwrap();
        assert!(!db.table_exists("nonexistent").await);

        // Drop again (idempotent)
        db.drop_table_if_exists(&table).await.unwrap();
    }

    #[tokio::test]
    async fn law_transaction_atomicity_success() {
        let db = MockWritableDatabase::new();
        let stmts = vec![
            DmlStatement::parse("INSERT INTO foo (id) VALUES (1)").unwrap(),
            DmlStatement::parse("INSERT INTO foo (id) VALUES (2)").unwrap(),
        ];
        db.execute_in_transaction(&stmts).await.unwrap();
    }

    #[tokio::test]
    async fn law_transaction_atomicity_failure() {
        let db = MockWritableDatabase::new();
        db.set_fail_next_transaction(true);

        let stmts = vec![DmlStatement::parse("INSERT INTO foo (id) VALUES (1)").unwrap()];
        let result = db.execute_in_transaction(&stmts).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn insert_into_nonexistent_table_fails() {
        let db = MockWritableDatabase::new();
        let batch =
            InsertBatch::new("nope", vec!["id".into()], vec![vec![serde_json::json!(1)]]).unwrap();
        let result = db.insert_batch(&batch).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn insert_column_mismatch_fails() {
        let db = MockWritableDatabase::new();
        let ddl = DdlStatement::parse("CREATE TABLE t (id INT, name TEXT)").unwrap();
        db.execute_ddl(&ddl).await.unwrap();

        // Wrong columns
        let batch = InsertBatch::new(
            "t",
            vec!["id".into(), "age".into()], // "age" not in schema
            vec![vec![serde_json::json!(1), serde_json::json!(30)]],
        )
        .unwrap();
        let result = db.insert_batch(&batch).await;
        assert!(result.is_err());
    }

    /// Verify that DdlStatement AST metadata is used correctly by the mock.
    #[tokio::test]
    async fn ddl_ast_metadata_round_trip() {
        let db = MockWritableDatabase::new();

        // Parse DDL — smart constructor extracts table_name and columns from AST
        let ddl =
            DdlStatement::parse("CREATE TABLE products (id INT, name TEXT, price INT)").unwrap();
        assert_eq!(ddl.table_name(), "products");
        assert_eq!(ddl.columns(), &["id", "name", "price"]);
        assert_eq!(ddl.kind(), DdlKind::CreateTable);

        // Mock uses that metadata (not string parsing) to create the table
        db.execute_ddl(&ddl).await.unwrap();

        let (cols, _rows) = db.read_rows("products").await.unwrap();
        assert_eq!(cols, vec!["id", "name", "price"]);
    }
}
