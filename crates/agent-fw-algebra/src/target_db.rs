//! TargetDatabase — async read-only access to a customer database.
//!
//! This trait abstracts SQL database access for introspection, profiling,
//! and query execution. Implementations must be object-safe for dynamic dispatch.
//!
//! # Laws
//!
//! L1 (Read-Only Safety): `validate_read_only(sql)` rejects mutations
//! L2 (Determinism): Same query + params → same rows (within transaction)
//! L3 (Timeout): Queries that exceed `timeout()` return `DbError::Timeout`
//! L4 (Column Fidelity): `DbRow.columns()` matches the SQL result columns

use agent_fw_core::DatabaseType;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Default query timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Database query error.
#[derive(Debug, Clone, Error)]
pub enum DbError {
    #[error("Query validation failed: {0}")]
    InvalidQuery(String),

    #[error("Query execution failed: {0}")]
    Execution(String),

    #[error("Query timed out after {0:?}")]
    Timeout(Duration),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),
}

/// Query parameter for parameterized SQL.
#[derive(Debug, Clone)]
pub enum QueryParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Json(serde_json::Value),
}

impl From<&str> for QueryParam {
    fn from(s: &str) -> Self {
        QueryParam::Text(s.to_string())
    }
}

impl From<String> for QueryParam {
    fn from(s: String) -> Self {
        QueryParam::Text(s)
    }
}

impl From<i32> for QueryParam {
    fn from(v: i32) -> Self {
        QueryParam::Int(v as i64)
    }
}

impl From<i64> for QueryParam {
    fn from(v: i64) -> Self {
        QueryParam::Int(v)
    }
}

impl From<f64> for QueryParam {
    fn from(v: f64) -> Self {
        QueryParam::Float(v)
    }
}

impl From<bool> for QueryParam {
    fn from(v: bool) -> Self {
        QueryParam::Bool(v)
    }
}

impl From<serde_json::Value> for QueryParam {
    fn from(v: serde_json::Value) -> Self {
        QueryParam::Json(v)
    }
}

impl From<rust_decimal::Decimal> for QueryParam {
    fn from(v: rust_decimal::Decimal) -> Self {
        QueryParam::Text(v.to_string())
    }
}

impl QueryParam {
    /// Convert a JSON value to the most specific `QueryParam` variant.
    ///
    /// Unlike `From<serde_json::Value>` (which always produces `Json`),
    /// this performs discriminated conversion:
    ///
    /// - `null`   → `Null`
    /// - `bool`   → `Bool`
    /// - `number` → `Int` or `Float`
    /// - `string` → `Text`
    /// - object/array → `Json`
    ///
    /// This is the canonical conversion for LLM tool inputs where JSON
    /// values must be bound as typed SQL parameters.
    pub fn from_json_value(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => QueryParam::Null,
            serde_json::Value::Bool(b) => QueryParam::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    QueryParam::Int(i)
                } else if let Some(f) = n.as_f64() {
                    QueryParam::Float(f)
                } else {
                    QueryParam::Text(n.to_string())
                }
            }
            serde_json::Value::String(s) => QueryParam::Text(s),
            other => QueryParam::Json(other),
        }
    }
}

/// A row from a database query result.
#[derive(Debug, Clone, Default)]
pub struct DbRow {
    columns: Vec<String>,
    values: HashMap<String, serde_json::Value>,
}

impl DbRow {
    /// Create a row from column names and corresponding values.
    pub fn new(columns: Vec<String>, values: Vec<serde_json::Value>) -> Self {
        let map = columns
            .iter()
            .zip(values.into_iter())
            .map(|(k, v)| (k.clone(), v))
            .collect();
        Self {
            columns,
            values: map,
        }
    }

    /// Get a value by column name.
    pub fn get(&self, column: &str) -> Option<&serde_json::Value> {
        self.values.get(column)
    }

    /// Get a typed value by column name.
    pub fn get_as<T: DeserializeOwned>(&self, column: &str) -> Result<Option<T>, DbError> {
        match self.values.get(column) {
            None => Ok(None),
            Some(v) => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|e| DbError::Deserialization(format!("{column}: {e}"))),
        }
    }

    /// Get the column names in order.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Check if a column exists.
    pub fn has_column(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// Get the underlying map.
    pub fn as_map(&self) -> &HashMap<String, serde_json::Value> {
        &self.values
    }
}

/// A validated read-only SQL query.
///
/// Smart constructor parses the SQL via `sqlparser` and validates it is a
/// read-only statement (SELECT, WITH, EXPLAIN, SHOW). Mutations are rejected
/// at construction time — the `TargetDatabase` trait surface never accepts
/// raw `&str` for SQL.
///
/// This is the same pattern as `DdlStatement`/`DmlStatement` in `WritableDatabase`:
/// parse once at the boundary, carry the validated string through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyQuery {
    sql: String,
}

impl ReadOnlyQuery {
    /// Parse and validate a read-only SQL query.
    ///
    /// Accepts: SELECT, WITH, EXPLAIN, SHOW.
    /// Rejects: INSERT, UPDATE, DELETE, DDL, multi-statement.
    pub fn parse(sql: impl Into<String>) -> Result<Self, DbError> {
        Self::parse_for(sql, DatabaseType::PostgreSQL)
    }

    /// Parse and validate a read-only SQL query for a specific backend dialect.
    pub fn parse_for(sql: impl Into<String>, database_type: DatabaseType) -> Result<Self, DbError> {
        let sql = sql.into();
        validate_read_only_for(&sql, database_type)?;
        Ok(Self { sql })
    }

    /// Access the validated SQL string.
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

impl std::fmt::Display for ReadOnlyQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sql)
    }
}

/// Async database access (read-only queries + schema discovery).
///
/// Implementations may target PostgreSQL, MySQL, SQLite, or any SQL database.
/// Domain-specific query methods (e.g. query_products) should be defined
/// as extension traits in the consuming application.
///
/// All query methods accept validated `ReadOnlyQuery` newtypes, not raw
/// `&str`. This ensures read-only safety at the type level.
#[async_trait]
pub trait TargetDatabase: Send + Sync {
    /// The concrete database engine behind this handle.
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    /// Execute a validated parameterized SQL query.
    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError>;

    /// Execute a validated query and return at most one row.
    async fn query_one(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Option<DbRow>, DbError> {
        let rows = self.query(query, params).await?;
        Ok(rows.into_iter().next())
    }

    /// Check that the database connection is healthy.
    async fn health_check(&self) -> Result<(), DbError>;

    /// Query timeout (implementations may override).
    fn timeout(&self) -> Duration {
        DEFAULT_TIMEOUT
    }

    /// List all tables visible to the connection.
    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError>;

    /// Get column metadata for a table.
    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError>;

    /// Sample rows from a table (for profiling / enrichment).
    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError>;
}

#[async_trait]
impl<T: TargetDatabase + ?Sized> TargetDatabase for &T {
    fn database_type(&self) -> DatabaseType {
        (**self).database_type()
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        (**self).query(query, params).await
    }

    async fn health_check(&self) -> Result<(), DbError> {
        (**self).health_check().await
    }

    fn timeout(&self) -> Duration {
        (**self).timeout()
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        (**self).list_tables().await
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        (**self).get_table_columns(table_name).await
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        (**self).sample_table(table_name, limit).await
    }
}

#[async_trait]
impl<T: TargetDatabase + ?Sized> TargetDatabase for Arc<T> {
    fn database_type(&self) -> DatabaseType {
        self.as_ref().database_type()
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        self.as_ref().query(query, params).await
    }

    async fn health_check(&self) -> Result<(), DbError> {
        self.as_ref().health_check().await
    }

    fn timeout(&self) -> Duration {
        self.as_ref().timeout()
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        self.as_ref().list_tables().await
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        self.as_ref().get_table_columns(table_name).await
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        self.as_ref().sample_table(table_name, limit).await
    }
}

/// Extension trait for typed query results.
#[async_trait]
pub trait TargetDatabaseExt: TargetDatabase {
    /// Execute a validated query and deserialize each row into `T`.
    async fn query_as<T: DeserializeOwned + Send>(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<T>, DbError> {
        let rows = self.query(query, params).await?;
        rows.into_iter()
            .map(|row| {
                let val = serde_json::to_value(row.as_map())
                    .map_err(|e| DbError::Deserialization(e.to_string()))?;
                serde_json::from_value(val).map_err(|e| DbError::Deserialization(e.to_string()))
            })
            .collect()
    }
}

impl<T: TargetDatabase + ?Sized> TargetDatabaseExt for T {}

/// Validate that a SQL string is read-only (SELECT/WITH/EXPLAIN only).
///
/// Uses `sqlparser` for AST-level validation — parses the SQL and inspects
/// statement types rather than doing string matching. This prevents bypass
/// via subqueries, CTEs, or creative formatting.
///
/// Rejects multi-statement inputs (only one statement allowed).
pub fn validate_read_only(sql: &str) -> Result<(), DbError> {
    validate_read_only_for(sql, DatabaseType::PostgreSQL)
}

/// Validate a SQL string as read-only for a specific database dialect.
pub fn validate_read_only_for(sql: &str, database_type: DatabaseType) -> Result<(), DbError> {
    use sqlparser::ast::Statement;
    use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};
    use sqlparser::parser::Parser;

    let statements = match database_type {
        DatabaseType::PostgreSQL => Parser::parse_sql(&PostgreSqlDialect {}, sql),
        DatabaseType::MySQL => Parser::parse_sql(&MySqlDialect {}, sql),
        DatabaseType::SQLite => Parser::parse_sql(&SQLiteDialect {}, sql),
    }
    .map_err(|e| DbError::InvalidQuery(format!("SQL parse error: {e}")))?;

    if statements.is_empty() {
        return Err(DbError::InvalidQuery("Empty SQL statement".into()));
    }

    if statements.len() > 1 {
        return Err(DbError::InvalidQuery(
            "Multiple statements not allowed".into(),
        ));
    }

    match &statements[0] {
        Statement::Query(q) => check_query_safety(q),
        Statement::Explain { .. } | Statement::ExplainTable { .. } => Ok(()),
        // SHOW and SET (read-only introspection) are safe
        Statement::ShowVariable { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCollation { .. } => Ok(()),
        other => Err(DbError::InvalidQuery(format!(
            "Only SELECT/WITH/EXPLAIN/SHOW queries allowed, got: {}",
            statement_kind(other)
        ))),
    }
}

/// Verify a parsed `Query` has no side-effecting clauses.
///
/// Rejects `FOR UPDATE/SHARE/...` locking and `SELECT INTO` at any nesting depth.
/// Recurses into WITH (CTE) bodies, ORDER BY, LIMIT, LIMIT BY, OFFSET, and FETCH
/// — all of which can contain subqueries (and, for CTEs, write-bearing bodies).
fn check_query_safety(q: &sqlparser::ast::Query) -> Result<(), DbError> {
    // WITH (CTE) bodies: each CTE is itself a Query. In sqlparser 0.53 a
    // CTE-leading INSERT/UPDATE parses as `Cte.query.body == SetExpr::Insert/Update`
    // (a data-modifying CTE), which the outer `q.body` check never sees. Recurse
    // into every CTE with the same read-only rules so write-bearing CTEs — at any
    // nesting depth — are rejected, while read-only and recursive SELECT CTEs pass.
    if let Some(ref with) = q.with {
        for cte in &with.cte_tables {
            check_query_safety(&cte.query)?;
        }
    }
    if !q.locks.is_empty() {
        let lock_kinds: Vec<String> = q.locks.iter().map(|l| format!("{}", l.lock_type)).collect();
        return Err(DbError::InvalidQuery(format!(
            "Locking clauses not allowed in read-only queries: FOR {}",
            lock_kinds.join(", FOR ")
        )));
    }
    check_set_expr_safety(&q.body)?;
    // ORDER BY expressions
    if let Some(ref order_by) = q.order_by {
        for obe in &order_by.exprs {
            check_expr_safety(&obe.expr)?;
        }
    }
    // LIMIT (can be a scalar subquery)
    if let Some(ref limit) = q.limit {
        check_expr_safety(limit)?;
    }
    // LIMIT BY (ClickHouse)
    for expr in &q.limit_by {
        check_expr_safety(expr)?;
    }
    // OFFSET
    if let Some(ref offset) = q.offset {
        check_expr_safety(&offset.value)?;
    }
    // FETCH
    if let Some(ref fetch) = q.fetch {
        if let Some(ref qty) = fetch.quantity {
            check_expr_safety(qty)?;
        }
    }
    Ok(())
}

/// Recursively check a `SetExpr` tree for side-effecting clauses.
///
/// This match is exhaustive — sqlparser's `SetExpr` does not use `#[non_exhaustive]`,
/// so new variants cause compile errors.
fn check_set_expr_safety(expr: &sqlparser::ast::SetExpr) -> Result<(), DbError> {
    use sqlparser::ast::SetExpr;
    match expr {
        SetExpr::Select(select) => check_select_safety(select),
        SetExpr::Query(q) => check_query_safety(q),
        SetExpr::SetOperation { left, right, .. } => {
            check_set_expr_safety(left)?;
            check_set_expr_safety(right)
        }
        SetExpr::Values(_) | SetExpr::Table(_) => Ok(()),
        // Insert/Update inside SetExpr are already rejected by the top-level match
        SetExpr::Insert(_) | SetExpr::Update(_) => Err(DbError::InvalidQuery(
            "Mutation statements not allowed in read-only queries".into(),
        )),
    }
}

/// Check a `Select` node for side-effecting clauses, recursing into ALL
/// subquery-bearing positions: INTO, FROM, WHERE, HAVING, SELECT list, JOINs,
/// GROUP BY, CLUSTER/DISTRIBUTE/SORT BY, PREWHERE, QUALIFY, LATERAL VIEW,
/// CONNECT BY.
fn check_select_safety(select: &sqlparser::ast::Select) -> Result<(), DbError> {
    if select.into.is_some() {
        return Err(DbError::InvalidQuery(
            "SELECT INTO not allowed in read-only queries".into(),
        ));
    }
    // FROM clause: table factors and joins (via shared helper)
    for table_with_joins in &select.from {
        check_table_with_joins_safety(table_with_joins)?;
    }
    // WHERE clause
    if let Some(ref expr) = select.selection {
        check_expr_safety(expr)?;
    }
    // HAVING clause
    if let Some(ref expr) = select.having {
        check_expr_safety(expr)?;
    }
    // SELECT list: expressions in projections
    for item in &select.projection {
        match item {
            sqlparser::ast::SelectItem::UnnamedExpr(e)
            | sqlparser::ast::SelectItem::ExprWithAlias { expr: e, .. } => {
                check_expr_safety(e)?;
            }
            _ => {}
        }
    }
    // GROUP BY expressions
    if let sqlparser::ast::GroupByExpr::Expressions(exprs, _) = &select.group_by {
        for expr in exprs {
            check_expr_safety(expr)?;
        }
    }
    // Hive: CLUSTER BY, DISTRIBUTE BY, SORT BY
    for expr in &select.cluster_by {
        check_expr_safety(expr)?;
    }
    for expr in &select.distribute_by {
        check_expr_safety(expr)?;
    }
    for expr in &select.sort_by {
        check_expr_safety(expr)?;
    }
    // ClickHouse: PREWHERE
    if let Some(ref expr) = select.prewhere {
        check_expr_safety(expr)?;
    }
    // Snowflake: QUALIFY
    if let Some(ref expr) = select.qualify {
        check_expr_safety(expr)?;
    }
    // Hive: LATERAL VIEW
    for lv in &select.lateral_views {
        check_expr_safety(&lv.lateral_view)?;
    }
    // Oracle: CONNECT BY
    if let Some(ref cb) = select.connect_by {
        check_expr_safety(&cb.condition)?;
        for expr in &cb.relationships {
            check_expr_safety(expr)?;
        }
    }
    Ok(())
}

/// Recurse into table factors that can contain subqueries.
///
/// **Fail-closed**: all `TableFactor` variants are explicit. Unknown variants
/// are rejected, matching the `check_expr_safety` design.
fn check_table_factor_safety(tf: &sqlparser::ast::TableFactor) -> Result<(), DbError> {
    use sqlparser::ast::TableFactor;
    match tf {
        TableFactor::Derived { subquery, .. } => check_query_safety(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => check_table_with_joins_safety(table_with_joins),
        // Pivot wraps a Box<TableFactor> — recurse into inner table.
        // PivotValueSource can carry subqueries, expressions, or OrderByExprs.
        TableFactor::Pivot {
            table,
            value_source,
            default_on_null,
            aggregate_functions,
            ..
        } => {
            check_table_factor_safety(table)?;
            match value_source {
                sqlparser::ast::PivotValueSource::Subquery(q) => check_query_safety(q)?,
                sqlparser::ast::PivotValueSource::List(exprs) => {
                    for ewf in exprs {
                        check_expr_safety(&ewf.expr)?;
                    }
                }
                sqlparser::ast::PivotValueSource::Any(order_exprs) => {
                    for obe in order_exprs {
                        check_expr_safety(&obe.expr)?;
                    }
                }
            }
            if let Some(expr) = default_on_null {
                check_expr_safety(expr)?;
            }
            for ewf in aggregate_functions {
                check_expr_safety(&ewf.expr)?;
            }
            Ok(())
        }
        // Unpivot wraps a Box<TableFactor>
        TableFactor::Unpivot { table, .. } => check_table_factor_safety(table),
        // MatchRecognize wraps a Box<TableFactor> + contains expressions in
        // partition_by, order_by, measures, and symbol definitions.
        TableFactor::MatchRecognize {
            table,
            partition_by,
            order_by,
            measures,
            symbols,
            ..
        } => {
            check_table_factor_safety(table)?;
            for expr in partition_by {
                check_expr_safety(expr)?;
            }
            for obe in order_by {
                check_expr_safety(&obe.expr)?;
            }
            for m in measures {
                check_expr_safety(&m.expr)?;
            }
            for s in symbols {
                check_expr_safety(&s.definition)?;
            }
            Ok(())
        }
        // TableFunction contains an Expr (e.g. generate_series(1, 10))
        TableFactor::TableFunction { expr, .. } => check_expr_safety(expr),
        // Function args can contain expressions
        TableFactor::Function { args, .. } => {
            for arg in args {
                check_function_arg_safety(arg)?;
            }
            Ok(())
        }
        // UNNEST contains expressions
        TableFactor::UNNEST { array_exprs, .. } => {
            for expr in array_exprs {
                check_expr_safety(expr)?;
            }
            Ok(())
        }
        // JsonTable / OpenJsonTable contain expressions
        TableFactor::JsonTable { json_expr, .. } => check_expr_safety(json_expr),
        TableFactor::OpenJsonTable { json_expr, .. } => check_expr_safety(json_expr),
        // Table: the common leaf variant. with_hints and json_path can carry exprs.
        TableFactor::Table {
            with_hints,
            json_path,
            ..
        } => {
            for expr in with_hints {
                check_expr_safety(expr)?;
            }
            if let Some(ref jp) = json_path {
                check_json_path_safety(jp)?;
            }
            Ok(())
        }
        // Fail-closed: reject unknown TableFactor variants.
        #[allow(unreachable_patterns)]
        other => Err(DbError::InvalidQuery(format!(
            "Unsupported table factor in read-only validation: {}",
            other
        ))),
    }
}

/// Shared helper: recurse into a `TableWithJoins` (used by both
/// `check_select_safety` and `check_table_factor_safety`).
fn check_table_with_joins_safety(twj: &sqlparser::ast::TableWithJoins) -> Result<(), DbError> {
    check_table_factor_safety(&twj.relation)?;
    for join in &twj.joins {
        check_table_factor_safety(&join.relation)?;
        if let Some(constraint) = join_constraint(&join.join_operator) {
            if let sqlparser::ast::JoinConstraint::On(expr) = constraint {
                check_expr_safety(expr)?;
            }
        }
        if let sqlparser::ast::JoinOperator::AsOf {
            match_condition, ..
        } = &join.join_operator
        {
            check_expr_safety(match_condition)?;
        }
    }
    Ok(())
}

/// Extract the `JoinConstraint` from a `JoinOperator`, if present.
fn join_constraint(op: &sqlparser::ast::JoinOperator) -> Option<&sqlparser::ast::JoinConstraint> {
    use sqlparser::ast::JoinOperator::*;
    match op {
        Inner(c) | LeftOuter(c) | RightOuter(c) | FullOuter(c) | Semi(c) | LeftSemi(c)
        | RightSemi(c) | Anti(c) | LeftAnti(c) | RightAnti(c) => Some(c),
        AsOf { constraint, .. } => Some(constraint),
        CrossJoin | CrossApply | OuterApply => None,
    }
}

/// Walk an expression tree, recursing into all subquery- and expression-bearing
/// nodes. **Fail-closed**: unknown `Expr` variants are rejected rather than
/// silently approved, so a new sqlparser variant carrying a hidden `Box<Query>`
/// cannot bypass validation.
///
/// Organized into three groups:
/// 1. Subquery-bearing — recurse via `check_query_safety`
/// 2. Expression-bearing — recurse via `check_expr_safety`
/// 3. Known leaf variants — no subexpressions, safe
fn check_expr_safety(expr: &sqlparser::ast::Expr) -> Result<(), DbError> {
    use sqlparser::ast::Expr;
    match expr {
        // ── Group 1: Subquery-bearing ──────────────────────────────────
        Expr::Subquery(q) => check_query_safety(q),
        Expr::Exists { subquery, .. } => check_query_safety(subquery),
        Expr::InSubquery { expr, subquery, .. } => {
            check_expr_safety(expr)?;
            check_query_safety(subquery)
        }

        // ── Group 2: Expression-bearing (recurse children) ─────────────
        Expr::BinaryOp { left, right, .. }
        | Expr::Like {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::ILike {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::SimilarTo {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::RLike {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::AnyOp { left, right, .. }
        | Expr::AllOp { left, right, .. }
        | Expr::IsDistinctFrom(left, right)
        | Expr::IsNotDistinctFrom(left, right) => {
            check_expr_safety(left)?;
            check_expr_safety(right)
        }
        Expr::UnaryOp { expr, .. }
        | Expr::Nested(expr)
        | Expr::Cast { expr, .. }
        | Expr::Collate { expr, .. }
        | Expr::Named { expr, .. }
        | Expr::AtTimeZone {
            timestamp: expr, ..
        }
        | Expr::Extract { expr, .. }
        | Expr::Ceil { expr, .. }
        | Expr::Floor { expr, .. }
        | Expr::Prior(expr)
        | Expr::OuterJoin(expr) => check_expr_safety(expr),
        Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::IsTrue(e)
        | Expr::IsFalse(e)
        | Expr::IsNotTrue(e)
        | Expr::IsNotFalse(e)
        | Expr::IsUnknown(e)
        | Expr::IsNotUnknown(e) => check_expr_safety(e),
        Expr::Between {
            expr, low, high, ..
        } => {
            check_expr_safety(expr)?;
            check_expr_safety(low)?;
            check_expr_safety(high)
        }
        Expr::Position { expr, r#in } => {
            check_expr_safety(expr)?;
            check_expr_safety(r#in)
        }
        Expr::Overlay {
            expr,
            overlay_what,
            overlay_from,
            overlay_for,
        } => {
            check_expr_safety(expr)?;
            check_expr_safety(overlay_what)?;
            check_expr_safety(overlay_from)?;
            if let Some(e) = overlay_for {
                check_expr_safety(e)?;
            }
            Ok(())
        }
        Expr::Substring {
            expr,
            substring_from,
            substring_for,
            ..
        } => {
            check_expr_safety(expr)?;
            if let Some(e) = substring_from {
                check_expr_safety(e)?;
            }
            if let Some(e) = substring_for {
                check_expr_safety(e)?;
            }
            Ok(())
        }
        Expr::Trim {
            expr,
            trim_what,
            trim_characters,
            ..
        } => {
            check_expr_safety(expr)?;
            if let Some(e) = trim_what {
                check_expr_safety(e)?;
            }
            if let Some(chars) = trim_characters {
                for e in chars {
                    check_expr_safety(e)?;
                }
            }
            Ok(())
        }
        Expr::Convert { expr, styles, .. } => {
            check_expr_safety(expr)?;
            for s in styles {
                check_expr_safety(s)?;
            }
            Ok(())
        }
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            if let Some(op) = operand {
                check_expr_safety(op)?;
            }
            for cond in conditions {
                check_expr_safety(cond)?;
            }
            for res in results {
                check_expr_safety(res)?;
            }
            if let Some(el) = else_result {
                check_expr_safety(el)?;
            }
            Ok(())
        }
        Expr::InList { expr, list, .. } => {
            check_expr_safety(expr)?;
            for e in list {
                check_expr_safety(e)?;
            }
            Ok(())
        }
        Expr::InUnnest {
            expr, array_expr, ..
        } => {
            check_expr_safety(expr)?;
            check_expr_safety(array_expr)
        }
        Expr::Function(f) => check_function_safety(f),
        Expr::Method(m) => {
            check_expr_safety(&m.expr)?;
            for func in &m.method_chain {
                check_function_safety(func)?;
            }
            Ok(())
        }
        Expr::JsonAccess { value, path } => {
            check_expr_safety(value)?;
            check_json_path_safety(path)
        }
        Expr::CompositeAccess { expr, .. } => check_expr_safety(expr),
        Expr::MapAccess { column, keys } => {
            check_expr_safety(column)?;
            for key in keys {
                check_expr_safety(&key.key)?;
            }
            Ok(())
        }
        Expr::Subscript { expr, subscript } => {
            check_expr_safety(expr)?;
            check_subscript_safety(subscript)
        }
        Expr::Tuple(exprs)
        | Expr::Struct { values: exprs, .. }
        | Expr::Array(sqlparser::ast::Array { elem: exprs, .. }) => {
            for e in exprs {
                check_expr_safety(e)?;
            }
            Ok(())
        }
        Expr::GroupingSets(groups) | Expr::Cube(groups) | Expr::Rollup(groups) => {
            for group in groups {
                for e in group {
                    check_expr_safety(e)?;
                }
            }
            Ok(())
        }
        Expr::Dictionary(fields) => {
            for f in fields {
                check_expr_safety(&f.value)?;
            }
            Ok(())
        }
        Expr::Map(map) => {
            for entry in &map.entries {
                check_expr_safety(&entry.key)?;
                check_expr_safety(&entry.value)?;
            }
            Ok(())
        }
        Expr::Interval(interval) => check_expr_safety(&interval.value),
        Expr::Lambda(lf) => check_expr_safety(&lf.body),

        // ── Group 3: Known leaf variants (no subexpressions) ───────────
        Expr::Identifier(_)
        | Expr::CompoundIdentifier(_)
        | Expr::Value(_)
        | Expr::IntroducedString { .. }
        | Expr::TypedString { .. }
        | Expr::Wildcard(_)
        | Expr::QualifiedWildcard(_, _)
        | Expr::MatchAgainst { .. } => Ok(()),

        // ── Fail-closed: reject unknown Expr variants ──────────────────
        // If sqlparser adds a new variant, it hits this arm and we get a
        // clear error instead of silently approving a potential subquery.
        #[allow(unreachable_patterns)]
        other => Err(DbError::InvalidQuery(format!(
            "Unsupported expression in read-only validation: {}",
            other
        ))),
    }
}

/// Check a `Function` node for dangerous names, then subqueries in its
/// arguments, parameters, and filter.
fn check_function_safety(f: &sqlparser::ast::Function) -> Result<(), DbError> {
    if let Some(name) = unqualified_function_name(f) {
        if is_dangerous_function(&name) {
            return Err(DbError::InvalidQuery(format!(
                "Function '{name}' is not permitted in read-only queries"
            )));
        }
    }
    check_function_arguments_safety(&f.args)?;
    check_function_arguments_safety(&f.parameters)?;
    if let Some(ref filter) = f.filter {
        check_expr_safety(filter)?;
    }
    Ok(())
}

/// Return the unqualified (last path segment), lower-cased function name.
///
/// PostgreSQL folds unquoted identifiers to lower case, so comparing on the
/// lower-cased value matches both `PG_READ_FILE(...)` and `pg_read_file(...)`.
/// A quoted mixed-case identifier would not resolve to a dangerous built-in
/// anyway, so this comparison is correct without needing to preserve case.
fn unqualified_function_name(f: &sqlparser::ast::Function) -> Option<String> {
    f.name
        .0
        .last()
        .map(|ident| ident.value.to_ascii_lowercase())
}

/// Defense-in-depth denylist of functions that can read files, list/import/
/// export large objects, open outbound connections, execute server-side
/// programs, render data to XML, or otherwise escape the read-only contract.
///
/// This is layered *in front of* the exhaustive AST recursion that follows.
/// It is intentionally narrow: it targets names that no legitimate analytics
/// read query uses (verified: none appear in any test, eval fixture, example,
/// or runtime-generated SQL in this repository). It is **not** the primary
/// control — a read-only database role is the real fix — but it closes the
/// obvious exfiltration/SSRF primitives (`pg_read_file`, `dblink`,
/// `lo_import`, `pg_execute_server_program`, …) until that lands.
fn is_dangerous_function(name: &str) -> bool {
    // Prefix families: each is a coherent set of dangerous built-ins/extensions.
    const DANGEROUS_PREFIXES: &[&str] = &[
        "pg_read_", // pg_read_file, pg_read_binary_file, pg_read_dir
        "pg_ls_",   // pg_ls_dir, pg_ls_logdir, pg_ls_waldir, pg_ls_archive_statusdir
        "lo_",      // lo_import, lo_export, lo_create (large objects)
        "dblink",   // dblink, dblink_connect, dblink_exec, dblink_send_query, …
    ];
    // Exact names that do not share a clean prefix.
    const DANGEROUS_EXACT: &[&str] = &[
        "pg_execute_server_program",
        "pg_stat_file",
        "pg_sleep",
        "pg_terminate_backend",
        "pg_reload_conf",
        "query_to_xml",
        "query_to_xml_and_xmlschema",
        "database_to_xml",
        "database_to_xml_and_xmlschema",
        "table_to_xml",
        "table_to_xml_and_xmlschema",
        "cursor_to_xml",
        "xml_is_well_formed",
    ];

    DANGEROUS_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || DANGEROUS_EXACT.iter().any(|exact| *exact == name)
}

/// Check `FunctionArguments` for subqueries and expressions.
fn check_function_arguments_safety(
    args: &sqlparser::ast::FunctionArguments,
) -> Result<(), DbError> {
    match args {
        sqlparser::ast::FunctionArguments::List(arg_list) => {
            for arg in &arg_list.args {
                check_function_arg_safety(arg)?;
            }
            Ok(())
        }
        sqlparser::ast::FunctionArguments::Subquery(q) => check_query_safety(q),
        sqlparser::ast::FunctionArguments::None => Ok(()),
    }
}

/// Check a single `FunctionArg` for subqueries and expressions.
fn check_function_arg_safety(arg: &sqlparser::ast::FunctionArg) -> Result<(), DbError> {
    match arg {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(e))
        | sqlparser::ast::FunctionArg::Named {
            arg: sqlparser::ast::FunctionArgExpr::Expr(e),
            ..
        } => check_expr_safety(e),
        _ => Ok(()),
    }
}

/// Recurse into a `Subscript` (array index or slice).
fn check_subscript_safety(sub: &sqlparser::ast::Subscript) -> Result<(), DbError> {
    match sub {
        sqlparser::ast::Subscript::Index { index } => check_expr_safety(index),
        sqlparser::ast::Subscript::Slice {
            lower_bound,
            upper_bound,
            stride,
        } => {
            if let Some(e) = lower_bound {
                check_expr_safety(e)?;
            }
            if let Some(e) = upper_bound {
                check_expr_safety(e)?;
            }
            if let Some(e) = stride {
                check_expr_safety(e)?;
            }
            Ok(())
        }
    }
}

/// Recurse into a `JsonPath` — bracket keys can contain expressions.
fn check_json_path_safety(path: &sqlparser::ast::JsonPath) -> Result<(), DbError> {
    for elem in &path.path {
        if let sqlparser::ast::JsonPathElem::Bracket { key } = elem {
            check_expr_safety(key)?;
        }
    }
    Ok(())
}

// ─── SQL Escaping ─────────────────────────────────────────────────────────

/// Escape a SQL identifier (column/table name) for use in double-quoted context.
/// Doubles any embedded double-quotes per PostgreSQL identifier rules.
pub fn escape_identifier(name: &str) -> String {
    name.replace('"', "\"\"")
}

/// Escape a SQL string literal for use in single-quoted context.
/// Doubles any embedded single-quotes per PostgreSQL literal rules.
pub fn escape_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// Human-readable name for a SQL statement kind (for error messages).
///
/// Used by both `validate_read_only` (target_db) and smart constructors (writable_db)
/// to produce consistent error messages across the crate.
pub(crate) fn statement_kind(stmt: &sqlparser::ast::Statement) -> &'static str {
    use sqlparser::ast::Statement;
    match stmt {
        Statement::Query(_) => "SELECT",
        Statement::Explain { .. } => "EXPLAIN",
        Statement::Insert(_) => "INSERT",
        Statement::Update { .. } => "UPDATE",
        Statement::Delete(_) => "DELETE",
        Statement::Drop { .. } => "DROP",
        Statement::CreateTable { .. } => "CREATE TABLE",
        Statement::CreateView { .. } => "CREATE VIEW",
        Statement::CreateIndex(_) => "CREATE INDEX",
        Statement::AlterTable { .. } => "ALTER TABLE",
        Statement::Truncate { .. } => "TRUNCATE",
        Statement::Copy { .. } => "COPY",
        Statement::Grant { .. } => "GRANT",
        Statement::Revoke { .. } => "REVOKE",
        Statement::Call(_) => "CALL",
        Statement::SetVariable { .. } => "SET",
        Statement::StartTransaction { .. } => "BEGIN",
        Statement::Commit { .. } => "COMMIT",
        Statement::Rollback { .. } => "ROLLBACK",
        Statement::Savepoint { .. } => "SAVEPOINT",
        // Explicit about all known mutation types so new sqlparser versions
        // surface as "disallowed statement" rather than silently matching.
        _ => "disallowed statement",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_row_basic_operations() {
        let row = DbRow::new(
            vec!["name".into(), "age".into()],
            vec![serde_json::json!("Alice"), serde_json::json!(30)],
        );
        assert_eq!(row.get("name"), Some(&serde_json::json!("Alice")));
        assert_eq!(row.get("age"), Some(&serde_json::json!(30)));
        assert_eq!(row.get("missing"), None);
        assert!(row.has_column("name"));
        assert!(!row.has_column("missing"));
        assert_eq!(row.columns(), &["name", "age"]);
    }

    #[test]
    fn db_row_get_as() {
        let row = DbRow::new(vec!["count".into()], vec![serde_json::json!(42)]);
        let val: Option<i64> = row.get_as("count").unwrap();
        assert_eq!(val, Some(42));
        let missing: Option<i64> = row.get_as("nope").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn query_param_conversions() {
        let _p1: QueryParam = "hello".into();
        let _p2: QueryParam = String::from("world").into();
        let _p3: QueryParam = 42i32.into();
        let _p4: QueryParam = 100i64.into();
        let _p5: QueryParam = 3.14f64.into();
        let _p6: QueryParam = true.into();
        let _p7: QueryParam = serde_json::json!({"key": "val"}).into();
    }

    #[test]
    fn query_param_from_decimal_preserves_precision() {
        let param = QueryParam::from(rust_decimal::Decimal::new(314, 2));
        assert!(matches!(param, QueryParam::Text(text) if text == "3.14"));
    }

    #[test]
    fn from_json_value_discriminated_conversion() {
        // null → Null
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(null)),
            QueryParam::Null
        ));

        // bool → Bool
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(true)),
            QueryParam::Bool(true)
        ));
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(false)),
            QueryParam::Bool(false)
        ));

        // integer → Int
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(42)),
            QueryParam::Int(42)
        ));
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(-1)),
            QueryParam::Int(-1)
        ));

        // float → Float
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!(3.14)),
            QueryParam::Float(f) if (f - 3.14).abs() < f64::EPSILON
        ));

        // string → Text
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!("hello")),
            QueryParam::Text(s) if s == "hello"
        ));

        // object → Json (not decomposed)
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!({"key": "val"})),
            QueryParam::Json(_)
        ));

        // array → Json (not decomposed)
        assert!(matches!(
            QueryParam::from_json_value(serde_json::json!([1, 2, 3])),
            QueryParam::Json(_)
        ));
    }

    #[test]
    fn validate_read_only_accepts_select() {
        assert!(validate_read_only("SELECT * FROM users").is_ok());
        assert!(validate_read_only("select id from users where x = 1").is_ok());
        assert!(validate_read_only("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
        assert!(validate_read_only("EXPLAIN SELECT * FROM users").is_ok());
    }

    #[test]
    fn validate_read_only_rejects_mutations() {
        assert!(validate_read_only("INSERT INTO users VALUES (1)").is_err());
        assert!(validate_read_only("UPDATE users SET name = 'x'").is_err());
        assert!(validate_read_only("DELETE FROM users").is_err());
        assert!(validate_read_only("DROP TABLE users").is_err());
        assert!(validate_read_only("CREATE TABLE foo (id INT)").is_err());
        assert!(validate_read_only("ALTER TABLE users ADD col INT").is_err());
        assert!(validate_read_only("TRUNCATE users").is_err());
    }

    #[test]
    fn validate_read_only_rejects_semicolons() {
        assert!(validate_read_only("SELECT 1; DROP TABLE users").is_err());
    }

    // --- ReadOnlyQuery smart constructor ---

    #[test]
    fn read_only_query_accepts_selects() {
        let q = ReadOnlyQuery::parse("SELECT * FROM users").unwrap();
        assert_eq!(q.sql(), "SELECT * FROM users");
        assert_eq!(q.to_string(), "SELECT * FROM users");
    }

    #[test]
    fn read_only_query_rejects_mutations() {
        assert!(ReadOnlyQuery::parse("INSERT INTO users VALUES (1)").is_err());
        assert!(ReadOnlyQuery::parse("DROP TABLE users").is_err());
    }

    #[test]
    fn read_only_query_display_matches_sql() {
        let q = ReadOnlyQuery::parse("SELECT 1").unwrap();
        assert_eq!(format!("{q}"), q.sql());
    }

    // --- QueryParam::from_json_value discriminated conversion ---

    #[test]
    fn from_json_value_null() {
        assert!(matches!(
            QueryParam::from_json_value(serde_json::Value::Null),
            QueryParam::Null
        ));
    }

    #[test]
    fn from_json_value_bool() {
        match QueryParam::from_json_value(serde_json::json!(true)) {
            QueryParam::Bool(true) => {}
            other => panic!("Expected Bool(true), got {other:?}"),
        }
        match QueryParam::from_json_value(serde_json::json!(false)) {
            QueryParam::Bool(false) => {}
            other => panic!("Expected Bool(false), got {other:?}"),
        }
    }

    #[test]
    fn from_json_value_int() {
        match QueryParam::from_json_value(serde_json::json!(42)) {
            QueryParam::Int(42) => {}
            other => panic!("Expected Int(42), got {other:?}"),
        }
        match QueryParam::from_json_value(serde_json::json!(-7)) {
            QueryParam::Int(-7) => {}
            other => panic!("Expected Int(-7), got {other:?}"),
        }
    }

    #[test]
    fn from_json_value_float() {
        match QueryParam::from_json_value(serde_json::json!(3.14)) {
            QueryParam::Float(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            other => panic!("Expected Float(3.14), got {other:?}"),
        }
    }

    #[test]
    fn from_json_value_string() {
        match QueryParam::from_json_value(serde_json::json!("hello")) {
            QueryParam::Text(s) => assert_eq!(s, "hello"),
            other => panic!("Expected Text(\"hello\"), got {other:?}"),
        }
    }

    #[test]
    fn from_json_value_object_becomes_json() {
        let obj = serde_json::json!({"key": "val"});
        match QueryParam::from_json_value(obj.clone()) {
            QueryParam::Json(v) => assert_eq!(v, obj),
            other => panic!("Expected Json, got {other:?}"),
        }
    }

    #[test]
    fn from_json_value_array_becomes_json() {
        let arr = serde_json::json!([1, 2, 3]);
        match QueryParam::from_json_value(arr.clone()) {
            QueryParam::Json(v) => assert_eq!(v, arr),
            other => panic!("Expected Json, got {other:?}"),
        }
    }

    /// Law: from_json_value is a refinement of From<serde_json::Value>.
    /// For non-primitive JSON (objects, arrays), both produce Json variant.
    #[test]
    fn from_json_value_agrees_with_from_for_complex_types() {
        let obj = serde_json::json!({"a": 1});
        let via_from: QueryParam = obj.clone().into();
        let via_method = QueryParam::from_json_value(obj);
        match (via_from, via_method) {
            (QueryParam::Json(a), QueryParam::Json(b)) => assert_eq!(a, b),
            _ => panic!("Both should produce Json for objects"),
        }
    }

    // --- Security: locking clause & SELECT INTO rejection ---

    #[test]
    fn validate_read_only_rejects_for_update() {
        let err = validate_read_only("SELECT * FROM users FOR UPDATE").unwrap_err();
        assert!(err.to_string().contains("Locking clauses"), "got: {err}");
    }

    #[test]
    fn validate_read_only_rejects_for_share() {
        let err = validate_read_only("SELECT * FROM users FOR SHARE").unwrap_err();
        assert!(err.to_string().contains("Locking clauses"), "got: {err}");
    }

    #[test]
    fn validate_read_only_rejects_select_into() {
        let err = validate_read_only("SELECT * INTO backup FROM users").unwrap_err();
        assert!(err.to_string().contains("SELECT INTO"), "got: {err}");
    }

    #[test]
    fn validate_read_only_rejects_for_update_skip_locked() {
        let err = validate_read_only("SELECT * FROM users FOR UPDATE SKIP LOCKED").unwrap_err();
        assert!(err.to_string().contains("Locking clauses"), "got: {err}");
    }

    #[test]
    fn validate_read_only_rejects_cte_with_for_update() {
        let err =
            validate_read_only("WITH cte AS (SELECT 1) SELECT * FROM cte FOR UPDATE").unwrap_err();
        assert!(err.to_string().contains("Locking clauses"), "got: {err}");
    }

    // --- Security: write-bearing CTE bodies (M8) ---

    #[test]
    fn validate_read_only_rejects_insert_cte() {
        // sqlparser 0.53 parses a CTE-leading INSERT as `Cte.query.body == SetExpr::Insert`.
        // The outer query body is a plain SELECT, so this only fails once CTE bodies are
        // traversed by check_query_safety.
        let err =
            validate_read_only("WITH x AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM x")
                .unwrap_err();
        assert!(
            err.to_string().contains("Mutation statements not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_read_only_rejects_update_cte() {
        let err = validate_read_only("WITH x AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM x")
            .unwrap_err();
        assert!(
            err.to_string().contains("Mutation statements not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_read_only_rejects_insert_in_second_cte() {
        // Proves every CTE is checked, not just the first.
        let err = validate_read_only(
            "WITH a AS (SELECT 1), b AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM a",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Mutation statements not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_read_only_rejects_nested_cte_insert() {
        // Proves transitive recursion: the INSERT is two WITH levels deep.
        let err = validate_read_only(
            "WITH a AS (WITH b AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM b) SELECT * FROM a",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Mutation statements not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_read_only_rejects_for_update_in_cte() {
        // Defense in depth: FOR UPDATE inside the CTE body (not the outer query)
        // is only reachable once CTE bodies are traversed.
        let err = validate_read_only("WITH x AS (SELECT * FROM users FOR UPDATE) SELECT * FROM x")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_read_only_accepts_recursive_select_cte() {
        assert!(validate_read_only(
            "WITH RECURSIVE x AS (SELECT 1 UNION ALL SELECT 2) SELECT * FROM x"
        )
        .is_ok());
    }

    #[test]
    fn validate_read_only_accepts_multi_select_cte() {
        assert!(validate_read_only(
            "WITH a AS (SELECT 1), b AS (SELECT 2) SELECT * FROM a JOIN b ON true"
        )
        .is_ok());
    }

    #[test]
    fn validate_read_only_accepts_nested_select_cte() {
        assert!(validate_read_only(
            "WITH a AS (WITH b AS (SELECT 1) SELECT * FROM b) SELECT * FROM a"
        )
        .is_ok());
    }

    // --- Security: recursive subquery validation ---

    #[test]
    fn validate_rejects_for_update_in_from_subquery() {
        let err =
            validate_read_only("SELECT * FROM (SELECT * FROM users FOR UPDATE) sub").unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_where_subquery() {
        let err = validate_read_only("SELECT * FROM t WHERE id IN (SELECT id FROM t FOR UPDATE)")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_exists() {
        let err = validate_read_only("SELECT * FROM t WHERE EXISTS (SELECT 1 FROM t FOR UPDATE)")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_select_list() {
        let err = validate_read_only("SELECT (SELECT 1 FROM t FOR UPDATE) FROM t").unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_join() {
        let err =
            validate_read_only("SELECT * FROM t JOIN (SELECT * FROM t FOR UPDATE) sub ON true")
                .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_select_into_with_safe_subquery() {
        let err =
            validate_read_only("SELECT * INTO backup FROM (SELECT * FROM t) sub").unwrap_err();
        assert!(err.to_string().contains("SELECT INTO"), "got: {err}");
    }

    #[test]
    fn validate_accepts_safe_nested_subquery() {
        assert!(validate_read_only("SELECT * FROM (SELECT id FROM t) sub").is_ok());
    }

    #[test]
    fn validate_accepts_safe_where_subquery() {
        assert!(validate_read_only("SELECT * FROM t WHERE id IN (SELECT id FROM t)").is_ok());
    }

    // --- Recursive validation: additional coverage ---

    #[test]
    fn validate_accepts_function_with_subquery_arg() {
        // Function arguments with a safe subquery
        assert!(validate_read_only("SELECT COALESCE((SELECT 1), 0) FROM t").is_ok());
    }

    #[test]
    fn validate_accepts_case_with_subquery() {
        assert!(validate_read_only(
            "SELECT CASE WHEN (SELECT 1) = 1 THEN 'yes' ELSE 'no' END FROM t"
        )
        .is_ok());
    }

    #[test]
    fn validate_rejects_for_update_in_case_subquery() {
        let err = validate_read_only(
            "SELECT CASE WHEN (SELECT 1 FROM t FOR UPDATE) = 1 THEN 'yes' ELSE 'no' END FROM t",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_function_arg() {
        let err = validate_read_only("SELECT COALESCE((SELECT 1 FROM t FOR UPDATE), 0) FROM t")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    // --- GROUP BY, ORDER BY, LIMIT recursion ---

    #[test]
    fn validate_rejects_for_update_in_order_by() {
        let err = validate_read_only("SELECT * FROM t ORDER BY (SELECT 1 FROM t FOR UPDATE)")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_limit() {
        let err =
            validate_read_only("SELECT * FROM t LIMIT (SELECT 1 FROM t FOR UPDATE)").unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_rejects_for_update_in_group_by() {
        let err = validate_read_only("SELECT * FROM t GROUP BY (SELECT 1 FROM t FOR UPDATE)")
            .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_accepts_safe_order_by_subquery() {
        assert!(validate_read_only("SELECT * FROM t ORDER BY (SELECT 1)").is_ok());
    }

    #[test]
    fn validate_accepts_safe_group_by() {
        assert!(validate_read_only("SELECT x, COUNT(*) FROM t GROUP BY x").is_ok());
    }

    // --- Subscript, MapAccess, JsonAccess recursion ---

    #[test]
    fn validate_rejects_for_update_in_subscript() {
        let err =
            validate_read_only("SELECT arr[(SELECT 1 FROM t FOR UPDATE)] FROM t").unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_accepts_safe_subscript() {
        assert!(validate_read_only("SELECT arr[1] FROM t").is_ok());
    }

    #[test]
    fn validate_rejects_for_update_in_having() {
        let err = validate_read_only(
            "SELECT x FROM t GROUP BY x HAVING COUNT(*) > (SELECT 1 FROM t FOR UPDATE)",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Locking"), "got: {err}");
    }

    #[test]
    fn validate_accepts_safe_having_subquery() {
        assert!(
            validate_read_only("SELECT x FROM t GROUP BY x HAVING COUNT(*) > (SELECT 1)").is_ok()
        );
    }

    // ── SQL escaping ────────────────────────────────────────────────────

    #[test]
    fn escape_identifier_no_quotes() {
        assert_eq!(escape_identifier("region"), "region");
    }

    #[test]
    fn escape_identifier_with_quotes() {
        assert_eq!(escape_identifier(r#"col"name"#), r#"col""name"#);
    }

    #[test]
    fn escape_literal_no_quotes() {
        assert_eq!(escape_literal("hello"), "hello");
    }

    #[test]
    fn escape_literal_with_quotes() {
        assert_eq!(escape_literal("it's"), "it''s");
    }

    struct DummyDatabase;

    #[async_trait]
    impl TargetDatabase for DummyDatabase {
        async fn query(
            &self,
            _query: &ReadOnlyQuery,
            _params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![DbRow::new(
                vec!["id".into()],
                vec![serde_json::json!("row-1")],
            )])
        }

        async fn health_check(&self) -> Result<(), DbError> {
            Ok(())
        }

        async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![DbRow::new(
                vec!["table_name".into()],
                vec![serde_json::json!("dim_products")],
            )])
        }

        async fn get_table_columns(&self, _table_name: &str) -> Result<Vec<DbRow>, DbError> {
            Ok(vec![DbRow::new(
                vec!["column_name".into()],
                vec![serde_json::json!("product_id")],
            )])
        }

        async fn sample_table(
            &self,
            _table_name: &str,
            limit: usize,
        ) -> Result<Vec<serde_json::Value>, DbError> {
            Ok((0..limit)
                .map(|i| serde_json::json!({ "row": i }))
                .collect())
        }
    }

    #[tokio::test]
    async fn borrowed_target_database_forwards() {
        let db = DummyDatabase;
        let query = ReadOnlyQuery::parse("SELECT 1").unwrap();
        let rows = (&db).query(&query, &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn arc_target_database_forwards() {
        use std::sync::Arc;

        let db: Arc<dyn TargetDatabase> = Arc::new(DummyDatabase);
        let samples = db.sample_table("dim_products", 2).await.unwrap();
        assert_eq!(samples.len(), 2);
    }

    // ── Hegel property-based tests ───────────────────────────────────────

    mod hegel_laws {
        use super::*;
        use hegel::generators as gs;

        /// L1 (Read-Only Safety): no mutation keyword prefix bypasses validation.
        #[hegel::test]
        fn law_read_only_rejects_mutation_prefix(tc: hegel::TestCase) {
            let mutation = tc.draw(gs::sampled_from(vec![
                "INSERT", "UPDATE", "DELETE", "DROP", "CREATE", "ALTER", "TRUNCATE", "GRANT",
                "REVOKE", "COPY", "insert", "update", "delete", "drop",
            ]));
            let suffix = tc.draw(gs::text().max_size(100));
            let sql = format!("{mutation} {suffix}");

            // Must be rejected (either parse error or statement-type rejection)
            let result = validate_read_only(&sql);
            assert!(result.is_err(), "Mutation SQL accepted: {sql}");
        }

        /// L1 (Read-Only Safety): wrapping any mutation as a CTE body is always
        /// rejected. Restricted to INSERT/UPDATE because DELETE/MERGE do not parse
        /// inside a CTE in sqlparser 0.53 (they error at parse time, which would
        /// not exercise the CTE-body traversal added for M8).
        #[hegel::test]
        fn law_read_only_rejects_mutation_in_cte(tc: hegel::TestCase) {
            let mutation = tc.draw(gs::sampled_from(vec![
                "INSERT INTO t VALUES(1)",
                "UPDATE t SET a=1",
            ]));
            let sql = format!("WITH x AS ({mutation} RETURNING *) SELECT * FROM x");
            let result = validate_read_only(&sql);
            assert!(result.is_err(), "Mutation CTE accepted: {sql}");
        }

        /// L1: SELECT with arbitrary columns/tables is accepted.
        #[hegel::test]
        fn law_simple_select_accepted(tc: hegel::TestCase) {
            let table = tc.draw(gs::sampled_from(vec![
                "users",
                "orders",
                "products",
                "dim_products",
            ]));
            let col = tc.draw(gs::sampled_from(vec![
                "id",
                "name",
                "created_at",
                "price",
                "status",
            ]));
            let result = validate_read_only(&format!("SELECT {col} FROM {table}"));
            assert!(
                result.is_ok(),
                "Simple SELECT rejected: SELECT {col} FROM {table}"
            );
        }

        /// L1: Semicolon injection is always rejected.
        #[hegel::test]
        fn law_semicolon_injection_rejected(tc: hegel::TestCase) {
            let safe_prefix = tc.draw(gs::sampled_from(vec![
                "SELECT 1",
                "SELECT * FROM users",
                "SELECT id FROM t WHERE id = 1",
            ]));
            let mutation = tc.draw(gs::sampled_from(vec![
                "DROP TABLE users",
                "DELETE FROM users",
                "INSERT INTO t VALUES(1)",
                "UPDATE t SET x=1",
                "TRUNCATE t",
            ]));
            let sql = format!("{safe_prefix}; {mutation}");
            assert!(
                validate_read_only(&sql).is_err(),
                "Semicolon injection accepted: {sql}"
            );
        }

        /// L1: FOR UPDATE/SHARE locking clauses are always rejected.
        #[hegel::test]
        fn law_locking_always_rejected(tc: hegel::TestCase) {
            let table = tc.draw(gs::sampled_from(vec!["users", "orders", "t"]));
            let lock = tc.draw(gs::sampled_from(vec![
                "FOR UPDATE",
                "FOR SHARE",
                "FOR NO KEY UPDATE",
                "FOR KEY SHARE",
                "FOR UPDATE SKIP LOCKED",
                "FOR UPDATE NOWAIT",
            ]));
            let sql = format!("SELECT * FROM {table} {lock}");
            assert!(
                validate_read_only(&sql).is_err(),
                "Locking clause accepted: {sql}"
            );
        }

        /// L1: Empty/whitespace-only input is rejected.
        #[hegel::test]
        fn law_empty_input_rejected(tc: hegel::TestCase) {
            let whitespace = tc.draw(gs::sampled_from(vec!["", " ", "\n", "\t", "  \n  "]));
            assert!(
                validate_read_only(whitespace).is_err(),
                "Empty/whitespace input accepted: {:?}",
                whitespace
            );
        }
    }
}
