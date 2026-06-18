//! SQLite-backed TargetDatabase via rusqlite.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::ffi::ErrorCode;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, InterruptHandle};

use agent_fw_algebra::target_db::{
    escape_identifier, DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase,
};
use agent_fw_algebra::writable_db::TableName;
use agent_fw_core::DatabaseType;

/// SQLite-backed [`TargetDatabase`].
///
/// Uses a single connection guarded by a mutex and executes all work in
/// `spawn_blocking`, matching the framework's other SQLite interpreters.
#[derive(Clone)]
pub struct SqliteTargetDatabase {
    conn: Arc<Mutex<Connection>>,
    interrupt_handle: Arc<InterruptHandle>,
    timeout: Duration,
}

impl SqliteTargetDatabase {
    /// Open (or create) a SQLite target database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let conn = Connection::open(path).map_err(|e| DbError::Connection(e.to_string()))?;
        Self::from_connection(conn, true)
    }

    /// Create an in-memory SQLite target database.
    pub fn in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory().map_err(|e| DbError::Connection(e.to_string()))?;
        Self::from_connection(conn, false)
    }

    fn from_connection(conn: Connection, use_wal: bool) -> Result<Self, DbError> {
        let pragmas = if use_wal {
            "PRAGMA journal_mode = WAL;\nPRAGMA busy_timeout = 5000;\nPRAGMA foreign_keys = ON;"
        } else {
            "PRAGMA busy_timeout = 5000;\nPRAGMA foreign_keys = ON;"
        };
        conn.execute_batch(pragmas).map_err(|e| {
            DbError::Connection(format!("Failed to initialize SQLite pragmas: {e}"))
        })?;
        let interrupt_handle = Arc::new(conn.get_interrupt_handle());
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            interrupt_handle,
            timeout: Duration::from_secs(30),
        })
    }

    /// Override the query timeout.
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

    fn row_value_to_json(row: &rusqlite::Row<'_>, idx: usize) -> serde_json::Value {
        match row.get_ref(idx) {
            Ok(ValueRef::Null) => serde_json::Value::Null,
            Ok(ValueRef::Integer(value)) => serde_json::json!(value),
            Ok(ValueRef::Real(value)) => serde_json::json!(value),
            Ok(ValueRef::Text(value)) => {
                serde_json::Value::String(String::from_utf8_lossy(value).into_owned())
            }
            Ok(ValueRef::Blob(value)) => {
                serde_json::Value::String(format!("<blob {} bytes>", value.len()))
            }
            Err(_) => serde_json::Value::Null,
        }
    }

    fn is_interrupt_error(error: &rusqlite::Error) -> bool {
        error.sqlite_error_code() == Some(ErrorCode::OperationInterrupted)
    }

    async fn run_interruptible<T, F>(
        &self,
        operation: F,
        map_sqlite_error: fn(rusqlite::Error) -> DbError,
    ) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let interrupt_handle = Arc::clone(&self.interrupt_handle);
        let timeout = self.timeout;
        let worker = tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| {
                SqliteTargetOpError::Connection(format!("SQLite lock poisoned: {e}"))
            })?;
            operation(&conn).map_err(SqliteTargetOpError::Sqlite)
        });
        tokio::pin!(worker);

        let delay = tokio::time::sleep(timeout);
        tokio::pin!(delay);

        tokio::select! {
            result = &mut worker => Self::finish_worker(result, map_sqlite_error, false, timeout),
            _ = &mut delay => {
                if worker.is_finished() {
                    return Self::finish_worker(worker.await, map_sqlite_error, false, timeout);
                }

                interrupt_handle.interrupt();
                Self::finish_worker(worker.await, map_sqlite_error, true, timeout)
            }
        }
    }

    fn finish_worker<T>(
        result: Result<Result<T, SqliteTargetOpError>, tokio::task::JoinError>,
        map_sqlite_error: fn(rusqlite::Error) -> DbError,
        timed_out: bool,
        timeout: Duration,
    ) -> Result<T, DbError> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(SqliteTargetOpError::Connection(message))) => Err(DbError::Connection(message)),
            Ok(Err(SqliteTargetOpError::Sqlite(error))) => {
                if timed_out && Self::is_interrupt_error(&error) {
                    Err(DbError::Timeout(timeout))
                } else {
                    Err(map_sqlite_error(error))
                }
            }
            Err(error) => Err(DbError::Execution(format!(
                "SQLite worker join failed: {error}"
            ))),
        }
    }

    fn quote_table_name(table_name: &str) -> Result<String, DbError> {
        let validated =
            TableName::parse(table_name).map_err(|e| DbError::InvalidQuery(e.to_string()))?;
        Ok(validated
            .as_str()
            .split('.')
            .map(|part| format!("\"{}\"", escape_identifier(part)))
            .collect::<Vec<_>>()
            .join("."))
    }
}

#[async_trait]
impl TargetDatabase for SqliteTargetDatabase {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::SQLite
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        let sql = query.sql().to_string();
        let params: Vec<Value> = params.iter().map(Self::sqlite_param).collect();
        self.run_interruptible(
            move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> = stmt
                    .column_names()
                    .iter()
                    .map(|name| name.to_string())
                    .collect();

                let mapped = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    let values = (0..columns.len())
                        .map(|idx| Self::row_value_to_json(row, idx))
                        .collect::<Vec<_>>();
                    Ok(DbRow::new(columns.clone(), values))
                })?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()
            },
            |error| DbError::Execution(error.to_string()),
        )
        .await
    }

    async fn health_check(&self) -> Result<(), DbError> {
        self.run_interruptible(
            move |conn| {
                conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map(|_| ())
            },
            |error| DbError::Connection(error.to_string()),
        )
        .await
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        let query = ReadOnlyQuery::parse_for(
            "SELECT name AS table_name, \
                    'main' AS schema_name, \
                    CASE type WHEN 'view' THEN 'VIEW' ELSE 'BASE TABLE' END AS table_type \
             FROM sqlite_master \
             WHERE type IN ('table', 'view') \
               AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
            DatabaseType::SQLite,
        )?;
        self.query(&query, &[]).await
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        let query = ReadOnlyQuery::parse_for(
            "SELECT name AS column_name, \
                    type AS data_type, \
                    CASE WHEN \"notnull\" = 0 THEN 'YES' ELSE 'NO' END AS is_nullable, \
                    dflt_value AS column_default, \
                    cid + 1 AS ordinal_position, \
                    pk > 0 AS is_primary_key \
             FROM pragma_table_info(?1) \
             ORDER BY cid",
            DatabaseType::SQLite,
        )?;
        self.query(&query, &[table_name.to_string().into()]).await
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let quoted = Self::quote_table_name(table_name)?;
        let query = ReadOnlyQuery::parse_for(
            format!("SELECT * FROM {quoted} LIMIT {}", limit.min(100)),
            DatabaseType::SQLite,
        )?;
        let rows = self.query(&query, &[]).await?;
        Ok(rows
            .into_iter()
            .map(|row| serde_json::Value::Object(row.as_map().clone().into_iter().collect()))
            .collect())
    }
}

enum SqliteTargetOpError {
    Connection(String),
    Sqlite(rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn slow_recursive_sum_sql(limit: usize) -> String {
        format!(
            "WITH RECURSIVE cnt(x) AS (\
                SELECT 1 \
                UNION ALL \
                SELECT x + 1 FROM cnt WHERE x < {limit}\
            ) \
            SELECT sum(x) AS total FROM cnt"
        )
    }

    fn init_file_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE items (id INTEGER PRIMARY KEY, payload TEXT);",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn sqlite_target_db_supports_query_and_discovery() {
        let db = SqliteTargetDatabase::in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL);
                 INSERT INTO products (name, price) VALUES ('tea', 1.5), ('coffee', 2.0);",
            )
            .unwrap();
        }

        let query = ReadOnlyQuery::parse_for(
            "SELECT name, price FROM products ORDER BY id",
            DatabaseType::SQLite,
        )
        .unwrap();
        let rows = db.query(&query, &[]).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), Some(&serde_json::json!("tea")));

        let tables = db.list_tables().await.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(
            tables[0].get("table_name"),
            Some(&serde_json::json!("products"))
        );

        let columns = db.get_table_columns("products").await.unwrap();
        assert_eq!(columns.len(), 3);

        let sample = db.sample_table("products", 1).await.unwrap();
        assert_eq!(sample.len(), 1);
    }

    #[tokio::test]
    async fn sqlite_target_db_preserves_text_values_that_look_like_json() {
        let db = SqliteTargetDatabase::in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE products (payload TEXT, code TEXT);
                 INSERT INTO products (payload, code) VALUES ('{\"kind\":\"tea\"}', '123');",
            )
            .unwrap();
        }

        let query =
            ReadOnlyQuery::parse_for("SELECT payload, code FROM products", DatabaseType::SQLite)
                .unwrap();
        let rows = db.query(&query, &[]).await.unwrap();

        assert_eq!(
            rows[0].get("payload"),
            Some(&serde_json::json!("{\"kind\":\"tea\"}"))
        );
        assert_eq!(rows[0].get("code"), Some(&serde_json::json!("123")));
    }

    #[tokio::test]
    async fn sqlite_target_db_timeout_interrupts_running_query() {
        let db = SqliteTargetDatabase::in_memory()
            .unwrap()
            .with_timeout(Duration::from_millis(10));
        let slow_query =
            ReadOnlyQuery::parse_for(slow_recursive_sum_sql(100_000_000), DatabaseType::SQLite)
                .unwrap();

        let err = db.query(&slow_query, &[]).await.unwrap_err();
        assert!(matches!(err, DbError::Timeout(_)));

        let quick_query =
            ReadOnlyQuery::parse_for("SELECT 1 AS value", DatabaseType::SQLite).unwrap();
        let rows = tokio::time::timeout(Duration::from_secs(1), db.query(&quick_query, &[]))
            .await
            .expect("follow-up query should not block behind timed-out work")
            .unwrap();
        assert_eq!(rows[0].get("value"), Some(&serde_json::json!(1)));
    }

    #[tokio::test]
    async fn sqlite_target_db_times_out_read_without_wedging_file_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.db");
        init_file_db(&path);

        let db = SqliteTargetDatabase::open(&path)
            .unwrap()
            .with_timeout(Duration::from_millis(10));
        let slow_query =
            ReadOnlyQuery::parse_for(slow_recursive_sum_sql(100_000_000), DatabaseType::SQLite)
                .unwrap();

        let err = db.query(&slow_query, &[]).await.unwrap_err();
        assert!(matches!(err, DbError::Timeout(_)));

        let quick_query = ReadOnlyQuery::parse_for(
            "SELECT COUNT(*) AS row_count FROM items",
            DatabaseType::SQLite,
        )
        .unwrap();
        let rows = db.query(&quick_query, &[]).await.unwrap();
        assert_eq!(rows[0].get("row_count"), Some(&serde_json::json!(0)));
    }
}
