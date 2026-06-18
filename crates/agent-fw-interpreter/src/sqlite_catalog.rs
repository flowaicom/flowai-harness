//! SQLite-backed catalog handle with scoped DataCatalog + CatalogWriter views.
//!
//! Uses rusqlite with WAL mode. FTS5 is enabled via the `bundled` feature
//! of rusqlite. All operations go through `spawn_blocking`.

use agent_fw_catalog::{
    relation_kind, CatalogEntry, CatalogError, CatalogKind, CatalogRef, CatalogRelation,
    CatalogScope, CatalogWriter, DataCatalog, JoinHop, JoinPath, RelationshipMetadata,
    TableMetadata,
};
use async_trait::async_trait;
use rusqlite::Connection;
use std::collections::{HashSet, VecDeque};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing;

const MATERIALIZED_RELATION_DESCRIPTION_PREFIX: &str = "[materialized_relationship] ";
const MATERIALIZED_RELATION_DESCRIPTION_LIKE_PATTERN: &str = "[materialized_relationship] %";

/// SQLite-backed catalog store.
///
/// Use [`SqliteCatalog::with_scope`] to obtain the reader/writer view for a
/// tenant/workspace.
#[derive(Clone)]
pub struct SqliteCatalog {
    conn: Arc<Mutex<Connection>>,
}

/// SQLite catalog view bound to one tenant/workspace scope.
#[derive(Clone)]
pub struct ScopedSqliteCatalog {
    catalog: SqliteCatalog,
    scope: CatalogScope,
}

impl SqliteCatalog {
    /// Open (or create) a catalog backed by a file.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, CatalogError> {
        let conn = Connection::open(path)
            .map_err(|e| CatalogError::Unavailable(format!("Failed to open database: {e}")))?;
        Self::from_connection(conn, true)
    }

    /// Create an in-memory catalog (for testing).
    pub fn in_memory() -> Result<Self, CatalogError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| CatalogError::Unavailable(format!("Failed to open in-memory: {e}")))?;
        Self::from_connection(conn, false)
    }

    fn from_connection(conn: Connection, use_wal: bool) -> Result<Self, CatalogError> {
        let pragmas = if use_wal {
            "PRAGMA journal_mode = WAL;\nPRAGMA busy_timeout = 5000;"
        } else {
            "PRAGMA busy_timeout = 5000;"
        };

        conn.execute_batch(&format!(
            "{pragmas}
            CREATE TABLE IF NOT EXISTS catalog_entries (
                tenant_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                id TEXT NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT,
                content TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '[]',
                metadata TEXT NOT NULL DEFAULT '{{}}',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (tenant_id, workspace_id, id)
            );
            CREATE TABLE IF NOT EXISTS catalog_relations (
                tenant_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                description TEXT,
                PRIMARY KEY (tenant_id, workspace_id, source_id, target_id, kind)
            );
            CREATE INDEX IF NOT EXISTS idx_catalog_entries_scope_kind
                ON catalog_entries (tenant_id, workspace_id, kind);
            CREATE INDEX IF NOT EXISTS idx_catalog_entries_scope_qname
                ON catalog_entries (tenant_id, workspace_id, qualified_name);
            CREATE INDEX IF NOT EXISTS idx_catalog_entries_scope_kind_name
                ON catalog_entries (tenant_id, workspace_id, kind, name);
            CREATE INDEX IF NOT EXISTS idx_catalog_relations_scope_source
                ON catalog_relations (tenant_id, workspace_id, source_id);
            CREATE INDEX IF NOT EXISTS idx_catalog_relations_scope_target
                ON catalog_relations (tenant_id, workspace_id, target_id);"
        ))
        .map_err(|e| CatalogError::Unavailable(format!("Failed to initialize schema: {e}")))?;

        drop_sqlite_search_artifacts(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Bind this catalog to an explicit tenant/workspace scope.
    pub fn with_scope(&self, scope: CatalogScope) -> ScopedSqliteCatalog {
        ScopedSqliteCatalog {
            catalog: self.clone(),
            scope,
        }
    }
}

fn drop_sqlite_search_artifacts(conn: &Connection) -> Result<(), CatalogError> {
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS catalog_ai;
        DROP TRIGGER IF EXISTS catalog_ad;
        DROP TRIGGER IF EXISTS catalog_au;
        DROP TABLE IF EXISTS catalog_fts;
        ",
    )
    .map_err(|e| CatalogError::Unavailable(format!("Drop SQLite FTS artifacts: {e}")))?;

    let mut stmt = conn
        .prepare("PRAGMA table_info(catalog_entries)")
        .map_err(|e| CatalogError::Unavailable(format!("Inspect catalog_entries: {e}")))?;
    let column_names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| CatalogError::Unavailable(format!("Read catalog_entries columns: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Read catalog_entries columns: {e}")))?;

    if column_names.iter().any(|name| name == "search_content") {
        conn.execute("ALTER TABLE catalog_entries DROP COLUMN search_content", [])
            .map_err(|e| CatalogError::Unavailable(format!("Drop search_content column: {e}")))?;
    }
    Ok(())
}

impl ScopedSqliteCatalog {
    pub fn scope(&self) -> &CatalogScope {
        &self.scope
    }
}

fn row_to_entry(
    conn: &Connection,
    scope: &CatalogScope,
    row: &rusqlite::Row,
) -> rusqlite::Result<CatalogEntry> {
    let id: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let name: String = row.get(2)?;
    let qualified_name: Option<String> = row.get(3)?;
    let content: String = row.get(4)?;
    let tags_str: String = row.get(5)?;
    let metadata_str: String = row.get(6)?;

    // Forward-compatible fallback: unknown kind → Special (logged, not silent).
    let kind = CatalogKind::from_str(&kind_str).unwrap_or_else(|_| {
        tracing::warn!(kind = %kind_str, id = %id, "Unknown CatalogKind, falling back to Special");
        CatalogKind::Special
    });
    let tags: Vec<String> = serde_json::from_str(&tags_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;

    // Load relations for this entry — errors propagate, not silently swallowed.
    let mut stmt = conn.prepare_cached(
        "SELECT target_id, kind, description FROM catalog_relations
         WHERE tenant_id = ?1 AND workspace_id = ?2 AND source_id = ?3",
    )?;
    let links: Vec<CatalogRelation> = stmt
        .query_map(
            rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str(), id],
            |r| {
                Ok(CatalogRelation {
                    target_id: r.get(0)?,
                    kind: r.get(1)?,
                    description: r.get(2)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(CatalogEntry {
        id,
        kind,
        name,
        qualified_name,
        content,
        tags,
        links,
        metadata,
    })
}

fn get_entry_by_id(
    conn: &Connection,
    scope: &CatalogScope,
    id: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1 AND workspace_id = ?2 AND id = ?3",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

    let result = stmt.query_row(
        rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str(), id],
        |row| row_to_entry(conn, scope, row),
    );

    match result {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CatalogError::Unavailable(e.to_string())),
    }
}

fn save_entry(
    conn: &Connection,
    scope: &CatalogScope,
    entry: &CatalogEntry,
) -> Result<(), CatalogError> {
    if let Some(old_entry) = get_entry_by_id(conn, scope, &entry.id)? {
        cleanup_relationship_table_edges(conn, scope, &old_entry, std::slice::from_ref(&entry.id))?;
    }

    let tags_json = serde_json::to_string(&entry.tags)
        .map_err(|e| CatalogError::Unavailable(format!("Failed to serialize tags: {e}")))?;
    let metadata_json = serde_json::to_string(&entry.metadata)
        .map_err(|e| CatalogError::Unavailable(format!("Failed to serialize metadata: {e}")))?;

    conn.execute(
        "INSERT OR REPLACE INTO catalog_entries
         (tenant_id, workspace_id, id, kind, name, qualified_name, content, tags, metadata, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
        rusqlite::params![
            scope.tenant_id.as_str(),
            scope.workspace_id.as_str(),
            entry.id,
            entry.kind.as_str(),
            entry.name,
            entry.qualified_name,
            entry.content,
            tags_json,
            metadata_json,
        ],
    )
    .map_err(|e| CatalogError::Unavailable(format!("Save failed: {e}")))?;

    if entry.kind == CatalogKind::Relationship {
        conn.execute(
            "DELETE FROM catalog_relations
             WHERE tenant_id = ?1 AND workspace_id = ?2 AND source_id = ?3",
            rusqlite::params![
                scope.tenant_id.as_str(),
                scope.workspace_id.as_str(),
                entry.id
            ],
        )
        .map_err(|e| CatalogError::Unavailable(format!("Delete relations failed: {e}")))?;
    } else {
        conn.execute(
            "DELETE FROM catalog_relations
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND source_id = ?3
               AND (description IS NULL OR description NOT LIKE ?4)",
            rusqlite::params![
                scope.tenant_id.as_str(),
                scope.workspace_id.as_str(),
                entry.id,
                MATERIALIZED_RELATION_DESCRIPTION_LIKE_PATTERN
            ],
        )
        .map_err(|e| CatalogError::Unavailable(format!("Delete relations failed: {e}")))?;
    }

    for link in &entry.links {
        insert_relation(conn, scope, &entry.id, link)?;
    }

    materialize_relationship_table_edges(conn, scope, entry)?;

    Ok(())
}

fn insert_relation(
    conn: &Connection,
    scope: &CatalogScope,
    source_id: &str,
    link: &CatalogRelation,
) -> Result<(), CatalogError> {
    conn.execute(
        "INSERT OR IGNORE INTO catalog_relations
         (tenant_id, workspace_id, source_id, target_id, kind, description)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            scope.tenant_id.as_str(),
            scope.workspace_id.as_str(),
            source_id,
            link.target_id,
            link.kind,
            link.description
        ],
    )
    .map_err(|e| CatalogError::Unavailable(format!("Insert relation failed: {e}")))?;
    Ok(())
}

fn materialize_relationship_table_edges(
    conn: &Connection,
    scope: &CatalogScope,
    entry: &CatalogEntry,
) -> Result<(), CatalogError> {
    for (source_id, link) in validated_relationship_table_edges(conn, scope, entry)? {
        insert_relation(conn, scope, &source_id, &link)?;
    }
    Ok(())
}

fn cleanup_relationship_table_edges(
    conn: &Connection,
    scope: &CatalogScope,
    entry: &CatalogEntry,
    excluded_relationship_ids: &[String],
) -> Result<(), CatalogError> {
    for (source_id, link) in relationship_table_edges(entry) {
        if relationship_table_edge_has_support(
            conn,
            scope,
            &source_id,
            &link,
            excluded_relationship_ids,
        )? {
            continue;
        }
        conn.execute(
            "DELETE FROM catalog_relations
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND source_id = ?3
               AND target_id = ?4
               AND kind = ?5
               AND description = ?6",
            rusqlite::params![
                scope.tenant_id.as_str(),
                scope.workspace_id.as_str(),
                source_id,
                link.target_id,
                link.kind,
                link.description
            ],
        )
        .map_err(|e| {
            CatalogError::Unavailable(format!("Delete materialized relation failed: {e}"))
        })?;
    }
    Ok(())
}

fn relationship_table_edge_has_support(
    conn: &Connection,
    scope: &CatalogScope,
    source_id: &str,
    link: &CatalogRelation,
    excluded_relationship_ids: &[String],
) -> Result<bool, CatalogError> {
    for entry in relationship_entries(conn, scope)?
        .into_iter()
        .filter(|entry| !excluded_relationship_ids.contains(&entry.id))
    {
        if validated_relationship_table_edges(conn, scope, &entry)?
            .into_iter()
            .any(|(supported_source_id, supported_link)| {
                supported_source_id == source_id
                    && supported_link.target_id == link.target_id
                    && supported_link.kind == link.kind
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn relationship_entries(
    conn: &Connection,
    scope: &CatalogScope,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND kind = 'relationship'",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

    let entries = stmt
        .query_map(
            rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str()],
            |row| row_to_entry(conn, scope, row),
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;
    Ok(entries)
}

fn relationship_table_edges(entry: &CatalogEntry) -> Vec<(String, CatalogRelation)> {
    if entry.kind != CatalogKind::Relationship {
        return vec![];
    }

    let Ok(metadata) = serde_json::from_value::<RelationshipMetadata>(entry.metadata.clone())
    else {
        return vec![];
    };
    if metadata.source_table_id.is_empty() || metadata.target_table_id.is_empty() {
        return vec![];
    }

    vec![
        (
            entry.id.clone(),
            CatalogRelation {
                target_id: metadata.source_table_id.clone(),
                kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                description: materialized_relationship_description("relationship source table"),
            },
        ),
        (
            entry.id.clone(),
            CatalogRelation {
                target_id: metadata.target_table_id.clone(),
                kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                description: materialized_relationship_description("relationship target table"),
            },
        ),
        (
            metadata.source_table_id.clone(),
            CatalogRelation {
                target_id: metadata.target_table_id.clone(),
                kind: relation_kind::REFERENCES_TABLE.to_string(),
                description: relationship_join_description_from_metadata(
                    &metadata.source_table,
                    &metadata.source_column,
                    &metadata.target_table,
                    &metadata.target_column,
                ),
            },
        ),
        (
            metadata.target_table_id.clone(),
            CatalogRelation {
                target_id: metadata.source_table_id.clone(),
                kind: relation_kind::REFERENCED_BY_TABLE.to_string(),
                description: relationship_join_description_from_metadata(
                    &metadata.target_table,
                    &metadata.target_column,
                    &metadata.source_table,
                    &metadata.source_column,
                ),
            },
        ),
    ]
}

fn materialized_relationship_description(description: &str) -> Option<String> {
    Some(format!(
        "{MATERIALIZED_RELATION_DESCRIPTION_PREFIX}{description}"
    ))
}

fn validated_relationship_table_edges(
    conn: &Connection,
    scope: &CatalogScope,
    entry: &CatalogEntry,
) -> Result<Vec<(String, CatalogRelation)>, CatalogError> {
    if entry.kind != CatalogKind::Relationship {
        return Ok(vec![]);
    }

    let Ok(metadata) = serde_json::from_value::<RelationshipMetadata>(entry.metadata.clone())
    else {
        return Ok(vec![]);
    };
    if !relationship_targets_match_database(conn, scope, &metadata)? {
        return Ok(vec![]);
    }

    Ok(relationship_table_edges(entry))
}

fn relationship_targets_match_database(
    conn: &Connection,
    scope: &CatalogScope,
    metadata: &RelationshipMetadata,
) -> Result<bool, CatalogError> {
    let Some(source_database_id) = table_database_id(conn, scope, &metadata.source_table_id)?
    else {
        return Ok(false);
    };
    let Some(target_database_id) = table_database_id(conn, scope, &metadata.target_table_id)?
    else {
        return Ok(false);
    };
    Ok(source_database_id == metadata.database_id && target_database_id == metadata.database_id)
}

fn table_database_id(
    conn: &Connection,
    scope: &CatalogScope,
    table_id: &str,
) -> Result<Option<String>, CatalogError> {
    let Some(entry) = get_entry_by_id(conn, scope, table_id)? else {
        return Ok(None);
    };
    if entry.kind != CatalogKind::Table {
        return Ok(None);
    }
    let Ok(metadata) = serde_json::from_value::<TableMetadata>(entry.metadata) else {
        return Ok(None);
    };
    Ok(Some(metadata.database_id))
}

fn relationship_join_description_from_metadata(
    from_table: &str,
    from_column: &str,
    to_table: &str,
    to_column: &str,
) -> Option<String> {
    let description = if from_column.is_empty() || to_column.is_empty() {
        format!("{from_table} -> {to_table}")
    } else {
        format!("{from_table}.{from_column} -> {to_table}.{to_column}")
    };
    materialized_relationship_description(&description)
}

#[cfg(test)]
fn is_materialized_relationship_description(description: Option<&str>) -> bool {
    description
        .map(|description| description.starts_with(MATERIALIZED_RELATION_DESCRIPTION_PREFIX))
        .unwrap_or(false)
}

fn cleanup_relationships_for_delete(
    conn: &Connection,
    scope: &CatalogScope,
    ids: &[String],
) -> Result<(), CatalogError> {
    for id in ids {
        if let Some(entry) = get_entry_by_id(conn, scope, id)? {
            cleanup_relationship_table_edges(conn, scope, &entry, ids)?;
        }
    }
    Ok(())
}

fn resolve_ref_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    reference: &CatalogRef,
) -> Result<Option<CatalogEntry>, CatalogError> {
    match reference {
        CatalogRef::Id(id) => get_entry_by_id(conn, scope, id),
        CatalogRef::QualifiedName {
            kind: Some(kind),
            qualified_name,
        } => get_by_exact_qualified_name(conn, scope, *kind, qualified_name),
        CatalogRef::QualifiedName {
            kind: None,
            qualified_name,
        } => get_by_any_exact_qualified_name(conn, scope, qualified_name),
        CatalogRef::Name { kind, name, schema } => {
            if let Some(schema) = schema {
                let qualified_name = format!("{schema}.{name}");
                return get_by_exact_qualified_name(conn, scope, *kind, &qualified_name);
            }
            get_by_exact_name(conn, scope, *kind, name)
        }
    }
}

fn get_by_exact_qualified_name(
    conn: &Connection,
    scope: &CatalogScope,
    kind: CatalogKind,
    qualified_name: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND kind = ?3
               AND qualified_name = ?4",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let result = stmt.query_row(
        rusqlite::params![
            scope.tenant_id.as_str(),
            scope.workspace_id.as_str(),
            kind.as_str(),
            qualified_name
        ],
        |row| row_to_entry(conn, scope, row),
    );
    match result {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CatalogError::Unavailable(e.to_string())),
    }
}

fn get_by_any_exact_qualified_name(
    conn: &Connection,
    scope: &CatalogScope,
    qualified_name: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND qualified_name = ?3",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let result = stmt.query_row(
        rusqlite::params![
            scope.tenant_id.as_str(),
            scope.workspace_id.as_str(),
            qualified_name
        ],
        |row| row_to_entry(conn, scope, row),
    );
    match result {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CatalogError::Unavailable(e.to_string())),
    }
}

fn get_by_exact_name(
    conn: &Connection,
    scope: &CatalogScope,
    kind: CatalogKind,
    name: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    Ok(get_by_exact_name_entries(conn, scope, kind, name)?
        .into_iter()
        .next())
}

fn get_by_exact_name_entries(
    conn: &Connection,
    scope: &CatalogScope,
    kind: CatalogKind,
    name: &str,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND kind = ?3
               AND name = ?4
             ORDER BY qualified_name, id",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let entries = stmt
        .query_map(
            rusqlite::params![
                scope.tenant_id.as_str(),
                scope.workspace_id.as_str(),
                kind.as_str(),
                name
            ],
            |row| row_to_entry(conn, scope, row),
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;
    Ok(entries)
}

fn related_reverse_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let (sql, params): (&str, Vec<String>) = match relation_type {
        Some(kind) => (
            "SELECT r.source_id
             FROM catalog_relations r
             JOIN catalog_entries source
               ON source.tenant_id = r.tenant_id
              AND source.workspace_id = r.workspace_id
              AND source.id = r.source_id
             JOIN catalog_entries target
               ON target.tenant_id = r.tenant_id
              AND target.workspace_id = r.workspace_id
              AND target.id = r.target_id
             WHERE r.tenant_id = ?1
               AND r.workspace_id = ?2
               AND r.target_id = ?3
               AND r.kind = ?4",
            vec![
                scope.tenant_id.as_str().to_string(),
                scope.workspace_id.as_str().to_string(),
                id.to_string(),
                kind.to_string(),
            ],
        ),
        None => (
            "SELECT r.source_id
             FROM catalog_relations r
             JOIN catalog_entries source
               ON source.tenant_id = r.tenant_id
              AND source.workspace_id = r.workspace_id
              AND source.id = r.source_id
             JOIN catalog_entries target
               ON target.tenant_id = r.tenant_id
              AND target.workspace_id = r.workspace_id
              AND target.id = r.target_id
             WHERE r.tenant_id = ?1
               AND r.workspace_id = ?2
               AND r.target_id = ?3",
            vec![
                scope.tenant_id.as_str().to_string(),
                scope.workspace_id.as_str().to_string(),
                id.to_string(),
            ],
        ),
    };

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| row.get(0))
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;

    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(entry) = get_entry_by_id(conn, scope, &id)? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn get_columns_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    table_ref: &str,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    if let Some(table) = resolve_ref_in_scope(conn, scope, &CatalogRef::parse_table(table_ref))? {
        let columns =
            get_related_entries_in_scope(conn, scope, &table.id, Some(relation_kind::HAS_COLUMN))?
                .into_iter()
                .filter(|entry| entry.kind == CatalogKind::Column)
                .collect::<Vec<_>>();
        if !columns.is_empty() {
            return Ok(columns);
        }
    }

    let pattern = if table_ref.contains('.') {
        format!("{table_ref}.%")
    } else {
        format!("%.{}.%", escape_like(table_ref))
    };
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = ?1
               AND workspace_id = ?2
               AND kind = 'column'
               AND qualified_name LIKE ?3 ESCAPE '\\'
             ORDER BY name",
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

    let entries: Vec<CatalogEntry> = stmt
        .query_map(
            rusqlite::params![
                scope.tenant_id.as_str(),
                scope.workspace_id.as_str(),
                pattern
            ],
            |row| row_to_entry(conn, scope, row),
        )
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;
    Ok(entries)
}

fn get_related_entries_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let ids = related_ids_in_scope(conn, scope, id, relation_type)?;
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(entry) = get_entry_by_id(conn, scope, &id)? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn related_ids_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<String>, CatalogError> {
    let (sql, params): (&str, Vec<String>) = match relation_type {
        Some(kind) => (
            "SELECT r.target_id
             FROM catalog_relations r
             JOIN catalog_entries source
               ON source.tenant_id = r.tenant_id
              AND source.workspace_id = r.workspace_id
              AND source.id = r.source_id
             JOIN catalog_entries target
               ON target.tenant_id = r.tenant_id
              AND target.workspace_id = r.workspace_id
              AND target.id = r.target_id
             WHERE r.tenant_id = ?1
               AND r.workspace_id = ?2
               AND r.source_id = ?3
               AND r.kind = ?4",
            vec![
                scope.tenant_id.as_str().to_string(),
                scope.workspace_id.as_str().to_string(),
                id.to_string(),
                kind.to_string(),
            ],
        ),
        None => (
            "SELECT r.target_id
             FROM catalog_relations r
             JOIN catalog_entries source
               ON source.tenant_id = r.tenant_id
              AND source.workspace_id = r.workspace_id
              AND source.id = r.source_id
             JOIN catalog_entries target
               ON target.tenant_id = r.tenant_id
              AND target.workspace_id = r.workspace_id
              AND target.id = r.target_id
             WHERE r.tenant_id = ?1
               AND r.workspace_id = ?2
               AND r.source_id = ?3",
            vec![
                scope.tenant_id.as_str().to_string(),
                scope.workspace_id.as_str().to_string(),
                id.to_string(),
            ],
        ),
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let ids = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| row.get(0))
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;
    Ok(ids)
}

fn get_enum_values_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    column_ref: &str,
) -> Result<Vec<String>, CatalogError> {
    let Some(column) = resolve_ref_in_scope(conn, scope, &CatalogRef::parse_column(column_ref))?
    else {
        return legacy_packed_enum_values_in_scope(conn, scope, column_ref);
    };

    let mut enum_entries =
        related_reverse_in_scope(conn, scope, &column.id, Some(relation_kind::ENUM_VALUE_OF))?;
    enum_entries.extend(related_reverse_in_scope(
        conn,
        scope,
        &column.id,
        Some("enum_of"),
    )?);
    enum_entries.retain(|entry| entry.kind == CatalogKind::Enum);
    enum_entries.sort_by(|a, b| {
        let rank_a = a
            .metadata
            .get("rank")
            .and_then(|rank| rank.as_u64())
            .unwrap_or(u64::MAX);
        let rank_b = b
            .metadata
            .get("rank")
            .and_then(|rank| rank.as_u64())
            .unwrap_or(u64::MAX);
        rank_a.cmp(&rank_b).then_with(|| a.name.cmp(&b.name))
    });

    if enum_entries.is_empty() {
        return legacy_packed_enum_values_in_scope(conn, scope, &column.id);
    }

    Ok(enum_entries
        .into_iter()
        .map(|entry| {
            entry
                .metadata
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap_or(&entry.name)
                .to_string()
        })
        .collect())
}

fn legacy_packed_enum_values_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    id: &str,
) -> Result<Vec<String>, CatalogError> {
    let result: Result<String, _> = conn.query_row(
        "SELECT metadata
         FROM catalog_entries
         WHERE tenant_id = ?1
           AND workspace_id = ?2
           AND id = ?3
           AND kind = 'enum'",
        rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str(), id],
        |row| row.get(0),
    );

    match result {
        Ok(metadata_str) => {
            let metadata: serde_json::Value = serde_json::from_str(&metadata_str).map_err(|e| {
                CatalogError::Unavailable(format!("Corrupted metadata JSON for enum {id}: {e}"))
            })?;
            match metadata.get("values") {
                Some(values) => {
                    serde_json::from_value::<Vec<String>>(values.clone()).map_err(|e| {
                        CatalogError::Unavailable(format!("Corrupted enum values for {id}: {e}"))
                    })
                }
                None => Ok(vec![]),
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(vec![]),
        Err(e) => Err(CatalogError::Unavailable(e.to_string())),
    }
}

fn find_join_path_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    from_table: &str,
    to_table: &str,
) -> Result<Option<JoinPath>, CatalogError> {
    let Some(from) = resolve_ref_in_scope(conn, scope, &CatalogRef::parse_table(from_table))?
    else {
        return Ok(None);
    };
    let Some(to) = resolve_ref_in_scope(conn, scope, &CatalogRef::parse_table(to_table))? else {
        return Ok(None);
    };

    // Load the scope's relationship vertices ONCE (O(relationships)), not once
    // per BFS node, so the typed hop metadata can be attached without a
    // per-node rescan. Each entry is paired with its decoded metadata.
    let mut relationship_index: Vec<(String, RelationshipMetadata)> = Vec::new();
    for entry in relationship_entries(conn, scope)? {
        let Ok(metadata) = serde_json::from_value::<RelationshipMetadata>(entry.metadata.clone())
        else {
            continue;
        };
        if relationship_targets_match_database(conn, scope, &metadata)? {
            relationship_index.push((entry.id, metadata));
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

    queue.push_back((from.id.clone(), vec![from.id.clone()]));
    visited.insert(from.id);

    while let Some((current, path)) = queue.pop_front() {
        if current == to.id {
            let mut steps = Vec::new();
            for step_id in &path {
                if let Some(entry) = get_entry_by_id(conn, scope, step_id)? {
                    steps.push(entry);
                }
            }
            let length = steps.len();
            let hops = build_join_hops(&path, &relationship_index);
            return Ok(Some(JoinPath {
                steps,
                length,
                hops,
            }));
        }

        if path.len() > 10 {
            continue;
        }

        for neighbor in table_neighbors_in_scope(conn, scope, &current)? {
            if visited.insert(neighbor.clone()) {
                let mut new_path = path.clone();
                new_path.push(neighbor.clone());
                queue.push_back((neighbor, new_path));
            }
        }
    }

    Ok(None)
}

/// Build typed [`JoinHop`] metadata for each consecutive id pair in `path`.
///
/// `relationship_index` is the pre-decoded set of relationship vertices for the
/// scope. For each `(prev, next)` pair we find the relationship vertex whose
/// `(source_table_id, target_table_id)` matches in either direction and emit the
/// join columns. Pairs without a matching relationship vertex (non-FK table
/// links) yield a hop carrying only the relation kind.
fn build_join_hops(
    path: &[String],
    relationship_index: &[(String, RelationshipMetadata)],
) -> Vec<JoinHop> {
    path.windows(2)
        .map(|pair| {
            let (prev, next) = (&pair[0], &pair[1]);
            for (relationship_id, metadata) in relationship_index {
                if &metadata.source_table_id == prev && &metadata.target_table_id == next {
                    return JoinHop {
                        relation_kind: relation_kind::REFERENCES_TABLE.to_string(),
                        from_column: Some(metadata.source_column.clone()),
                        to_column: Some(metadata.target_column.clone()),
                        join_type: Some("inner".to_string()),
                        description: relationship_join_description_from_metadata(
                            &metadata.source_table,
                            &metadata.source_column,
                            &metadata.target_table,
                            &metadata.target_column,
                        ),
                        relationship_id: Some(relationship_id.clone()),
                    };
                }
                if &metadata.target_table_id == prev && &metadata.source_table_id == next {
                    return JoinHop {
                        relation_kind: relation_kind::REFERENCED_BY_TABLE.to_string(),
                        from_column: Some(metadata.target_column.clone()),
                        to_column: Some(metadata.source_column.clone()),
                        join_type: Some("inner".to_string()),
                        description: relationship_join_description_from_metadata(
                            &metadata.target_table,
                            &metadata.target_column,
                            &metadata.source_table,
                            &metadata.source_column,
                        ),
                        relationship_id: Some(relationship_id.clone()),
                    };
                }
            }
            JoinHop {
                relation_kind: relation_kind::REFERENCES_TABLE.to_string(),
                from_column: None,
                to_column: None,
                join_type: None,
                description: None,
                relationship_id: None,
            }
        })
        .collect()
}

fn table_neighbors_in_scope(
    conn: &Connection,
    scope: &CatalogScope,
    table_id: &str,
) -> Result<Vec<String>, CatalogError> {
    let mut neighbors =
        related_ids_in_scope(conn, scope, table_id, Some(relation_kind::REFERENCES_TABLE))?;
    neighbors.extend(related_ids_in_scope(
        conn,
        scope,
        table_id,
        Some(relation_kind::REFERENCED_BY_TABLE),
    )?);

    let incoming =
        related_reverse_in_scope(conn, scope, table_id, Some(relation_kind::REFERENCES_TABLE))?;
    neighbors.extend(
        incoming
            .into_iter()
            .filter(|entry| entry.kind == CatalogKind::Table)
            .map(|entry| entry.id),
    );
    let incoming_referenced_by = related_reverse_in_scope(
        conn,
        scope,
        table_id,
        Some(relation_kind::REFERENCED_BY_TABLE),
    )?;
    neighbors.extend(
        incoming_referenced_by
            .into_iter()
            .filter(|entry| entry.kind == CatalogKind::Table)
            .map(|entry| entry.id),
    );
    neighbors.sort();
    neighbors.dedup();
    Ok(neighbors)
}

fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[async_trait]
impl DataCatalog for ScopedSqliteCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            get_entry_by_id(&conn, &scope, &id)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            let mut results = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Some(entry) = get_entry_by_id(&conn, &scope, id)? {
                    results.push(entry);
                }
            }
            Ok(results)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let kind_str = kind.as_str().to_string();
        let qualified_name = qualified_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, kind, name, qualified_name, content, tags, metadata
                     FROM catalog_entries
                     WHERE tenant_id = ?1
                       AND workspace_id = ?2
                       AND kind = ?3
                       AND qualified_name = ?4",
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

            let result = stmt.query_row(
                rusqlite::params![
                    scope.tenant_id.as_str(),
                    scope.workspace_id.as_str(),
                    kind_str,
                    qualified_name
                ],
                |row| row_to_entry(&conn, &scope, row),
            );

            match result {
                Ok(entry) => Ok(Some(entry)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(CatalogError::Unavailable(e.to_string())),
            }
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            get_by_exact_name_entries(&conn, &scope, kind, &name)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn resolve_ref(
        &self,
        reference: &CatalogRef,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let reference = reference.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            resolve_ref_in_scope(&conn, &scope, &reference)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let kind_str = kind.as_str().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, name, qualified_name, content, tags, metadata
                     FROM catalog_entries
                     WHERE tenant_id = ?1 AND workspace_id = ?2 AND kind = ?3
                     ORDER BY name
                     LIMIT ?4",
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

            let entries: Vec<CatalogEntry> = stmt
                .query_map(
                    rusqlite::params![
                        scope.tenant_id.as_str(),
                        scope.workspace_id.as_str(),
                        kind_str,
                        limit as i64
                    ],
                    |row| row_to_entry(&conn, &scope, row),
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;

            Ok(entries)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let id = id.to_string();
        let relation_type = relation_type.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            get_related_entries_in_scope(&conn, &scope, &id, relation_type.as_deref())
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_related_reverse(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let id = id.to_string();
        let relation_type = relation_type.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            related_reverse_in_scope(&conn, &scope, &id, relation_type.as_deref())
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn find_join_path(
        &self,
        from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let from = from_table.to_string();
        let to = to_table.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            find_join_path_in_scope(&conn, &scope, &from, &to)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, name, qualified_name, content, tags, metadata
                     FROM catalog_entries
                     WHERE tenant_id = ?1 AND workspace_id = ?2 AND kind = 'table'
                     ORDER BY name",
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?;

            let entries: Vec<CatalogEntry> = stmt
                .query_map(
                    rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str()],
                    |row| row_to_entry(&conn, &scope, row),
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CatalogError::Unavailable(format!("Row error: {e}")))?;

            Ok(entries)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let table_name = table_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            get_columns_in_scope(&conn, &scope, &table_name)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let column_id = column_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            get_enum_values_in_scope(&conn, &scope, &column_id)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn health_check(&self) -> Result<(), CatalogError> {
        let conn = self.catalog.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;
            conn.execute_batch("SELECT 1")
                .map_err(|e| CatalogError::Unavailable(format!("Health check: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }
}

#[async_trait]
impl CatalogWriter for ScopedSqliteCatalog {
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;

            let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
            for item in &items {
                save_entry(&conn, &scope, item)?;
            }
            Ok(ids)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;

            cleanup_relationships_for_delete(&conn, &scope, &ids)?;

            let mut count = 0u32;
            for id in &ids {
                let rows = conn
                    .execute(
                        "DELETE FROM catalog_entries
                         WHERE tenant_id = ?1 AND workspace_id = ?2 AND id = ?3",
                        rusqlite::params![
                            scope.tenant_id.as_str(),
                            scope.workspace_id.as_str(),
                            id
                        ],
                    )
                    .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
                conn.execute(
                    "DELETE FROM catalog_relations
                     WHERE tenant_id = ?1
                       AND workspace_id = ?2
                       AND (source_id = ?3 OR target_id = ?3)",
                    rusqlite::params![scope.tenant_id.as_str(), scope.workspace_id.as_str(), id],
                )
                .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
                count += u32::try_from(rows).unwrap_or(u32::MAX);
            }
            Ok(count)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }

    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        let conn = self.catalog.conn.clone();
        let scope = self.scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn
                .lock()
                .map_err(|e| CatalogError::Unavailable(format!("Lock: {e}")))?;

            let tx = conn
                .transaction()
                .map_err(|e| CatalogError::Unavailable(format!("Transaction: {e}")))?;

            let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
            for item in &items {
                save_entry(&tx, &scope, item)?;
            }

            tx.commit()
                .map_err(|e| CatalogError::Unavailable(format!("Commit: {e}")))?;

            Ok(ids)
        })
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_catalog() -> ScopedSqliteCatalog {
        SqliteCatalog::in_memory()
            .unwrap()
            .with_scope(CatalogScope::legacy_unscoped())
    }

    fn make_entry(id: &str, kind: CatalogKind, name: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: Some(name.to_string()),
            content: format!("{name} description"),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn schema_does_not_create_storage_search_artifacts() {
        let catalog = SqliteCatalog::in_memory().unwrap();
        let conn = catalog.conn.lock().unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(catalog_entries)").unwrap();
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            !columns.iter().any(|column| column == "search_content"),
            "catalog_entries should not carry a storage-owned search_content blob"
        );

        let fts_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'catalog_fts' OR name IN ('catalog_ai', 'catalog_ad', 'catalog_au')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            fts_count, 0,
            "SQLite catalog should not create FTS artifacts"
        );
    }

    #[tokio::test]
    async fn save_and_get_by_id() {
        let catalog = test_catalog();
        let entry = make_entry("t1", CatalogKind::Table, "users");
        catalog.save_items(vec![entry.clone()]).await.unwrap();

        let retrieved = catalog.get_by_id("t1").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "users");
    }

    #[tokio::test]
    async fn get_by_id_missing() {
        let catalog = test_catalog();
        let retrieved = catalog.get_by_id("nonexistent").await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn exact_qualified_name_lookup_uses_storage_index() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![
                make_entry("t1", CatalogKind::Table, "public.users"),
                make_entry("t2", CatalogKind::Table, "public.orders"),
            ])
            .await
            .unwrap();

        let entry = catalog
            .get_by_qualified_name(CatalogKind::Table, "public.users")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.id, "t1");
    }

    #[tokio::test]
    async fn exact_name_lookup_filters_by_kind() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![
                make_entry("t1", CatalogKind::Table, "product_name"),
                make_entry("c1", CatalogKind::Column, "product_name"),
            ])
            .await
            .unwrap();

        let hits = catalog
            .get_by_name(CatalogKind::Column, "product_name")
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c1");
    }

    #[tokio::test]
    async fn schema_qualified_name_miss_does_not_fallback_to_unqualified_name() {
        let catalog = test_catalog();
        let mut entry = make_entry("t1", CatalogKind::Table, "orders");
        entry.qualified_name = Some("public.orders".to_string());
        catalog.save_items(vec![entry]).await.unwrap();

        let resolved = catalog
            .resolve_ref(&CatalogRef::Name {
                kind: CatalogKind::Table,
                name: "orders".to_string(),
                schema: Some("archive".to_string()),
            })
            .await
            .unwrap();

        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn list_tables() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![
                make_entry("t1", CatalogKind::Table, "users"),
                make_entry("c1", CatalogKind::Column, "name"),
            ])
            .await
            .unwrap();

        let tables = catalog.list_tables().await.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "users");
    }

    #[tokio::test]
    async fn delete_items() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![make_entry("t1", CatalogKind::Table, "users")])
            .await
            .unwrap();

        let count = catalog.delete_items(&["t1".to_string()]).await.unwrap();
        assert_eq!(count, 1);

        let retrieved = catalog.get_by_id("t1").await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn save_in_transaction() {
        let catalog = test_catalog();
        let ids = catalog
            .save_in_transaction(vec![
                make_entry("t1", CatalogKind::Table, "users"),
                make_entry("t2", CatalogKind::Table, "orders"),
            ])
            .await
            .unwrap();
        assert_eq!(ids.len(), 2);

        let entries = catalog
            .get_by_ids(&["t1".into(), "t2".into()])
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn get_related() {
        let catalog = test_catalog();
        let mut entry = make_entry("t1", CatalogKind::Table, "users");
        entry.links.push(CatalogRelation {
            target_id: "t2".to_string(),
            kind: "foreign_key".to_string(),
            description: Some("users -> orders".into()),
        });

        catalog
            .save_items(vec![entry, make_entry("t2", CatalogKind::Table, "orders")])
            .await
            .unwrap();

        let related = catalog.get_related("t1", None).await.unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].name, "orders");

        let fk_related = catalog
            .get_related("t1", Some("foreign_key"))
            .await
            .unwrap();
        assert_eq!(fk_related.len(), 1);

        let no_related = catalog
            .get_related("t1", Some("nonexistent"))
            .await
            .unwrap();
        assert_eq!(no_related.len(), 0);
    }

    #[tokio::test]
    async fn upsert_semantics() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![make_entry("t1", CatalogKind::Table, "users")])
            .await
            .unwrap();
        catalog
            .save_items(vec![make_entry("t1", CatalogKind::Table, "customers")])
            .await
            .unwrap();

        let entry = catalog.get_by_id("t1").await.unwrap().unwrap();
        assert_eq!(entry.name, "customers");
    }

    #[test]
    fn detects_materialized_relationship_descriptions() {
        assert!(is_materialized_relationship_description(Some(
            "[materialized_relationship] fact_sales.product_id -> dim_products.product_id"
        )));
        assert!(!is_materialized_relationship_description(Some(
            "fact_sales.product_id -> dim_products.product_id"
        )));
        assert!(!is_materialized_relationship_description(None));
    }

    fn table_with_qn(id: &str, qualified_name: &str) -> CatalogEntry {
        table_with_qn_database(id, qualified_name, "warehouse")
    }

    fn table_with_qn_database(id: &str, qualified_name: &str, database_id: &str) -> CatalogEntry {
        let mut parts = qualified_name.rsplitn(2, '.');
        let table_name = parts.next().unwrap_or(qualified_name);
        let schema_name = parts.next().unwrap_or("public");
        CatalogEntry {
            id: id.to_string(),
            kind: CatalogKind::Table,
            name: table_name.to_string(),
            qualified_name: Some(qualified_name.to_string()),
            content: format!("{qualified_name} description"),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({
                "databaseId": database_id,
                "schemaName": schema_name,
                "tableName": table_name,
                "relationType": "base_table",
                "rowCount": null,
                "columnCount": 0,
                "preferredQuerySurface": true,
                "source": {}
            }),
        }
    }

    fn relationship_vertex(
        id: &str,
        source_table_id: &str,
        target_table_id: &str,
        source_table: &str,
        source_column: &str,
        target_table: &str,
        target_column: &str,
    ) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind: CatalogKind::Relationship,
            name: format!("{source_table}_to_{target_table}"),
            qualified_name: None,
            content: format!(
                "{source_table}.{source_column} references {target_table}.{target_column}"
            ),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({
                "databaseId": "warehouse",
                "sourceTableId": source_table_id,
                "targetTableId": target_table_id,
                "sourceSchema": "public",
                "sourceTable": source_table,
                "sourceColumn": source_column,
                "targetSchema": "public",
                "targetTable": target_table,
                "targetColumn": target_column,
                "sourceCardinality": "many",
                "targetCardinality": "one",
                "relationshipKind": "foreign_key",
                "confidence": 1.0
            }),
        }
    }

    #[tokio::test]
    async fn relationship_provenance_survives_persistence_and_materialization() {
        let catalog = test_catalog();
        let mut relationship = relationship_vertex(
            "relationship:fact_sales_dim_products",
            "table:public.fact_sales",
            "table:public.dim_products",
            "fact_sales",
            "product_id",
            "dim_products",
            "product_id",
        );
        relationship.metadata["source"] = serde_json::json!({
            "origin": agent_fw_catalog::provenance_origin::LLM_ENRICHMENT,
            "profilingRunId": "profile-1",
            "enrichmentSource": "fresh",
            "modelId": "claude-test-model"
        });

        catalog
            .save_items(vec![
                table_with_qn("table:public.fact_sales", "public.fact_sales"),
                table_with_qn("table:public.dim_products", "public.dim_products"),
                relationship,
            ])
            .await
            .unwrap();

        let loaded = catalog
            .get_by_id("relationship:fact_sales_dim_products")
            .await
            .unwrap()
            .expect("relationship entry should persist");
        let metadata: RelationshipMetadata = serde_json::from_value(loaded.metadata).unwrap();

        assert_eq!(
            metadata.source.origin.as_deref(),
            Some(agent_fw_catalog::provenance_origin::LLM_ENRICHMENT)
        );
        assert_eq!(
            metadata.source.profiling_run_id.as_deref(),
            Some("profile-1")
        );
        assert_eq!(
            metadata.source.model_id.as_deref(),
            Some("claude-test-model")
        );

        let relationship_source = catalog
            .get_related(
                "relationship:fact_sales_dim_products",
                Some(relation_kind::RELATIONSHIP_SOURCE_TABLE),
            )
            .await
            .unwrap();
        assert_eq!(relationship_source.len(), 1);
        assert_eq!(relationship_source[0].id, "table:public.fact_sales");

        let relationship_target = catalog
            .get_related(
                "relationship:fact_sales_dim_products",
                Some(relation_kind::RELATIONSHIP_TARGET_TABLE),
            )
            .await
            .unwrap();
        assert_eq!(relationship_target.len(), 1);
        assert_eq!(relationship_target[0].id, "table:public.dim_products");

        let related = catalog
            .get_related(
                "table:public.fact_sales",
                Some(relation_kind::REFERENCES_TABLE),
            )
            .await
            .unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, "table:public.dim_products");
    }

    #[tokio::test]
    async fn relationship_vertex_does_not_materialize_cross_database_table_edges() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![
                table_with_qn_database("table:public.fact_sales", "public.fact_sales", "warehouse"),
                table_with_qn_database(
                    "table:public.dim_products",
                    "public.dim_products",
                    "other_warehouse",
                ),
                relationship_vertex(
                    "relationship:fact_sales_dim_products",
                    "table:public.fact_sales",
                    "table:public.dim_products",
                    "fact_sales",
                    "product_id",
                    "dim_products",
                    "product_id",
                ),
            ])
            .await
            .unwrap();

        let related = catalog
            .get_related(
                "table:public.fact_sales",
                Some(relation_kind::REFERENCES_TABLE),
            )
            .await
            .unwrap();
        assert!(
            related.is_empty(),
            "relationship edges must not cross catalog database_id boundaries"
        );

        let path = catalog
            .find_join_path("public.fact_sales", "public.dim_products")
            .await
            .unwrap();
        assert!(
            path.is_none(),
            "join paths must not use cross-database relationship vertices"
        );
    }

    #[tokio::test]
    async fn find_join_path_populates_typed_hops_from_relationship_vertex() {
        let catalog = test_catalog();
        catalog
            .save_items(vec![
                table_with_qn("table:public.fact_sales", "public.fact_sales"),
                table_with_qn("table:public.dim_products", "public.dim_products"),
                relationship_vertex(
                    "relationship:fact_sales_dim_products",
                    "table:public.fact_sales",
                    "table:public.dim_products",
                    "fact_sales",
                    "product_id",
                    "dim_products",
                    "product_id",
                ),
            ])
            .await
            .unwrap();

        let path = catalog
            .find_join_path("public.fact_sales", "public.dim_products")
            .await
            .unwrap()
            .expect("a single-hop join path must be found");

        // Legacy shape is unchanged: steps includes the `from` table.
        assert_eq!(path.steps.len(), 2);
        assert_eq!(path.steps[0].id, "table:public.fact_sales");
        assert_eq!(path.steps[1].id, "table:public.dim_products");
        assert_eq!(path.length, 2);

        // Typed hops carry the relationship-vertex join metadata.
        assert_eq!(
            path.hops.len(),
            path.steps.len() - 1,
            "hops must describe each edge of the path"
        );
        let hop = &path.hops[0];
        assert_eq!(hop.relation_kind, relation_kind::REFERENCES_TABLE);
        assert_eq!(hop.from_column.as_deref(), Some("product_id"));
        assert_eq!(hop.to_column.as_deref(), Some("product_id"));
        assert_eq!(hop.join_type.as_deref(), Some("inner"));
        assert_eq!(
            hop.relationship_id.as_deref(),
            Some("relationship:fact_sales_dim_products")
        );
    }
}
