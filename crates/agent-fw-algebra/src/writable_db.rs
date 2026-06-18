//! WritableDatabase — async DDL + DML access for ETL pipelines.
//!
//! Separate from [`TargetDatabase`](super::target_db::TargetDatabase) which is
//! read-only by design. This trait provides the write path needed for
//! star-schema creation, dimension loading, and fact insertion.
//!
//! # Design
//!
//! Two distinct algebras rather than one with a "mode" flag:
//! - `TargetDatabase`: read queries, validated read-only
//! - `WritableDatabase`: DDL + DML for schema management and data loading
//!
//! All SQL-carrying operations use **smart-constructor newtypes** (`DdlStatement`,
//! `DmlStatement`) that parse and validate the SQL at construction time. The trait
//! surface never accepts raw `&str` for SQL — callers must go through the
//! validated newtype. This is the same pattern as `EncryptedPayload::new`
//! enforcing a 12-byte nonce.
//!
//! # Laws
//!
//! - **L1 (Execute-DDL)**: `execute_ddl(valid_ddl)` succeeds. Invalid DDL is
//!   rejected at `DdlStatement::parse` time (before it reaches the trait).
//! - **L2 (Insert-Readable)**: After `insert_batch(batch)`, rows are retrievable
//!   via the corresponding `TargetDatabase::query`.
//! - **L3 (Idempotent-Drop)**: `drop_table_if_exists(t)` on a non-existent
//!   table succeeds without error.
//! - **L4 (Transaction-Atomicity)**: `execute_in_transaction(ops)` either
//!   commits all operations or rolls back completely.

use agent_fw_core::DatabaseType;
use async_trait::async_trait;
use std::time::Duration;
use thiserror::Error;

// ============================================================================
// Errors
// ============================================================================

/// Errors from writable database operations.
#[derive(Debug, Clone, Error)]
pub enum WriteDbError {
    #[error("DDL execution failed: {0}")]
    Ddl(String),

    #[error("DML execution failed: {0}")]
    Dml(String),

    #[error("Transaction failed: {0}")]
    Transaction(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Write operation timed out after {0:?}")]
    Timeout(Duration),

    #[error("Invalid SQL: {0}")]
    InvalidSql(String),
}

// ============================================================================
// Smart-constructor newtypes (make illegal states unrepresentable)
// ============================================================================

/// The kind of DDL statement, extracted from the parsed AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdlKind {
    CreateTable,
    CreateIndex,
    CreateView,
    AlterTable,
    AlterIndex,
}

/// A validated DDL statement with metadata extracted from the parsed AST.
///
/// Smart constructor parses the SQL via `sqlparser`, validates it is DDL, and
/// **retains the extracted metadata** (kind, table name, columns) so that
/// interpreters never need to re-parse the SQL string.
///
/// # Carried metadata
///
/// - `kind` — The DDL operation (CREATE TABLE, CREATE INDEX, etc.)
/// - `table_name` — The target table (extracted from AST, not string matching).
///   Always present: every DDL variant targets a named object.
/// - `columns` — Column names for CREATE TABLE (empty for other DDL kinds)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdlStatement {
    sql: String,
    kind: DdlKind,
    table_name: String,
    columns: Vec<String>,
}

impl DdlStatement {
    /// Parse and validate a DDL statement.
    ///
    /// Accepts: CREATE TABLE, CREATE INDEX, CREATE VIEW, ALTER TABLE, ALTER INDEX.
    /// Rejects: SELECT, INSERT, UPDATE, DELETE, EXPLAIN, multi-statement.
    ///
    /// Extracts table name and column definitions from the AST so interpreters
    /// can access them via `table_name()` and `columns()` without re-parsing.
    pub fn parse(sql: &str) -> Result<Self, WriteDbError> {
        Self::parse_for(sql, DatabaseType::PostgreSQL)
    }

    /// Parse and validate a DDL statement for a specific backend dialect.
    pub fn parse_for(sql: &str, database_type: DatabaseType) -> Result<Self, WriteDbError> {
        use sqlparser::ast::Statement;
        use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};
        use sqlparser::parser::Parser;

        let statements = match database_type {
            DatabaseType::PostgreSQL => Parser::parse_sql(&PostgreSqlDialect {}, sql),
            DatabaseType::MySQL => Parser::parse_sql(&MySqlDialect {}, sql),
            DatabaseType::SQLite => Parser::parse_sql(&SQLiteDialect {}, sql),
        }
        .map_err(|e| WriteDbError::InvalidSql(format!("SQL parse error: {e}")))?;

        if statements.is_empty() {
            return Err(WriteDbError::InvalidSql("Empty SQL statement".into()));
        }
        if statements.len() > 1 {
            return Err(WriteDbError::InvalidSql(
                "Multiple statements not allowed in DDL".into(),
            ));
        }

        match &statements[0] {
            Statement::CreateTable(ct) => {
                let table_name = ct.name.to_string();
                let columns = ct.columns.iter().map(|c| c.name.value.clone()).collect();
                Ok(Self {
                    sql: sql.to_string(),
                    kind: DdlKind::CreateTable,
                    table_name,
                    columns,
                })
            }
            Statement::CreateView { name, .. } => Ok(Self {
                sql: sql.to_string(),
                kind: DdlKind::CreateView,
                table_name: name.to_string(),
                columns: Vec::new(),
            }),
            Statement::CreateIndex(ci) => Ok(Self {
                sql: sql.to_string(),
                kind: DdlKind::CreateIndex,
                table_name: ci.table_name.to_string(),
                columns: Vec::new(),
            }),
            Statement::AlterTable { name, .. } => Ok(Self {
                sql: sql.to_string(),
                kind: DdlKind::AlterTable,
                table_name: name.to_string(),
                columns: Vec::new(),
            }),
            Statement::AlterIndex { name, .. } => Ok(Self {
                sql: sql.to_string(),
                kind: DdlKind::AlterIndex,
                table_name: name.to_string(),
                columns: Vec::new(),
            }),
            other => Err(WriteDbError::InvalidSql(format!(
                "Expected DDL (CREATE/ALTER), got: {}",
                super::target_db::statement_kind(other)
            ))),
        }
    }

    /// Access the validated SQL string.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// The kind of DDL operation.
    pub fn kind(&self) -> DdlKind {
        self.kind
    }

    /// The target table name, extracted from the AST.
    ///
    /// Always present — every DDL variant targets a named object.
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Column names extracted from CREATE TABLE definitions.
    /// Empty for non-CREATE TABLE statements.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }
}

/// The kind of DML statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmlKind {
    Insert,
    Update,
    Delete,
}

/// A validated DML statement (INSERT, UPDATE, DELETE) with extracted metadata.
///
/// Smart constructor parses the SQL, rejects non-DML statements, and extracts
/// the target table name from the AST (symmetric with `DdlStatement`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmlStatement {
    sql: String,
    kind: DmlKind,
    /// Target table name, extracted from the AST.
    /// Always present for INSERT/UPDATE/DELETE on a single table.
    table_name: Option<String>,
}

impl DmlStatement {
    /// Parse and validate a DML statement.
    ///
    /// Accepts: INSERT, UPDATE, DELETE.
    /// Rejects: SELECT, DDL, multi-statement.
    ///
    /// Extracts the target table name from the AST where possible.
    pub fn parse(sql: &str) -> Result<Self, WriteDbError> {
        Self::parse_for(sql, DatabaseType::PostgreSQL)
    }

    /// Parse and validate a DML statement for a specific backend dialect.
    pub fn parse_for(sql: &str, database_type: DatabaseType) -> Result<Self, WriteDbError> {
        use sqlparser::ast::{Statement, TableFactor};
        use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};
        use sqlparser::parser::Parser;

        let statements = match database_type {
            DatabaseType::PostgreSQL => Parser::parse_sql(&PostgreSqlDialect {}, sql),
            DatabaseType::MySQL => Parser::parse_sql(&MySqlDialect {}, sql),
            DatabaseType::SQLite => Parser::parse_sql(&SQLiteDialect {}, sql),
        }
        .map_err(|e| WriteDbError::InvalidSql(format!("SQL parse error: {e}")))?;

        if statements.is_empty() {
            return Err(WriteDbError::InvalidSql("Empty SQL statement".into()));
        }
        if statements.len() > 1 {
            return Err(WriteDbError::InvalidSql(
                "Multiple statements not allowed in DML".into(),
            ));
        }

        let (kind, table_name) = match &statements[0] {
            Statement::Insert(insert) => (DmlKind::Insert, Some(insert.table_name.to_string())),
            Statement::Update { table, .. } => {
                let name = match &table.relation {
                    TableFactor::Table { name, .. } => Some(name.to_string()),
                    _ => None,
                };
                (DmlKind::Update, name)
            }
            Statement::Delete(delete) => {
                let name = Self::extract_delete_table(delete);
                (DmlKind::Delete, name)
            }
            other => {
                return Err(WriteDbError::InvalidSql(format!(
                    "Expected DML (INSERT/UPDATE/DELETE), got: {}",
                    super::target_db::statement_kind(other)
                )));
            }
        };

        Ok(Self {
            sql: sql.to_string(),
            kind,
            table_name,
        })
    }

    /// Extract the target table name from a DELETE statement's FROM clause.
    fn extract_delete_table(delete: &sqlparser::ast::Delete) -> Option<String> {
        use sqlparser::ast::{FromTable, TableFactor};
        let tables = match &delete.from {
            FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
        };
        if let Some(first) = tables.first() {
            if let TableFactor::Table { name, .. } = &first.relation {
                return Some(name.to_string());
            }
        }
        None
    }

    /// Access the validated SQL string.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// The kind of DML operation.
    pub fn kind(&self) -> DmlKind {
        self.kind
    }

    /// The target table name, extracted from the AST.
    ///
    /// Always `Some` for standard single-table INSERT/UPDATE/DELETE.
    /// May be `None` for exotic multi-table DELETE syntax.
    pub fn table_name(&self) -> Option<&str> {
        self.table_name.as_deref()
    }
}

// ============================================================================
// Coercion Policy (numeric type coercion diagnostics)
// ============================================================================

/// Policy for handling non-conforming values in batch inserts.
///
/// Makes non-numeric coercion behavior explicit so callers choose their
/// tolerance instead of inheriting implicit bind-time behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionPolicy {
    /// Reject non-conforming values (default — current behavior).
    Strict,
    /// Coerce non-conforming values to NULL, optionally logging.
    Coerce { log: bool },
}

impl Default for CoercionPolicy {
    fn default() -> Self {
        Self::Strict
    }
}

/// A single coercion event observed during batch construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoercionEvent {
    /// Row index (0-based).
    pub row: usize,
    /// Column index (0-based).
    pub column: usize,
    /// Column name.
    pub column_name: String,
    /// Original value (as JSON string).
    pub original_value: String,
    /// What happened.
    pub kind: CoercionKind,
}

/// The kind of coercion applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionKind {
    /// A string value in a numeric-looking column was coerced to NULL.
    StringToNull,
    /// A non-finite float (NaN, Infinity) was coerced to NULL.
    NonFiniteToNull,
}

/// Result of constructing an InsertBatch with coercion policy.
#[derive(Debug, Clone)]
pub struct InsertBatchResult {
    /// The validated batch (values may have been coerced).
    pub batch: InsertBatch,
    /// Coercion events observed during construction (empty under `Strict`).
    pub coercions: Vec<CoercionEvent>,
}

/// A validated batch insert — table name, column names, and row data.
///
/// # Invariants (enforced by smart constructor)
///
/// - Table name is non-empty and contains no SQL injection characters
/// - At least one column
/// - Every row has exactly `columns.len()` values (rectangular)
/// - At least one row
/// - `returning_column` (if set) is a valid identifier
#[derive(Debug, Clone)]
pub struct InsertBatch {
    table: String,
    columns: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
    /// Column name to use for `INSERT ... RETURNING <col>`.
    /// Defaults to `"id"` when `None`. Validated at construction time
    /// via [`with_returning_column`](Self::with_returning_column).
    returning_column: Option<String>,
}

impl InsertBatch {
    /// Construct a validated insert batch.
    pub fn new(
        table: impl Into<String>,
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    ) -> Result<Self, WriteDbError> {
        let table = table.into();

        // Validate table name using the same rules as TableName
        let _ = TableName::parse(&table)
            .map_err(|e| WriteDbError::InvalidSql(format!("InsertBatch table: {e}")))?;
        if columns.is_empty() {
            return Err(WriteDbError::InvalidSql(
                "At least one column required".into(),
            ));
        }
        for col in &columns {
            if col.is_empty() {
                return Err(WriteDbError::InvalidSql(
                    "Column name must not be empty".into(),
                ));
            }
            if !col.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(WriteDbError::InvalidSql(format!(
                    "Column name contains invalid characters (only alphanumeric + underscore allowed): {col}"
                )));
            }
        }
        if rows.is_empty() {
            return Err(WriteDbError::InvalidSql("At least one row required".into()));
        }
        let width = columns.len();
        for (i, row) in rows.iter().enumerate() {
            if row.len() != width {
                return Err(WriteDbError::InvalidSql(format!(
                    "Row {i} has {} values, expected {width} (columns: {:?})",
                    row.len(),
                    columns,
                )));
            }
        }

        Ok(Self {
            table,
            columns,
            rows,
            returning_column: None,
        })
    }

    /// Construct a validated insert batch with an explicit coercion policy.
    ///
    /// Under `CoercionPolicy::Coerce`, non-numeric values in numeric-looking
    /// columns are replaced with `serde_json::Value::Null`, and the coercion
    /// is recorded in the returned `InsertBatchResult.coercions`.
    ///
    /// Under `CoercionPolicy::Strict` (default), this behaves identically
    /// to [`new`](Self::new).
    pub fn new_with_policy(
        table: impl Into<String>,
        columns: Vec<String>,
        mut rows: Vec<Vec<serde_json::Value>>,
        policy: CoercionPolicy,
    ) -> Result<InsertBatchResult, WriteDbError> {
        let mut coercions = Vec::new();

        if let CoercionPolicy::Coerce { log } = policy {
            for (row_idx, row) in rows.iter_mut().enumerate() {
                for (col_idx, value) in row.iter_mut().enumerate() {
                    let coercion = match value {
                        // Non-finite floats (NaN, Infinity) → NULL
                        serde_json::Value::Number(n) => {
                            if let Some(f) = n.as_f64() {
                                if !f.is_finite() {
                                    Some(CoercionKind::NonFiniteToNull)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        // String in a value position where we expect a number
                        // (heuristic: if the string fails to parse as a number)
                        serde_json::Value::String(s) => {
                            if s.parse::<f64>().is_err() && s.parse::<i64>().is_err() {
                                Some(CoercionKind::StringToNull)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some(kind) = coercion {
                        let col_name = columns
                            .get(col_idx)
                            .cloned()
                            .unwrap_or_else(|| format!("col_{col_idx}"));
                        if log {
                            coercions.push(CoercionEvent {
                                row: row_idx,
                                column: col_idx,
                                column_name: col_name,
                                original_value: value.to_string(),
                                kind,
                            });
                        }
                        *value = serde_json::Value::Null;
                    }
                }
            }
        }

        let batch = Self::new(table, columns, rows)?;
        Ok(InsertBatchResult { batch, coercions })
    }

    /// Set the column name used by `INSERT ... RETURNING <col>`.
    ///
    /// Validates the column name is a safe identifier (alphanumeric + underscore).
    /// Defaults to `"id"` when not set.
    pub fn with_returning_column(mut self, column: &str) -> Result<Self, WriteDbError> {
        if column.is_empty() {
            return Err(WriteDbError::InvalidSql(
                "Returning column name must not be empty".into(),
            ));
        }
        if !column.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(WriteDbError::InvalidSql(format!(
                "Returning column contains invalid characters: {column}"
            )));
        }
        self.returning_column = Some(column.to_string());
        Ok(self)
    }

    /// The column name for `RETURNING` clauses. Defaults to `"id"`.
    pub fn returning_column(&self) -> &str {
        self.returning_column.as_deref().unwrap_or("id")
    }

    pub fn table(&self) -> &str {
        &self.table
    }
    pub fn columns(&self) -> &[String] {
        &self.columns
    }
    pub fn rows(&self) -> &[Vec<serde_json::Value>] {
        &self.rows
    }
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// A validated table identifier (non-empty, valid SQL identifier characters).
///
/// # Allowed characters (positive allowlist)
///
/// - Alphanumeric (`a-z`, `A-Z`, `0-9`)
/// - Underscore (`_`)
/// - Dot (`.`) for schema-qualified names like `schema.table`
///
/// Everything else is rejected. This is strictly safer than a blocklist
/// because novel injection vectors can't bypass an allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableName(String);

impl TableName {
    /// Parse and validate a table name.
    ///
    /// Accepts only alphanumeric + underscore + dot characters.
    /// Must not be empty, must not start with a dot, must not end with a dot,
    /// must not contain consecutive dots.
    pub fn parse(name: &str) -> Result<Self, WriteDbError> {
        if name.is_empty() {
            return Err(WriteDbError::InvalidSql(
                "Table name must not be empty".into(),
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Err(WriteDbError::InvalidSql(format!(
                "Table name contains invalid characters (only alphanumeric, underscore, dot allowed): {name}"
            )));
        }
        if name.starts_with('.') || name.ends_with('.') || name.contains("..") {
            return Err(WriteDbError::InvalidSql(format!(
                "Table name has invalid dot placement: {name}"
            )));
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TableName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// Trait
// ============================================================================

/// Async writable database access for ETL operations.
///
/// All SQL-carrying methods accept validated newtypes, not raw strings.
/// Consumers declare which capability they need via trait bounds:
/// an ETL pipeline requires `WritableDatabase`; an agent tool requires
/// only `TargetDatabase`. The type system enforces the separation.
#[async_trait]
pub trait WritableDatabase: Send + Sync {
    /// The concrete database engine behind this handle.
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    /// Execute a validated DDL statement.
    async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError>;

    /// Execute a validated DML statement with parameters.
    ///
    /// Returns the number of rows affected.
    async fn execute_dml(
        &self,
        stmt: &DmlStatement,
        params: &[super::target_db::QueryParam],
    ) -> Result<u64, WriteDbError>;

    /// Insert a validated batch of rows into a table.
    ///
    /// Implementations should use bulk insert for performance (e.g.,
    /// multi-row INSERT or COPY for PostgreSQL).
    ///
    /// Returns the number of rows inserted.
    async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError>;

    /// Insert a validated batch of rows and return the assigned auto-increment IDs.
    ///
    /// This is essential for databases with pre-existing data where
    /// auto-increment IDs do not start at 1. Callers must use the returned
    /// IDs for foreign-key lookups instead of assuming `1..=n`.
    ///
    /// # Default implementation
    ///
    /// Falls back to [`insert_batch`](Self::insert_batch) and generates
    /// sequential 1-based IDs (`1..=row_count`). This preserves backward
    /// compatibility for interpreters that have not yet overridden this method.
    /// Interpreters backed by a real database (PostgreSQL, SQLite) should
    /// override this to use `INSERT ... RETURNING id`.
    async fn insert_batch_returning(&self, batch: &InsertBatch) -> Result<Vec<i64>, WriteDbError> {
        let count = self.insert_batch(batch).await?;
        Ok((1..=count as i64).collect())
    }

    /// Drop a table if it exists (idempotent — L3).
    async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError>;

    /// Execute multiple validated DML statements within a single transaction.
    ///
    /// All statements succeed (commit) or all fail (rollback) — L4.
    async fn execute_in_transaction(&self, statements: &[DmlStatement])
        -> Result<(), WriteDbError>;

    /// Insert multiple batches atomically — all succeed or all fail.
    ///
    /// # Default implementation
    ///
    /// Falls back to sequential `insert_batch` calls (no atomicity).
    /// Interpreters backed by a real database should override this to
    /// wrap all inserts in a single transaction.
    ///
    /// # Returns
    ///
    /// Total number of rows inserted across all batches.
    async fn insert_batches_atomically(
        &self,
        batches: &[InsertBatch],
    ) -> Result<u64, WriteDbError> {
        let mut total = 0u64;
        for batch in batches {
            total += self.insert_batch(batch).await?;
        }
        Ok(total)
    }

    /// Check that the writable connection is healthy.
    async fn health_check(&self) -> Result<(), WriteDbError>;

    /// Write operation timeout (implementations may override).
    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

/// Extension trait for common ETL operations built on `WritableDatabase`.
#[async_trait]
pub trait WritableDatabaseExt: WritableDatabase {
    /// Create a table from a DDL statement, dropping it first if it exists.
    async fn recreate_table(
        &self,
        table: &TableName,
        create_ddl: &DdlStatement,
    ) -> Result<(), WriteDbError> {
        self.drop_table_if_exists(table).await?;
        self.execute_ddl(create_ddl).await
    }
}

#[async_trait]
impl<T: WritableDatabase + ?Sized> WritableDatabaseExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // DdlStatement smart constructor
    // =========================================================================

    #[test]
    fn ddl_accepts_create_table() {
        let stmt = DdlStatement::parse("CREATE TABLE foo (id INT PRIMARY KEY, name TEXT)");
        assert!(stmt.is_ok());
        assert!(stmt.unwrap().sql().contains("CREATE TABLE"));
    }

    #[test]
    fn ddl_accepts_create_index() {
        let stmt = DdlStatement::parse("CREATE INDEX idx_foo ON foo (name)");
        assert!(stmt.is_ok());
    }

    #[test]
    fn ddl_accepts_alter_table() {
        let stmt = DdlStatement::parse("ALTER TABLE foo ADD COLUMN age INT");
        assert!(stmt.is_ok());
    }

    #[test]
    fn ddl_rejects_select() {
        let stmt = DdlStatement::parse("SELECT * FROM foo");
        assert!(stmt.is_err());
        assert!(stmt.unwrap_err().to_string().contains("Expected DDL"));
    }

    #[test]
    fn ddl_rejects_insert() {
        let stmt = DdlStatement::parse("INSERT INTO foo VALUES (1)");
        assert!(stmt.is_err());
    }

    #[test]
    fn ddl_rejects_multi_statement() {
        let stmt = DdlStatement::parse("CREATE TABLE a (id INT); CREATE TABLE b (id INT)");
        assert!(stmt.is_err());
        assert!(stmt
            .unwrap_err()
            .to_string()
            .contains("Multiple statements"));
    }

    #[test]
    fn ddl_rejects_empty() {
        let stmt = DdlStatement::parse("");
        assert!(stmt.is_err());
    }

    // =========================================================================
    // DmlStatement smart constructor
    // =========================================================================

    #[test]
    fn dml_accepts_insert() {
        let stmt = DmlStatement::parse("INSERT INTO foo (id) VALUES (1)");
        assert!(stmt.is_ok());
    }

    #[test]
    fn dml_accepts_update() {
        let stmt = DmlStatement::parse("UPDATE foo SET name = 'bar' WHERE id = 1");
        assert!(stmt.is_ok());
    }

    #[test]
    fn dml_accepts_delete() {
        let stmt = DmlStatement::parse("DELETE FROM foo WHERE id = 1");
        assert!(stmt.is_ok());
    }

    #[test]
    fn dml_rejects_select() {
        let stmt = DmlStatement::parse("SELECT * FROM foo");
        assert!(stmt.is_err());
    }

    #[test]
    fn dml_rejects_ddl() {
        let stmt = DmlStatement::parse("CREATE TABLE foo (id INT)");
        assert!(stmt.is_err());
    }

    #[test]
    fn dml_insert_extracts_table_name() {
        let stmt = DmlStatement::parse("INSERT INTO users (id) VALUES (1)").unwrap();
        assert_eq!(stmt.kind(), DmlKind::Insert);
        assert_eq!(stmt.table_name(), Some("users"));
    }

    #[test]
    fn dml_update_extracts_table_name() {
        let stmt = DmlStatement::parse("UPDATE orders SET status = 'done' WHERE id = 1").unwrap();
        assert_eq!(stmt.kind(), DmlKind::Update);
        assert_eq!(stmt.table_name(), Some("orders"));
    }

    #[test]
    fn dml_delete_extracts_table_name() {
        let stmt = DmlStatement::parse("DELETE FROM products WHERE id = 1").unwrap();
        assert_eq!(stmt.kind(), DmlKind::Delete);
        assert_eq!(stmt.table_name(), Some("products"));
    }

    // =========================================================================
    // InsertBatch smart constructor
    // =========================================================================

    #[test]
    fn batch_accepts_valid() {
        let batch = InsertBatch::new(
            "users",
            vec!["id".into(), "name".into()],
            vec![
                vec![serde_json::json!(1), serde_json::json!("Alice")],
                vec![serde_json::json!(2), serde_json::json!("Bob")],
            ],
        );
        assert!(batch.is_ok());
        let b = batch.unwrap();
        assert_eq!(b.table(), "users");
        assert_eq!(b.columns().len(), 2);
        assert_eq!(b.row_count(), 2);
    }

    #[test]
    fn batch_rejects_empty_table() {
        let batch = InsertBatch::new("", vec!["id".into()], vec![vec![serde_json::json!(1)]]);
        assert!(batch.is_err());
    }

    #[test]
    fn batch_rejects_injection_in_table() {
        let batch = InsertBatch::new(
            "users; DROP TABLE--",
            vec!["id".into()],
            vec![vec![serde_json::json!(1)]],
        );
        assert!(batch.is_err());
        // Also rejects spaces, parens, etc.
        let batch2 = InsertBatch::new(
            "table (id)",
            vec!["id".into()],
            vec![vec![serde_json::json!(1)]],
        );
        assert!(batch2.is_err());
    }

    #[test]
    fn batch_rejects_empty_columns() {
        let batch = InsertBatch::new("foo", vec![], vec![vec![]]);
        assert!(batch.is_err());
    }

    #[test]
    fn batch_rejects_empty_rows() {
        let batch: Result<InsertBatch, WriteDbError> =
            InsertBatch::new("foo", vec!["id".into()], vec![]);
        assert!(batch.is_err());
    }

    #[test]
    fn batch_rejects_empty_column_name() {
        let batch = InsertBatch::new("foo", vec!["".into()], vec![vec![serde_json::json!(1)]]);
        assert!(batch.is_err());
        assert!(batch
            .unwrap_err()
            .to_string()
            .contains("Column name must not be empty"));
    }

    #[test]
    fn batch_rejects_injection_in_column_name() {
        let batch = InsertBatch::new(
            "foo",
            vec!["id; DROP TABLE x".into()],
            vec![vec![serde_json::json!(1)]],
        );
        assert!(batch.is_err());
        assert!(batch
            .unwrap_err()
            .to_string()
            .contains("invalid characters"));
    }

    #[test]
    fn batch_rejects_ragged_rows() {
        let batch = InsertBatch::new(
            "foo",
            vec!["id".into(), "name".into()],
            vec![
                vec![serde_json::json!(1), serde_json::json!("ok")],
                vec![serde_json::json!(2)], // missing column
            ],
        );
        assert!(batch.is_err());
        assert!(batch
            .unwrap_err()
            .to_string()
            .contains("Row 1 has 1 values"));
    }

    // =========================================================================
    // InsertBatch returning_column
    // =========================================================================

    #[test]
    fn batch_returning_column_defaults_to_id() {
        let batch =
            InsertBatch::new("t", vec!["a".into()], vec![vec![serde_json::json!(1)]]).unwrap();
        assert_eq!(batch.returning_column(), "id");
    }

    #[test]
    fn batch_with_returning_column() {
        let batch = InsertBatch::new("t", vec!["a".into()], vec![vec![serde_json::json!(1)]])
            .unwrap()
            .with_returning_column("user_id")
            .unwrap();
        assert_eq!(batch.returning_column(), "user_id");
    }

    #[test]
    fn batch_returning_column_rejects_empty() {
        let batch = InsertBatch::new("t", vec!["a".into()], vec![vec![serde_json::json!(1)]])
            .unwrap()
            .with_returning_column("");
        assert!(batch.is_err());
    }

    #[test]
    fn batch_returning_column_rejects_injection() {
        let batch = InsertBatch::new("t", vec!["a".into()], vec![vec![serde_json::json!(1)]])
            .unwrap()
            .with_returning_column("id; DROP TABLE--");
        assert!(batch.is_err());
    }

    // =========================================================================
    // TableName smart constructor
    // =========================================================================

    #[test]
    fn table_name_accepts_valid() {
        assert!(TableName::parse("users").is_ok());
        assert!(TableName::parse("public.users").is_ok());
        assert!(TableName::parse("dim_product").is_ok());
        assert!(TableName::parse("Table123").is_ok());
    }

    #[test]
    fn table_name_rejects_empty() {
        assert!(TableName::parse("").is_err());
    }

    #[test]
    fn table_name_rejects_non_identifier_chars() {
        assert!(TableName::parse("users; DROP TABLE x").is_err());
        assert!(TableName::parse("users--comment").is_err());
        assert!(TableName::parse("users' OR 1=1").is_err());
        assert!(TableName::parse("users\n; DROP TABLE x").is_err());
        assert!(TableName::parse("table (").is_err());
        assert!(TableName::parse("tab`le").is_err());
        assert!(TableName::parse("tab\"le").is_err());
    }

    #[test]
    fn table_name_rejects_bad_dot_placement() {
        assert!(TableName::parse(".users").is_err());
        assert!(TableName::parse("users.").is_err());
        assert!(TableName::parse("schema..table").is_err());
    }

    // =========================================================================
    // Error display
    // =========================================================================

    #[test]
    fn write_db_error_display() {
        let e = WriteDbError::Ddl("syntax error".into());
        assert_eq!(e.to_string(), "DDL execution failed: syntax error");

        let e = WriteDbError::InvalidSql("bad sql".into());
        assert_eq!(e.to_string(), "Invalid SQL: bad sql");
    }

    #[test]
    fn write_db_error_variants() {
        let _ = WriteDbError::Dml("constraint violation".into());
        let _ = WriteDbError::Transaction("deadlock".into());
        let _ = WriteDbError::Connection("refused".into());
        let _ = WriteDbError::Timeout(Duration::from_secs(30));
    }

    // =========================================================================
    // insert_batch_returning default implementation
    // =========================================================================

    /// Minimal WritableDatabase impl to verify the default `insert_batch_returning`.
    struct StubWritableDb;

    #[async_trait]
    impl WritableDatabase for StubWritableDb {
        async fn execute_ddl(&self, _stmt: &DdlStatement) -> Result<(), WriteDbError> {
            Ok(())
        }
        async fn execute_dml(
            &self,
            _stmt: &DmlStatement,
            _params: &[crate::target_db::QueryParam],
        ) -> Result<u64, WriteDbError> {
            Ok(0)
        }
        async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
            Ok(batch.row_count() as u64)
        }
        async fn drop_table_if_exists(&self, _table: &TableName) -> Result<(), WriteDbError> {
            Ok(())
        }
        async fn execute_in_transaction(
            &self,
            _statements: &[DmlStatement],
        ) -> Result<(), WriteDbError> {
            Ok(())
        }
        async fn health_check(&self) -> Result<(), WriteDbError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn insert_batch_returning_default_generates_1_based_ids() {
        let db = StubWritableDb;
        let batch = InsertBatch::new(
            "test_table",
            vec!["a".into(), "b".into()],
            vec![
                vec![serde_json::json!(1), serde_json::json!("x")],
                vec![serde_json::json!(2), serde_json::json!("y")],
                vec![serde_json::json!(3), serde_json::json!("z")],
            ],
        )
        .unwrap();

        let ids = db.insert_batch_returning(&batch).await.unwrap();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn insert_batch_returning_default_single_row() {
        let db = StubWritableDb;
        let batch = InsertBatch::new(
            "test_table",
            vec!["id".into()],
            vec![vec![serde_json::json!(42)]],
        )
        .unwrap();

        let ids = db.insert_batch_returning(&batch).await.unwrap();
        assert_eq!(ids, vec![1]);
    }
}
