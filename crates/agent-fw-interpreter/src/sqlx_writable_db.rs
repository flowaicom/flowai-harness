//! PostgreSQL-backed WritableDatabase via sqlx.
//!
//! Provides production-grade DDL + DML database access for ETL pipelines.
//! Uses validated smart-constructor newtypes from the WritableDatabase algebra.
//!
//! # Feature Gate
//!
//! This module requires the `postgres` feature:
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["postgres"] }
//! ```
//!
//! # Laws Satisfied
//!
//! - L1 (Execute-DDL): Validated DDL executes successfully
//! - L2 (Insert-Readable): After insert_batch, rows retrievable via TargetDatabase::query
//! - L3 (Idempotent-Drop): drop_table_if_exists on non-existent table succeeds
//! - L4 (Transaction-Atomicity): execute_in_transaction commits all or rolls back

use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgPool;
use sqlx::Row;

use agent_fw_algebra::target_db::QueryParam;
use agent_fw_algebra::writable_db::{
    DdlStatement, DmlStatement, InsertBatch, TableName, WritableDatabase, WriteDbError,
};
use agent_fw_core::DatabaseType;

/// PostgreSQL-backed [`WritableDatabase`] using sqlx connection pooling.
///
/// All SQL-carrying operations use validated smart-constructor newtypes.
/// The interpreter trusts the smart constructors and executes the validated SQL.
pub struct SqlxWritableDatabase {
    pool: PgPool,
    timeout: Duration,
    schema: String,
}

impl SqlxWritableDatabase {
    /// Create from an existing connection pool with default settings.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            timeout: Duration::from_secs(120),
            schema: "public".to_string(),
        }
    }

    /// Connect to a database URL with default settings.
    pub async fn connect(url: &str) -> Result<Self, WriteDbError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| WriteDbError::Connection(e.to_string()))?;
        Ok(Self::new(pool))
    }

    /// Set the write operation timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the schema (default: "public").
    ///
    /// Validates the schema name is a safe identifier (same rules as TableName).
    pub fn with_schema(mut self, schema: impl Into<String>) -> Result<Self, WriteDbError> {
        let s = schema.into();
        agent_fw_algebra::writable_db::TableName::parse(&s)?;
        self.schema = s;
        Ok(self)
    }

    /// Get a reference to the underlying pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Bind QueryParam values to a sqlx query.
    fn bind_params<'a>(
        query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
        params: &'a [QueryParam],
    ) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
        let mut q = query;
        for param in params {
            q = match param {
                QueryParam::Null => q.bind(Option::<String>::None),
                QueryParam::Bool(b) => q.bind(b),
                QueryParam::Int(n) => q.bind(n),
                QueryParam::Float(f) => q.bind(f),
                QueryParam::Text(s) => q.bind(s),
                QueryParam::Json(v) => q.bind(v),
            };
        }
        q
    }

    /// Format a schema-qualified table reference: `"schema"."table"`.
    ///
    /// When schema is `"public"`, emits just `"table"` (PostgreSQL default).
    fn qualified_table(schema: &str, table: &str) -> String {
        if schema == "public" {
            format!("\"{}\"", table)
        } else {
            format!("\"{}\".\"{}\"", schema, table)
        }
    }

    /// Build a multi-row INSERT statement from an InsertBatch.
    ///
    /// When `schema` is `"public"` (default), generates:
    ///   `INSERT INTO "table" ("col1", "col2") VALUES ($1, $2), ($3, $4)`
    ///
    /// Otherwise, generates schema-qualified:
    ///   `INSERT INTO "schema"."table" ("col1", "col2") VALUES ($1, $2), ($3, $4)`
    fn build_insert_sql(schema: &str, batch: &InsertBatch) -> String {
        let quoted_cols: Vec<String> = batch
            .columns()
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect();
        let col_list = quoted_cols.join(", ");

        let mut param_idx = 1usize;
        let row_placeholders: Vec<String> = batch
            .rows()
            .iter()
            .map(|_row| {
                let placeholders: Vec<String> = batch
                    .columns()
                    .iter()
                    .map(|_| {
                        let p = format!("${param_idx}");
                        param_idx += 1;
                        p
                    })
                    .collect();
                format!("({})", placeholders.join(", "))
            })
            .collect();

        format!(
            "INSERT INTO {} ({}) VALUES {}",
            Self::qualified_table(schema, batch.table()),
            col_list,
            row_placeholders.join(", ")
        )
    }

    /// Bind all row values from an InsertBatch to a query.
    fn bind_batch_params<'a>(
        query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
        batch: &'a InsertBatch,
    ) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
        let mut q = query;
        for row in batch.rows() {
            for value in row {
                q = match value {
                    serde_json::Value::Null => q.bind(Option::<String>::None),
                    serde_json::Value::Bool(b) => q.bind(b),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            q.bind(i)
                        } else if let Some(f) = n.as_f64() {
                            q.bind(f)
                        } else {
                            q.bind(n.to_string())
                        }
                    }
                    serde_json::Value::String(s) => q.bind(s.as_str()),
                    other => q.bind(other),
                };
            }
        }
        q
    }
}

#[async_trait]
impl WritableDatabase for SqlxWritableDatabase {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError> {
        tokio::time::timeout(self.timeout, sqlx::query(stmt.sql()).execute(&self.pool))
            .await
            .map_err(|_| WriteDbError::Timeout(self.timeout))?
            .map_err(|e| WriteDbError::Ddl(e.to_string()))?;
        Ok(())
    }

    async fn execute_dml(
        &self,
        stmt: &DmlStatement,
        params: &[QueryParam],
    ) -> Result<u64, WriteDbError> {
        let q = sqlx::query(stmt.sql());
        let q = Self::bind_params(q, params);

        let result = tokio::time::timeout(self.timeout, q.execute(&self.pool))
            .await
            .map_err(|_| WriteDbError::Timeout(self.timeout))?
            .map_err(|e| WriteDbError::Dml(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
        let sql = Self::build_insert_sql(&self.schema, batch);
        let q = sqlx::query(&sql);
        let q = Self::bind_batch_params(q, batch);

        let result = tokio::time::timeout(self.timeout, q.execute(&self.pool))
            .await
            .map_err(|_| WriteDbError::Timeout(self.timeout))?
            .map_err(|e| WriteDbError::Dml(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn insert_batch_returning(&self, batch: &InsertBatch) -> Result<Vec<i64>, WriteDbError> {
        let col = batch.returning_column();
        let sql = format!(
            "{} RETURNING \"{}\"",
            Self::build_insert_sql(&self.schema, batch),
            col
        );
        let q = sqlx::query(&sql);
        let q = Self::bind_batch_params(q, batch);

        let rows = tokio::time::timeout(self.timeout, q.fetch_all(&self.pool))
            .await
            .map_err(|_| WriteDbError::Timeout(self.timeout))?
            .map_err(|e| WriteDbError::Dml(e.to_string()))?;

        let mut ids = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let id = row
                .try_get::<i64, _>(col)
                .or_else(|_| {
                    // Fallback: try positional index 0 (some drivers return unnamed columns)
                    row.try_get::<i64, _>(0)
                })
                .map_err(|e| {
                    WriteDbError::Dml(format!(
                        "Row {i}: RETURNING column \"{col}\" is not i64: {e}"
                    ))
                })?;
            ids.push(id);
        }

        Ok(ids)
    }

    async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError> {
        let sql = format!(
            "DROP TABLE IF EXISTS {}",
            Self::qualified_table(&self.schema, table.as_str())
        );

        tokio::time::timeout(self.timeout, sqlx::query(&sql).execute(&self.pool))
            .await
            .map_err(|_| WriteDbError::Timeout(self.timeout))?
            .map_err(|e| WriteDbError::Ddl(e.to_string()))?;

        Ok(())
    }

    async fn execute_in_transaction(
        &self,
        statements: &[DmlStatement],
    ) -> Result<(), WriteDbError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| WriteDbError::Transaction(format!("Failed to begin: {e}")))?;

        for stmt in statements {
            sqlx::query(stmt.sql())
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    // Transaction will be rolled back on drop
                    WriteDbError::Transaction(format!("Statement failed (will rollback): {e}"))
                })?;
        }

        tx.commit()
            .await
            .map_err(|e| WriteDbError::Transaction(format!("Commit failed: {e}")))?;

        Ok(())
    }

    async fn health_check(&self) -> Result<(), WriteDbError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WriteDbError::Connection(e.to_string()))?;
        Ok(())
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_insert_sql_single_row() {
        let batch = InsertBatch::new(
            "users",
            vec!["id".into(), "name".into()],
            vec![vec![serde_json::json!(1), serde_json::json!("Alice")]],
        )
        .unwrap();

        let sql = SqlxWritableDatabase::build_insert_sql("public", &batch);
        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"id\", \"name\") VALUES ($1, $2)"
        );
    }

    #[test]
    fn build_insert_sql_multi_row() {
        let batch = InsertBatch::new(
            "users",
            vec!["id".into(), "name".into()],
            vec![
                vec![serde_json::json!(1), serde_json::json!("Alice")],
                vec![serde_json::json!(2), serde_json::json!("Bob")],
            ],
        )
        .unwrap();

        let sql = SqlxWritableDatabase::build_insert_sql("public", &batch);
        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"id\", \"name\") VALUES ($1, $2), ($3, $4)"
        );
    }

    #[test]
    fn build_insert_sql_three_columns() {
        let batch = InsertBatch::new(
            "products",
            vec!["id".into(), "name".into(), "price".into()],
            vec![vec![
                serde_json::json!(1),
                serde_json::json!("Widget"),
                serde_json::json!(9.99),
            ]],
        )
        .unwrap();

        let sql = SqlxWritableDatabase::build_insert_sql("public", &batch);
        assert_eq!(
            sql,
            "INSERT INTO \"products\" (\"id\", \"name\", \"price\") VALUES ($1, $2, $3)"
        );
    }

    #[test]
    fn build_insert_sql_with_schema() {
        let batch = InsertBatch::new(
            "users",
            vec!["id".into(), "name".into()],
            vec![vec![serde_json::json!(1), serde_json::json!("Alice")]],
        )
        .unwrap();

        let sql = SqlxWritableDatabase::build_insert_sql("analytics", &batch);
        assert_eq!(
            sql,
            "INSERT INTO \"analytics\".\"users\" (\"id\", \"name\") VALUES ($1, $2)"
        );
    }
}
