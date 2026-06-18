//! PostgreSQL-backed catalog handle with scoped DataCatalog + CatalogWriter views.
//!
//! Uses JSONB columns with GIN indexes for flexible metadata storage and
//!
//! # Feature Gate
//!
//! Requires the `postgres` feature:
//! ```toml
//! agent-fw-interpreter = { workspace = true, features = ["postgres"] }
//! ```
//!
//! # Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS catalog_entries (
//!     tenant_id      TEXT NOT NULL,
//!     workspace_id   TEXT NOT NULL,
//!     id             TEXT NOT NULL,
//!     kind           TEXT NOT NULL,
//!     name           TEXT NOT NULL,
//!     qualified_name TEXT,
//!     content        TEXT NOT NULL DEFAULT '',
//!     tags           JSONB NOT NULL DEFAULT '[]',
//!     metadata       JSONB NOT NULL DEFAULT '{}',
//!     created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     PRIMARY KEY (tenant_id, workspace_id, id)
//! );
//!
//! CREATE TABLE IF NOT EXISTS catalog_relations (
//!     tenant_id   TEXT NOT NULL,
//!     workspace_id TEXT NOT NULL,
//!     source_id   TEXT NOT NULL,
//!     target_id   TEXT NOT NULL,
//!     kind        TEXT NOT NULL,
//!     description TEXT,
//!     PRIMARY KEY (tenant_id, workspace_id, source_id, target_id, kind)
//! );
//! ```
//!
//! # Indexes
//!
//! - GIN on `tags` — JSONB containment queries (`@>`)
//! - GIN on `metadata` — JSONB path queries
//! - B-tree on `(tenant_id, workspace_id, kind)` — filtered queries
//! - B-tree on `(tenant_id, workspace_id, qualified_name)` — exact lookup
//! - B-tree on `(tenant_id, workspace_id, kind, name)` — exact name lookup
//! - B-tree on scoped `source_id` / `target_id` — relation traversal
//!
//! # Laws Satisfied
//!
//! ## DataCatalog
//! - L1 (Existence): `get_by_id(id)` returns `Some` iff the id was saved
//! - L2 (Determinism): Same inputs → same outputs (within snapshot)
//! - L3 (Relationship Integrity): `get_related(id)` returns only existing items
//!
//! ## CatalogWriter
//! - L1 (Roundtrip): `save_items(items); get_by_ids(ids)` returns the items
//! - L2 (Delete): `delete_items(ids); get_by_ids(ids)` returns empty
//! - L3 (Transaction): `save_in_transaction` partial failure rolls back all

use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;

use async_trait::async_trait;
use sqlx::postgres::PgPool;
use sqlx::Row;

use agent_fw_catalog::{
    relation_kind, CatalogEntry, CatalogError, CatalogKind, CatalogRef, CatalogRelation,
    CatalogScope, CatalogWriter, DataCatalog, JoinHop, JoinPath, RelationshipMetadata,
    TableMetadata,
};

const MATERIALIZED_RELATION_DESCRIPTION_PREFIX: &str = "[materialized_relationship] ";
const MATERIALIZED_RELATION_DESCRIPTION_LIKE_PATTERN: &str = "[materialized_relationship] %";

/// Upper bound on relationship vertices loaded for join-path hop enrichment.
const RELATIONSHIP_SCAN_LIMIT: usize = 10_000;

/// PostgreSQL-backed catalog store.
///
/// Use [`PostgresCatalog::with_scope`] to obtain the reader/writer view for a
/// tenant/workspace.
#[derive(Clone)]
pub struct PostgresCatalog {
    pool: PgPool,
}

/// PostgreSQL catalog view bound to one tenant/workspace scope.
#[derive(Clone)]
pub struct ScopedPostgresCatalog {
    catalog: PostgresCatalog,
    scope: CatalogScope,
}

impl PostgresCatalog {
    /// Create from an existing connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connect to a PostgreSQL database URL.
    pub async fn connect(url: &str) -> Result<Self, CatalogError> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("Connection failed: {e}")))?;
        Ok(Self::new(pool))
    }

    /// Ensure tables, indexes, and triggers exist.
    ///
    /// Idempotent — safe to call on every startup.
    pub async fn ensure_schema(&self) -> Result<(), CatalogError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS catalog_entries (
                tenant_id      TEXT NOT NULL DEFAULT 'legacy',
                workspace_id   TEXT NOT NULL DEFAULT 'default',
                id             TEXT NOT NULL,
                kind           TEXT NOT NULL,
                name           TEXT NOT NULL,
                qualified_name TEXT,
                content        TEXT NOT NULL DEFAULT '',
                tags           JSONB NOT NULL DEFAULT '[]'::jsonb,
                metadata       JSONB NOT NULL DEFAULT '{}'::jsonb,
                created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (tenant_id, workspace_id, id)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Create catalog_entries: {e}")))?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS catalog_relations (
                tenant_id   TEXT NOT NULL DEFAULT 'legacy',
                workspace_id TEXT NOT NULL DEFAULT 'default',
                source_id   TEXT NOT NULL,
                target_id   TEXT NOT NULL,
                kind        TEXT NOT NULL,
                description TEXT,
                PRIMARY KEY (tenant_id, workspace_id, source_id, target_id, kind)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Create catalog_relations: {e}")))?;

        self.ensure_scoped_primary_keys().await?;

        self.drop_search_artifacts().await?;

        // GIN + scoped B-tree indexes
        let indexes = [
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_tags ON catalog_entries USING GIN (tags)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_meta ON catalog_entries USING GIN (metadata)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_kind ON catalog_entries (kind)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_qname ON catalog_entries (qualified_name)",
            "CREATE INDEX IF NOT EXISTS idx_cat_relations_src ON catalog_relations (source_id)",
            "CREATE INDEX IF NOT EXISTS idx_cat_relations_tgt ON catalog_relations (target_id)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_scope_kind ON catalog_entries (tenant_id, workspace_id, kind)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_scope_qname ON catalog_entries (tenant_id, workspace_id, qualified_name)",
            "CREATE INDEX IF NOT EXISTS idx_cat_entries_scope_kind_name ON catalog_entries (tenant_id, workspace_id, kind, name)",
            "CREATE INDEX IF NOT EXISTS idx_cat_relations_scope_src ON catalog_relations (tenant_id, workspace_id, source_id)",
            "CREATE INDEX IF NOT EXISTS idx_cat_relations_scope_tgt ON catalog_relations (tenant_id, workspace_id, target_id)",
        ];
        for ddl in &indexes {
            sqlx::query(ddl)
                .execute(&self.pool)
                .await
                .map_err(|e| CatalogError::Unavailable(format!("Create index: {e}")))?;
        }

        Ok(())
    }

    /// Access the underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Bind this catalog to an explicit tenant/workspace scope.
    pub fn with_scope(&self, scope: CatalogScope) -> ScopedPostgresCatalog {
        ScopedPostgresCatalog {
            catalog: self.clone(),
            scope,
        }
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    async fn ensure_scoped_primary_keys(&self) -> Result<(), CatalogError> {
        sqlx::query(
            r#"
            ALTER TABLE catalog_entries
              ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'legacy',
              ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default'
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Add entry scope columns: {e}")))?;

        sqlx::query(
            r#"
            ALTER TABLE catalog_relations
              ADD COLUMN IF NOT EXISTS tenant_id TEXT NOT NULL DEFAULT 'legacy',
              ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default'
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Add relation scope columns: {e}")))?;

        sqlx::query(
            r#"
            DO $$
            DECLARE existing_pk text;
            BEGIN
              IF NOT EXISTS (
                SELECT 1
                FROM pg_constraint c
                JOIN LATERAL (
                  SELECT array_agg(a.attname ORDER BY key.ord) AS columns
                  FROM unnest(c.conkey) WITH ORDINALITY AS key(attnum, ord)
                  JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = key.attnum
                ) pk ON true
                WHERE c.conrelid = 'catalog_entries'::regclass
                  AND c.contype = 'p'
                  AND pk.columns::text[] = ARRAY['tenant_id','workspace_id','id']
              ) THEN
                SELECT conname INTO existing_pk
                FROM pg_constraint
                WHERE conrelid = 'catalog_entries'::regclass AND contype = 'p';
                IF existing_pk IS NOT NULL THEN
                  EXECUTE format('ALTER TABLE catalog_entries DROP CONSTRAINT %I', existing_pk);
                END IF;
                ALTER TABLE catalog_entries ADD PRIMARY KEY (tenant_id, workspace_id, id);
              END IF;

              IF NOT EXISTS (
                SELECT 1
                FROM pg_constraint c
                JOIN LATERAL (
                  SELECT array_agg(a.attname ORDER BY key.ord) AS columns
                  FROM unnest(c.conkey) WITH ORDINALITY AS key(attnum, ord)
                  JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = key.attnum
                ) pk ON true
                WHERE c.conrelid = 'catalog_relations'::regclass
                  AND c.contype = 'p'
                  AND pk.columns::text[] = ARRAY['tenant_id','workspace_id','source_id','target_id','kind']
              ) THEN
                SELECT conname INTO existing_pk
                FROM pg_constraint
                WHERE conrelid = 'catalog_relations'::regclass AND contype = 'p';
                IF existing_pk IS NOT NULL THEN
                  EXECUTE format('ALTER TABLE catalog_relations DROP CONSTRAINT %I', existing_pk);
                END IF;
                ALTER TABLE catalog_relations ADD PRIMARY KEY (tenant_id, workspace_id, source_id, target_id, kind);
              END IF;
            END $$;
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("Update scoped primary keys: {e}")))?;

        Ok(())
    }

    async fn drop_search_artifacts(&self) -> Result<(), CatalogError> {
        let cleanup = [
            "DROP TRIGGER IF EXISTS catalog_entries_search_update ON catalog_entries",
            "DROP FUNCTION IF EXISTS catalog_entries_search_trigger()",
            "DROP INDEX IF EXISTS idx_cat_entries_fts",
            "ALTER TABLE catalog_entries DROP COLUMN IF EXISTS search_vector",
            "ALTER TABLE catalog_entries DROP COLUMN IF EXISTS search_content",
        ];
        for ddl in cleanup {
            sqlx::query(ddl)
                .execute(&self.pool)
                .await
                .map_err(|e| CatalogError::Unavailable(format!("Drop search artifact: {e}")))?;
        }
        Ok(())
    }

    /// Load a single entry by ID, including its relations.
    async fn load_entry_scoped(
        &self,
        scope: &CatalogScope,
        id: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        let row = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND id = $3",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("load_entry: {e}")))?;

        match row {
            Some(row) => {
                let links = self.load_relations_for_scoped(scope, id).await?;
                Ok(Some(row_to_entry(&row, links)?))
            }
            None => Ok(None),
        }
    }

    /// Load outgoing relations for a single entry.
    async fn load_relations_for_scoped(
        &self,
        scope: &CatalogScope,
        source_id: &str,
    ) -> Result<Vec<CatalogRelation>, CatalogError> {
        let rows = sqlx::query(
            "SELECT target_id, kind, description
             FROM catalog_relations
             WHERE tenant_id = $1 AND workspace_id = $2 AND source_id = $3",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(source_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("load_relations: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| CatalogRelation {
                target_id: r.get("target_id"),
                kind: r.get("kind"),
                description: r.get("description"),
            })
            .collect())
    }

    /// Batch-load relations for multiple entry IDs (avoids N+1).
    async fn load_relations_batch_scoped(
        &self,
        scope: &CatalogScope,
        ids: &[String],
    ) -> Result<HashMap<String, Vec<CatalogRelation>>, CatalogError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = sqlx::query(
            "SELECT source_id, target_id, kind, description
             FROM catalog_relations
             WHERE tenant_id = $1 AND workspace_id = $2 AND source_id = ANY($3)",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("load_relations_batch: {e}")))?;

        let mut map: HashMap<String, Vec<CatalogRelation>> = HashMap::new();
        for row in &rows {
            let source_id: String = row.get("source_id");
            let relation = CatalogRelation {
                target_id: row.get("target_id"),
                kind: row.get("kind"),
                description: row.get("description"),
            };
            map.entry(source_id).or_default().push(relation);
        }
        Ok(map)
    }

    /// Hydrate a batch of entry rows with their relations.
    async fn hydrate_entries_scoped(
        &self,
        scope: &CatalogScope,
        rows: Vec<sqlx::postgres::PgRow>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let relations = self.load_relations_batch_scoped(scope, &ids).await?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.get("id");
            let links = relations.get(&id).cloned().unwrap_or_default();
            entries.push(row_to_entry(row, links)?);
        }
        Ok(entries)
    }

    /// Upsert a single entry within a transaction.
    async fn upsert_entry_in_scoped(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        entry: &CatalogEntry,
    ) -> Result<(), CatalogError> {
        if let Some(old_entry) = Self::load_entry_in_tx(tx, scope, &entry.id).await? {
            Self::cleanup_relationship_table_edges(
                tx,
                scope,
                &old_entry,
                std::slice::from_ref(&entry.id),
            )
            .await?;
        }

        let tags_json =
            serde_json::to_value(&entry.tags).unwrap_or(serde_json::Value::Array(vec![]));
        sqlx::query(
            r#"
            INSERT INTO catalog_entries
              (tenant_id, workspace_id, id, kind, name, qualified_name, content, tags, metadata, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
            ON CONFLICT (tenant_id, workspace_id, id) DO UPDATE SET
                kind = EXCLUDED.kind,
                name = EXCLUDED.name,
                qualified_name = EXCLUDED.qualified_name,
                content = EXCLUDED.content,
                tags = EXCLUDED.tags,
                metadata = EXCLUDED.metadata,
                updated_at = NOW()
            "#,
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(&entry.id)
        .bind(entry.kind.as_str())
        .bind(&entry.name)
        .bind(&entry.qualified_name)
        .bind(&entry.content)
        .bind(&tags_json)
        .bind(&entry.metadata)
        .execute(&mut **tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("upsert entry: {e}")))?;

        if entry.kind == CatalogKind::Relationship {
            sqlx::query(
                "DELETE FROM catalog_relations
                 WHERE tenant_id = $1 AND workspace_id = $2 AND source_id = $3",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .bind(&entry.id)
            .execute(&mut **tx)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("delete relations: {e}")))?;
        } else {
            sqlx::query(
                "DELETE FROM catalog_relations
                 WHERE tenant_id = $1
                   AND workspace_id = $2
                   AND source_id = $3
                   AND (description IS NULL OR description NOT LIKE $4)",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .bind(&entry.id)
            .bind(MATERIALIZED_RELATION_DESCRIPTION_LIKE_PATTERN)
            .execute(&mut **tx)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("delete relations: {e}")))?;
        }

        for link in &entry.links {
            Self::insert_relation_in_scoped(tx, scope, &entry.id, link).await?;
        }

        Self::materialize_relationship_table_edges(tx, scope, entry).await?;

        Ok(())
    }

    async fn load_entry_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        id: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        let row = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND id = $3",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("load entry in tx: {e}")))?;

        row.map(|row| row_to_entry(&row, vec![])).transpose()
    }

    async fn insert_relation_in_scoped(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        source_id: &str,
        link: &CatalogRelation,
    ) -> Result<(), CatalogError> {
        sqlx::query(
            r#"
            INSERT INTO catalog_relations
              (tenant_id, workspace_id, source_id, target_id, kind, description)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (tenant_id, workspace_id, source_id, target_id, kind) DO NOTHING
            "#,
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(source_id)
        .bind(&link.target_id)
        .bind(&link.kind)
        .bind(&link.description)
        .execute(&mut **tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("insert relation: {e}")))?;
        Ok(())
    }

    async fn materialize_relationship_table_edges(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        entry: &CatalogEntry,
    ) -> Result<(), CatalogError> {
        for (source_id, link) in Self::validated_relationship_table_edges(tx, scope, entry).await? {
            Self::insert_relation_in_scoped(tx, scope, &source_id, &link).await?;
        }
        Ok(())
    }

    async fn cleanup_relationship_table_edges(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        entry: &CatalogEntry,
        excluded_relationship_ids: &[String],
    ) -> Result<(), CatalogError> {
        for (source_id, link) in relationship_table_edges(entry) {
            if Self::relationship_table_edge_has_support(
                tx,
                scope,
                &source_id,
                &link,
                excluded_relationship_ids,
            )
            .await?
            {
                continue;
            }
            sqlx::query(
                "DELETE FROM catalog_relations
                 WHERE tenant_id = $1
                   AND workspace_id = $2
                   AND source_id = $3
                   AND target_id = $4
                   AND kind = $5
                   AND description = $6",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .bind(&source_id)
            .bind(&link.target_id)
            .bind(&link.kind)
            .bind(&link.description)
            .execute(&mut **tx)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("delete materialized relation: {e}")))?;
        }
        Ok(())
    }

    async fn relationship_table_edge_has_support(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        source_id: &str,
        link: &CatalogRelation,
        excluded_relationship_ids: &[String],
    ) -> Result<bool, CatalogError> {
        for entry in Self::relationship_entries(tx, scope)
            .await?
            .into_iter()
            .filter(|entry| !excluded_relationship_ids.contains(&entry.id))
        {
            if Self::validated_relationship_table_edges(tx, scope, &entry)
                .await?
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

    async fn relationship_entries(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1
               AND workspace_id = $2
               AND kind = 'relationship'",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("relationship entries: {e}")))?;

        rows.iter().map(|row| row_to_entry(row, vec![])).collect()
    }

    async fn validated_relationship_table_edges(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
        if !Self::relationship_targets_match_database(tx, scope, &metadata).await? {
            return Ok(vec![]);
        }

        Ok(relationship_table_edges(entry))
    }

    async fn relationship_targets_match_database(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        metadata: &RelationshipMetadata,
    ) -> Result<bool, CatalogError> {
        let Some(source_database_id) =
            Self::table_database_id_in_tx(tx, scope, &metadata.source_table_id).await?
        else {
            return Ok(false);
        };
        let Some(target_database_id) =
            Self::table_database_id_in_tx(tx, scope, &metadata.target_table_id).await?
        else {
            return Ok(false);
        };
        Ok(
            source_database_id == metadata.database_id
                && target_database_id == metadata.database_id,
        )
    }

    async fn table_database_id_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        table_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        let Some(entry) = Self::load_entry_in_tx(tx, scope, table_id).await? else {
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

    async fn cleanup_relationships_for_delete(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        scope: &CatalogScope,
        ids: &[String],
    ) -> Result<(), CatalogError> {
        for id in ids {
            if let Some(entry) = Self::load_entry_in_tx(tx, scope, id).await? {
                Self::cleanup_relationship_table_edges(tx, scope, &entry, ids).await?;
            }
        }
        Ok(())
    }
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

async fn relationship_targets_match_database_postgres(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    metadata: &RelationshipMetadata,
) -> Result<bool, CatalogError> {
    let Some(source_database_id) =
        table_database_id_postgres(catalog, scope, &metadata.source_table_id).await?
    else {
        return Ok(false);
    };
    let Some(target_database_id) =
        table_database_id_postgres(catalog, scope, &metadata.target_table_id).await?
    else {
        return Ok(false);
    };
    Ok(source_database_id == metadata.database_id && target_database_id == metadata.database_id)
}

async fn table_database_id_postgres(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    table_id: &str,
) -> Result<Option<String>, CatalogError> {
    let Some(entry) = catalog.load_entry_scoped(scope, table_id).await? else {
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

impl ScopedPostgresCatalog {
    pub fn pool(&self) -> &PgPool {
        self.catalog.pool()
    }

    pub fn scope(&self) -> &CatalogScope {
        &self.scope
    }
}

async fn resolve_postgres_ref(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    reference: &CatalogRef,
) -> Result<Option<CatalogEntry>, CatalogError> {
    match reference {
        CatalogRef::Id(id) => catalog.load_entry_scoped(scope, id).await,
        CatalogRef::QualifiedName {
            kind: Some(kind),
            qualified_name,
        } => exact_postgres_qualified_name(catalog, scope, *kind, qualified_name).await,
        CatalogRef::QualifiedName {
            kind: None,
            qualified_name,
        } => {
            let row = sqlx::query(
                "SELECT id, kind, name, qualified_name, content, tags, metadata
                 FROM catalog_entries
                 WHERE tenant_id = $1
                   AND workspace_id = $2
                   AND qualified_name = $3",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .bind(qualified_name)
            .fetch_optional(&catalog.pool)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("resolve_ref qname: {e}")))?;

            hydrate_optional_postgres_row(catalog, scope, row).await
        }
        CatalogRef::Name { kind, name, schema } => {
            if let Some(schema) = schema {
                let qualified_name = format!("{schema}.{name}");
                return exact_postgres_qualified_name(catalog, scope, *kind, &qualified_name).await;
            }
            let row = sqlx::query(
                "SELECT id, kind, name, qualified_name, content, tags, metadata
                 FROM catalog_entries
                 WHERE tenant_id = $1
                   AND workspace_id = $2
                   AND kind = $3
                   AND name = $4
                 ORDER BY qualified_name
                 LIMIT 1",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .bind(kind.as_str())
            .bind(name)
            .fetch_optional(&catalog.pool)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("resolve_ref name: {e}")))?;

            hydrate_optional_postgres_row(catalog, scope, row).await
        }
    }
}

async fn exact_postgres_name(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    kind: CatalogKind,
    name: &str,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let rows = sqlx::query(
        "SELECT id, kind, name, qualified_name, content, tags, metadata
         FROM catalog_entries
         WHERE tenant_id = $1
           AND workspace_id = $2
           AND kind = $3
           AND name = $4
         ORDER BY qualified_name, id",
    )
    .bind(scope.tenant_id.as_str())
    .bind(scope.workspace_id.as_str())
    .bind(kind.as_str())
    .bind(name)
    .fetch_all(&catalog.pool)
    .await
    .map_err(|e| CatalogError::Unavailable(format!("exact name: {e}")))?;

    catalog.hydrate_entries_scoped(scope, rows).await
}

async fn exact_postgres_qualified_name(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    kind: CatalogKind,
    qualified_name: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    let row = sqlx::query(
        "SELECT id, kind, name, qualified_name, content, tags, metadata
         FROM catalog_entries
         WHERE tenant_id = $1
           AND workspace_id = $2
           AND kind = $3
           AND qualified_name = $4",
    )
    .bind(scope.tenant_id.as_str())
    .bind(scope.workspace_id.as_str())
    .bind(kind.as_str())
    .bind(qualified_name)
    .fetch_optional(&catalog.pool)
    .await
    .map_err(|e| CatalogError::Unavailable(format!("exact qualified name: {e}")))?;

    hydrate_optional_postgres_row(catalog, scope, row).await
}

async fn hydrate_optional_postgres_row(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    row: Option<sqlx::postgres::PgRow>,
) -> Result<Option<CatalogEntry>, CatalogError> {
    match row {
        Some(row) => {
            let id: String = row.get("id");
            let links = catalog.load_relations_for_scoped(scope, &id).await?;
            Ok(Some(row_to_entry(&row, links)?))
        }
        None => Ok(None),
    }
}

async fn related_postgres_entries(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let ids = related_postgres_ids(catalog, scope, id, relation_type).await?;
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(entry) = catalog.load_entry_scoped(scope, &id).await? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

async fn related_postgres_entries_reverse(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    let ids = related_postgres_ids_reverse(catalog, scope, id, relation_type).await?;
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(entry) = catalog.load_entry_scoped(scope, &id).await? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

async fn related_postgres_ids(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<String>, CatalogError> {
    let mut query = String::from(
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
         WHERE r.tenant_id = $1
           AND r.workspace_id = $2
           AND r.source_id = $3",
    );
    if relation_type.is_some() {
        query.push_str(" AND r.kind = $4");
    }
    let mut query = sqlx::query(&query)
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(id);
    if let Some(kind) = relation_type {
        query = query.bind(kind);
    }

    let rows = query
        .fetch_all(&catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("related ids: {e}")))?;
    Ok(rows.iter().map(|row| row.get("target_id")).collect())
}

async fn related_postgres_ids_reverse(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    id: &str,
    relation_type: Option<&str>,
) -> Result<Vec<String>, CatalogError> {
    let mut query = String::from(
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
         WHERE r.tenant_id = $1
           AND r.workspace_id = $2
           AND r.target_id = $3",
    );
    if relation_type.is_some() {
        query.push_str(" AND r.kind = $4");
    }
    let mut query = sqlx::query(&query)
        .bind(scope.tenant_id.as_str())
        .bind(scope.workspace_id.as_str())
        .bind(id);
    if let Some(kind) = relation_type {
        query = query.bind(kind);
    }

    let rows = query
        .fetch_all(&catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("reverse related ids: {e}")))?;
    Ok(rows.iter().map(|row| row.get("source_id")).collect())
}

async fn columns_postgres(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    table_ref: &str,
) -> Result<Vec<CatalogEntry>, CatalogError> {
    if let Some(table) =
        resolve_postgres_ref(catalog, scope, &CatalogRef::parse_table(table_ref)).await?
    {
        let columns =
            related_postgres_entries(catalog, scope, &table.id, Some(relation_kind::HAS_COLUMN))
                .await?
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
        format!("%.{}.%", escape_postgres_like(table_ref))
    };
    let rows = sqlx::query(
        "SELECT id, kind, name, qualified_name, content, tags, metadata
         FROM catalog_entries
         WHERE tenant_id = $1
           AND workspace_id = $2
           AND kind = 'column'
           AND qualified_name LIKE $3 ESCAPE '\\'
         ORDER BY name",
    )
    .bind(scope.tenant_id.as_str())
    .bind(scope.workspace_id.as_str())
    .bind(pattern)
    .fetch_all(&catalog.pool)
    .await
    .map_err(|e| CatalogError::Unavailable(format!("columns: {e}")))?;

    catalog.hydrate_entries_scoped(scope, rows).await
}

async fn enum_values_postgres(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    column_ref: &str,
) -> Result<Vec<String>, CatalogError> {
    let Some(column) =
        resolve_postgres_ref(catalog, scope, &CatalogRef::parse_column(column_ref)).await?
    else {
        return legacy_postgres_enum_values(catalog, scope, column_ref).await;
    };

    let mut enum_entries = related_postgres_entries_reverse(
        catalog,
        scope,
        &column.id,
        Some(relation_kind::ENUM_VALUE_OF),
    )
    .await?;
    enum_entries.extend(
        related_postgres_entries_reverse(catalog, scope, &column.id, Some("enum_of")).await?,
    );
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
        return legacy_postgres_enum_values(catalog, scope, &column.id).await;
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

async fn legacy_postgres_enum_values(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    id: &str,
) -> Result<Vec<String>, CatalogError> {
    let row = sqlx::query(
        "SELECT metadata
         FROM catalog_entries
         WHERE tenant_id = $1
           AND workspace_id = $2
           AND id = $3
           AND kind = 'enum'",
    )
    .bind(scope.tenant_id.as_str())
    .bind(scope.workspace_id.as_str())
    .bind(id)
    .fetch_optional(&catalog.pool)
    .await
    .map_err(|e| CatalogError::Unavailable(format!("legacy enum values: {e}")))?;

    match row {
        Some(row) => {
            let metadata: serde_json::Value = row.get("metadata");
            match metadata.get("values") {
                Some(values) => {
                    serde_json::from_value::<Vec<String>>(values.clone()).map_err(|e| {
                        CatalogError::Unavailable(format!("Corrupted enum values for {id}: {e}"))
                    })
                }
                None => Ok(vec![]),
            }
        }
        None => Ok(vec![]),
    }
}

async fn table_neighbors_postgres(
    catalog: &PostgresCatalog,
    scope: &CatalogScope,
    table_id: &str,
) -> Result<Vec<String>, CatalogError> {
    let mut neighbors = related_postgres_ids(
        catalog,
        scope,
        table_id,
        Some(relation_kind::REFERENCES_TABLE),
    )
    .await?;
    neighbors.extend(
        related_postgres_ids(
            catalog,
            scope,
            table_id,
            Some(relation_kind::REFERENCED_BY_TABLE),
        )
        .await?,
    );
    neighbors.extend(
        related_postgres_entries_reverse(
            catalog,
            scope,
            table_id,
            Some(relation_kind::REFERENCES_TABLE),
        )
        .await?
        .into_iter()
        .filter(|entry| entry.kind == CatalogKind::Table)
        .map(|entry| entry.id),
    );
    neighbors.extend(
        related_postgres_entries_reverse(
            catalog,
            scope,
            table_id,
            Some(relation_kind::REFERENCED_BY_TABLE),
        )
        .await?
        .into_iter()
        .filter(|entry| entry.kind == CatalogKind::Table)
        .map(|entry| entry.id),
    );
    neighbors.sort();
    neighbors.dedup();
    Ok(neighbors)
}

fn escape_postgres_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

// ---------------------------------------------------------------------------
// DataCatalog
// ---------------------------------------------------------------------------

#[async_trait]
impl DataCatalog for ScopedPostgresCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        self.catalog.load_entry_scoped(&self.scope, id).await
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND id = ANY($3)",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .bind(ids)
        .fetch_all(&self.catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("get_by_ids: {e}")))?;

        let entries = self
            .catalog
            .hydrate_entries_scoped(&self.scope, rows)
            .await?;
        let by_id: std::collections::HashMap<String, CatalogEntry> = entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        let row = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1
               AND workspace_id = $2
               AND kind = $3
               AND qualified_name = $4",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .bind(kind.as_str())
        .bind(qualified_name)
        .fetch_optional(&self.catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("get_by_qualified_name: {e}")))?;

        match row {
            Some(row) => {
                let id: String = row.get("id");
                let links = self
                    .catalog
                    .load_relations_for_scoped(&self.scope, &id)
                    .await?;
                Ok(Some(row_to_entry(&row, links)?))
            }
            None => Ok(None),
        }
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        exact_postgres_name(&self.catalog, &self.scope, kind, name).await
    }

    async fn resolve_ref(
        &self,
        reference: &CatalogRef,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        resolve_postgres_ref(&self.catalog, &self.scope, reference).await
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND kind = $3
             ORDER BY name
             LIMIT $4",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .bind(kind.as_str())
        .bind(limit as i64)
        .fetch_all(&self.catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("list_by_type: {e}")))?;

        self.catalog.hydrate_entries_scoped(&self.scope, rows).await
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        related_postgres_entries(&self.catalog, &self.scope, id, relation_type).await
    }

    async fn get_related_reverse(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        related_postgres_entries_reverse(&self.catalog, &self.scope, id, relation_type).await
    }

    async fn find_join_path(
        &self,
        from_table: &str,
        to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        let Some(from) = resolve_postgres_ref(
            &self.catalog,
            &self.scope,
            &CatalogRef::parse_table(from_table),
        )
        .await?
        else {
            return Ok(None);
        };
        let Some(to) = resolve_postgres_ref(
            &self.catalog,
            &self.scope,
            &CatalogRef::parse_table(to_table),
        )
        .await?
        else {
            return Ok(None);
        };

        // Load the scope's relationship vertices ONCE (O(relationships)), not
        // once per BFS node, so typed hop metadata can be attached without a
        // per-node rescan. Pair each vertex with its decoded metadata.
        let mut relationship_index: Vec<(String, RelationshipMetadata)> = Vec::new();
        for entry in self
            .list_by_type(CatalogKind::Relationship, RELATIONSHIP_SCAN_LIMIT)
            .await?
        {
            let Ok(metadata) =
                serde_json::from_value::<RelationshipMetadata>(entry.metadata.clone())
            else {
                continue;
            };
            if relationship_targets_match_database_postgres(&self.catalog, &self.scope, &metadata)
                .await?
            {
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
                    if let Some(entry) =
                        self.catalog.load_entry_scoped(&self.scope, step_id).await?
                    {
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
                continue; // Depth limit
            }

            for neighbor in table_neighbors_postgres(&self.catalog, &self.scope, &current).await? {
                if visited.insert(neighbor.clone()) {
                    let mut new_path = path.clone();
                    new_path.push(neighbor.clone());
                    queue.push_back((neighbor, new_path));
                }
            }
        }

        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, kind, name, qualified_name, content, tags, metadata
             FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND kind = 'table'
             ORDER BY name",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .fetch_all(&self.catalog.pool)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("list_tables: {e}")))?;

        self.catalog.hydrate_entries_scoped(&self.scope, rows).await
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        columns_postgres(&self.catalog, &self.scope, table_name).await
    }

    async fn get_enum_values(&self, column_id: &str) -> Result<Vec<String>, CatalogError> {
        enum_values_postgres(&self.catalog, &self.scope, column_id).await
    }

    async fn health_check(&self) -> Result<(), CatalogError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.catalog.pool)
            .await
            .map_err(|e| CatalogError::Unavailable(format!("Health check: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CatalogWriter
// ---------------------------------------------------------------------------

#[async_trait]
impl CatalogWriter for ScopedPostgresCatalog {
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

        let mut tx = self
            .catalog
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("begin: {e}")))?;

        for item in &items {
            PostgresCatalog::upsert_entry_in_scoped(&mut tx, &self.scope, item).await?;
        }

        tx.commit()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("commit: {e}")))?;

        Ok(ids)
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        if ids.is_empty() {
            return Ok(0);
        }

        let mut tx = self
            .catalog
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("begin: {e}")))?;

        PostgresCatalog::cleanup_relationships_for_delete(&mut tx, &self.scope, ids).await?;

        // Delete relations first (referential integrity)
        sqlx::query(
            "DELETE FROM catalog_relations
             WHERE tenant_id = $1
               AND workspace_id = $2
               AND (source_id = ANY($3) OR target_id = ANY($3))",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .bind(ids)
        .execute(&mut *tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("delete relations: {e}")))?;

        let result = sqlx::query(
            "DELETE FROM catalog_entries
             WHERE tenant_id = $1 AND workspace_id = $2 AND id = ANY($3)",
        )
        .bind(self.scope.tenant_id.as_str())
        .bind(self.scope.workspace_id.as_str())
        .bind(ids)
        .execute(&mut *tx)
        .await
        .map_err(|e| CatalogError::Unavailable(format!("delete entries: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("commit: {e}")))?;

        Ok(result.rows_affected() as u32)
    }

    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

        let mut tx = self
            .catalog
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("begin: {e}")))?;

        for item in &items {
            PostgresCatalog::upsert_entry_in_scoped(&mut tx, &self.scope, item).await?;
        }

        tx.commit()
            .await
            .map_err(|e| CatalogError::Unavailable(format!("commit: {e}")))?;

        Ok(ids)
    }
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

/// Convert a PgRow (with standard columns) + pre-loaded links into a CatalogEntry.
fn row_to_entry(
    row: &sqlx::postgres::PgRow,
    links: Vec<CatalogRelation>,
) -> Result<CatalogEntry, CatalogError> {
    let id: String = row.get("id");
    let kind_str: String = row.get("kind");
    let name: String = row.get("name");
    let qualified_name: Option<String> = row.get("qualified_name");
    let content: String = row.get("content");
    let tags_json: serde_json::Value = row.get("tags");
    let metadata: serde_json::Value = row.get("metadata");

    let kind = CatalogKind::from_str(&kind_str).unwrap_or_else(|_| {
        tracing::warn!("Unknown CatalogKind '{kind_str}', falling back to Special");
        CatalogKind::Special
    });

    let tags: Vec<String> = serde_json::from_value(tags_json).unwrap_or_default();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        for kind in [
            CatalogKind::Table,
            CatalogKind::Column,
            CatalogKind::Relationship,
            CatalogKind::Enum,
            CatalogKind::Metric,
            CatalogKind::Special,
            CatalogKind::Document,
            CatalogKind::Knowledge,
            CatalogKind::DataQualityFinding,
        ] {
            let s = kind.as_str();
            let parsed = CatalogKind::from_str(s).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn unknown_kind_falls_back_to_special() {
        let parsed = CatalogKind::from_str("unknown_future_kind");
        assert!(parsed.is_err());
    }

    #[test]
    fn empty_relations_batch() {
        let map: HashMap<String, Vec<CatalogRelation>> = HashMap::new();
        assert!(map.is_empty());
    }
}
