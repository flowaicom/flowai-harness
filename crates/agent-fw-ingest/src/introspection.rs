//! Introspection service — queries information_schema for database structure.
//!
//! This is a service struct (not a trait) because there is exactly one way
//! to introspect PostgreSQL via information_schema. Follows D1: "don't abstract
//! until you have three implementations."
//!
//! ```text
//! IntrospectionService
//!   ┌───────────┐
//!   │ Arc<dyn   │
//!   │ TargetDB> │─── query(sql, params) ───→ Vec<DbRow>
//!   └───────────┘
//! ```
//!
//! All methods are `&self` — immutable, shareable, safe for concurrent use.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_fw_algebra::{
    escape_identifier, DbError, DbRow, QueryParam, ReadOnlyQuery, TableName, TargetDatabase,
};
use agent_fw_catalog::{
    ColumnInfo, ConstraintInfo, ConstraintKind, ForeignKeyEdge, ForeignKeyRef, IndexInfo,
    PhysicalTable, TableInfo, TableType,
};
use agent_fw_core::DatabaseType;

/// Introspection cache — stores results of expensive schema queries.
#[derive(Default)]
struct IntrospectionCache {
    /// schema_name → Vec<TableInfo>
    tables: HashMap<String, Vec<TableInfo>>,
    /// (schema, table) → PhysicalTable
    physical_tables: HashMap<(String, String), PhysicalTable>,
    /// schema → Vec<ForeignKeyEdge>
    foreign_keys: HashMap<String, Vec<ForeignKeyEdge>>,
}

/// Introspection service composes over `TargetDatabase`.
///
/// Optionally caches schema introspection results. Call [`refresh_schema`]
/// to invalidate a specific schema's cache, or [`refresh_all`] to clear
/// the entire cache (e.g., after a DDL change).
///
/// # Mutex poison recovery
///
/// All cache access uses `Mutex::lock()`. If a thread panics while
/// holding the lock, the mutex becomes poisoned. This is handled as:
///
/// - **Read path** (`list_tables`, `introspect_table`, `list_foreign_keys`):
///   If the lock is poisoned, the cache miss falls through to a fresh
///   database query. The result is not cached (the lock is still poisoned),
///   but correctness is preserved — just slower.
///
/// - **Write/invalidation path** (`refresh_schema`, `refresh_all`):
///   If the lock is poisoned, invalidation silently does nothing. This means
///   stale data may be served from the (inaccessible) cache until the
///   process restarts. This is safe because stale schema metadata cannot
///   cause incorrect query results — only potentially outdated column lists.
///
/// In both cases we prefer availability over panic propagation: the
/// introspection service continues functioning (with degraded caching)
/// rather than crashing.
pub struct IntrospectionService {
    db: Arc<dyn TargetDatabase>,
    cache: Mutex<IntrospectionCache>,
}

impl IntrospectionService {
    pub fn new(db: Arc<dyn TargetDatabase>) -> Self {
        Self {
            db,
            cache: Mutex::new(IntrospectionCache::default()),
        }
    }

    /// Access the underlying `TargetDatabase` for direct query execution.
    pub fn target_db(&self) -> Arc<dyn TargetDatabase> {
        self.db.clone()
    }

    fn database_type(&self) -> DatabaseType {
        self.db.database_type()
    }

    /// Invalidate cached introspection data for a specific schema.
    ///
    /// Forces subsequent calls to `list_tables`, `introspect_table`, and
    /// `list_foreign_keys` to re-query the database for this schema.
    ///
    /// No-op if the cache mutex is poisoned (see struct-level docs).
    pub fn refresh_schema(&self, schema: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.tables.remove(schema);
            cache.physical_tables.retain(|key, _| key.0 != schema);
            cache.foreign_keys.remove(schema);
            tracing::debug!(schema, "Schema cache invalidated");
        }
    }

    /// Invalidate all cached introspection data.
    ///
    /// No-op if the cache mutex is poisoned (see struct-level docs).
    pub fn refresh_all(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.tables.clear();
            cache.physical_tables.clear();
            cache.foreign_keys.clear();
            tracing::debug!("All introspection caches invalidated");
        }
    }

    /// List all schemas in the database (excluding system schemas).
    pub async fn list_schemas(&self) -> Result<Vec<String>, DbError> {
        match self.database_type() {
            DatabaseType::SQLite => Ok(vec!["main".to_string()]),
            _ => {
                let q = ReadOnlyQuery::parse(
                    "SELECT schema_name FROM information_schema.schemata \
                     WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'pg_toast') \
                     ORDER BY schema_name",
                )?;
                let rows = self.db.query(&q, &[]).await?;

                Ok(rows
                    .iter()
                    .filter_map(|r| {
                        r.get("schema_name")
                            .and_then(|v| v.as_str().map(String::from))
                    })
                    .collect())
            }
        }
    }

    /// List tables in a schema with estimated row counts and column counts.
    ///
    /// Results are cached per-schema. Call [`refresh_schema`] to invalidate.
    ///
    /// Runs three concurrent queries via `tokio::join!`:
    /// 1. Tables from information_schema.tables
    /// 2. Estimated row counts from pg_class
    /// 3. Column counts from information_schema.columns
    pub async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>, DbError> {
        // Check cache
        if let Ok(cache) = self.cache.lock() {
            if let Some(cached) = cache.tables.get(schema) {
                return Ok(cached.clone());
            }
        }
        let result = match self.database_type() {
            DatabaseType::SQLite => self.list_tables_sqlite().await?,
            _ => self.list_tables_postgres(schema).await?,
        };

        // Cache result
        if let Ok(mut cache) = self.cache.lock() {
            cache.tables.insert(schema.to_string(), result.clone());
        }

        Ok(result)
    }

    /// Introspect a single table — columns, constraints, indexes, row count.
    ///
    /// Results are cached per (schema, table). Call [`refresh_schema`] to invalidate.
    ///
    /// Runs all 6 independent queries concurrently via `tokio::join!`.
    pub async fn introspect_table(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<PhysicalTable, DbError> {
        // Check cache
        let cache_key = (schema.to_string(), table.to_string());
        if let Ok(cache) = self.cache.lock() {
            if let Some(cached) = cache.physical_tables.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let result = match self.database_type() {
            DatabaseType::SQLite => self.introspect_table_sqlite(schema, table).await?,
            _ => self.introspect_table_postgres(schema, table).await?,
        };

        // Cache result
        if let Ok(mut cache) = self.cache.lock() {
            cache.physical_tables.insert(cache_key, result.clone());
        }

        Ok(result)
    }

    /// Discover all foreign key edges in a schema (single query).
    ///
    /// Used by the ingestion orchestrator to build FK-aware database context
    /// for LLM enrichment without introspecting each table first.
    pub async fn list_foreign_keys(&self, schema: &str) -> Result<Vec<ForeignKeyEdge>, DbError> {
        match self.database_type() {
            DatabaseType::SQLite => self.list_foreign_keys_sqlite().await,
            _ => self.list_foreign_keys_postgres(schema).await,
        }
    }

    /// Sample rows from a table.
    pub async fn sample_rows(
        &self,
        _schema: &str,
        table: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DbError> {
        self.db.sample_table(table, limit.min(100)).await
    }

    async fn list_tables_postgres(&self, schema: &str) -> Result<Vec<TableInfo>, DbError> {
        let params_a = [QueryParam::Text(schema.to_string())];
        let params_b = [QueryParam::Text(schema.to_string())];
        let params_c = [QueryParam::Text(schema.to_string())];

        let q_tables = ReadOnlyQuery::parse(
            "SELECT table_schema, table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema = $1 \
             ORDER BY table_name",
        )?;
        let q_stats = ReadOnlyQuery::parse(
            "SELECT c.relname, \
                    CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::bigint END AS row_count \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 \
               AND c.relkind IN ('r', 'v', 'm', 'f', 'p')",
        )?;
        let q_cols = ReadOnlyQuery::parse(
            "SELECT table_name, COUNT(*)::bigint AS column_count \
             FROM information_schema.columns \
             WHERE table_schema = $1 \
             GROUP BY table_name",
        )?;

        let (table_rows, stat_rows, col_count_rows) = tokio::join!(
            self.db.query(&q_tables, &params_a),
            self.db.query(&q_stats, &params_b),
            self.db.query(&q_cols, &params_c),
        );

        let table_rows = table_rows?;

        let row_counts: std::collections::HashMap<String, i64> = match stat_rows {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| {
                    let name = r.get("relname")?.as_str()?.to_string();
                    let count = r.get("row_count")?.as_i64()?;
                    Some((name, count))
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Row count query failed — estimates unavailable");
                std::collections::HashMap::new()
            }
        };

        let col_counts: std::collections::HashMap<String, i64> = match col_count_rows {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| {
                    let name = r.get("table_name")?.as_str()?.to_string();
                    let count = r.get("column_count")?.as_i64()?;
                    Some((name, count))
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Column count query failed — counts unavailable");
                std::collections::HashMap::new()
            }
        };

        Ok(table_rows
            .iter()
            .map(|r| {
                let table_type_str = r
                    .get("table_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("BASE TABLE");
                let table_name = r
                    .get("table_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let row_count = row_counts.get(&table_name).copied();
                let column_count = col_counts.get(&table_name).copied();
                TableInfo {
                    schema_name: r
                        .get("table_schema")
                        .and_then(|v| v.as_str())
                        .unwrap_or(schema)
                        .to_string(),
                    table_name,
                    table_type: match table_type_str {
                        "VIEW" => TableType::View,
                        "MATERIALIZED VIEW" => TableType::MaterializedView,
                        "FOREIGN" | "FOREIGN TABLE" => TableType::Foreign,
                        _ => TableType::BaseTable,
                    },
                    row_count,
                    column_count,
                    description: None,
                }
            })
            .collect())
    }

    async fn list_tables_sqlite(&self) -> Result<Vec<TableInfo>, DbError> {
        let rows = self.db.list_tables().await?;
        let mut tables = Vec::with_capacity(rows.len());
        for row in rows {
            let table_name = get_str(&row, "table_name").to_string();
            let column_count = self
                .db
                .get_table_columns(&table_name)
                .await
                .ok()
                .map(|columns| columns.len() as i64);
            let row_count = match ReadOnlyQuery::parse_for(
                format!(
                    "SELECT COUNT(*) AS row_count FROM {}",
                    sqlite_table_identifier(&table_name)?
                ),
                DatabaseType::SQLite,
            ) {
                Ok(query) => self.db.query(&query, &[]).await.ok().and_then(|rows| {
                    rows.first().and_then(|count_row| {
                        count_row.get("row_count").and_then(|value| value.as_i64())
                    })
                }),
                Err(error) => {
                    tracing::warn!(table = %table_name, error = %error, "Failed to build SQLite row count query");
                    None
                }
            };
            tables.push(TableInfo {
                schema_name: row
                    .get("schema_name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("main")
                    .to_string(),
                table_name,
                table_type: sqlite_table_type(
                    row.get("table_type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("BASE TABLE"),
                ),
                row_count,
                column_count,
                description: None,
            });
        }
        Ok(tables)
    }

    async fn introspect_table_postgres(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<PhysicalTable, DbError> {
        let params = [
            QueryParam::Text(schema.to_string()),
            QueryParam::Text(table.to_string()),
        ];

        let q_cols = ReadOnlyQuery::parse(
            "SELECT column_name, data_type, is_nullable, column_default, ordinal_position \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
        )?;
        let q_pks = ReadOnlyQuery::parse(
            "SELECT kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             WHERE tc.constraint_type = 'PRIMARY KEY' \
               AND tc.table_schema = $1 AND tc.table_name = $2",
        )?;
        let q_fks = ReadOnlyQuery::parse(
            "SELECT kcu.column_name, \
                    ccu.table_schema AS referenced_schema, \
                    ccu.table_name AS referenced_table, \
                    ccu.column_name AS referenced_column, \
                    tc.constraint_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name \
               AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' \
               AND tc.table_schema = $1 AND tc.table_name = $2",
        )?;
        let q_constraints = ReadOnlyQuery::parse(
            "SELECT tc.constraint_name, tc.constraint_type, \
                    string_agg(kcu.column_name, ',' ORDER BY kcu.ordinal_position) as columns \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_schema = $1 AND tc.table_name = $2 \
             GROUP BY tc.constraint_name, tc.constraint_type",
        )?;
        let q_indexes = ReadOnlyQuery::parse(
            "SELECT i.relname AS index_name, \
                    ix.indisunique AS is_unique, \
                    array_to_string(ARRAY(SELECT a.attname \
                        FROM unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord) \
                        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
                        ORDER BY k.ord), ',') AS columns \
             FROM pg_index ix \
             JOIN pg_class c ON c.oid = ix.indrelid \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 \
               AND NOT ix.indisprimary",
        )?;
        let q_count = ReadOnlyQuery::parse(format!(
            "SELECT COUNT(*)::bigint AS row_count FROM {}",
            postgres_qualified_table_identifier(schema, table)
        ))?;

        let (col_rows, pk_rows, fk_rows, constraint_rows, index_rows, count_rows) = tokio::join!(
            self.db.query(&q_cols, &params),
            self.db.query(&q_pks, &params),
            self.db.query(&q_fks, &params),
            self.db.query(&q_constraints, &params),
            self.db.query(&q_indexes, &params),
            self.db.query(&q_count, &[]),
        );

        let col_rows = col_rows?;
        let pk_rows = pk_rows?;
        let fk_rows = fk_rows?;
        let constraint_rows = constraint_rows?;
        let index_rows = match index_rows {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(schema, table, error = %e, "Index query failed");
                vec![]
            }
        };
        let count_rows = count_rows?;

        let pk_columns: Vec<String> = pk_rows
            .iter()
            .filter_map(|r| {
                r.get("column_name")
                    .and_then(|v| v.as_str().map(String::from))
            })
            .collect();

        let mut fk_map: std::collections::HashMap<String, ForeignKeyRef> =
            std::collections::HashMap::new();
        for r in &fk_rows {
            let col = get_str(r, "column_name").to_string();
            fk_map.insert(
                col,
                ForeignKeyRef {
                    referenced_schema: get_str(r, "referenced_schema").to_string(),
                    referenced_table: get_str(r, "referenced_table").to_string(),
                    referenced_column: get_str(r, "referenced_column").to_string(),
                    constraint_name: get_str(r, "constraint_name").to_string(),
                },
            );
        }

        let columns: Vec<ColumnInfo> = col_rows
            .iter()
            .map(|r| {
                let col_name = get_str(r, "column_name").to_string();
                ColumnInfo {
                    is_primary_key: pk_columns.contains(&col_name),
                    foreign_key: fk_map.remove(&col_name),
                    column_name: col_name,
                    data_type: get_str(r, "data_type").to_string(),
                    is_nullable: get_str(r, "is_nullable") == "YES",
                    column_default: r
                        .get("column_default")
                        .and_then(|v| v.as_str().map(String::from)),
                    ordinal_position: r
                        .get("ordinal_position")
                        .and_then(|v| v.as_i64())
                        .and_then(|v| i32::try_from(v).ok())
                        .unwrap_or(0),
                }
            })
            .collect();

        let constraints: Vec<ConstraintInfo> = constraint_rows
            .iter()
            .map(|r| ConstraintInfo {
                name: get_str(r, "constraint_name").to_string(),
                constraint_type: get_str(r, "constraint_type")
                    .parse::<ConstraintKind>()
                    .unwrap_or(ConstraintKind::Check),
                columns: get_str(r, "columns")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            })
            .collect();

        let indexes: Vec<IndexInfo> = index_rows
            .iter()
            .filter_map(|r| {
                let name = r.get("index_name").and_then(|v| v.as_str())?.to_string();
                let is_unique = r
                    .get("is_unique")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let columns: Vec<String> = get_str(r, "columns")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Some(IndexInfo {
                    name,
                    columns,
                    is_unique,
                })
            })
            .collect();

        let row_count = count_rows
            .first()
            .and_then(|r| r.get("row_count").and_then(|v| v.as_i64()))
            .unwrap_or(0);

        Ok(PhysicalTable {
            schema_name: schema.to_string(),
            table_name: table.to_string(),
            columns,
            constraints,
            indexes,
            row_count,
        })
    }

    async fn introspect_table_sqlite(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<PhysicalTable, DbError> {
        let col_rows = self.db.get_table_columns(table).await?;

        let q_fks = ReadOnlyQuery::parse_for(
            "SELECT id, \"from\" AS column_name, \
                    'main' AS referenced_schema, \
                    \"table\" AS referenced_table, \
                    \"to\" AS referenced_column \
             FROM pragma_foreign_key_list(?1) \
             ORDER BY id, seq",
            DatabaseType::SQLite,
        )?;
        let fk_rows = self.db.query(&q_fks, &[table.to_string().into()]).await?;

        let q_indexes = ReadOnlyQuery::parse_for(
            "SELECT name AS index_name, \"unique\" AS is_unique, origin \
             FROM pragma_index_list(?1) \
             WHERE origin != 'pk' \
             ORDER BY seq",
            DatabaseType::SQLite,
        )?;
        let index_rows = self
            .db
            .query(&q_indexes, &[table.to_string().into()])
            .await?;

        let pk_columns: Vec<String> = col_rows
            .iter()
            .filter_map(|row| {
                row.get("is_primary_key")
                    .and_then(json_truthy)
                    .filter(|value| *value)
                    .and_then(|_| row.get("column_name").and_then(|value| value.as_str()))
                    .map(|name| name.to_string())
            })
            .collect();

        let mut fk_map = std::collections::HashMap::new();
        let mut fk_constraints = std::collections::BTreeMap::<String, Vec<String>>::new();
        for row in &fk_rows {
            let id = row.get("id").and_then(|value| value.as_i64()).unwrap_or(0);
            let column_name = get_str(row, "column_name").to_string();
            let constraint_name = format!("fk_{table}_{id}");
            fk_map.insert(
                column_name.clone(),
                ForeignKeyRef {
                    referenced_schema: get_str(row, "referenced_schema").to_string(),
                    referenced_table: get_str(row, "referenced_table").to_string(),
                    referenced_column: get_str(row, "referenced_column").to_string(),
                    constraint_name: constraint_name.clone(),
                },
            );
            fk_constraints
                .entry(constraint_name)
                .or_default()
                .push(column_name);
        }

        let columns = col_rows
            .iter()
            .map(|row| {
                let column_name = get_str(row, "column_name").to_string();
                ColumnInfo {
                    column_name: column_name.clone(),
                    data_type: get_str(row, "data_type").to_string(),
                    is_nullable: get_str(row, "is_nullable") == "YES",
                    column_default: row
                        .get("column_default")
                        .and_then(|value| value.as_str().map(String::from)),
                    ordinal_position: row
                        .get("ordinal_position")
                        .and_then(|value| value.as_i64())
                        .and_then(|value| i32::try_from(value).ok())
                        .unwrap_or(0),
                    is_primary_key: pk_columns.contains(&column_name),
                    foreign_key: fk_map.remove(&column_name),
                }
            })
            .collect::<Vec<_>>();

        let mut indexes = Vec::new();
        let mut constraints = Vec::new();
        if !pk_columns.is_empty() {
            constraints.push(ConstraintInfo {
                name: format!("pk_{table}"),
                constraint_type: ConstraintKind::PrimaryKey,
                columns: pk_columns,
            });
        }

        for row in &index_rows {
            let index_name = get_str(row, "index_name").to_string();
            let q_index_cols = ReadOnlyQuery::parse_for(
                "SELECT name AS column_name FROM pragma_index_info(?1) ORDER BY seqno",
                DatabaseType::SQLite,
            )?;
            let column_rows = self
                .db
                .query(&q_index_cols, &[index_name.clone().into()])
                .await?;
            let index_columns = column_rows
                .iter()
                .filter_map(|value| {
                    value
                        .get("column_name")
                        .and_then(|column| column.as_str())
                        .map(|column| column.to_string())
                })
                .collect::<Vec<_>>();
            let is_unique = row.get("is_unique").and_then(json_truthy).unwrap_or(false);
            let origin = get_str(row, "origin");

            indexes.push(IndexInfo {
                name: index_name.clone(),
                columns: index_columns.clone(),
                is_unique,
            });

            if is_unique && origin == "u" && !index_columns.is_empty() {
                constraints.push(ConstraintInfo {
                    name: index_name,
                    constraint_type: ConstraintKind::Unique,
                    columns: index_columns,
                });
            }
        }

        for (constraint_name, columns) in fk_constraints {
            constraints.push(ConstraintInfo {
                name: constraint_name,
                constraint_type: ConstraintKind::ForeignKey,
                columns,
            });
        }

        let q_count = ReadOnlyQuery::parse_for(
            format!(
                "SELECT COUNT(*) AS row_count FROM {}",
                sqlite_table_identifier(table)?
            ),
            DatabaseType::SQLite,
        )?;
        let count_rows = self.db.query(&q_count, &[]).await?;
        let row_count = count_rows
            .first()
            .and_then(|row| row.get("row_count").and_then(|value| value.as_i64()))
            .unwrap_or(0);

        Ok(PhysicalTable {
            schema_name: schema.to_string(),
            table_name: table.to_string(),
            columns,
            constraints,
            indexes,
            row_count,
        })
    }

    async fn list_foreign_keys_postgres(
        &self,
        schema: &str,
    ) -> Result<Vec<ForeignKeyEdge>, DbError> {
        let params = [QueryParam::Text(schema.to_string())];
        let q = ReadOnlyQuery::parse(
            "SELECT kcu.table_name AS source_table, \
                    kcu.column_name AS source_column, \
                    ccu.table_name AS target_table, \
                    ccu.column_name AS target_column \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name \
               AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' \
               AND tc.table_schema = $1 \
             ORDER BY kcu.table_name, kcu.column_name",
        )?;
        let rows = self.db.query(&q, &params).await?;

        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(ForeignKeyEdge {
                    source_table: r.get("source_table")?.as_str()?.to_string(),
                    source_column: r.get("source_column")?.as_str()?.to_string(),
                    target_table: r.get("target_table")?.as_str()?.to_string(),
                    target_column: r.get("target_column")?.as_str()?.to_string(),
                })
            })
            .collect())
    }

    async fn list_foreign_keys_sqlite(&self) -> Result<Vec<ForeignKeyEdge>, DbError> {
        let tables = self.db.list_tables().await?;
        let q_fks = ReadOnlyQuery::parse_for(
            "SELECT \"from\" AS source_column, \
                    \"table\" AS target_table, \
                    \"to\" AS target_column \
             FROM pragma_foreign_key_list(?1) \
             ORDER BY id, seq",
            DatabaseType::SQLite,
        )?;
        let mut edges = Vec::new();
        for table in tables {
            let table_name = get_str(&table, "table_name").to_string();
            let rows = self.db.query(&q_fks, &[table_name.clone().into()]).await?;
            edges.extend(rows.into_iter().filter_map(|row| {
                Some(ForeignKeyEdge {
                    source_table: table_name.clone(),
                    source_column: row.get("source_column")?.as_str()?.to_string(),
                    target_table: row.get("target_table")?.as_str()?.to_string(),
                    target_column: row.get("target_column")?.as_str()?.to_string(),
                })
            }));
        }
        Ok(edges)
    }
}

/// Extract a string from a DbRow, returning "" on missing/non-string.
fn get_str<'a>(row: &'a DbRow, key: &str) -> &'a str {
    row.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn sqlite_table_type(kind: &str) -> TableType {
    match kind {
        "VIEW" => TableType::View,
        _ => TableType::BaseTable,
    }
}

fn sqlite_table_identifier(table: &str) -> Result<String, DbError> {
    let table = TableName::parse(table).map_err(|e| DbError::InvalidQuery(e.to_string()))?;
    Ok(table
        .as_str()
        .split('.')
        .map(|part| format!("\"{}\"", escape_identifier(part)))
        .collect::<Vec<_>>()
        .join("."))
}

fn postgres_qualified_table_identifier(schema: &str, table: &str) -> String {
    format!(
        "\"{}\".\"{}\"",
        escape_identifier(schema),
        escape_identifier(table)
    )
}

fn json_truthy(value: &serde_json::Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|number| number != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_fw_algebra::writable_db::{DdlStatement, WritableDatabase};
    use agent_fw_interpreter::{SqliteTargetDatabase, SqliteWritableDatabase};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct PostgresCountDb {
        queries: Mutex<Vec<String>>,
    }

    impl PostgresCountDb {
        fn queries(&self) -> Vec<String> {
            self.queries.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TargetDatabase for PostgresCountDb {
        fn database_type(&self) -> DatabaseType {
            DatabaseType::PostgreSQL
        }

        async fn query(
            &self,
            query: &ReadOnlyQuery,
            _params: &[QueryParam],
        ) -> Result<Vec<DbRow>, DbError> {
            self.queries.lock().unwrap().push(query.sql().to_string());
            let sql = query.sql();
            if sql.contains("FROM information_schema.columns") {
                return Ok(vec![DbRow::new(
                    vec![
                        "column_name".into(),
                        "data_type".into(),
                        "is_nullable".into(),
                        "column_default".into(),
                        "ordinal_position".into(),
                    ],
                    vec![
                        serde_json::json!("brand_id"),
                        serde_json::json!("integer"),
                        serde_json::json!("NO"),
                        serde_json::Value::Null,
                        serde_json::json!(1),
                    ],
                )]);
            }
            if sql.contains("tc.constraint_type = 'PRIMARY KEY'") {
                return Ok(vec![DbRow::new(
                    vec!["column_name".into()],
                    vec![serde_json::json!("brand_id")],
                )]);
            }
            if sql.contains("COUNT(*)::bigint AS row_count FROM \"public\".\"dim_brands\"") {
                return Ok(vec![DbRow::new(
                    vec!["row_count".into()],
                    vec![serde_json::json!(13)],
                )]);
            }
            Ok(vec![])
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

    #[tokio::test]
    async fn postgres_introspection_uses_exact_count_for_physical_table() {
        let db = Arc::new(PostgresCountDb::default());
        let service = IntrospectionService::new(db.clone());

        let table = service
            .introspect_table("public", "dim_brands")
            .await
            .unwrap();

        assert_eq!(table.row_count, 13);
        assert!(db
            .queries()
            .iter()
            .any(|query| query.contains("COUNT(*)::bigint AS row_count")));
        assert!(!db
            .queries()
            .iter()
            .any(|query| query.contains("reltuples::bigint AS row_count")));
    }

    #[tokio::test]
    async fn sqlite_introspection_discovers_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.db");

        let writer = SqliteWritableDatabase::open(&path).unwrap();
        let parent = DdlStatement::parse_for(
            "CREATE TABLE parents (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            DatabaseType::SQLite,
        )
        .unwrap();
        let child = DdlStatement::parse_for(
            "CREATE TABLE children (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_id INTEGER NOT NULL REFERENCES parents(id),
                status TEXT NOT NULL
            )",
            DatabaseType::SQLite,
        )
        .unwrap();
        writer.execute_ddl(&parent).await.unwrap();
        writer.execute_ddl(&child).await.unwrap();
        writer
            .execute_dml(
                &agent_fw_algebra::writable_db::DmlStatement::parse_for(
                    "INSERT INTO parents (name) VALUES ('p1')",
                    DatabaseType::SQLite,
                )
                .unwrap(),
                &[],
            )
            .await
            .unwrap();
        writer
            .execute_dml(
                &agent_fw_algebra::writable_db::DmlStatement::parse_for(
                    "INSERT INTO children (parent_id, status) VALUES (1, 'active')",
                    DatabaseType::SQLite,
                )
                .unwrap(),
                &[],
            )
            .await
            .unwrap();

        let db = Arc::new(SqliteTargetDatabase::open(&path).unwrap());
        let service = IntrospectionService::new(db);

        assert_eq!(
            service.list_schemas().await.unwrap(),
            vec!["main".to_string()]
        );

        let tables = service.list_tables("main").await.unwrap();
        assert_eq!(tables.len(), 2);
        let listed_child = tables
            .iter()
            .find(|table| table.table_name == "children")
            .expect("children table should be listed");
        assert_eq!(listed_child.row_count, Some(1));

        let child_table = service.introspect_table("main", "children").await.unwrap();
        assert_eq!(child_table.columns.len(), 3);
        assert_eq!(child_table.row_count, 1);
        assert!(child_table
            .columns
            .iter()
            .any(|column| column.foreign_key.is_some()));

        let edges = service.list_foreign_keys("main").await.unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_table, "children");
        assert_eq!(edges[0].target_table, "parents");
    }
}
