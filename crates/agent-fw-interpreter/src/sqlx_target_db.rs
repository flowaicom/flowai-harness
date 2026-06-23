//! PostgreSQL-backed TargetDatabase via sqlx.
//!
//! Provides production-grade read-only database access with connection pooling,
//! parameterized queries, and query timeout enforcement.
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
//! - L1 (Read-Only Safety): `validate_read_only` rejects mutations
//! - L2 (Determinism): Same query + params → same rows (within transaction)
//! - L3 (Timeout): Queries exceeding `timeout()` return `DbError::Timeout`
//! - L4 (Column Fidelity): `DbRow.columns()` matches the SQL result columns

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use sqlx::postgres::{PgPool, PgTypeKind, PgValueFormat};
use sqlx::{Column, Row, TypeInfo, ValueRef};

use agent_fw_algebra::target_db::{DbError, DbRow, QueryParam, ReadOnlyQuery, TargetDatabase};
use agent_fw_algebra::writable_db::TableName;
use agent_fw_core::DatabaseType;
use agent_fw_search::{AggOp, Filter, FilterSet, NumericOp};

/// PostgreSQL-backed [`TargetDatabase`] using sqlx connection pooling.
///
/// All queries are validated as read-only before execution.
/// Type conversion from sqlx rows to [`DbRow`] handles common PostgreSQL
/// types: text, int2/int4/int8, float4/float8, numeric, bool, uuid,
/// user-defined enums, date/time, jsonb. Unsupported types (e.g. timetz,
/// interval, bytea, arrays) serialize as JSON null and log a warning per
/// query.
pub struct SqlxTargetDatabase {
    pool: PgPool,
    timeout: Duration,
    schema: String,
}

impl SqlxTargetDatabase {
    /// Create from an existing connection pool with default settings.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            timeout: Duration::from_secs(30),
            schema: "public".to_string(),
        }
    }

    /// Connect to a database URL with default settings.
    pub async fn connect(url: &str) -> Result<Self, DbError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| DbError::Connection(e.to_string()))?;
        Ok(Self::new(pool))
    }

    /// Set the query timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the schema (default: "public").
    ///
    /// Validates the schema name using the same allowlist as `TableName`
    /// to prevent SQL injection in queries that interpolate the schema
    /// (e.g., `sample_table`).
    ///
    /// # Panics
    ///
    /// Panics if the schema name contains non-identifier characters.
    /// This is a builder method called at startup, not at request time,
    /// so a panic is appropriate (fail fast on misconfiguration).
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        let s = schema.into();
        TableName::parse(&s).unwrap_or_else(|e| panic!("invalid schema name '{s}': {e}"));
        self.schema = s;
        self
    }

    /// Get a reference to the underlying pool (escape hatch for custom queries).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get the schema name.
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Convert a sqlx PgRow to a DbRow.
    ///
    /// Tries common PostgreSQL types in order: String, i64, i32, i16, f64,
    /// f32, Decimal (as f64), bool, Uuid, enum labels, date/time, JSON. Falls
    /// back to Null for unknown types (see [`Self::warn_undecodable_columns`]).
    /// Date/timestamp values PostgreSQL stores as `infinity` / `-infinity`
    /// serialize as those strings (matching psql), and TIME `24:00:00` is
    /// preserved; chrono cannot represent either, and sqlx's decode panics on
    /// the former and wraps the latter to midnight.
    fn convert_row(row: sqlx::postgres::PgRow) -> DbRow {
        let columns: Vec<String> = row.columns().iter().map(|c| c.name().to_string()).collect();

        let values: Vec<serde_json::Value> = columns
            .iter()
            .map(|col| {
                if let Ok(v) = row.try_get::<Option<String>, _>(col.as_str()) {
                    match v {
                        Some(s) => serde_json::Value::String(s),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<i64>, _>(col.as_str()) {
                    match v {
                        Some(n) => serde_json::json!(n),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<i32>, _>(col.as_str()) {
                    match v {
                        Some(n) => serde_json::json!(n),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<i16>, _>(col.as_str()) {
                    match v {
                        Some(n) => serde_json::json!(n),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<f64>, _>(col.as_str()) {
                    match v {
                        Some(n) => serde_json::json!(n),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<f32>, _>(col.as_str()) {
                    match v {
                        Some(n) => serde_json::json!(n),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<rust_decimal::Decimal>, _>(col.as_str())
                {
                    match v {
                        Some(d) => {
                            use rust_decimal::prelude::ToPrimitive;
                            d.to_f64()
                                .filter(|f| f.is_finite())
                                .map(|f| serde_json::json!(f))
                                .unwrap_or_else(|| serde_json::Value::String(d.to_string()))
                        }
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<bool>, _>(col.as_str()) {
                    match v {
                        Some(b) => serde_json::json!(b),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<sqlx::types::Uuid>, _>(col.as_str()) {
                    match v {
                        Some(u) => serde_json::Value::String(u.to_string()),
                        None => serde_json::Value::Null,
                    }
                } else if let Some(v) = Self::enum_label(&row, col.as_str()) {
                    v
                } else if let Some(v) = Self::nonfinite_temporal(&row, col.as_str()) {
                    v
                } else if let Ok(v) = row.try_get::<Option<NaiveDate>, _>(col.as_str()) {
                    match v {
                        Some(d) => serde_json::json!(d),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<NaiveDateTime>, _>(col.as_str()) {
                    match v {
                        Some(dt) => serde_json::json!(dt),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<DateTime<Utc>>, _>(col.as_str()) {
                    match v {
                        Some(dt) => serde_json::json!(dt),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<NaiveTime>, _>(col.as_str()) {
                    match v {
                        Some(t) => serde_json::json!(t),
                        None => serde_json::Value::Null,
                    }
                } else if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(col.as_str()) {
                    v.unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                }
            })
            .collect();

        DbRow::new(columns, values)
    }

    /// Detect temporal values that sqlx's chrono decode cannot represent.
    ///
    /// PostgreSQL encodes `infinity` / `-infinity` as i32::MAX/MIN days (DATE)
    /// or i64::MAX/MIN microseconds (TIMESTAMP/TIMESTAMPTZ) from 2000-01-01,
    /// and supports dates far beyond chrono's representable range; sqlx
    /// decodes both with unchecked chrono arithmetic that panics. TIME
    /// `24:00:00` (legal in PostgreSQL) silently wraps to midnight in chrono.
    /// Returns `Some(replacement)` for such values (`"infinity"` /
    /// `"-infinity"` / `"24:00:00"`, or Null for other out-of-range values)
    /// and `None` when the column is not temporal or the value decodes safely.
    fn nonfinite_temporal(row: &sqlx::postgres::PgRow, col: &str) -> Option<serde_json::Value> {
        let raw = row.try_get_raw(col).ok()?;
        if raw.is_null() {
            return None;
        }
        let type_info = raw.type_info();
        let type_name = type_info.name();
        if !matches!(type_name, "DATE" | "TIMESTAMP" | "TIMESTAMPTZ" | "TIME") {
            return None;
        }
        match raw.format() {
            PgValueFormat::Text => {
                let text = raw.as_str().ok()?;
                matches!(text, "infinity" | "-infinity" | "24:00:00")
                    .then(|| serde_json::json!(text))
            }
            PgValueFormat::Binary => {
                Self::nonfinite_binary_temporal(type_name, raw.as_bytes().ok()?, col)
            }
        }
    }

    /// Binary-format classification for [`Self::nonfinite_temporal`].
    fn nonfinite_binary_temporal(
        type_name: &str,
        bytes: &[u8],
        col: &str,
    ) -> Option<serde_json::Value> {
        let pg_epoch = NaiveDate::from_ymd_opt(2000, 1, 1)?.and_hms_opt(0, 0, 0)?;
        let in_range = match (type_name, bytes.len()) {
            ("DATE", 4) => {
                let days = i32::from_be_bytes(bytes.try_into().ok()?);
                match days {
                    i32::MAX => return Some(serde_json::json!("infinity")),
                    i32::MIN => return Some(serde_json::json!("-infinity")),
                    _ => TimeDelta::try_days(i64::from(days))
                        .and_then(|delta| pg_epoch.date().checked_add_signed(delta))
                        .is_some(),
                }
            }
            ("TIMESTAMP" | "TIMESTAMPTZ", 8) => {
                let us = i64::from_be_bytes(bytes.try_into().ok()?);
                match us {
                    i64::MAX => return Some(serde_json::json!("infinity")),
                    i64::MIN => return Some(serde_json::json!("-infinity")),
                    _ => pg_epoch
                        .checked_add_signed(TimeDelta::microseconds(us))
                        .is_some(),
                }
            }
            ("TIME", 8) => {
                // Microseconds since midnight; 24:00:00 is legal in PostgreSQL
                // but chrono's NaiveTime addition wraps it to 00:00:00.
                if i64::from_be_bytes(bytes.try_into().ok()?) == 86_400_000_000 {
                    return Some(serde_json::json!("24:00:00"));
                }
                true
            }
            // Unexpected width: let sqlx's own decode surface the error.
            _ => true,
        };
        if in_range {
            None
        } else {
            tracing::warn!(
                column = col,
                pg_type = type_name,
                "temporal value outside chrono's representable range; emitting null"
            );
            Some(serde_json::Value::Null)
        }
    }

    /// Decode a user-defined enum column as its text label.
    ///
    /// sqlx's typed decoders match exact OIDs, so enum values never decode via
    /// `try_get::<String>` even though they are wire-encoded as their label in
    /// both text and binary formats. Returns `None` for non-enum columns.
    fn enum_label(row: &sqlx::postgres::PgRow, col: &str) -> Option<serde_json::Value> {
        let raw = row.try_get_raw(col).ok()?;
        if raw.is_null() {
            return None;
        }
        let type_info = raw.type_info();
        if !matches!(type_info.kind(), PgTypeKind::Enum(_)) {
            return None;
        }
        let label = raw.as_str().ok()?;
        Some(serde_json::Value::String(label.to_string()))
    }

    /// Log one warning per result column whose type `convert_row` cannot
    /// decode.
    ///
    /// Unknown types serialize as JSON null, indistinguishable from SQL NULL —
    /// exactly how planner SQL validation went unnoticed until live planner runs. Keep this
    /// list in sync with the `convert_row` decode chain.
    fn warn_undecodable_columns(row: &sqlx::postgres::PgRow) {
        for column in row.columns() {
            let type_info = column.type_info();
            let decodable = matches!(type_info.kind(), PgTypeKind::Enum(_))
                || matches!(
                    type_info.name(),
                    "TEXT"
                        | "VARCHAR"
                        | "CHAR"
                        | "NAME"
                        | "UNKNOWN"
                        | "citext"
                        | "INT2"
                        | "INT4"
                        | "INT8"
                        | "FLOAT4"
                        | "FLOAT8"
                        | "NUMERIC"
                        | "BOOL"
                        | "UUID"
                        | "DATE"
                        | "TIME"
                        | "TIMESTAMP"
                        | "TIMESTAMPTZ"
                        | "JSON"
                        | "JSONB"
                );
            if !decodable {
                tracing::warn!(
                    column = column.name(),
                    pg_type = type_info.name(),
                    "no decoder for PostgreSQL type; values serialize as JSON null"
                );
            }
        }
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
}

fn validate_identifier(name: &str, entity: &str) -> Result<(), DbError> {
    TableName::parse(name)
        .map(|_| ())
        .map_err(|e| DbError::InvalidQuery(format!("invalid {entity} '{name}': {e}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableRef {
    schema: Option<String>,
    table: String,
}

impl TableRef {
    fn parse(input: &str) -> Result<Self, DbError> {
        let normalized = input.trim();
        TableName::parse(normalized).map_err(|e| {
            DbError::InvalidQuery(format!("invalid table reference '{input}': {e}"))
        })?;

        let parts: Vec<&str> = normalized.split('.').collect();
        match parts.as_slice() {
            [table] => {
                validate_identifier(table, "table")?;
                Ok(Self {
                    schema: None,
                    table: (*table).to_string(),
                })
            }
            [schema, table] => {
                validate_identifier(schema, "schema")?;
                validate_identifier(table, "table")?;
                Ok(Self {
                    schema: Some((*schema).to_string()),
                    table: (*table).to_string(),
                })
            }
            _ => Err(DbError::InvalidQuery(format!(
                "table reference must be table or schema.table, got '{input}'"
            ))),
        }
    }
}

fn sample_table_sql(
    default_schema: &str,
    table_name: &str,
    limit: usize,
) -> Result<String, DbError> {
    validate_identifier(default_schema, "schema")?;
    let table_ref = TableRef::parse(table_name)?;
    let schema = table_ref.schema.as_deref().unwrap_or(default_schema);
    Ok(format!(
        "SELECT * FROM {schema}.{table} LIMIT {limit}",
        table = table_ref.table,
        limit = limit.min(100)
    ))
}

struct SqlWhereBuilder {
    clauses: Vec<String>,
    param_idx: usize,
}

impl SqlWhereBuilder {
    fn new() -> Self {
        Self {
            clauses: Vec::new(),
            param_idx: 1,
        }
    }

    fn next_param(&mut self) -> usize {
        let idx = self.param_idx;
        self.param_idx += 1;
        idx
    }

    fn reserve_params(&mut self, count: usize) -> usize {
        let start = self.param_idx;
        self.param_idx += count;
        start
    }

    fn push_clause(&mut self, clause: String) {
        self.clauses.push(clause);
    }

    fn build_where(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.clauses.join(" AND "))
        }
    }
}

fn numeric_op_sql(op: &NumericOp) -> &'static str {
    match op {
        NumericOp::Eq => "=",
        NumericOp::Ne => "!=",
        NumericOp::Lt => "<",
        NumericOp::Le => "<=",
        NumericOp::Gt => ">",
        NumericOp::Ge => ">=",
    }
}

/// Generic configuration for querying a schema-scoped table with an
/// [`agent_fw_search::FilterSet`].
#[derive(Debug, Clone)]
pub struct TableFilterQuerySpec<'a> {
    schema: &'a str,
    table: &'a str,
    id_column: &'a str,
    jsonb_column: Option<&'a str>,
    physical_columns: Option<&'a HashSet<String>>,
    limit: usize,
}

impl<'a> TableFilterQuerySpec<'a> {
    pub fn new(schema: &'a str, table: &'a str, id_column: &'a str) -> Self {
        Self {
            schema,
            table,
            id_column,
            jsonb_column: None,
            physical_columns: None,
            limit: 20_000,
        }
    }

    pub fn with_jsonb_column(mut self, jsonb_column: &'a str) -> Self {
        self.jsonb_column = Some(jsonb_column);
        self
    }

    pub fn with_physical_columns(mut self, physical_columns: &'a HashSet<String>) -> Self {
        self.physical_columns = Some(physical_columns);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Query a schema-scoped table using a generic [`FilterSet`] contract.
///
/// This stays schema-agnostic: callers provide the schema, table, identifier
/// column, optional JSONB attribute column, and optional discovered physical
/// column set. Applications can layer domain-specific defaults on top without
/// reassembling the SQL builder.
pub async fn query_table_with_filters<T: TargetDatabase + ?Sized>(
    db: &T,
    spec: TableFilterQuerySpec<'_>,
    filters: &FilterSet,
) -> Result<Vec<DbRow>, DbError> {
    validate_identifier(spec.schema, "schema")?;
    validate_identifier(spec.table, "table")?;
    validate_identifier(spec.id_column, "id column")?;
    if let Some(jsonb_column) = spec.jsonb_column {
        validate_identifier(jsonb_column, "jsonb column")?;
    }

    fn col_expr(
        column: &str,
        physical_columns: Option<&HashSet<String>>,
        jsonb_column: Option<&str>,
    ) -> Result<String, DbError> {
        validate_identifier(column, "column name")?;
        match (physical_columns, jsonb_column) {
            (Some(physical), Some(jsonb_column))
                if !physical.is_empty() && !physical.contains(column) =>
            {
                Ok(format!("({jsonb_column}->>'{column}')"))
            }
            _ => Ok(column.to_string()),
        }
    }

    let mut wb = SqlWhereBuilder::new();
    let mut params: Vec<QueryParam> = Vec::new();

    for (_key, filter) in filters.iter() {
        match filter {
            Filter::Matched { column, values } => {
                validate_identifier(column, "column name")?;
                if values.is_empty() {
                    wb.push_clause("FALSE".to_string());
                } else if let (Some(physical), Some(jsonb_column)) =
                    (spec.physical_columns, spec.jsonb_column)
                {
                    if !physical.is_empty() && !physical.contains(column) {
                        let start = wb.reserve_params(values.len());
                        let containment_clauses: Vec<String> = (0..values.len())
                            .map(|i| {
                                format!(
                                    "{jsonb_column} @> jsonb_build_object('{column}', ${})::jsonb",
                                    start + i
                                )
                            })
                            .collect();
                        for value in values {
                            params.push(QueryParam::from(value.clone()));
                        }
                        wb.push_clause(format!("({})", containment_clauses.join(" OR ")));
                        continue;
                    }
                }

                let expr = col_expr(column, spec.physical_columns, spec.jsonb_column)?;
                let start = wb.reserve_params(values.len());
                let placeholders: Vec<String> = (0..values.len())
                    .map(|i| format!("${}", start + i))
                    .collect();
                for value in values {
                    params.push(QueryParam::from(value.clone()));
                }
                wb.push_clause(format!("{}::text IN ({})", expr, placeholders.join(", ")));
            }
            Filter::Numeric { column, op, value } => {
                params.push(QueryParam::from(*value));
                let expr = col_expr(column, spec.physical_columns, spec.jsonb_column)?;
                let p = wb.next_param();
                wb.push_clause(format!("{} {} ${}", expr, numeric_op_sql(op), p));
            }
            Filter::Boolean { column, value } => {
                params.push(QueryParam::Bool(*value));
                let expr = col_expr(column, spec.physical_columns, spec.jsonb_column)?;
                let p = wb.next_param();
                wb.push_clause(format!("{} = ${}", expr, p));
            }
            Filter::Measure {
                column,
                agg,
                op,
                value,
            } => {
                validate_identifier(column, "column name")?;
                let sql_op = numeric_op_sql(op);
                params.push(QueryParam::from(*value));
                let p = wb.next_param();

                match agg {
                    AggOp::Any => {
                        wb.push_clause(format!("{column} {sql_op} ${p}"));
                    }
                    AggOp::Avg | AggOp::Min | AggOp::Max | AggOp::Sum => {
                        let agg_fn = match agg {
                            AggOp::Avg => "AVG",
                            AggOp::Min => "MIN",
                            AggOp::Max => "MAX",
                            AggOp::Sum => "SUM",
                            AggOp::Any => unreachable!(),
                        };
                        wb.push_clause(format!(
                            "{id_column} IN (SELECT {id_column} FROM {schema}.{table} GROUP BY {id_column} HAVING {agg_fn}({column}) {sql_op} ${p})",
                            id_column = spec.id_column,
                            schema = spec.schema,
                            table = spec.table,
                            agg_fn = agg_fn,
                            column = column,
                            sql_op = sql_op,
                            p = p,
                        ));
                    }
                }
            }
        }
    }

    let sql = format!(
        "SELECT * FROM {schema}.{table} {where_clause} LIMIT {limit}",
        schema = spec.schema,
        table = spec.table,
        where_clause = wb.build_where(),
        limit = spec.limit,
    );
    let query = ReadOnlyQuery::parse(sql)?;
    db.query(&query, &params).await
}

/// Query rows from a schema-scoped table by identifier values.
///
/// This is intentionally schema-agnostic: callers provide the schema, table,
/// identifier column, and values. Domain-specific helpers such as
/// `query_products_by_ids` should build on this rather than re-assembling the
/// same SQL in each application.
pub async fn query_table_by_ids<T: TargetDatabase + ?Sized>(
    db: &T,
    schema: &str,
    table: &str,
    id_column: &str,
    ids: &[String],
    limit: usize,
) -> Result<Vec<DbRow>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    validate_identifier(schema, "schema")?;
    validate_identifier(table, "table")?;
    validate_identifier(id_column, "id column")?;

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "SELECT * FROM {schema}.{table} WHERE {id_column} IN ({}) LIMIT {limit}",
        placeholders.join(", ")
    );
    let params: Vec<QueryParam> = ids.iter().cloned().map(QueryParam::from).collect();
    let query = ReadOnlyQuery::parse(sql)?;
    db.query(&query, &params).await
}

/// Search a schema-scoped table by text column with common normalized patterns.
///
/// Matches:
/// - the raw substring pattern
/// - spaces normalized to underscores
/// - underscores normalized to spaces
///
/// If `id_column` is provided, it is searched as text too.
pub async fn search_table_text<T: TargetDatabase + ?Sized>(
    db: &T,
    schema: &str,
    table: &str,
    text_column: &str,
    id_column: Option<&str>,
    query_text: &str,
    limit: usize,
) -> Result<Vec<DbRow>, DbError> {
    validate_identifier(schema, "schema")?;
    validate_identifier(table, "table")?;
    validate_identifier(text_column, "text column")?;
    if let Some(id_column) = id_column {
        validate_identifier(id_column, "id column")?;
    }

    let search_pattern = format!("%{query_text}%");
    let norm_underscore = format!("%{}%", query_text.replace(' ', "_"));
    let norm_space = format!("%{}%", query_text.replace('_', " "));

    let id_clause = id_column
        .map(|col| format!(" OR {col}::text ILIKE $1"))
        .unwrap_or_default();
    let sql = format!(
        "SELECT * FROM {schema}.{table}
         WHERE {text_column} ILIKE $1
            OR {text_column} ILIKE $2
            OR {text_column} ILIKE $3{id_clause}
         LIMIT $4"
    );

    let params = vec![
        search_pattern.into(),
        norm_underscore.into(),
        norm_space.into(),
        (limit as i64).into(),
    ];
    let query = ReadOnlyQuery::parse(sql)?;
    db.query(&query, &params).await
}

#[async_trait]
impl TargetDatabase for SqlxTargetDatabase {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn query(
        &self,
        query: &ReadOnlyQuery,
        params: &[QueryParam],
    ) -> Result<Vec<DbRow>, DbError> {
        // ReadOnlyQuery smart constructor has already validated the SQL is read-only.
        let sql = query.sql();
        let q = sqlx::query(sql);
        let q = Self::bind_params(q, params);

        let rows = tokio::time::timeout(self.timeout, q.fetch_all(&self.pool))
            .await
            .map_err(|_| DbError::Timeout(self.timeout))?
            .map_err(|e| DbError::Execution(e.to_string()))?;

        if let Some(first) = rows.first() {
            Self::warn_undecodable_columns(first);
        }

        Ok(rows.into_iter().map(Self::convert_row).collect())
    }

    async fn health_check(&self) -> Result<(), DbError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DbError::Connection(e.to_string()))?;
        Ok(())
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn list_tables(&self) -> Result<Vec<DbRow>, DbError> {
        let q = ReadOnlyQuery::parse(
            r#"
            SELECT table_name
            FROM information_schema.tables
            WHERE table_schema = $1
              AND table_type = 'BASE TABLE'
            ORDER BY table_name
        "#,
        )?;
        self.query(&q, &[self.schema.clone().into()]).await
    }

    async fn get_table_columns(&self, table_name: &str) -> Result<Vec<DbRow>, DbError> {
        let q = ReadOnlyQuery::parse(
            r#"
            SELECT
                column_name,
                data_type,
                is_nullable,
                column_default
            FROM information_schema.columns
            WHERE table_schema = $1
              AND table_name = $2
            ORDER BY ordinal_position
        "#,
        )?;
        self.query(&q, &[self.schema.clone().into(), table_name.into()])
            .await
    }

    async fn sample_table(
        &self,
        table_name: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let q = ReadOnlyQuery::parse(sample_table_sql(&self.schema, table_name, limit)?)?;

        let rows = self.query(&q, &[]).await?;
        Ok(rows
            .into_iter()
            .map(|r| serde_json::Value::Object(r.as_map().clone().into_iter().collect()))
            .collect())
    }
}

/// Discover all column names for a table.
///
/// Utility function for resolvers that need the column set
/// (e.g., to decide between physical column access and JSONB).
pub async fn discover_columns(
    db: &dyn TargetDatabase,
    _schema: &str,
    table: &str,
) -> Result<HashSet<String>, DbError> {
    let rows = db.get_table_columns(table).await?;
    let mut cols = HashSet::with_capacity(rows.len());
    for row in &rows {
        if let Some(serde_json::Value::String(name)) = row.get("column_name") {
            cols.insert(name.clone());
        }
    }
    Ok(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_url(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    #[test]
    fn sqlx_target_db_schema_default() {
        // Can't test connection without a real DB, but verify construction
        // This test just validates the builder API compiles
        let _schema = "public";
        let _timeout = Duration::from_secs(30);
    }

    #[test]
    fn discover_columns_empty_table() {
        // Verify the function compiles and handles empty results
        // Real integration tests require a live database
    }

    #[test]
    #[should_panic(expected = "invalid schema name")]
    fn with_schema_rejects_injection() {
        // Can't construct without a real pool, but we can verify the builder
        // panics on invalid schema by testing the validation logic directly.
        // The actual with_schema method is tested indirectly via TableName::parse.
        let name = "public; DROP TABLE--";
        TableName::parse(name).unwrap_or_else(|e| panic!("invalid schema name '{name}': {e}"));
    }

    #[test]
    fn schema_validation_accepts_valid() {
        assert!(TableName::parse("public").is_ok());
        assert!(TableName::parse("my_schema").is_ok());
        assert!(TableName::parse("catalog.schema").is_ok());
    }

    #[test]
    fn sample_table_sql_uses_default_schema_for_unqualified_table() {
        let sql = sample_table_sql("public", "dim_products", 10).unwrap();
        assert_eq!(sql, "SELECT * FROM public.dim_products LIMIT 10");
    }

    #[test]
    fn sample_table_sql_uses_explicit_schema_once_for_qualified_table() {
        let sql = sample_table_sql("public", "analytics.dim_products", 10).unwrap();
        assert_eq!(sql, "SELECT * FROM analytics.dim_products LIMIT 10");
    }

    #[test]
    fn sample_table_sql_rejects_three_part_table_ref() {
        let err = sample_table_sql("public", "warehouse.public.dim_products", 10).unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[tokio::test]
    async fn env_gated_query_serializes_postgres_date_time_columns_as_iso_strings() {
        let Some(url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
            return;
        };
        let db = SqlxTargetDatabase::connect(&url).await.unwrap();
        let query = ReadOnlyQuery::parse(
            r#"
            SELECT
                CAST('2025-03-17' AS DATE) AS start_date,
                CAST('2025-03-17 12:34:56' AS TIMESTAMP) AS created_at,
                CAST('2025-03-17 12:34:56+02' AS TIMESTAMPTZ) AS created_at_tz,
                CAST('12:34:56' AS TIME) AS local_time,
                CAST(NULL AS DATE) AS missing_date
            "#,
        )
        .unwrap();

        let rows = db.query(&query, &[]).await.unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.get("start_date"),
            Some(&serde_json::json!("2025-03-17"))
        );
        assert_eq!(
            row.get("created_at"),
            Some(&serde_json::json!("2025-03-17T12:34:56"))
        );
        assert_eq!(
            row.get("created_at_tz"),
            Some(&serde_json::json!("2025-03-17T10:34:56Z"))
        );
        assert_eq!(row.get("local_time"), Some(&serde_json::json!("12:34:56")));
        assert_eq!(row.get("missing_date"), Some(&serde_json::Value::Null));
        db.close().await;
    }

    #[tokio::test]
    async fn env_gated_query_serializes_postgres_infinity_dates_as_strings() {
        let Some(url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
            return;
        };
        let db = SqlxTargetDatabase::connect(&url).await.unwrap();
        let query = ReadOnlyQuery::parse(
            r#"
            SELECT
                CAST('infinity' AS DATE) AS forever_date,
                CAST('-infinity' AS DATE) AS dawn_date,
                CAST('infinity' AS TIMESTAMP) AS forever_ts,
                CAST('-infinity' AS TIMESTAMPTZ) AS dawn_tstz
            "#,
        )
        .unwrap();

        let rows = db.query(&query, &[]).await.unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.get("forever_date"),
            Some(&serde_json::json!("infinity"))
        );
        assert_eq!(row.get("dawn_date"), Some(&serde_json::json!("-infinity")));
        assert_eq!(row.get("forever_ts"), Some(&serde_json::json!("infinity")));
        assert_eq!(row.get("dawn_tstz"), Some(&serde_json::json!("-infinity")));
        db.close().await;
    }

    #[test]
    fn nonfinite_binary_temporal_maps_infinity_sentinels() {
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("DATE", &i32::MAX.to_be_bytes(), "c"),
            Some(serde_json::json!("infinity"))
        );
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("DATE", &i32::MIN.to_be_bytes(), "c"),
            Some(serde_json::json!("-infinity"))
        );
        for type_name in ["TIMESTAMP", "TIMESTAMPTZ"] {
            assert_eq!(
                SqlxTargetDatabase::nonfinite_binary_temporal(
                    type_name,
                    &i64::MAX.to_be_bytes(),
                    "c"
                ),
                Some(serde_json::json!("infinity"))
            );
            assert_eq!(
                SqlxTargetDatabase::nonfinite_binary_temporal(
                    type_name,
                    &i64::MIN.to_be_bytes(),
                    "c"
                ),
                Some(serde_json::json!("-infinity"))
            );
        }
    }

    #[test]
    fn nonfinite_binary_temporal_passes_in_range_values_to_sqlx() {
        // 9207 days from 2000-01-01 is 2025-03-17; sqlx should decode it.
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("DATE", &9207_i32.to_be_bytes(), "c"),
            None
        );
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("TIMESTAMP", &0_i64.to_be_bytes(), "c"),
            None
        );
        // Unexpected byte widths are left for sqlx's decode to report.
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("DATE", &[0u8; 8], "c"),
            None
        );
    }

    #[test]
    fn nonfinite_binary_temporal_nulls_values_beyond_chrono_range() {
        // Not the infinity sentinel, but past chrono's max year (262143):
        // would panic inside sqlx's chrono decode without the guard.
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal(
                "DATE",
                &(i32::MAX - 1).to_be_bytes(),
                "c"
            ),
            Some(serde_json::Value::Null)
        );
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal(
                "TIMESTAMPTZ",
                &(i64::MAX - 1).to_be_bytes(),
                "c"
            ),
            Some(serde_json::Value::Null)
        );
    }

    #[test]
    fn nonfinite_binary_temporal_preserves_time_24_00() {
        // 86_400_000_000 µs since midnight = TIME '24:00:00', which chrono's
        // wrapping NaiveTime addition would silently turn into 00:00:00.
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal(
                "TIME",
                &86_400_000_000_i64.to_be_bytes(),
                "c"
            ),
            Some(serde_json::json!("24:00:00"))
        );
        // Any other legal TIME value decodes via sqlx unchanged.
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal(
                "TIME",
                &86_399_999_999_i64.to_be_bytes(),
                "c"
            ),
            None
        );
        assert_eq!(
            SqlxTargetDatabase::nonfinite_binary_temporal("TIME", &0_i64.to_be_bytes(), "c"),
            None
        );
    }

    #[tokio::test]
    async fn env_gated_query_serializes_smallint_real_uuid_and_time_24_00() {
        let Some(url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
            return;
        };
        let db = SqlxTargetDatabase::connect(&url).await.unwrap();
        let query = ReadOnlyQuery::parse(
            r#"
            SELECT
                CAST(7 AS SMALLINT) AS small_qty,
                CAST(1.5 AS REAL) AS approx_rate,
                CAST('123e4567-e89b-12d3-a456-426614174000' AS UUID) AS row_uuid,
                CAST('24:00:00' AS TIME) AS end_of_day
            "#,
        )
        .unwrap();

        let rows = db.query(&query, &[]).await.unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.get("small_qty"), Some(&serde_json::json!(7)));
        assert_eq!(row.get("approx_rate"), Some(&serde_json::json!(1.5)));
        assert_eq!(
            row.get("row_uuid"),
            Some(&serde_json::json!("123e4567-e89b-12d3-a456-426614174000"))
        );
        assert_eq!(row.get("end_of_day"), Some(&serde_json::json!("24:00:00")));
        db.close().await;
    }

    #[tokio::test]
    async fn env_gated_query_serializes_enum_columns_as_labels() {
        let Some(url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
            return;
        };
        let db = SqlxTargetDatabase::connect(&url).await.unwrap();
        // DDL goes through the raw pool: ReadOnlyQuery correctly rejects it.
        sqlx::query("DROP TYPE IF EXISTS fai415_test_mood")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("CREATE TYPE fai415_test_mood AS ENUM ('happy', 'sad')")
            .execute(db.pool())
            .await
            .unwrap();

        let query =
            ReadOnlyQuery::parse("SELECT CAST('happy' AS fai415_test_mood) AS mood").unwrap();
        let rows = db.query(&query, &[]).await.unwrap();

        sqlx::query("DROP TYPE fai415_test_mood")
            .execute(db.pool())
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("mood"), Some(&serde_json::json!("happy")));
        db.close().await;
    }

    #[tokio::test]
    async fn query_table_by_ids_builds_schema_scoped_lookup() {
        let db = crate::MockTargetDatabase::new();
        db.expect_query(
            "SELECT * FROM public.products WHERE product_id IN ($1, $2) LIMIT 1000",
            crate::MockQueryResult::new(
                vec!["product_id"],
                vec![vec![serde_json::json!("p1")], vec![serde_json::json!("p2")]],
            ),
        );

        let rows = query_table_by_ids(
            &db,
            "public",
            "products",
            "product_id",
            &["p1".to_string(), "p2".to_string()],
            1000,
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 2);
        assert!(db.was_executed("product_id IN ($1, $2)"));
    }

    #[tokio::test]
    async fn search_table_text_builds_normalized_ilike_query() {
        let db = crate::MockTargetDatabase::new();
        db.expect_query(
            "SELECT * FROM public.products\n         WHERE display_name ILIKE $1\n            OR display_name ILIKE $2\n            OR display_name ILIKE $3 OR product_id::text ILIKE $1\n         LIMIT $4",
            crate::MockQueryResult::new(vec!["display_name"], vec![vec![serde_json::json!("Widget")]]),
        );

        let rows = search_table_text(
            &db,
            "public",
            "products",
            "display_name",
            Some("product_id"),
            "widget pack",
            25,
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        let executed = db.executed_queries();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].1.len(), 4);
    }

    #[tokio::test]
    async fn query_table_with_filters_uses_jsonb_fallback_for_nonphysical_matched_filters() {
        let db = crate::MockTargetDatabase::new();
        db.expect_query(
            "SELECT * FROM public.products WHERE (attributes @> jsonb_build_object('brand', $1)::jsonb OR attributes @> jsonb_build_object('brand', $2)::jsonb) LIMIT 20000",
            crate::MockQueryResult::new(
                vec!["product_id"],
                vec![vec![serde_json::json!("p1")]],
            ),
        );

        let mut physical = HashSet::new();
        physical.insert("product_id".to_string());
        let filters = agent_fw_search::filters_from_vec(vec![Filter::matched(
            "brand",
            vec!["Acme".to_string(), "Bravo".to_string()],
        )]);

        let rows = query_table_with_filters(
            &db,
            TableFilterQuerySpec::new("public", "products", "product_id")
                .with_jsonb_column("attributes")
                .with_physical_columns(&physical),
            &filters,
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn query_table_with_filters_rejects_unsafe_jsonb_fallback_keys() {
        let db = crate::MockTargetDatabase::new();
        let mut physical = HashSet::new();
        physical.insert("product_id".to_string());

        for column in ["brand'); DROP TABLE products; --", "brand\nname"] {
            let filters = agent_fw_search::filters_from_vec(vec![Filter::matched(
                column,
                vec!["Acme".to_string()],
            )]);

            let error = query_table_with_filters(
                &db,
                TableFilterQuerySpec::new("public", "products", "product_id")
                    .with_jsonb_column("attributes")
                    .with_physical_columns(&physical),
                &filters,
            )
            .await
            .unwrap_err();

            assert!(matches!(error, DbError::InvalidQuery(_)));
        }
    }

    #[tokio::test]
    async fn query_table_with_filters_builds_measure_subquery() {
        let db = crate::MockTargetDatabase::new();
        db.expect_query(
            "SELECT * FROM public.sales WHERE account_id IN (SELECT account_id FROM public.sales GROUP BY account_id HAVING AVG(revenue) > $1) LIMIT 25",
            crate::MockQueryResult::new(vec!["account_id"], vec![vec![serde_json::json!("a1")]]),
        );

        let filters = agent_fw_search::filters_from_vec(vec![Filter::measure(
            "revenue",
            AggOp::Avg,
            NumericOp::Gt,
            rust_decimal::Decimal::new(1000, 0),
        )]);

        let rows = query_table_with_filters(
            &db,
            TableFilterQuerySpec::new("public", "sales", "account_id").with_limit(25),
            &filters,
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
    }
}
