//! Profiling service — column statistics via SQL queries.
//!
//! Like IntrospectionService, this is a struct composing `Arc<dyn TargetDatabase>`,
//! not a trait. There is exactly one way to profile PostgreSQL columns.
//!
//! # Two-Phase Design
//!
//! **Phase 1 — Fused base stats**: A single SQL query computes `null_count` and
//! `distinct_count` for ALL columns simultaneously. Reduces round-trips from 2N to N+1.
//!
//! **Phase 2 — Type-specific stats**: Parallel queries (bounded by semaphore)
//! for type-specific statistics (numeric percentiles, categorical top-values,
//! text lengths, temporal ranges). These vary by column type and cannot be fused.

use std::sync::Arc;

use agent_fw_algebra::{DbError, DbRow, ReadOnlyQuery, TargetDatabase};
use agent_fw_catalog::{
    CategoryValue, ColumnInfo, ColumnProfile, PhysicalTable, SemanticType, TableProfile,
    TypeSpecificStats,
};
use agent_fw_core::DatabaseType;

/// Maximum number of columns to profile concurrently.
const COLUMN_PROFILE_CONCURRENCY: usize = 8;

/// Maximum number of distinct values to fetch for categorical stats.
const CATEGORICAL_VALUE_LIMIT: usize = 50;

// =============================================================================
// Named parameter types — prevent positional swaps
// =============================================================================

/// Pre-computed base statistics from the fused single-table-scan query.
#[derive(Debug, Clone, Copy)]
struct BaseColumnStats {
    null_count: i64,
    distinct_count: i64,
}

/// Pre-computed context for type-specific column profiling.
struct ColumnStatsInput<'a> {
    column: &'a ColumnInfo,
    total_count: i64,
    base: BaseColumnStats,
}

/// Pre-validated SQL identifiers for a column profiling query.
///
/// Four stats methods all need `(schema, table, column)` — three `&str`
/// that are trivially swappable. `ColumnTarget` validates all three
/// on construction, eliminating repeated `validate_identifier` calls.
struct ColumnTarget<'a> {
    schema: &'a str,
    table: &'a str,
    column: &'a str,
}

impl<'a> ColumnTarget<'a> {
    fn validate(schema: &'a str, table: &'a str, column: &'a str) -> Result<Self, DbError> {
        Ok(Self {
            schema: validate_identifier(schema)?,
            table: validate_identifier(table)?,
            column: validate_identifier(column)?,
        })
    }
}

/// Validate that an identifier contains only safe characters before SQL interpolation.
///
/// Follows the `[a-zA-Z0-9_]` pattern.
pub fn validate_identifier(name: &str) -> Result<&str, DbError> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(name)
    } else {
        Err(DbError::InvalidQuery(format!(
            "Invalid SQL identifier: {:?}",
            name
        )))
    }
}

/// Build `CategoryValue` entries from rows with `value` and `count` columns.
fn build_category_values(rows: &[DbRow]) -> Vec<CategoryValue> {
    let total: f64 = rows
        .iter()
        .filter_map(|r| r.get("count").and_then(|v| v.as_f64()))
        .sum();
    rows.iter()
        .map(|r| {
            let count = r.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
            CategoryValue {
                value: r
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                count,
                percentage: if total > 0.0 {
                    (count as f64 / total) * 100.0
                } else {
                    0.0
                },
            }
        })
        .collect()
}

// =============================================================================
// ProfilingService
// =============================================================================

/// Profiling service composes over `TargetDatabase`.
pub struct ProfilingService {
    db: Arc<dyn TargetDatabase>,
}

impl ProfilingService {
    pub fn new(db: Arc<dyn TargetDatabase>) -> Self {
        Self { db }
    }

    /// Profile an entire table in two phases.
    pub async fn profile_table(
        &self,
        physical: &PhysicalTable,
        samples: &[serde_json::Value],
    ) -> Result<TableProfile, DbError> {
        if physical.columns.is_empty() {
            return Ok(TableProfile {
                table_name: physical.table_name.clone(),
                columns: vec![],
            });
        }

        let database_type = self.db.database_type();

        // Phase 1: Fused base stats — single query for all columns.
        let base_stats = Self::fused_base_stats(
            &self.db,
            database_type,
            &physical.schema_name,
            &physical.table_name,
            &physical.columns,
        )
        .await?;

        // Phase 2: Type-specific stats in parallel (bounded concurrency).
        let semaphore = Arc::new(tokio::sync::Semaphore::new(COLUMN_PROFILE_CONCURRENCY));

        let futures: Vec<_> = physical
            .columns
            .iter()
            .map(|col| {
                let sem = Arc::clone(&semaphore);
                let db = Arc::clone(&self.db);
                let schema = physical.schema_name.clone();
                let table = physical.table_name.clone();
                let col = col.clone();
                let row_count = physical.row_count;
                let BaseColumnStats {
                    null_count,
                    distinct_count,
                } = base_stats
                    .get(&col.column_name)
                    .copied()
                    .unwrap_or(BaseColumnStats {
                        null_count: 0,
                        distinct_count: 0,
                    });
                async move {
                    let _permit = sem
                        .acquire()
                        .await
                        .map_err(|_| DbError::Execution("semaphore closed".to_string()))?;
                    let input = ColumnStatsInput {
                        column: &col,
                        total_count: row_count,
                        base: BaseColumnStats {
                            null_count,
                            distinct_count,
                        },
                    };
                    Self::type_specific_stats(&db, database_type, &schema, &table, &input).await
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        let mut columns = Vec::with_capacity(results.len());
        for result in results {
            match result {
                Ok(profile) => columns.push(profile),
                Err(e) => tracing::warn!(
                    table = %physical.table_name,
                    error = %e,
                    "Column profiling failed, skipping"
                ),
            }
        }

        // Post-process: detect patterns in text columns from sample rows.
        if !samples.is_empty() {
            for col_profile in &mut columns {
                if col_profile.semantic_type == SemanticType::Text {
                    if let TypeSpecificStats::Text {
                        ref mut detected_pattern,
                        ..
                    } = col_profile.stats
                    {
                        *detected_pattern =
                            detect_pattern_from_samples(samples, &col_profile.column_name)
                                .map(|p| p.to_string());
                    }
                }
            }
        }

        Ok(TableProfile {
            table_name: physical.table_name.clone(),
            columns,
        })
    }

    /// Fused base stats: single query for all columns' null_count + distinct_count.
    async fn fused_base_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        schema: &str,
        table: &str,
        columns: &[ColumnInfo],
    ) -> Result<std::collections::HashMap<String, BaseColumnStats>, DbError> {
        let schema = validate_identifier(schema)?;
        let table = validate_identifier(table)?;

        let select_exprs: Vec<String> = columns
            .iter()
            .map(|c| {
                let col = validate_identifier(&c.column_name)?;
                Ok(match database_type {
                    DatabaseType::SQLite => format!(
                        "SUM(CASE WHEN \"{col}\" IS NULL THEN 1 ELSE 0 END) AS \"{col}_nulls\", \
                         COUNT(DISTINCT \"{col}\") AS \"{col}_distinct\""
                    ),
                    _ => format!(
                        "COUNT(*) FILTER (WHERE \"{col}\" IS NULL) AS \"{col}_nulls\", \
                         COUNT(DISTINCT \"{col}\") AS \"{col}_distinct\""
                    ),
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        let q = ReadOnlyQuery::parse_for(
            format!(
                "SELECT {} FROM {}",
                select_exprs.join(", "),
                qualified_table(database_type, schema, table)
            ),
            database_type,
        )?;
        let rows = db.query(&q, &[]).await?;
        let row = rows.first();

        let mut stats = std::collections::HashMap::with_capacity(columns.len());
        for col in columns {
            let null_key = format!("{}_nulls", col.column_name);
            let distinct_key = format!("{}_distinct", col.column_name);
            let null_count = row
                .and_then(|r| r.get(&null_key).and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let distinct_count = row
                .and_then(|r| r.get(&distinct_key).and_then(|v| v.as_i64()))
                .unwrap_or(0);
            stats.insert(
                col.column_name.clone(),
                BaseColumnStats {
                    null_count,
                    distinct_count,
                },
            );
        }
        Ok(stats)
    }

    /// Compute type-specific stats for a column (using pre-computed base stats).
    async fn type_specific_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        schema: &str,
        table: &str,
        input: &ColumnStatsInput<'_>,
    ) -> Result<ColumnProfile, DbError> {
        let column = input.column;
        let BaseColumnStats {
            null_count,
            distinct_count,
        } = input.base;
        let total_count = input.total_count;
        let semantic_type = infer_semantic_type(&column.data_type, distinct_count, total_count);

        let target = ColumnTarget::validate(schema, table, &column.column_name)?;
        let stats = match semantic_type {
            SemanticType::Numeric | SemanticType::Monetary => {
                Self::numeric_stats(db, database_type, &target).await?
            }
            SemanticType::Categorical => {
                Self::categorical_stats(db, database_type, &target).await?
            }
            SemanticType::Text => Self::text_stats(db, database_type, &target).await?,
            SemanticType::Temporal => Self::temporal_stats(db, database_type, &target).await?,
            _ => TypeSpecificStats::Unprofilable,
        };

        Ok(ColumnProfile {
            column_name: column.column_name.clone(),
            data_type: column.data_type.clone(),
            null_count,
            distinct_count,
            total_count,
            semantic_type,
            stats,
        })
    }

    // =========================================================================
    // Static methods — no &self borrow, safe for concurrent futures
    // =========================================================================

    async fn numeric_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        target: &ColumnTarget<'_>,
    ) -> Result<TypeSpecificStats, DbError> {
        let sql = match database_type {
            DatabaseType::SQLite => format!(
                "SELECT \
                   CAST(MIN(\"{col}\") AS REAL) AS min_val, \
                   CAST(MAX(\"{col}\") AS REAL) AS max_val, \
                   CAST(AVG(\"{col}\") AS REAL) AS mean_val, \
                   NULL AS p25, \
                   NULL AS p50, \
                   NULL AS p75 \
                 FROM {table_ref} \
                 WHERE \"{col}\" IS NOT NULL",
                col = target.column,
                table_ref = qualified_table(database_type, target.schema, target.table),
            ),
            _ => format!(
                "SELECT \
                   MIN(\"{col}\")::float8 AS min_val, \
                   MAX(\"{col}\")::float8 AS max_val, \
                   AVG(\"{col}\")::float8 AS mean_val, \
                   PERCENTILE_CONT(0.25) WITHIN GROUP (ORDER BY \"{col}\")::float8 AS p25, \
                   PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY \"{col}\")::float8 AS p50, \
                   PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY \"{col}\")::float8 AS p75 \
                 FROM {table_ref}",
                col = target.column,
                table_ref = qualified_table(database_type, target.schema, target.table),
            ),
        };
        let q = ReadOnlyQuery::parse_for(sql, database_type)?;
        let rows = db.query(&q, &[]).await?;
        let r = rows.first();
        Ok(TypeSpecificStats::Numeric {
            min: r.and_then(|r| r.get("min_val").and_then(|v| v.as_f64())),
            max: r.and_then(|r| r.get("max_val").and_then(|v| v.as_f64())),
            mean: r.and_then(|r| r.get("mean_val").and_then(|v| v.as_f64())),
            p25: r.and_then(|r| r.get("p25").and_then(|v| v.as_f64())),
            p50: r.and_then(|r| r.get("p50").and_then(|v| v.as_f64())),
            p75: r.and_then(|r| r.get("p75").and_then(|v| v.as_f64())),
        })
    }

    async fn categorical_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        target: &ColumnTarget<'_>,
    ) -> Result<TypeSpecificStats, DbError> {
        let value_expr = match database_type {
            DatabaseType::SQLite => format!("CAST(\"{}\" AS TEXT)", target.column),
            _ => format!("\"{}\"::text", target.column),
        };
        let q = ReadOnlyQuery::parse_for(
            format!(
                "SELECT {value_expr} AS value, COUNT(*) AS count \
                 FROM {table_ref} \
                 WHERE \"{col}\" IS NOT NULL \
                 GROUP BY \"{col}\" \
                 ORDER BY count DESC \
                 LIMIT {limit}",
                value_expr = value_expr,
                table_ref = qualified_table(database_type, target.schema, target.table),
                col = target.column,
                limit = CATEGORICAL_VALUE_LIMIT,
            ),
            database_type,
        )?;
        let rows = db.query(&q, &[]).await?;
        let top_values = build_category_values(&rows);
        Ok(TypeSpecificStats::Categorical { top_values })
    }

    async fn text_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        target: &ColumnTarget<'_>,
    ) -> Result<TypeSpecificStats, DbError> {
        let avg_expr = match database_type {
            DatabaseType::SQLite => format!("CAST(AVG(LENGTH(\"{}\")) AS REAL)", target.column),
            _ => format!("AVG(LENGTH(\"{}\"))::float8", target.column),
        };
        let q = ReadOnlyQuery::parse_for(
            format!(
                "SELECT \
                   MAX(LENGTH(\"{col}\")) AS max_length, \
                   MIN(LENGTH(\"{col}\")) AS min_length, \
                   {avg_expr} AS avg_length \
                 FROM {table_ref} \
                 WHERE \"{col}\" IS NOT NULL",
                col = target.column,
                avg_expr = avg_expr,
                table_ref = qualified_table(database_type, target.schema, target.table),
            ),
            database_type,
        )?;
        let rows = db.query(&q, &[]).await?;
        let r = rows.first();
        Ok(TypeSpecificStats::Text {
            max_length: r.and_then(|r| r.get("max_length").and_then(|v| v.as_i64())),
            min_length: r.and_then(|r| r.get("min_length").and_then(|v| v.as_i64())),
            avg_length: r.and_then(|r| r.get("avg_length").and_then(|v| v.as_f64())),
            detected_pattern: None, // populated from samples in profile_table
        })
    }

    async fn temporal_stats(
        db: &Arc<dyn TargetDatabase>,
        database_type: DatabaseType,
        target: &ColumnTarget<'_>,
    ) -> Result<TypeSpecificStats, DbError> {
        let q = ReadOnlyQuery::parse_for(
            format!(
                "SELECT {cast_min} AS min_time, {cast_max} AS max_time \
                 FROM {table_ref}",
                cast_min = temporal_cast(database_type, target.column, "MIN"),
                cast_max = temporal_cast(database_type, target.column, "MAX"),
                table_ref = qualified_table(database_type, target.schema, target.table),
            ),
            database_type,
        )?;
        let rows = db.query(&q, &[]).await?;
        let r = rows.first();
        Ok(TypeSpecificStats::Temporal {
            min_time: r.and_then(|r| r.get("min_time").and_then(|v| v.as_str().map(String::from))),
            max_time: r.and_then(|r| r.get("max_time").and_then(|v| v.as_str().map(String::from))),
        })
    }

    /// Extract enum-like columns (low cardinality) via live DB queries.
    ///
    /// **Prefer [`extract_enums_from_profile()`] when a `TableProfile` is already
    /// available** — it derives the same data without any database round-trips.
    pub async fn extract_enums(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnInfo],
        threshold: usize,
    ) -> Result<std::collections::HashMap<String, Vec<CategoryValue>>, DbError> {
        let database_type = self.db.database_type();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(COLUMN_PROFILE_CONCURRENCY));

        let eligible: Vec<_> = columns
            .iter()
            .filter(|c| {
                !c.is_primary_key
                    && matches!(
                        infer_semantic_type(&c.data_type, 0, 0),
                        SemanticType::Categorical | SemanticType::Text | SemanticType::Unknown
                    )
            })
            .collect();

        let futures: Vec<_> = eligible
            .into_iter()
            .map(|col| {
                let db = Arc::clone(&self.db);
                let sem = Arc::clone(&semaphore);
                let schema = schema.to_string();
                let table = table.to_string();
                let col_name = col.column_name.clone();
                async move {
                    let _permit = sem
                        .acquire()
                        .await
                        .map_err(|_| DbError::Execution("semaphore closed".to_string()))?;

                    let schema = validate_identifier(&schema)?;
                    let table = validate_identifier(&table)?;
                    let col = validate_identifier(&col_name)?;

                    let limit = threshold + 1;
                    let value_expr = match database_type {
                        DatabaseType::SQLite => format!("CAST(\"{}\" AS TEXT)", col),
                        _ => format!("\"{}\"::text", col),
                    };
                    let q = ReadOnlyQuery::parse_for(
                        format!(
                            "SELECT {value_expr} AS value, COUNT(*) AS count \
                             FROM {table_ref} \
                             WHERE \"{col}\" IS NOT NULL \
                             GROUP BY \"{col}\" \
                             ORDER BY count DESC \
                             LIMIT {limit}",
                            value_expr = value_expr,
                            table_ref = qualified_table(database_type, schema, table),
                            col = col,
                            limit = limit,
                        ),
                        database_type,
                    )?;
                    let rows = db.query(&q, &[]).await?;

                    if rows.len() > threshold {
                        return Ok::<_, DbError>(None);
                    }

                    let values = build_category_values(&rows);
                    if values.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some((col_name, values)))
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        let mut enums = std::collections::HashMap::new();
        for result in results {
            if let Some((col_name, values)) = result? {
                enums.insert(col_name, values);
            }
        }
        Ok(enums)
    }
}

// =============================================================================
// Pure functions
// =============================================================================

/// Extract enum-like columns from an already-computed profile — **pure, no IO**.
///
/// The algebraic dual of [`ProfilingService::extract_enums()`]: folds over
/// `TypeSpecificStats::Categorical { top_values }` that `profile_table()` already
/// computed. Same data, zero round-trips.
pub fn extract_enums_from_profile(
    profile: &TableProfile,
    columns: &[ColumnInfo],
    threshold: usize,
) -> std::collections::HashMap<String, Vec<CategoryValue>> {
    let pk_columns: std::collections::HashSet<&str> = columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| c.column_name.as_str())
        .collect();

    profile
        .columns
        .iter()
        .filter(|cp| !pk_columns.contains(cp.column_name.as_str()))
        .filter_map(|cp| match &cp.stats {
            TypeSpecificStats::Categorical { top_values }
                if !top_values.is_empty() && top_values.len() <= threshold =>
            {
                Some((cp.column_name.clone(), top_values.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Infer semantic type from SQL data type and distribution.
///
/// Handles core PostgreSQL types including arrays, binary, geographic,
/// monetary, and interval types. Low-cardinality numeric/text columns
/// are promoted to Categorical (threshold: 50 distinct values).
pub fn infer_semantic_type(data_type: &str, distinct_count: i64, total_count: i64) -> SemanticType {
    let dt = data_type.to_lowercase();

    // Array types
    if dt.ends_with("[]") || dt == "array" || dt.starts_with('_') {
        return SemanticType::Array;
    }

    // Binary/bytea
    if dt.contains("bytea") || dt.contains("bit varying") || dt == "bit" {
        return SemanticType::Binary;
    }
    if dt.contains("blob") {
        return SemanticType::Binary;
    }

    // Geographic/geometric types
    if dt.contains("geometry")
        || dt.contains("geography")
        || dt == "point"
        || dt == "line"
        || dt == "lseg"
        || dt == "box"
        || dt == "path"
        || dt == "polygon"
        || dt == "circle"
    {
        return SemanticType::Geographic;
    }

    // Monetary type
    if dt == "money" {
        return SemanticType::Monetary;
    }

    // Numeric types (check before text since "interval" contains "int")
    if (dt.contains("int") && !dt.contains("interval"))
        || dt.contains("float")
        || dt.contains("double")
        || dt.contains("numeric")
        || dt.contains("decimal")
        || dt.contains("real")
        || dt == "smallserial"
        || dt == "bigserial"
    {
        if total_count > 0 && distinct_count > 0 && distinct_count <= 50 {
            return SemanticType::Categorical;
        }
        return SemanticType::Numeric;
    }

    // Temporal types
    if dt.contains("timestamp")
        || dt.contains("date")
        || dt.contains("time")
        || dt.contains("interval")
    {
        return SemanticType::Temporal;
    }

    // JSON types
    if dt.contains("json") {
        return SemanticType::Json;
    }

    // Identifier types
    if dt.contains("uuid") || dt.contains("serial") {
        return SemanticType::Identifier;
    }

    // Boolean → always categorical
    if dt.contains("bool") {
        return SemanticType::Categorical;
    }

    // Network address types
    if dt == "inet" || dt == "cidr" || dt.contains("macaddr") {
        return SemanticType::Text;
    }

    // Text/varchar/char types
    if dt.contains("char") || dt.contains("text") || dt.contains("varchar") {
        if total_count > 0 && distinct_count > 0 && distinct_count <= 50 {
            return SemanticType::Categorical;
        }
        return SemanticType::Text;
    }

    SemanticType::Unknown
}

// =============================================================================
// Pattern detection — pure single-pass fold over sample data
// =============================================================================

/// A detected text pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextPattern {
    Email,
    Url,
    UuidLike,
    JsonString,
    Phone,
}

impl TextPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Url => "url",
            Self::UuidLike => "uuid-like",
            Self::JsonString => "json-string",
            Self::Phone => "phone",
        }
    }
}

impl std::fmt::Display for TextPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Single-pass pattern counters.
#[derive(Default)]
struct PatternCounters {
    total: usize,
    email: usize,
    url: usize,
    uuid: usize,
    json: usize,
    phone: usize,
}

impl PatternCounters {
    fn tally(&mut self, s: &str) {
        self.total += 1;
        self.email += looks_like_email(s) as usize;
        self.url += looks_like_url(s) as usize;
        self.uuid += looks_like_uuid(s) as usize;
        self.json += looks_like_json(s) as usize;
        self.phone += looks_like_phone(s) as usize;
    }

    fn dominant_pattern(&self) -> Option<TextPattern> {
        if self.total < 2 {
            return None;
        }
        // Ceiling division: for total=2, threshold=2 (not 1).
        // floor(2 * 0.8) = 1, which would let 1/2 (50%) pass as "80% dominant".
        // ceil ensures the threshold is never too permissive.
        let threshold = (self.total * 4 + 4) / 5; // = ceil(total * 0.8)
        if self.email >= threshold {
            Some(TextPattern::Email)
        } else if self.url >= threshold {
            Some(TextPattern::Url)
        } else if self.uuid >= threshold {
            Some(TextPattern::UuidLike)
        } else if self.json >= threshold {
            Some(TextPattern::JsonString)
        } else if self.phone >= threshold {
            Some(TextPattern::Phone)
        } else {
            None
        }
    }
}

/// Detect common patterns in text column values from sample rows.
///
/// Pure function — single-pass fold over in-memory sample data.
pub fn detect_pattern_from_samples(
    samples: &[serde_json::Value],
    column_name: &str,
) -> Option<TextPattern> {
    let mut counters = PatternCounters::default();
    for row in samples {
        if let Some(s) = row.get(column_name).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                counters.tally(s);
            }
        }
    }
    counters.dominant_pattern()
}

/// Detect patterns from raw string values (e.g. extracted enum values).
///
/// Pure function — the algebraic dual of [`detect_pattern_from_samples`],
/// operating on extracted values rather than sample rows.
pub fn detect_pattern_from_values(values: &[&str]) -> Option<TextPattern> {
    let mut counters = PatternCounters::default();
    for s in values {
        if !s.is_empty() {
            counters.tally(s);
        }
    }
    counters.dominant_pattern()
}

fn looks_like_email(s: &str) -> bool {
    let at = s.find('@');
    let dot = s.rfind('.');
    matches!((at, dot), (Some(a), Some(d)) if a > 0 && d > a + 1 && d < s.len() - 1 && !s.contains(' '))
}

fn looks_like_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn looks_like_json(s: &str) -> bool {
    let trimmed = s.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

fn looks_like_phone(s: &str) -> bool {
    let (digit_count, all_valid) = s.chars().fold((0usize, true), |(digits, valid), c| {
        (
            digits + c.is_ascii_digit() as usize,
            valid && matches!(c, '0'..='9' | '+' | '-' | ' ' | '(' | ')' | '.'),
        )
    });
    all_valid && digit_count >= 7 && digit_count <= 15
}

fn qualified_table(database_type: DatabaseType, schema: &str, table: &str) -> String {
    match database_type {
        DatabaseType::SQLite if schema == "main" || schema.is_empty() => format!("\"{}\"", table),
        _ => format!("\"{}\".\"{}\"", schema, table),
    }
}

fn temporal_cast(database_type: DatabaseType, column: &str, aggregate: &str) -> String {
    match database_type {
        DatabaseType::SQLite => format!("CAST({aggregate}(\"{column}\") AS TEXT)"),
        _ => format!("{aggregate}(\"{column}\")::text"),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agent_fw_algebra::writable_db::{DdlStatement, InsertBatch, WritableDatabase};
    use agent_fw_interpreter::{SqliteTargetDatabase, SqliteWritableDatabase};

    #[tokio::test]
    async fn sqlite_profile_table_supports_numeric_and_categorical_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.db");

        let writer = SqliteWritableDatabase::open(&path).unwrap();
        let ddl = DdlStatement::parse_for(
            "CREATE TABLE products (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                brand TEXT NOT NULL,
                price REAL,
                created_at TEXT
            )",
            DatabaseType::SQLite,
        )
        .unwrap();
        writer.execute_ddl(&ddl).await.unwrap();

        let rows = (1..=60)
            .map(|price| {
                vec![
                    serde_json::json!(if price % 2 == 0 { "Acme" } else { "Bravo" }),
                    serde_json::json!(price as f64),
                    serde_json::json!(format!("2025-01-{:02}", (price % 28) + 1)),
                ]
            })
            .collect::<Vec<_>>();
        let batch = InsertBatch::new(
            "products",
            vec![
                "brand".to_string(),
                "price".to_string(),
                "created_at".to_string(),
            ],
            rows,
        )
        .unwrap();
        writer.insert_batch(&batch).await.unwrap();

        let db: Arc<dyn TargetDatabase> = Arc::new(SqliteTargetDatabase::open(&path).unwrap());
        let introspection = crate::introspection::IntrospectionService::new(Arc::clone(&db));
        let physical = introspection
            .introspect_table("main", "products")
            .await
            .unwrap();
        let samples = introspection
            .sample_rows("main", "products", 3)
            .await
            .unwrap();

        let service = ProfilingService::new(db);
        let profile = service.profile_table(&physical, &samples).await.unwrap();

        let brand = profile
            .columns
            .iter()
            .find(|column| column.column_name == "brand")
            .unwrap();
        assert_eq!(brand.semantic_type, SemanticType::Categorical);

        let price = profile
            .columns
            .iter()
            .find(|column| column.column_name == "price")
            .unwrap();
        match &price.stats {
            TypeSpecificStats::Numeric {
                min,
                max,
                mean,
                p25,
                p50,
                p75,
            } => {
                assert_eq!(*min, Some(1.0));
                assert_eq!(*max, Some(60.0));
                assert_eq!(*mean, Some(30.5));
                assert_eq!(*p25, None);
                assert_eq!(*p50, None);
                assert_eq!(*p75, None);
            }
            other => panic!("expected numeric stats, got {other:?}"),
        }
    }

    // ── Identifier validation ────────────────────────────────────────

    #[test]
    fn validate_identifier_accepts_valid_names() {
        assert!(validate_identifier("display_name").is_ok());
        assert!(validate_identifier("dim_products").is_ok());
        assert!(validate_identifier("col1").is_ok());
        assert!(validate_identifier("A_Z_0_9").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_invalid_names() {
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("col name").is_err());
        assert!(validate_identifier("table;--").is_err());
        assert!(validate_identifier("col\"name").is_err());
        assert!(validate_identifier("Robert'); DROP TABLE students;--").is_err());
    }

    // ── Semantic type inference ──────────────────────────────────────

    #[test]
    fn infer_numeric_types() {
        assert_eq!(infer_semantic_type("integer", 0, 0), SemanticType::Numeric);
        assert_eq!(infer_semantic_type("float8", 0, 0), SemanticType::Numeric);
        assert_eq!(
            infer_semantic_type("double precision", 0, 0),
            SemanticType::Numeric
        );
        assert_eq!(
            infer_semantic_type("numeric(10,2)", 0, 0),
            SemanticType::Numeric
        );
        assert_eq!(infer_semantic_type("decimal", 0, 0), SemanticType::Numeric);
        assert_eq!(infer_semantic_type("real", 0, 0), SemanticType::Numeric);
    }

    #[test]
    fn infer_numeric_low_cardinality_becomes_categorical() {
        assert_eq!(
            infer_semantic_type("integer", 10, 1000),
            SemanticType::Categorical
        );
        assert_eq!(
            infer_semantic_type("integer", 50, 1000),
            SemanticType::Categorical
        );
        assert_eq!(
            infer_semantic_type("integer", 51, 1000),
            SemanticType::Numeric
        );
    }

    #[test]
    fn infer_temporal_types() {
        assert_eq!(
            infer_semantic_type("timestamp", 0, 0),
            SemanticType::Temporal
        );
        assert_eq!(infer_semantic_type("date", 0, 0), SemanticType::Temporal);
        assert_eq!(
            infer_semantic_type("timestamp with time zone", 0, 0),
            SemanticType::Temporal
        );
    }

    #[test]
    fn infer_json_type() {
        assert_eq!(infer_semantic_type("json", 0, 0), SemanticType::Json);
        assert_eq!(infer_semantic_type("jsonb", 0, 0), SemanticType::Json);
    }

    #[test]
    fn infer_identifier_types() {
        assert_eq!(infer_semantic_type("uuid", 0, 0), SemanticType::Identifier);
        assert_eq!(
            infer_semantic_type("serial", 0, 0),
            SemanticType::Identifier
        );
    }

    #[test]
    fn infer_boolean_is_categorical() {
        assert_eq!(
            infer_semantic_type("boolean", 0, 0),
            SemanticType::Categorical
        );
    }

    #[test]
    fn infer_text_types() {
        assert_eq!(
            infer_semantic_type("varchar(255)", 1000, 5000),
            SemanticType::Text
        );
        assert_eq!(infer_semantic_type("text", 0, 0), SemanticType::Text);
        assert_eq!(
            infer_semantic_type("character varying", 0, 0),
            SemanticType::Text
        );
    }

    #[test]
    fn infer_text_low_cardinality_becomes_categorical() {
        assert_eq!(
            infer_semantic_type("varchar", 10, 1000),
            SemanticType::Categorical
        );
        assert_eq!(
            infer_semantic_type("text", 3, 500),
            SemanticType::Categorical
        );
    }

    #[test]
    fn infer_array_types() {
        assert_eq!(infer_semantic_type("integer[]", 0, 0), SemanticType::Array);
        assert_eq!(infer_semantic_type("text[]", 0, 0), SemanticType::Array);
        assert_eq!(infer_semantic_type("ARRAY", 0, 0), SemanticType::Array);
        assert_eq!(infer_semantic_type("_int4", 0, 0), SemanticType::Array);
    }

    #[test]
    fn infer_binary_types() {
        assert_eq!(infer_semantic_type("bytea", 0, 0), SemanticType::Binary);
        assert_eq!(
            infer_semantic_type("bit varying", 0, 0),
            SemanticType::Binary
        );
        assert_eq!(infer_semantic_type("bit", 0, 0), SemanticType::Binary);
    }

    #[test]
    fn infer_geographic_types() {
        assert_eq!(
            infer_semantic_type("geometry", 0, 0),
            SemanticType::Geographic
        );
        assert_eq!(
            infer_semantic_type("geography", 0, 0),
            SemanticType::Geographic
        );
        assert_eq!(infer_semantic_type("point", 0, 0), SemanticType::Geographic);
        assert_eq!(
            infer_semantic_type("polygon", 0, 0),
            SemanticType::Geographic
        );
    }

    #[test]
    fn infer_monetary_type() {
        assert_eq!(infer_semantic_type("money", 0, 0), SemanticType::Monetary);
    }

    #[test]
    fn infer_interval_is_temporal() {
        assert_eq!(
            infer_semantic_type("interval", 0, 0),
            SemanticType::Temporal
        );
    }

    #[test]
    fn infer_network_types_as_text() {
        assert_eq!(infer_semantic_type("inet", 0, 0), SemanticType::Text);
        assert_eq!(infer_semantic_type("cidr", 0, 0), SemanticType::Text);
        assert_eq!(infer_semantic_type("macaddr", 0, 0), SemanticType::Text);
    }

    #[test]
    fn infer_unknown_type() {
        assert_eq!(infer_semantic_type("tsvector", 0, 0), SemanticType::Unknown);
    }

    // ── Pattern detection ───────────────────────────────────────────

    #[test]
    fn detect_email_pattern() {
        let samples: Vec<serde_json::Value> = vec![
            serde_json::json!({"email": "alice@example.com"}),
            serde_json::json!({"email": "bob@test.org"}),
            serde_json::json!({"email": "charlie@domain.co.uk"}),
        ];
        assert_eq!(
            detect_pattern_from_samples(&samples, "email"),
            Some(TextPattern::Email)
        );
    }

    #[test]
    fn detect_url_pattern() {
        let samples: Vec<serde_json::Value> = vec![
            serde_json::json!({"link": "https://example.com/page"}),
            serde_json::json!({"link": "https://test.org/api"}),
            serde_json::json!({"link": "http://localhost:3000"}),
        ];
        assert_eq!(
            detect_pattern_from_samples(&samples, "link"),
            Some(TextPattern::Url)
        );
    }

    #[test]
    fn detect_uuid_pattern() {
        let samples: Vec<serde_json::Value> = vec![
            serde_json::json!({"id": "550e8400-e29b-41d4-a716-446655440000"}),
            serde_json::json!({"id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8"}),
            serde_json::json!({"id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"}),
        ];
        assert_eq!(
            detect_pattern_from_samples(&samples, "id"),
            Some(TextPattern::UuidLike)
        );
    }

    #[test]
    fn detect_json_string_pattern() {
        let samples: Vec<serde_json::Value> = vec![
            serde_json::json!({"meta": "{\"key\":\"value\"}"}),
            serde_json::json!({"meta": "{\"items\":[1,2]}"}),
            serde_json::json!({"meta": "[1,2,3]"}),
        ];
        assert_eq!(
            detect_pattern_from_samples(&samples, "meta"),
            Some(TextPattern::JsonString)
        );
    }

    #[test]
    fn detect_no_pattern_for_mixed_data() {
        let samples: Vec<serde_json::Value> = vec![
            serde_json::json!({"name": "Alice"}),
            serde_json::json!({"name": "Bob"}),
            serde_json::json!({"name": "Charlie"}),
        ];
        assert_eq!(detect_pattern_from_samples(&samples, "name"), None);
    }

    #[test]
    fn detect_no_pattern_with_insufficient_samples() {
        let samples: Vec<serde_json::Value> =
            vec![serde_json::json!({"email": "alice@example.com"})];
        assert_eq!(detect_pattern_from_samples(&samples, "email"), None);
    }

    #[test]
    fn text_pattern_display_roundtrips() {
        assert_eq!(TextPattern::Email.as_str(), "email");
        assert_eq!(TextPattern::Url.as_str(), "url");
        assert_eq!(TextPattern::UuidLike.as_str(), "uuid-like");
        assert_eq!(TextPattern::JsonString.as_str(), "json-string");
        assert_eq!(TextPattern::Phone.as_str(), "phone");
    }

    #[test]
    fn looks_like_email_positives() {
        assert!(looks_like_email("alice@example.com"));
        assert!(looks_like_email("user+tag@domain.co.uk"));
    }

    #[test]
    fn looks_like_email_negatives() {
        assert!(!looks_like_email("just a string"));
        assert!(!looks_like_email("@missing-local.com"));
        assert!(!looks_like_email("no-at-sign.com"));
    }

    #[test]
    fn looks_like_uuid_positives() {
        assert!(looks_like_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(looks_like_uuid("f47ac10b-58cc-4372-a567-0e02b2c3d479"));
    }

    #[test]
    fn looks_like_uuid_negatives() {
        assert!(!looks_like_uuid("not-a-uuid"));
        assert!(!looks_like_uuid("550e8400e29b41d4a716446655440000"));
    }

    // ── extract_enums_from_profile ──────────────────────────────────

    fn make_column_info(name: &str, is_pk: bool) -> ColumnInfo {
        ColumnInfo {
            column_name: name.to_string(),
            data_type: "text".to_string(),
            is_nullable: true,
            column_default: None,
            ordinal_position: 1,
            is_primary_key: is_pk,
            foreign_key: None,
        }
    }

    fn make_categorical_profile(name: &str, values: Vec<(&str, i64)>) -> ColumnProfile {
        let total: f64 = values.iter().map(|(_, c)| *c as f64).sum();
        ColumnProfile {
            column_name: name.to_string(),
            data_type: "text".to_string(),
            null_count: 0,
            distinct_count: values.len() as i64,
            total_count: total as i64,
            semantic_type: SemanticType::Categorical,
            stats: TypeSpecificStats::Categorical {
                top_values: values
                    .into_iter()
                    .map(|(v, c)| CategoryValue {
                        value: v.to_string(),
                        count: c,
                        percentage: if total > 0.0 {
                            (c as f64 / total) * 100.0
                        } else {
                            0.0
                        },
                    })
                    .collect(),
            },
        }
    }

    #[test]
    fn extract_enums_from_empty_profile() {
        let profile = TableProfile {
            table_name: "t".into(),
            columns: vec![],
        };
        let result = extract_enums_from_profile(&profile, &[], 50);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_enums_from_categorical_columns() {
        let profile = TableProfile {
            table_name: "dim_products".into(),
            columns: vec![
                make_categorical_profile("brand", vec![("Nike", 100), ("Adidas", 80)]),
                make_categorical_profile("color", vec![("Red", 50), ("Blue", 30), ("Green", 20)]),
                ColumnProfile {
                    column_name: "price".to_string(),
                    data_type: "numeric".to_string(),
                    null_count: 0,
                    distinct_count: 500,
                    total_count: 1000,
                    semantic_type: SemanticType::Numeric,
                    stats: TypeSpecificStats::Numeric {
                        min: Some(1.0),
                        max: Some(999.0),
                        mean: Some(50.0),
                        p25: None,
                        p50: None,
                        p75: None,
                    },
                },
            ],
        };

        let columns = vec![
            make_column_info("brand", false),
            make_column_info("color", false),
            make_column_info("price", false),
        ];

        let enums = extract_enums_from_profile(&profile, &columns, 50);
        assert_eq!(enums.len(), 2);
        assert!(enums.contains_key("brand"));
        assert!(enums.contains_key("color"));
    }

    #[test]
    fn extract_enums_excludes_primary_keys_and_over_threshold() {
        let mut many_values: Vec<(&str, i64)> = Vec::new();
        let names: Vec<String> = (0..51).map(|i| format!("v{}", i)).collect();
        for name in &names {
            many_values.push((name.as_str(), 1));
        }

        let profile = TableProfile {
            table_name: "t".into(),
            columns: vec![
                make_categorical_profile("id", vec![("pk1", 1), ("pk2", 1)]),
                make_categorical_profile("status", vec![("active", 90), ("archived", 10)]),
                ColumnProfile {
                    column_name: "high_card".to_string(),
                    data_type: "text".to_string(),
                    null_count: 0,
                    distinct_count: 51,
                    total_count: 51,
                    semantic_type: SemanticType::Categorical,
                    stats: TypeSpecificStats::Categorical {
                        top_values: many_values
                            .into_iter()
                            .map(|(v, c)| CategoryValue {
                                value: v.to_string(),
                                count: c,
                                percentage: 100.0 / 51.0,
                            })
                            .collect(),
                    },
                },
            ],
        };

        let columns = vec![
            make_column_info("id", true),
            make_column_info("status", false),
            make_column_info("high_card", false),
        ];

        let enums = extract_enums_from_profile(&profile, &columns, 50);
        assert_eq!(enums.len(), 1);
        assert!(enums.contains_key("status"));
        assert!(!enums.contains_key("id"), "PKs must be excluded");
        assert!(
            !enums.contains_key("high_card"),
            "Over-threshold must be excluded"
        );
    }
}
