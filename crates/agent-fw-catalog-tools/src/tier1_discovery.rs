//! Tier 1: Read-only query execution.
//!
//! This module exposes `execute_query`, the surface's validated read-only SQL
//! entry point. It is the only tier-1 path still consumed by the public
//! catalog surface.

use agent_fw_algebra::{QueryParam, ReadOnlyQuery, TargetDatabase};
use agent_fw_core::DatabaseType;
use serde::{Deserialize, Serialize};
use sqlparser::ast::{Query, SetExpr, Statement};
use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use tracing::instrument;

use crate::CatalogToolError;

// =============================================================================
// execute_query
// =============================================================================

/// Input for `execute_query` tool.
#[derive(Debug, Clone, Deserialize, agent_fw_tool_macro::ToolSchema)]
pub struct ExecuteQueryInput {
    /// SQL query to execute. Only SELECT queries, including WITH/CTE SELECTs,
    /// are allowed. Mutations (INSERT, UPDATE, DELETE, DDL) and unsupported
    /// read-only statements are rejected at the AST level via sqlparser.
    #[schema(description = "SQL query to execute (SELECT/WITH only; automatically limited)")]
    pub sql: String,

    /// Positional query parameters ($1, $2, ...).
    #[schema(description = "Positional query parameters ($1, $2, ...)")]
    pub params: Option<Vec<serde_json::Value>>,

    /// Maximum number of rows to return (default: 100, max: 1000).
    #[schema(description = "Maximum number of rows to return")]
    pub limit: Option<usize>,
}

/// Output from `execute_query` tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryOutput {
    /// Column names in result order.
    pub columns: Vec<String>,
    /// Result rows (each row is a JSON object keyed by column name).
    pub rows: Vec<serde_json::Value>,
    /// Number of rows returned.
    pub row_count: usize,
    /// Whether the result was truncated by the limit.
    pub truncated: bool,
}

fn prepare_limited_select_query(
    sql: &str,
    database_type: DatabaseType,
    limit: usize,
    mut params: Vec<QueryParam>,
) -> Result<(ReadOnlyQuery, Vec<QueryParam>), CatalogToolError> {
    let validated = ReadOnlyQuery::parse_for(sql, database_type)?;
    let statement = ensure_wrappable_select(validated.sql(), database_type)?;

    let fetch_limit = limit + 1;
    let limit_placeholder = limit_placeholder(database_type, params.len() + 1);
    params.push(QueryParam::Int(fetch_limit as i64));

    let statement_sql = statement.to_string();
    let inner_sql = strip_trailing_statement_separator(&statement_sql);
    let wrapped_sql =
        format!("SELECT * FROM ({inner_sql}) AS agent_fw_limited LIMIT {limit_placeholder}");
    let wrapped = ReadOnlyQuery::parse_for(wrapped_sql, database_type)?;
    Ok((wrapped, params))
}

fn ensure_wrappable_select(
    sql: &str,
    database_type: DatabaseType,
) -> Result<Statement, CatalogToolError> {
    let statement = parse_single_statement(sql, database_type)?;
    if let Statement::Query(ref query) = statement {
        if query_is_wrappable_select(query, database_type)? {
            return Ok(statement);
        }
    }

    Err(CatalogToolError::Validation(
        "Only safe SELECT queries can be automatically limited. Add an explicit LIMIT to a SELECT query or rewrite unsupported SQL before retrying.".to_string(),
    ))
}

fn parse_single_statement(
    sql: &str,
    database_type: DatabaseType,
) -> Result<Statement, CatalogToolError> {
    let statements = match database_type {
        DatabaseType::PostgreSQL => Parser::parse_sql(&PostgreSqlDialect {}, sql),
        DatabaseType::MySQL => Parser::parse_sql(&MySqlDialect {}, sql),
        DatabaseType::SQLite => Parser::parse_sql(&SQLiteDialect {}, sql),
    }
    .map_err(|e| {
        CatalogToolError::Database(agent_fw_algebra::DbError::InvalidQuery(format!(
            "SQL parse error: {e}"
        )))
    })?;

    if statements.len() != 1 {
        return Err(CatalogToolError::Database(
            agent_fw_algebra::DbError::InvalidQuery("Multiple statements not allowed".into()),
        ));
    }

    Ok(statements
        .into_iter()
        .next()
        .expect("one statement checked"))
}

fn query_is_wrappable_select(
    query: &Query,
    database_type: DatabaseType,
) -> Result<bool, CatalogToolError> {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            let cte_sql = cte.query.to_string();
            ReadOnlyQuery::parse_for(cte_sql, database_type)?;
            if !query_is_wrappable_select(&cte.query, database_type)? {
                return Ok(false);
            }
        }
    }

    set_expr_is_wrappable_select(&query.body, database_type)
}

fn set_expr_is_wrappable_select(
    expr: &SetExpr,
    database_type: DatabaseType,
) -> Result<bool, CatalogToolError> {
    match expr {
        SetExpr::Select(_) => Ok(true),
        SetExpr::Query(query) => query_is_wrappable_select(query, database_type),
        SetExpr::SetOperation { left, right, .. } => {
            Ok(set_expr_is_wrappable_select(left, database_type)?
                && set_expr_is_wrappable_select(right, database_type)?)
        }
        SetExpr::Values(_) | SetExpr::Table(_) | SetExpr::Insert(_) | SetExpr::Update(_) => {
            Ok(false)
        }
    }
}

fn limit_placeholder(database_type: DatabaseType, position: usize) -> String {
    match database_type {
        DatabaseType::PostgreSQL => format!("${position}"),
        DatabaseType::MySQL | DatabaseType::SQLite => "?".to_string(),
    }
}

fn strip_trailing_statement_separator(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim_end()
}

/// Execute a validated read-only SQL query against the target database.
///
/// Uses `ReadOnlyQuery::parse_for()` for AST-level validation via sqlparser.
/// Only SELECT queries, including WITH/CTE SELECTs, are allowed because the
/// tool must be able to push its row limit into SQL before execution.
/// Mutations and unsupported read-only statements are rejected before reaching
/// the database.
///
/// # Safety
///
/// - SQL is parsed and validated by sqlparser (PostgreSQL dialect)
/// - Only read-only statements reach the database
/// - Parameters are bound positionally (no string interpolation)
/// - Results are capped by `limit` (default 100, max 1000)
#[instrument(skip(target_db))]
pub async fn execute_query(
    target_db: Option<&dyn TargetDatabase>,
    input: ExecuteQueryInput,
) -> Result<ExecuteQueryOutput, CatalogToolError> {
    let db = target_db.ok_or_else(|| {
        CatalogToolError::Validation(
            "No TargetDatabase available. Connect a data source first.".into(),
        )
    })?;

    // Convert JSON params to QueryParam
    let params: Vec<QueryParam> = input
        .params
        .unwrap_or_default()
        .into_iter()
        .map(QueryParam::from_json_value)
        .collect();

    let limit = input.limit.unwrap_or(100).min(1000);
    let (validated, params) =
        prepare_limited_select_query(&input.sql, db.database_type(), limit, params)?;

    let rows = db.query(&validated, &params).await?;
    let truncated = rows.len() > limit;
    let rows: Vec<_> = rows.into_iter().take(limit).collect();

    // Extract column names from first row (or empty)
    let columns = rows
        .first()
        .map(|r| r.columns().to_vec())
        .unwrap_or_default();

    // Convert DbRows to JSON objects
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let map: serde_json::Map<String, serde_json::Value> = row
                .as_map()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            serde_json::Value::Object(map)
        })
        .collect();

    let row_count = json_rows.len();

    Ok(ExecuteQueryOutput {
        columns,
        rows: json_rows,
        row_count,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::target_db::{DbError, DbRow};
    use agent_fw_tool::ToolSchema;
    use std::sync::{Arc, Mutex};

    // ── Minimal mock TargetDatabase for testing pure tool functions ──
    // Tests against the algebra, not the interpreter.

    struct StubTargetDb {
        database_type: DatabaseType,
        rows: Vec<DbRow>,
        executed_queries: Arc<Mutex<Vec<(String, Vec<QueryParam>)>>>,
    }

    impl StubTargetDb {
        fn with_rows(rows: Vec<DbRow>) -> Self {
            Self {
                database_type: DatabaseType::PostgreSQL,
                rows,
                executed_queries: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_type(mut self, database_type: DatabaseType) -> Self {
            self.database_type = database_type;
            self
        }

        fn empty() -> Self {
            Self::with_rows(vec![])
        }

        fn executed_queries(&self) -> Vec<(String, Vec<QueryParam>)> {
            self.executed_queries.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TargetDatabase for StubTargetDb {
        fn database_type(&self) -> DatabaseType {
            self.database_type
        }

        async fn query(
            &self,
            query: &ReadOnlyQuery,
            params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            self.executed_queries
                .lock()
                .unwrap()
                .push((query.sql().to_string(), params.to_vec()));

            let mut rows = self.rows.clone();
            if query.sql().contains("agent_fw_limited") {
                if let Some(QueryParam::Int(limit)) = params.last() {
                    rows.truncate(*limit as usize);
                }
            }
            Ok(rows)
        }

        async fn health_check(&self) -> Result<(), DbError> {
            Ok(())
        }

        async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![])
        }

        async fn get_table_columns(&self, _table_name: &str) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![])
        }

        async fn sample_table(
            &self,
            _table_name: &str,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, DbError> {
            Ok(vec![])
        }
    }

    // ── Schema tests ──

    #[test]
    fn execute_query_input_schema() {
        let schema = ExecuteQueryInput::json_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("sql")));
        assert!(schema["properties"]["params"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    // ── execute_query behavioral tests ──

    #[tokio::test]
    async fn execute_query_rejects_mutations_at_ast_level() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "DROP TABLE users".into(),
            params: None,
            limit: None,
        };
        let result = execute_query(Some(&db), input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CatalogToolError::Database(e) => {
                assert!(
                    e.to_string().contains("Only SELECT"),
                    "Expected read-only validation error, got: {e}"
                );
            }
            other => panic!("Expected Database error from ReadOnlyQuery, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_query_rejects_insert() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "INSERT INTO users VALUES (1, 'bob')".into(),
            params: None,
            limit: None,
        };
        assert!(execute_query(Some(&db), input).await.is_err());
    }

    #[tokio::test]
    async fn execute_query_rejects_multi_statement() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "SELECT 1; DELETE FROM users".into(),
            params: None,
            limit: None,
        };
        assert!(execute_query(Some(&db), input).await.is_err());
    }

    #[tokio::test]
    async fn execute_query_requires_target_db() {
        let input = ExecuteQueryInput {
            sql: "SELECT 1".into(),
            params: None,
            limit: None,
        };
        let result = execute_query(None, input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CatalogToolError::Validation(msg) => {
                assert!(msg.contains("No TargetDatabase"));
            }
            other => panic!("Expected Validation error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_query_returns_rows_from_target_db() {
        let db = StubTargetDb::with_rows(vec![
            DbRow::new(
                vec!["id".into(), "name".into()],
                vec![serde_json::json!(1), serde_json::json!("Alice")],
            ),
            DbRow::new(
                vec!["id".into(), "name".into()],
                vec![serde_json::json!(2), serde_json::json!("Bob")],
            ),
        ]);

        let input = ExecuteQueryInput {
            sql: "SELECT id, name FROM users".into(),
            params: None,
            limit: None,
        };
        let output = execute_query(Some(&db), input).await.unwrap();
        assert_eq!(output.row_count, 2);
        assert_eq!(output.columns, vec!["id", "name"]);
        assert!(!output.truncated);
        assert_eq!(output.rows[0]["name"], serde_json::json!("Alice"));
        assert_eq!(output.rows[1]["name"], serde_json::json!("Bob"));
    }

    #[tokio::test]
    async fn execute_query_truncates_at_limit() {
        // Build 5 rows
        let rows: Vec<DbRow> = (0..5)
            .map(|i| DbRow::new(vec!["n".into()], vec![serde_json::json!(i)]))
            .collect();
        let db = StubTargetDb::with_rows(rows);

        let input = ExecuteQueryInput {
            sql: "SELECT n FROM nums".into(),
            params: None,
            limit: Some(3),
        };
        let output = execute_query(Some(&db), input).await.unwrap();
        assert_eq!(output.row_count, 3);
        assert!(output.truncated);
    }

    #[tokio::test]
    async fn execute_query_does_not_mark_exact_limit_as_truncated() {
        let rows: Vec<DbRow> = (0..3)
            .map(|i| DbRow::new(vec!["n".into()], vec![serde_json::json!(i)]))
            .collect();
        let db = StubTargetDb::with_rows(rows);

        let input = ExecuteQueryInput {
            sql: "SELECT n FROM nums".into(),
            params: None,
            limit: Some(3),
        };
        let output = execute_query(Some(&db), input).await.unwrap();
        assert_eq!(output.row_count, 3);
        assert!(!output.truncated);
    }

    #[tokio::test]
    async fn execute_query_wraps_unlimited_select_with_sql_limit() {
        let rows: Vec<DbRow> = (0..5)
            .map(|i| DbRow::new(vec!["n".into()], vec![serde_json::json!(i)]))
            .collect();
        let db = StubTargetDb::with_rows(rows);

        let input = ExecuteQueryInput {
            sql: "SELECT n FROM nums ORDER BY n".into(),
            params: None,
            limit: Some(3),
        };
        let output = execute_query(Some(&db), input).await.unwrap();

        assert_eq!(output.row_count, 3);
        let executed = db.executed_queries();
        assert_eq!(executed.len(), 1);
        assert_eq!(
            executed[0].0,
            "SELECT * FROM (SELECT n FROM nums ORDER BY n) AS agent_fw_limited LIMIT $1"
        );
        assert!(matches!(executed[0].1.as_slice(), [QueryParam::Int(4)]));
    }

    #[tokio::test]
    async fn execute_query_uses_next_param_for_wrapped_limit() {
        let db = StubTargetDb::with_rows(vec![DbRow::new(
            vec!["id".into()],
            vec![serde_json::json!(7)],
        )]);

        let input = ExecuteQueryInput {
            sql: "SELECT id FROM users WHERE id = $1".into(),
            params: Some(vec![serde_json::json!(7)]),
            limit: Some(10),
        };
        let output = execute_query(Some(&db), input).await.unwrap();

        assert_eq!(output.row_count, 1);
        let executed = db.executed_queries();
        assert_eq!(
            executed[0].0,
            "SELECT * FROM (SELECT id FROM users WHERE id = $1) AS agent_fw_limited LIMIT $2"
        );
        assert!(matches!(
            executed[0].1.as_slice(),
            [QueryParam::Int(7), QueryParam::Int(11)]
        ));
    }

    #[tokio::test]
    async fn execute_query_uses_question_mark_limit_placeholder_for_sqlite() {
        let db = StubTargetDb::with_rows(vec![DbRow::new(
            vec!["id".into()],
            vec![serde_json::json!(7)],
        )])
        .with_type(DatabaseType::SQLite);

        let input = ExecuteQueryInput {
            sql: "SELECT id FROM users WHERE id = ?".into(),
            params: Some(vec![serde_json::json!(7)]),
            limit: Some(10),
        };
        let output = execute_query(Some(&db), input).await.unwrap();

        assert_eq!(output.row_count, 1);
        let executed = db.executed_queries();
        assert_eq!(
            executed[0].0,
            "SELECT * FROM (SELECT id FROM users WHERE id = ?) AS agent_fw_limited LIMIT ?"
        );
        assert!(matches!(
            executed[0].1.as_slice(),
            [QueryParam::Int(7), QueryParam::Int(11)]
        ));
    }

    #[tokio::test]
    async fn execute_query_uses_question_mark_limit_placeholder_for_mysql() {
        let db = StubTargetDb::empty().with_type(DatabaseType::MySQL);

        let input = ExecuteQueryInput {
            sql: "SELECT id FROM users".into(),
            params: None,
            limit: Some(10),
        };
        let output = execute_query(Some(&db), input).await.unwrap();

        assert_eq!(output.row_count, 0);
        let executed = db.executed_queries();
        assert_eq!(
            executed[0].0,
            "SELECT * FROM (SELECT id FROM users) AS agent_fw_limited LIMIT ?"
        );
        assert!(matches!(executed[0].1.as_slice(), [QueryParam::Int(11)]));
    }

    #[tokio::test]
    async fn execute_query_strips_trailing_line_comment_before_wrapping_limit() {
        let db = StubTargetDb::empty();

        let input = ExecuteQueryInput {
            sql: "SELECT id FROM users -- caller note".into(),
            params: None,
            limit: Some(10),
        };
        execute_query(Some(&db), input).await.unwrap();

        let executed = db.executed_queries();
        assert_eq!(
            executed[0].0,
            "SELECT * FROM (SELECT id FROM users) AS agent_fw_limited LIMIT $1"
        );
    }

    #[tokio::test]
    async fn execute_query_limit_caps_at_1000() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "SELECT 1".into(),
            params: None,
            limit: Some(9999),
        };
        // Should not panic — limit is clamped internally
        let output = execute_query(Some(&db), input).await.unwrap();
        assert_eq!(output.row_count, 0);
        assert!(!output.truncated);
    }

    #[tokio::test]
    async fn execute_query_accepts_cte() {
        let db = StubTargetDb::with_rows(vec![DbRow::new(
            vec!["x".into()],
            vec![serde_json::json!(1)],
        )]);

        let input = ExecuteQueryInput {
            sql: "WITH cte AS (SELECT 1 AS x) SELECT * FROM cte".into(),
            params: None,
            limit: None,
        };
        let output = execute_query(Some(&db), input).await.unwrap();
        assert_eq!(output.row_count, 1);
    }

    #[tokio::test]
    async fn execute_query_rejects_locking_clause_inside_cte() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "WITH locked AS (SELECT * FROM users FOR UPDATE) SELECT * FROM locked".into(),
            params: None,
            limit: Some(10),
        };
        let result = execute_query(Some(&db), input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CatalogToolError::Database(e) => {
                assert!(
                    e.to_string().contains("Locking clauses not allowed"),
                    "expected locking validation error, got: {e}"
                );
            }
            other => panic!("Expected Database error from ReadOnlyQuery, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_query_rejects_explain_because_it_cannot_be_sql_limited() {
        let db = StubTargetDb::empty();
        let input = ExecuteQueryInput {
            sql: "EXPLAIN SELECT * FROM users".into(),
            params: None,
            limit: None,
        };
        let result = execute_query(Some(&db), input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CatalogToolError::Validation(msg) => {
                assert!(msg.contains("LIMIT"), "expected LIMIT guidance, got: {msg}");
            }
            other => panic!("Expected Validation error, got: {other:?}"),
        }
    }
}
