//! SQLite-backed WritableDatabase via rusqlite.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::ffi::ErrorCode;
use rusqlite::types::Value;
use rusqlite::{Connection, InterruptHandle};

use agent_fw_algebra::target_db::{escape_identifier, QueryParam};
use agent_fw_algebra::writable_db::{
    DdlStatement, DmlStatement, InsertBatch, TableName, WritableDatabase, WriteDbError,
};
use agent_fw_core::DatabaseType;

/// SQLite-backed [`WritableDatabase`].
#[derive(Clone)]
pub struct SqliteWritableDatabase {
    conn: Arc<Mutex<Connection>>,
    interrupt_handle: Arc<InterruptHandle>,
    timeout: Duration,
}

impl SqliteWritableDatabase {
    /// Open (or create) a writable SQLite database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WriteDbError> {
        let conn = Connection::open(path).map_err(|e| WriteDbError::Connection(e.to_string()))?;
        Self::from_connection(conn, true)
    }

    /// Create an in-memory writable SQLite database.
    pub fn in_memory() -> Result<Self, WriteDbError> {
        let conn =
            Connection::open_in_memory().map_err(|e| WriteDbError::Connection(e.to_string()))?;
        Self::from_connection(conn, false)
    }

    fn from_connection(conn: Connection, use_wal: bool) -> Result<Self, WriteDbError> {
        let pragmas = if use_wal {
            "PRAGMA journal_mode = WAL;\nPRAGMA busy_timeout = 5000;\nPRAGMA foreign_keys = ON;"
        } else {
            "PRAGMA busy_timeout = 5000;\nPRAGMA foreign_keys = ON;"
        };
        conn.execute_batch(pragmas).map_err(|e| {
            WriteDbError::Connection(format!("Failed to initialize SQLite pragmas: {e}"))
        })?;
        let interrupt_handle = Arc::new(conn.get_interrupt_handle());
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            interrupt_handle,
            timeout: Duration::from_secs(120),
        })
    }

    /// Override the write timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn sqlite_param(param: &QueryParam) -> Value {
        match param {
            QueryParam::Null => Value::Null,
            QueryParam::Bool(value) => Value::Integer(i64::from(*value)),
            QueryParam::Int(value) => Value::Integer(*value),
            QueryParam::Float(value) => Value::Real(*value),
            QueryParam::Text(value) => Value::Text(value.clone()),
            QueryParam::Json(value) => Value::Text(value.to_string()),
        }
    }

    fn json_value_to_sqlite(value: &serde_json::Value) -> Value {
        match value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(value) => Value::Integer(i64::from(*value)),
            serde_json::Value::Number(value) => {
                if let Some(integer) = value.as_i64() {
                    Value::Integer(integer)
                } else if let Some(real) = value.as_f64() {
                    Value::Real(real)
                } else {
                    Value::Text(value.to_string())
                }
            }
            serde_json::Value::String(value) => Value::Text(value.clone()),
            other => Value::Text(other.to_string()),
        }
    }

    fn quote_table_name(table: &TableName) -> String {
        table
            .as_str()
            .split('.')
            .map(|part| format!("\"{}\"", escape_identifier(part)))
            .collect::<Vec<_>>()
            .join(".")
    }

    fn build_insert_sql(batch: &InsertBatch) -> String {
        let columns = batch
            .columns()
            .iter()
            .map(|column| format!("\"{}\"", escape_identifier(column)))
            .collect::<Vec<_>>()
            .join(", ");

        let mut next_param = 1usize;
        let values = batch
            .rows()
            .iter()
            .map(|row| {
                let placeholders = row
                    .iter()
                    .map(|_| {
                        let placeholder = format!("?{next_param}");
                        next_param += 1;
                        placeholder
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({placeholders})")
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "INSERT INTO {} ({columns}) VALUES {values}",
            Self::quote_table_name(
                &TableName::parse(batch.table()).expect("validated batch table")
            )
        )
    }

    fn is_interrupt_error(error: &rusqlite::Error) -> bool {
        error.sqlite_error_code() == Some(ErrorCode::OperationInterrupted)
    }

    async fn run_interruptible<T, F>(
        &self,
        operation: F,
        map_sqlite_error: fn(rusqlite::Error) -> WriteDbError,
        join_error_variant: fn(String) -> WriteDbError,
    ) -> Result<T, WriteDbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let interrupt_handle = Arc::clone(&self.interrupt_handle);
        let timeout = self.timeout;
        let worker = tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| {
                SqliteWriteOpError::Connection(format!("SQLite lock poisoned: {e}"))
            })?;
            operation(&conn).map_err(SqliteWriteOpError::Sqlite)
        });
        tokio::pin!(worker);

        let delay = tokio::time::sleep(timeout);
        tokio::pin!(delay);

        tokio::select! {
            result = &mut worker => {
                Self::finish_worker(result, map_sqlite_error, join_error_variant, false, timeout)
            }
            _ = &mut delay => {
                if worker.is_finished() {
                    return Self::finish_worker(
                        worker.await,
                        map_sqlite_error,
                        join_error_variant,
                        false,
                        timeout,
                    );
                }

                interrupt_handle.interrupt();
                Self::finish_worker(
                    worker.await,
                    map_sqlite_error,
                    join_error_variant,
                    true,
                    timeout,
                )
            }
        }
    }

    fn finish_worker<T>(
        result: Result<Result<T, SqliteWriteOpError>, tokio::task::JoinError>,
        map_sqlite_error: fn(rusqlite::Error) -> WriteDbError,
        join_error_variant: fn(String) -> WriteDbError,
        timed_out: bool,
        timeout: Duration,
    ) -> Result<T, WriteDbError> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(SqliteWriteOpError::Connection(message))) => {
                Err(WriteDbError::Connection(message))
            }
            Ok(Err(SqliteWriteOpError::Sqlite(error))) => {
                if timed_out && Self::is_interrupt_error(&error) {
                    Err(WriteDbError::Timeout(timeout))
                } else {
                    Err(map_sqlite_error(error))
                }
            }
            Err(error) => Err(join_error_variant(format!(
                "SQLite worker join failed: {error}"
            ))),
        }
    }
}

#[async_trait]
impl WritableDatabase for SqliteWritableDatabase {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::SQLite
    }

    async fn execute_ddl(&self, stmt: &DdlStatement) -> Result<(), WriteDbError> {
        let sql = stmt.sql().to_string();
        self.run_interruptible(
            move |conn| conn.execute_batch(&sql).map(|_| ()),
            |error| WriteDbError::Ddl(error.to_string()),
            WriteDbError::Ddl,
        )
        .await
    }

    async fn execute_dml(
        &self,
        stmt: &DmlStatement,
        params: &[QueryParam],
    ) -> Result<u64, WriteDbError> {
        let sql = stmt.sql().to_string();
        let params: Vec<Value> = params.iter().map(Self::sqlite_param).collect();
        self.run_interruptible(
            move |conn| {
                conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                    .map(|count| count as u64)
            },
            |error| WriteDbError::Dml(error.to_string()),
            WriteDbError::Dml,
        )
        .await
    }

    async fn insert_batch(&self, batch: &InsertBatch) -> Result<u64, WriteDbError> {
        let sql = Self::build_insert_sql(batch);
        let params: Vec<Value> = batch
            .rows()
            .iter()
            .flat_map(|row| row.iter().map(Self::json_value_to_sqlite))
            .collect();
        self.run_interruptible(
            move |conn| {
                conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                    .map(|count| count as u64)
            },
            |error| WriteDbError::Dml(error.to_string()),
            WriteDbError::Dml,
        )
        .await
    }

    async fn insert_batch_returning(&self, batch: &InsertBatch) -> Result<Vec<i64>, WriteDbError> {
        let sql = format!(
            "{} RETURNING \"{}\"",
            Self::build_insert_sql(batch),
            escape_identifier(batch.returning_column())
        );
        let params: Vec<Value> = batch
            .rows()
            .iter()
            .flat_map(|row| row.iter().map(Self::json_value_to_sqlite))
            .collect();
        self.run_interruptible(
            move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let mapped = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    row.get::<_, i64>(0)
                })?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()
            },
            |error| WriteDbError::Dml(error.to_string()),
            WriteDbError::Dml,
        )
        .await
    }

    async fn drop_table_if_exists(&self, table: &TableName) -> Result<(), WriteDbError> {
        let sql = format!("DROP TABLE IF EXISTS {}", Self::quote_table_name(table));
        self.run_interruptible(
            move |conn| conn.execute_batch(&sql).map(|_| ()),
            |error| WriteDbError::Ddl(error.to_string()),
            WriteDbError::Ddl,
        )
        .await
    }

    async fn execute_in_transaction(
        &self,
        statements: &[DmlStatement],
    ) -> Result<(), WriteDbError> {
        let statements = statements
            .iter()
            .map(|stmt| stmt.sql().to_string())
            .collect::<Vec<_>>();
        self.run_interruptible(
            move |conn| {
                let tx = conn.unchecked_transaction()?;
                for statement in statements {
                    tx.execute_batch(&statement)?;
                }
                tx.commit()
            },
            |error| WriteDbError::Transaction(error.to_string()),
            WriteDbError::Transaction,
        )
        .await
    }

    async fn insert_batches_atomically(
        &self,
        batches: &[InsertBatch],
    ) -> Result<u64, WriteDbError> {
        let statements = batches
            .iter()
            .map(|batch| {
                (
                    Self::build_insert_sql(batch),
                    batch
                        .rows()
                        .iter()
                        .flat_map(|row| row.iter().map(Self::json_value_to_sqlite))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        self.run_interruptible(
            move |conn| {
                let tx = conn.unchecked_transaction()?;
                let mut total = 0u64;
                for (sql, params) in statements {
                    total += tx.execute(&sql, rusqlite::params_from_iter(params.iter()))? as u64;
                }
                tx.commit()?;
                Ok(total)
            },
            |error| WriteDbError::Transaction(error.to_string()),
            WriteDbError::Transaction,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), WriteDbError> {
        self.run_interruptible(
            move |conn| {
                conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map(|_| ())
            },
            |error| WriteDbError::Connection(error.to_string()),
            WriteDbError::Connection,
        )
        .await
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}

enum SqliteWriteOpError {
    Connection(String),
    Sqlite(rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn sqlite_writable_db_supports_ddl_and_returning_ids() {
        let db = SqliteWritableDatabase::in_memory().unwrap();
        let ddl = DdlStatement::parse_for(
            "CREATE TABLE products (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            DatabaseType::SQLite,
        )
        .unwrap();
        db.execute_ddl(&ddl).await.unwrap();

        let batch = InsertBatch::new(
            "products",
            vec!["name".to_string()],
            vec![
                vec![serde_json::json!("tea")],
                vec![serde_json::json!("coffee")],
            ],
        )
        .unwrap();

        let ids = db.insert_batch_returning(&batch).await.unwrap();
        assert_eq!(ids, vec![1, 2]);
    }

    #[tokio::test]
    async fn sqlite_writable_db_timeout_interrupts_long_running_write() {
        let db = SqliteWritableDatabase::in_memory()
            .unwrap()
            .with_timeout(Duration::from_millis(10));
        let ddl = DdlStatement::parse_for(
            "CREATE TABLE products (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            DatabaseType::SQLite,
        )
        .unwrap();
        db.execute_ddl(&ddl).await.unwrap();

        let slow_insert = DmlStatement::parse_for(
            "INSERT INTO products (name)
             WITH RECURSIVE cnt(x) AS (
                 SELECT 1
                 UNION ALL
                 SELECT x + 1 FROM cnt WHERE x < 100000000
             )
             SELECT printf('item-%d', x) FROM cnt",
            DatabaseType::SQLite,
        )
        .unwrap();

        let err = db.execute_dml(&slow_insert, &[]).await.unwrap_err();
        assert!(matches!(err, WriteDbError::Timeout(_)));

        let inserted = InsertBatch::new(
            "products",
            vec!["name".to_string()],
            vec![vec![serde_json::json!("after-timeout")]],
        )
        .unwrap();
        assert_eq!(db.insert_batch(&inserted).await.unwrap(), 1);

        let conn = db.conn.lock().unwrap();
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM products", [], |row| row.get(0))
            .unwrap();
        assert_eq!(row_count, 1);
    }
}
