//! Storage descriptor parsing and interpreter construction for Flow AI runtimes.
//!
//! The framework already ships several storage interpreters. This module is the
//! runtime-level adapter that turns serialisable language-facade config into
//! the concrete trait objects used by [`RuntimeDeps`](crate::RuntimeDeps).

use std::collections::HashMap;
use std::convert::TryFrom;
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use agent_fw_algebra::{KVStore, TargetDatabase};
use agent_fw_catalog::{
    diagnose_catalog_relations, CatalogEntry, CatalogError, CatalogKind,
    CatalogRelationDiagnostics, CatalogScope, CatalogSearchBackend, CatalogSearchHealth,
    CatalogWriter, DataCatalog, SemanticEntity,
};
use agent_fw_catalog_index::{CatalogDocumentProjection, TantivyCatalogIndex};
use agent_fw_core::{TenantId, WorkspaceContext, WorkspaceId};
use agent_fw_ingest::knowledge_store;
use agent_fw_interpreter::{
    DashMapKVStore, MockCatalog, PostgresCatalog, PostgresKVStore, RedisKVStore, SqliteCatalog,
    SqliteKVStore, SqliteTargetDatabase, SqlxTargetDatabase,
};
use async_trait::async_trait;
use serde::{de, Deserialize};
use sqlx::Row;
use tokio::sync::Mutex;

use crate::RuntimeDeps;

const INDEX_REBUILD_KIND_LIMIT: usize = 100_000;
const MAX_CATALOG_INDEX_WRITE_LOCKS: usize = 128;

static CATALOG_INDEX_WRITE_LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

/// Full data-environment storage config accepted by the runtime.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataEnvironmentConfig {
    /// Optional catalog scope tenant. Runtime construction rejects this when it
    /// does not match the runtime tenant; command-line data jobs may use it as
    /// their default write scope.
    #[serde(
        default,
        rename = "tenantId",
        alias = "tenant_id",
        deserialize_with = "deserialize_optional_tenant_id"
    )]
    pub tenant_id: Option<TenantId>,
    /// Optional catalog scope workspace. Defaults to the framework's default
    /// workspace when omitted.
    #[serde(
        default,
        rename = "workspaceId",
        alias = "workspace_id",
        deserialize_with = "deserialize_optional_workspace_id"
    )]
    pub workspace_id: Option<WorkspaceId>,
    /// Runtime KV state: references, plans, pending approval audit, caches.
    #[serde(default)]
    pub kv: Option<KvStorageConfig>,
    /// Data catalog used by catalog-backed tools.
    #[serde(default)]
    pub catalog: Option<CatalogStorageConfig>,
    /// Tantivy catalog search index used by the replacement catalog toolkit.
    #[serde(default, rename = "catalogSearch", alias = "catalog_search")]
    pub catalog_search: Option<CatalogSearchConfig>,
    /// Structured target database descriptor.
    #[serde(default, alias = "target_database")]
    pub target_database: Option<TargetDatabaseStorageConfig>,
    /// Backwards-compatible target DB URL shortcut from target DB URL compatibility.
    #[serde(default, rename = "targetDatabaseUrl", alias = "target_database_url")]
    pub legacy_target_database_url: Option<String>,
    /// Backwards-compatible target DB schema shortcut.
    #[serde(
        default,
        rename = "targetDatabaseSchema",
        alias = "target_database_schema"
    )]
    pub legacy_target_database_schema: Option<String>,
}

/// Runtime KV storage options.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum KvStorageConfig {
    Memory,
    Sqlite {
        url: String,
        #[serde(default, rename = "ensureSchema", alias = "ensure_schema")]
        ensure_schema: bool,
    },
    Postgres {
        #[serde(default)]
        url: Option<String>,
        #[serde(default, rename = "urlEnv", alias = "url_env")]
        url_env: Option<String>,
        #[serde(default)]
        table: Option<String>,
        #[serde(default, rename = "ensureSchema", alias = "ensure_schema")]
        ensure_schema: bool,
    },
    Redis {
        #[serde(default)]
        url: Option<String>,
        #[serde(default, rename = "urlEnv", alias = "url_env")]
        url_env: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
    },
}

/// Data catalog storage options.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum CatalogStorageConfig {
    Inline {
        #[serde(default)]
        entries: Vec<CatalogEntry>,
    },
    Empty,
    Sqlite {
        url: String,
        #[serde(default, rename = "ensureSchema", alias = "ensure_schema")]
        ensure_schema: bool,
    },
    Postgres {
        #[serde(default)]
        url: Option<String>,
        #[serde(default, rename = "urlEnv", alias = "url_env")]
        url_env: Option<String>,
        #[serde(default, rename = "ensureSchema", alias = "ensure_schema")]
        ensure_schema: bool,
    },
}

/// Tantivy catalog search lifecycle options.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogSearchConfig {
    /// Root directory for `.agent-fw/indexes/catalog/<tenant>/<workspace>`.
    #[serde(rename = "indexPath", alias = "index_path")]
    pub index_path: PathBuf,
    /// Rebuild the scope index before the runtime serves catalog search.
    #[serde(default, rename = "rebuildOnStart", alias = "rebuild_on_start")]
    pub rebuild_on_start: bool,
    /// Update the Tantivy side index after catalog writes. Failures mark the
    /// index stale and do not roll back the authoritative catalog write.
    #[serde(default, rename = "writeThrough", alias = "write_through")]
    pub write_through: bool,
}

/// Read-only target database storage options.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum TargetDatabaseStorageConfig {
    Sqlite {
        url: String,
    },
    Postgres {
        #[serde(default)]
        url: Option<String>,
        #[serde(default, rename = "urlEnv", alias = "url_env")]
        url_env: Option<String>,
        #[serde(default)]
        schema: Option<String>,
    },
}

/// Error returned while parsing or opening storage descriptors.
#[derive(Debug, thiserror::Error)]
pub enum StorageConfigError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Open(String),
}

/// Opened catalog backend with both read and write interfaces.
///
/// Profiling and ingestion commands need a durable sink for catalog artifact
/// writes, but later command phases also need the read surface for validation,
/// parity checks, or follow-up queries. This helper keeps both trait-object
/// views over the same underlying backend.
#[derive(Clone)]
pub struct OpenedCatalog {
    pub reader: Arc<dyn DataCatalog>,
    pub writer: Arc<dyn CatalogWriter>,
}

/// Summary returned after rebuilding one scoped catalog search index.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchRebuildSummary {
    pub tenant_id: String,
    pub workspace_id: String,
    pub indexed_entries: usize,
    pub skipped_entries: usize,
    pub warnings: Vec<String>,
}

/// Health report for one scoped catalog search index.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchDoctorReport {
    pub tenant_id: String,
    pub workspace_id: String,
    pub index_path: String,
    pub health: CatalogSearchHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_diagnostics: Option<CatalogRelationDiagnostics>,
}

/// Runtime-owned Tantivy lifecycle handle.
///
/// The handle is also the `CatalogSearchBackend` attached to tool
/// environments. Runtime-owned write paths call the explicit lifecycle methods
/// so `rebuild` / `upsert` / `delete` are serialized per scope.
pub struct CatalogSearchIndexHandle {
    index: Arc<TantivyCatalogIndex>,
}

impl CatalogSearchIndexHandle {
    pub fn new(index_path: impl Into<PathBuf>) -> Self {
        Self {
            index: Arc::new(TantivyCatalogIndex::new(index_path.into())),
        }
    }

    pub fn index(&self) -> &TantivyCatalogIndex {
        &self.index
    }

    async fn write_lock_for_scope(&self, scope: &CatalogScope) -> Arc<Mutex<()>> {
        let key = self
            .index
            .paths()
            .scope_path(scope)
            .to_string_lossy()
            .to_string();
        let mut locks = CATALOG_INDEX_WRITE_LOCKS
            .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !locks.contains_key(&key) && locks.len() >= MAX_CATALOG_INDEX_WRITE_LOCKS {
            let Some(evict_key) = locks
                .iter()
                .find(|(_, lock)| Arc::strong_count(lock) == 1)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            locks.remove(&evict_key);
        }
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn rebuild_from_catalog(
        &self,
        scope: &CatalogScope,
        catalog: &dyn DataCatalog,
        kv: Option<&dyn KVStore>,
    ) -> Result<CatalogSearchRebuildSummary, StorageConfigError> {
        let lock = self.write_lock_for_scope(scope).await;
        let _guard = lock.lock().await;
        let mut projections = Vec::new();
        let mut warnings = Vec::new();
        let mut skipped_entries = 0usize;
        let workspace_tenant_id =
            WorkspaceContext::from_ids(scope.tenant_id.clone(), Some(scope.workspace_id.as_str()))
                .workspace_tenant_id()
                .to_string();

        for kind in indexed_catalog_kinds() {
            let entries = catalog
                .list_by_type(kind, INDEX_REBUILD_KIND_LIMIT)
                .await
                .map_err(|error| {
                    StorageConfigError::Open(format!(
                        "failed to list {kind} entries for catalog index rebuild: {error}"
                    ))
                })?;
            if entries.len() == INDEX_REBUILD_KIND_LIMIT {
                warnings.push(format!(
                    "catalog index rebuild reached the per-kind limit of {INDEX_REBUILD_KIND_LIMIT} for kind {kind}"
                ));
            }
            for entry in entries {
                match project_catalog_entry(entry, kv, &workspace_tenant_id).await {
                    Ok((Some(projection), entry_warnings)) => {
                        warnings.extend(entry_warnings);
                        projections.push(projection);
                    }
                    Ok((None, entry_warnings)) => warnings.extend(entry_warnings),
                    Err(message) => {
                        skipped_entries += 1;
                        warnings.push(message);
                    }
                }
            }
        }

        let indexed_entries = projections.len();
        // A successful rebuild yields a Ready index even when some entries were
        // skipped or a per-kind limit was hit: those are partial-success signals
        // surfaced via `warnings` / `skipped_entries` in the returned summary (and
        // the rebuild CLI), not index-unusable conditions. `TantivyCatalogIndex::
        // rebuild` already clears any prior stale marker on success. Genuinely
        // unusable rebuilds (catalog enumeration or Tantivy commit failure) are
        // the `?`-propagated Err paths above; the write-through-failure stale path
        // lives in `mark_stale_after_write_through_failure` and is unaffected.
        // `TantivyCatalogIndex::rebuild` runs blocking filesystem I/O
        // (`delete_all_documents`, `add_document`, `commit`). Offload it to the
        // blocking pool so the commit never consumes an async worker thread.
        // The per-scope write guard (`_guard`) is still held across this
        // `.await`, which only yields the async task; it does not release the
        // guard, so per-scope write serialization is preserved.
        let index = self.index.clone();
        let scope_owned = scope.clone();
        tokio::task::spawn_blocking(move || index.rebuild(&scope_owned, projections))
            .await
            .map_err(|join| {
                StorageConfigError::Open(format!(
                    "catalog search index rebuild task failed: {join}"
                ))
            })?
            .map_err(|error| {
                StorageConfigError::Open(format!("failed to rebuild catalog search index: {error}"))
            })?;

        Ok(CatalogSearchRebuildSummary {
            tenant_id: scope.tenant_id.as_str().to_string(),
            workspace_id: scope.workspace_id.as_str().to_string(),
            indexed_entries,
            skipped_entries,
            warnings,
        })
    }

    async fn upsert_after_catalog_write(
        &self,
        scope: &CatalogScope,
        items: &[CatalogEntry],
        kv: Option<&dyn KVStore>,
        workspace_tenant_id: &str,
    ) -> Result<(), String> {
        let lock = self.write_lock_for_scope(scope).await;
        let _guard = lock.lock().await;
        self.ensure_write_through_index_ready(scope).await?;
        // Projection awaits KV (document body hydration), so it must stay on the
        // async side. Collect every projection first, then commit them in a
        // single blocking task: each `upsert` calls `IndexWriter::commit`
        // (blocking I/O), and batching avoids per-item thread-pool churn while
        // keeping the whole multi-item write under the one per-scope guard.
        let mut projections = Vec::new();
        for item in items {
            let (projection, warnings) =
                project_catalog_entry(item.clone(), kv, workspace_tenant_id).await?;
            for warning in warnings {
                tracing::warn!(
                    tenant_id = %scope.tenant_id.as_str(),
                    workspace_id = %scope.workspace_id.as_str(),
                    warning = %warning,
                    "catalog search write-through projection warning"
                );
            }
            if let Some(projection) = projection {
                projections.push(projection);
            }
        }
        let index = self.index.clone();
        let scope_owned = scope.clone();
        tokio::task::spawn_blocking(move || {
            for projection in projections {
                index.upsert(&scope_owned, projection)?;
            }
            Ok::<(), agent_fw_catalog_index::CatalogIndexError>(())
        })
        .await
        .map_err(|join| format!("catalog search write-through upsert task failed: {join}"))?
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn delete_after_catalog_write(
        &self,
        scope: &CatalogScope,
        ids: &[String],
    ) -> Result<(), String> {
        let lock = self.write_lock_for_scope(scope).await;
        let _guard = lock.lock().await;
        self.ensure_write_through_index_ready(scope).await?;
        // Each `delete` calls `IndexWriter::commit` (blocking I/O); batch them
        // into one blocking task under the single per-scope guard.
        let index = self.index.clone();
        let scope_owned = scope.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            for id in &ids {
                index.delete(&scope_owned, id)?;
            }
            Ok::<(), agent_fw_catalog_index::CatalogIndexError>(())
        })
        .await
        .map_err(|join| format!("catalog search write-through delete task failed: {join}"))?
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn ensure_write_through_index_ready(&self, scope: &CatalogScope) -> Result<(), String> {
        match self
            .index
            .health(scope)
            .await
            .map_err(|error| error.to_string())?
        {
            CatalogSearchHealth::Ready { .. } => Ok(()),
            CatalogSearchHealth::Stale { reason, .. } => Err(format!(
                "catalog search index is stale before write-through: {reason}"
            )),
            CatalogSearchHealth::Unavailable { reason } => Err(format!(
                "catalog search index is unavailable before write-through: {reason}"
            )),
        }
    }

    async fn mark_stale_after_write_through_failure(&self, scope: &CatalogScope, reason: &str) {
        let lock = self.write_lock_for_scope(scope).await;
        let _guard = lock.lock().await;
        // `mark_stale` does a single small `std::fs::write`. Offload it for
        // consistency so no blocking filesystem write runs on an async worker
        // under the per-scope guard.
        let index = self.index.clone();
        let scope_owned = scope.clone();
        let reason_owned = reason.to_string();
        let result =
            tokio::task::spawn_blocking(move || index.mark_stale(&scope_owned, &reason_owned))
                .await;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(join) => {
                tracing::warn!(
                    tenant_id = %scope.tenant_id.as_str(),
                    workspace_id = %scope.workspace_id.as_str(),
                    error = %join,
                    "catalog search index mark-stale task failed"
                );
                return;
            }
        };
        if let Err(error) = outcome {
            tracing::warn!(
                tenant_id = %scope.tenant_id.as_str(),
                workspace_id = %scope.workspace_id.as_str(),
                error = %error,
                "failed to mark catalog search index stale after write-through failure"
            );
        }
    }
}

#[async_trait]
impl CatalogSearchBackend for CatalogSearchIndexHandle {
    async fn search(
        &self,
        scope: &CatalogScope,
        request: agent_fw_catalog::CatalogSearchRequest,
    ) -> Result<agent_fw_catalog::CatalogSearchResults, CatalogError> {
        self.index.search(scope, request).await
    }

    async fn health(&self, scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogError> {
        self.index.health(scope).await
    }
}

struct WriteThroughCatalogWriter {
    inner: Arc<dyn CatalogWriter>,
    index: Arc<CatalogSearchIndexHandle>,
    scope: CatalogScope,
    write_through: bool,
    kv: Option<Arc<dyn KVStore>>,
    workspace_tenant_id: String,
}

impl WriteThroughCatalogWriter {
    fn new(
        inner: Arc<dyn CatalogWriter>,
        index: Arc<CatalogSearchIndexHandle>,
        scope: CatalogScope,
        write_through: bool,
        kv: Option<Arc<dyn KVStore>>,
    ) -> Self {
        let workspace_tenant_id =
            WorkspaceContext::from_ids(scope.tenant_id.clone(), Some(scope.workspace_id.as_str()))
                .workspace_tenant_id()
                .to_string();
        Self {
            inner,
            index,
            scope,
            write_through,
            kv,
            workspace_tenant_id,
        }
    }

    async fn update_index_after_save(&self, items: &[CatalogEntry]) {
        if !self.write_through {
            self.mark_stale_for_deferred_rebuild("catalog write").await;
            return;
        }
        if let Err(reason) = self
            .index
            .upsert_after_catalog_write(
                &self.scope,
                items,
                self.kv.as_deref(),
                &self.workspace_tenant_id,
            )
            .await
        {
            let reason = format!("catalog search write-through upsert failed: {reason}");
            tracing::warn!(
                tenant_id = %self.scope.tenant_id.as_str(),
                workspace_id = %self.scope.workspace_id.as_str(),
                reason = %reason,
                "catalog search write-through failed after catalog write"
            );
            self.index
                .mark_stale_after_write_through_failure(&self.scope, &reason)
                .await;
        }
    }

    async fn update_index_after_delete(&self, ids: &[String]) {
        if !self.write_through {
            self.mark_stale_for_deferred_rebuild("catalog delete").await;
            return;
        }
        if let Err(reason) = self
            .index
            .delete_after_catalog_write(&self.scope, ids)
            .await
        {
            let reason = format!("catalog search write-through delete failed: {reason}");
            tracing::warn!(
                tenant_id = %self.scope.tenant_id.as_str(),
                workspace_id = %self.scope.workspace_id.as_str(),
                reason = %reason,
                "catalog search write-through failed after catalog delete"
            );
            self.index
                .mark_stale_after_write_through_failure(&self.scope, &reason)
                .await;
        }
    }

    async fn mark_stale_for_deferred_rebuild(&self, operation: &str) {
        let reason =
            format!("catalog search write-through is disabled; rebuild required after {operation}");
        self.index
            .mark_stale_after_write_through_failure(&self.scope, &reason)
            .await;
    }
}

#[async_trait]
impl CatalogWriter for WriteThroughCatalogWriter {
    async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
        let ids = self.inner.save_items(items.clone()).await?;
        self.update_index_after_save(&items).await;
        Ok(ids)
    }

    async fn delete_items(&self, ids: &[String]) -> Result<u32, CatalogError> {
        let deleted = self.inner.delete_items(ids).await?;
        self.update_index_after_delete(ids).await;
        Ok(deleted)
    }

    async fn save_in_transaction(
        &self,
        items: Vec<CatalogEntry>,
    ) -> Result<Vec<String>, CatalogError> {
        let ids = self.inner.save_in_transaction(items.clone()).await?;
        self.update_index_after_save(&items).await;
        Ok(ids)
    }
}

fn deserialize_optional_tenant_id<'de, D>(deserializer: D) -> Result<Option<TenantId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            TenantId::new(value).ok_or_else(|| de::Error::custom("tenant ID must not be blank"))
        })
        .transpose()
}

fn deserialize_optional_workspace_id<'de, D>(
    deserializer: D,
) -> Result<Option<WorkspaceId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            WorkspaceId::new(value)
                .ok_or_else(|| de::Error::custom("workspace ID must not be blank"))
        })
        .transpose()
}

/// Apply a storage config to already-constructed runtime dependencies.
///
/// Rust callers may continue injecting dependencies directly through
/// [`RuntimeDeps`]. This helper exists for language facades and simple hosts
/// that want the runtime to construct supported framework interpreters.
pub async fn apply_to_runtime_deps(
    mut deps: RuntimeDeps,
    config: DataEnvironmentConfig,
) -> Result<RuntimeDeps, StorageConfigError> {
    let catalog_scope = catalog_scope_for_runtime_deps(&deps, &config)?;
    deps = deps.with_data_workspace_context(WorkspaceContext::from_ids(
        catalog_scope.tenant_id.clone(),
        Some(catalog_scope.workspace_id.as_str()),
    ));

    if let Some(kv) = config.kv.clone() {
        deps.kv = build_kv_store(kv).await?;
    }

    if let Some(catalog) = config.catalog.clone() {
        deps =
            deps.with_data_catalog(build_catalog_for_scope(catalog, catalog_scope.clone()).await?);
    }

    if let Some(search) = config.catalog_search.clone() {
        let handle = Arc::new(CatalogSearchIndexHandle::new(search.index_path));
        if search.rebuild_on_start {
            let Some(catalog) = deps.data_catalog.as_ref() else {
                return Err(StorageConfigError::Invalid(
                    "data_environment.catalog_search.rebuild_on_start requires data_environment.catalog"
                        .to_string(),
                ));
            };
            handle
                .rebuild_from_catalog(&catalog_scope, catalog.as_ref(), Some(deps.kv.as_ref()))
                .await?;
        }
        let backend: Arc<dyn CatalogSearchBackend> = handle;
        deps = deps.with_catalog_search_backend(backend);
    }

    if config.target_database.is_some() && config.legacy_target_database_url.is_some() {
        return Err(StorageConfigError::Invalid(
            "data_environment accepts either target_database or target_database_url, not both"
                .to_string(),
        ));
    }

    if let Some(target) = config.target_database {
        deps = deps.with_target_database(build_target_database(target).await?);
    } else if let Some(url) = config.legacy_target_database_url {
        let schema = config
            .legacy_target_database_schema
            .unwrap_or_else(|| "public".to_string());
        deps = deps.with_target_database(build_legacy_target_database(&url, &schema).await?);
    }

    Ok(deps)
}

/// Resolve a target database directly from a full `DataEnvironmentConfig`.
pub async fn build_target_database_from_environment(
    config: &DataEnvironmentConfig,
) -> Result<Arc<dyn TargetDatabase>, StorageConfigError> {
    if config.target_database.is_some() && config.legacy_target_database_url.is_some() {
        return Err(StorageConfigError::Invalid(
            "data_environment accepts either target_database or target_database_url, not both"
                .to_string(),
        ));
    }

    if let Some(target) = config.target_database.clone() {
        build_target_database(target).await
    } else if let Some(url) = &config.legacy_target_database_url {
        let schema = config
            .legacy_target_database_schema
            .clone()
            .unwrap_or_else(|| "public".to_string());
        build_legacy_target_database(url, &schema).await
    } else {
        Err(StorageConfigError::Invalid(
            "profiling/ingestion requires data_environment.target_database or data_environment.target_database_url"
                .to_string(),
        ))
    }
}

/// Resolve a writable catalog backend directly from a full `DataEnvironmentConfig`.
pub async fn open_writable_catalog_from_environment(
    config: &DataEnvironmentConfig,
) -> Result<OpenedCatalog, StorageConfigError> {
    let scope = catalog_scope_from_data_environment(
        config,
        TenantId::new_unchecked(crate::data::DEFAULT_DATA_TENANT_ID),
    );
    open_writable_catalog_from_environment_for_scope(config, scope).await
}

/// Resolve a writable catalog backend for a caller-selected catalog scope.
pub async fn open_writable_catalog_from_environment_for_scope(
    config: &DataEnvironmentConfig,
    scope: CatalogScope,
) -> Result<OpenedCatalog, StorageConfigError> {
    let Some(catalog) = config.catalog.clone() else {
        return Err(StorageConfigError::Invalid(
            "profiling/ingestion requires data_environment.catalog".to_string(),
        ));
    };
    let mut opened = open_catalog_for_writes_for_scope(catalog, scope.clone()).await?;
    if let Some(search) = config.catalog_search.clone() {
        let handle = Arc::new(CatalogSearchIndexHandle::new(search.index_path));
        let kv = if search.rebuild_on_start || search.write_through {
            match &config.kv {
                Some(_) => Some(build_kv_store_from_environment(config).await?),
                None => None,
            }
        } else {
            None
        };
        if search.rebuild_on_start {
            handle
                .rebuild_from_catalog(&scope, opened.reader.as_ref(), kv.as_deref())
                .await?;
        }
        opened.writer = Arc::new(WriteThroughCatalogWriter::new(
            opened.writer,
            handle,
            scope,
            search.write_through,
            kv,
        ));
    }
    Ok(opened)
}

/// Resolve a KV store directly from a full `DataEnvironmentConfig`.
pub async fn build_kv_store_from_environment(
    config: &DataEnvironmentConfig,
) -> Result<Arc<dyn KVStore>, StorageConfigError> {
    let Some(kv) = config.kv.clone() else {
        return Err(StorageConfigError::Invalid(
            "knowledge ingestion requires data_environment.kv".to_string(),
        ));
    };
    build_kv_store(kv).await
}

/// Build the configured catalog search backend as a trait object.
pub fn build_catalog_search_backend(
    config: &DataEnvironmentConfig,
) -> Result<Arc<dyn CatalogSearchBackend>, StorageConfigError> {
    let handle = build_catalog_search_index_handle(config)?;
    Ok(handle)
}

/// Build the configured catalog search lifecycle handle.
pub fn build_catalog_search_index_handle(
    config: &DataEnvironmentConfig,
) -> Result<Arc<CatalogSearchIndexHandle>, StorageConfigError> {
    let Some(search) = &config.catalog_search else {
        return Err(StorageConfigError::Invalid(
            "data_environment.catalog_search is required for catalog index lifecycle commands"
                .to_string(),
        ));
    };
    Ok(Arc::new(CatalogSearchIndexHandle::new(
        search.index_path.clone(),
    )))
}

/// Rebuild the configured catalog search index for one scope.
pub async fn rebuild_catalog_search_index_from_environment(
    config: &DataEnvironmentConfig,
    scope: CatalogScope,
) -> Result<CatalogSearchRebuildSummary, StorageConfigError> {
    let Some(catalog) = config.catalog.clone() else {
        return Err(StorageConfigError::Invalid(
            "catalog index rebuild requires data_environment.catalog".to_string(),
        ));
    };
    let catalog = build_catalog_for_scope(catalog, scope.clone()).await?;
    let kv = match &config.kv {
        Some(_) => Some(build_kv_store_from_environment(config).await?),
        None => None,
    };
    let handle = build_catalog_search_index_handle(config)?;
    handle
        .rebuild_from_catalog(&scope, catalog.as_ref(), kv.as_deref())
        .await
}

/// Inspect configured catalog search index health for one scope.
pub async fn doctor_catalog_search_index_from_environment(
    config: &DataEnvironmentConfig,
    scope: CatalogScope,
) -> Result<CatalogSearchDoctorReport, StorageConfigError> {
    let handle = build_catalog_search_index_handle(config)?;
    let health = handle.health(&scope).await.map_err(|error| {
        StorageConfigError::Open(format!("failed to inspect catalog search index: {error}"))
    })?;
    let relation_diagnostics = catalog_relation_diagnostics_from_environment(config, scope.clone())
        .await
        .map_err(|error| {
            StorageConfigError::Open(format!(
                "failed to inspect catalog relation diagnostics: {error}"
            ))
        })?;
    Ok(CatalogSearchDoctorReport {
        tenant_id: scope.tenant_id.as_str().to_string(),
        workspace_id: scope.workspace_id.as_str().to_string(),
        index_path: handle
            .index()
            .paths()
            .scope_path(&scope)
            .to_string_lossy()
            .to_string(),
        health,
        relation_diagnostics,
    })
}

async fn catalog_relation_diagnostics_from_environment(
    config: &DataEnvironmentConfig,
    scope: CatalogScope,
) -> Result<Option<CatalogRelationDiagnostics>, StorageConfigError> {
    let Some(catalog_config) = config.catalog.clone() else {
        return Ok(None);
    };
    let catalog = build_catalog_for_scope(catalog_config.clone(), scope.clone()).await?;
    let mut entries = Vec::new();
    for kind in indexed_catalog_kinds() {
        let mut kind_entries = catalog
            .list_by_type(kind, INDEX_REBUILD_KIND_LIMIT)
            .await
            .map_err(|error| {
                StorageConfigError::Open(format!(
                    "failed to list {kind} entries for catalog relation diagnostics: {error}"
                ))
            })?;
        entries.append(&mut kind_entries);
    }
    let mut diagnostics = diagnose_catalog_relations(&entries);
    record_missing_relation_sources(&catalog_config, &scope, &mut diagnostics).await?;
    Ok(Some(diagnostics))
}

async fn record_missing_relation_sources(
    catalog_config: &CatalogStorageConfig,
    scope: &CatalogScope,
    diagnostics: &mut CatalogRelationDiagnostics,
) -> Result<(), StorageConfigError> {
    match catalog_config {
        CatalogStorageConfig::Sqlite { url, .. } => {
            let conn = rusqlite::Connection::open(sqlite_path(url)).map_err(|error| {
                StorageConfigError::Open(format!(
                    "failed to open catalog sqlite store '{}' for relation diagnostics: {error}",
                    redact_url(url)
                ))
            })?;
            let mut stmt = conn
                .prepare(
                    "SELECT r.source_id, r.target_id, r.kind
                     FROM catalog_relations r
                     LEFT JOIN catalog_entries source
                       ON source.tenant_id = r.tenant_id
                      AND source.workspace_id = r.workspace_id
                      AND source.id = r.source_id
                     WHERE r.tenant_id = ?1
                       AND r.workspace_id = ?2
                       AND source.id IS NULL",
                )
                .map_err(|error| {
                    StorageConfigError::Open(format!(
                        "failed to prepare catalog sqlite relation diagnostics: {error}"
                    ))
                })?;
            let rows = stmt
                .query_map(
                    [
                        scope.tenant_id.as_str().to_string(),
                        scope.workspace_id.as_str().to_string(),
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|error| {
                    StorageConfigError::Open(format!(
                        "failed to query catalog sqlite relation diagnostics: {error}"
                    ))
                })?;
            for row in rows {
                let (source_id, target_id, kind) = row.map_err(|error| {
                    StorageConfigError::Open(format!(
                        "failed to read catalog sqlite relation diagnostics row: {error}"
                    ))
                })?;
                diagnostics.record_missing_source_relation(source_id, target_id, kind);
            }
        }
        CatalogStorageConfig::Postgres {
            url,
            url_env,
            ensure_schema: _,
        } => {
            let url = resolve_url("catalog", url.clone(), url_env.clone())?;
            let pool = sqlx::PgPool::connect(&url).await.map_err(|error| {
                StorageConfigError::Open(format!(
                    "failed to open catalog postgres store '{}' for relation diagnostics: {error}",
                    redact_url(&url)
                ))
            })?;
            let rows = sqlx::query(
                "SELECT r.source_id, r.target_id, r.kind
                 FROM catalog_relations r
                 LEFT JOIN catalog_entries source
                   ON source.tenant_id = r.tenant_id
                  AND source.workspace_id = r.workspace_id
                  AND source.id = r.source_id
                 WHERE r.tenant_id = $1
                   AND r.workspace_id = $2
                   AND source.id IS NULL",
            )
            .bind(scope.tenant_id.as_str())
            .bind(scope.workspace_id.as_str())
            .fetch_all(&pool)
            .await
            .map_err(|error| {
                StorageConfigError::Open(format!(
                    "failed to query catalog postgres relation diagnostics: {error}"
                ))
            })?;
            for row in rows {
                diagnostics.record_missing_source_relation(
                    row.try_get::<String, _>("source_id").map_err(|error| {
                        StorageConfigError::Open(format!(
                            "failed to read catalog postgres relation diagnostic source_id: {error}"
                        ))
                    })?,
                    row.try_get::<String, _>("target_id").map_err(|error| {
                        StorageConfigError::Open(format!(
                            "failed to read catalog postgres relation diagnostic target_id: {error}"
                        ))
                    })?,
                    row.try_get::<String, _>("kind").map_err(|error| {
                        StorageConfigError::Open(format!(
                            "failed to read catalog postgres relation diagnostic kind: {error}"
                        ))
                    })?,
                );
            }
        }
        CatalogStorageConfig::Inline { .. } | CatalogStorageConfig::Empty => {}
    }
    Ok(())
}

fn indexed_catalog_kinds() -> impl Iterator<Item = CatalogKind> {
    [
        CatalogKind::Table,
        CatalogKind::Column,
        CatalogKind::Relationship,
        CatalogKind::Enum,
        CatalogKind::Metric,
        CatalogKind::Document,
        CatalogKind::Knowledge,
        CatalogKind::DataQualityFinding,
    ]
    .into_iter()
}

async fn project_catalog_entry(
    entry: CatalogEntry,
    kv: Option<&dyn KVStore>,
    workspace_tenant_id: &str,
) -> Result<(Option<CatalogDocumentProjection>, Vec<String>), String> {
    if !entry.kind.is_public_searchable() {
        return Ok((None, Vec::new()));
    }
    let entry_id = entry.id.clone();
    let entity = SemanticEntity::try_from(entry)
        .map_err(|error| format!("skipped catalog entry {entry_id}: {error}"))?;
    let mut warnings = Vec::new();
    let document_body = match (&entity, kv) {
        (SemanticEntity::Document { metadata, .. }, Some(kv)) => {
            match knowledge_store::get_document(
                kv,
                workspace_tenant_id,
                &metadata.source_document_id,
            )
            .await
            {
                Ok(Some(document)) => Some(document.content),
                Ok(None) => {
                    if metadata.content_available {
                        warnings.push(format!(
                            "document body for catalog entry {entry_id} is marked available but was not found in KV; indexing metadata only"
                        ));
                    }
                    None
                }
                Err(error) => {
                    return Err(format!(
                        "skipped document body for catalog entry {entry_id}: {error}"
                    ));
                }
            }
        }
        (SemanticEntity::Document { metadata, .. }, None) if metadata.content_available => {
            warnings.push(format!(
                "document body for catalog entry {entry_id} is marked available but no KV store is configured; indexing metadata only"
            ));
            None
        }
        _ => None,
    };
    CatalogDocumentProjection::project(&entity, document_body)
        .map(|projection| (Some(projection), warnings))
        .map_err(|error| format!("skipped catalog entry {entry_id}: {error}"))
}

/// Build a KV store from a descriptor.
pub async fn build_kv_store(
    config: KvStorageConfig,
) -> Result<Arc<dyn KVStore>, StorageConfigError> {
    match config {
        KvStorageConfig::Memory => Ok(Arc::new(DashMapKVStore::new())),
        KvStorageConfig::Sqlite { url, .. } => {
            let path = sqlite_path(&url);
            let store = SqliteKVStore::open(path).map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open kv sqlite store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            Ok(Arc::new(store))
        }
        KvStorageConfig::Postgres {
            url,
            url_env,
            table,
            ensure_schema,
        } => {
            if let Some(table) = table.as_deref() {
                validate_sql_identifier("data_environment.kv.table", table)?;
            }
            let url = resolve_url("kv", url, url_env)?;
            let mut store = PostgresKVStore::connect(&url).await.map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open kv postgres store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            if let Some(table) = table {
                store = store.with_table(table);
            }
            if ensure_schema {
                store.ensure_schema().await.map_err(|err| {
                    StorageConfigError::Open(format!(
                        "failed to initialize kv postgres store '{}': {err}",
                        redact_url(&url)
                    ))
                })?;
            }
            Ok(Arc::new(store))
        }
        KvStorageConfig::Redis {
            url,
            url_env,
            prefix,
        } => {
            let url = resolve_url("kv", url, url_env)?;
            let store = match prefix {
                Some(prefix) => RedisKVStore::connect_with_prefix(&url, &prefix).await,
                None => RedisKVStore::connect(&url).await,
            }
            .map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open kv redis store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            Ok(Arc::new(store))
        }
    }
}

/// Build a data catalog from a descriptor.
pub async fn build_catalog(
    config: CatalogStorageConfig,
) -> Result<Arc<dyn DataCatalog>, StorageConfigError> {
    build_catalog_for_scope(config, CatalogScope::legacy_unscoped()).await
}

/// Build a data catalog from a descriptor under an explicit catalog scope.
pub async fn build_catalog_for_scope(
    config: CatalogStorageConfig,
    scope: CatalogScope,
) -> Result<Arc<dyn DataCatalog>, StorageConfigError> {
    match config {
        CatalogStorageConfig::Inline { entries } => {
            let catalog = Arc::new(MockCatalog::new());
            catalog.load(entries).await;
            Ok(catalog)
        }
        CatalogStorageConfig::Empty => Ok(Arc::new(MockCatalog::new())),
        CatalogStorageConfig::Sqlite { url, .. } => {
            let path = sqlite_path(&url);
            let catalog = SqliteCatalog::open(path).map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open catalog sqlite store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            Ok(Arc::new(catalog.with_scope(scope)))
        }
        CatalogStorageConfig::Postgres {
            url,
            url_env,
            ensure_schema,
        } => {
            let url = resolve_url("catalog", url, url_env)?;
            let catalog = PostgresCatalog::connect(&url).await.map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open catalog postgres store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            if ensure_schema {
                catalog.ensure_schema().await.map_err(|err| {
                    StorageConfigError::Open(format!(
                        "failed to initialize catalog postgres store '{}': {err}",
                        redact_url(&url)
                    ))
                })?;
            }
            Ok(Arc::new(catalog.with_scope(scope)))
        }
    }
}

/// Open a durable catalog backend for profiling / ingestion writes.
///
/// `inline` and `empty` catalogs are intentionally rejected here. They are
/// valid read-only runtime inputs for toolkit dispatch, but they are not
/// durable sinks for generated profiling artifacts.
pub async fn open_catalog_for_writes(
    config: CatalogStorageConfig,
) -> Result<OpenedCatalog, StorageConfigError> {
    open_catalog_for_writes_for_scope(config, CatalogScope::legacy_unscoped()).await
}

/// Open a durable catalog backend for profiling / ingestion writes under an
/// explicit tenant/workspace scope.
pub async fn open_catalog_for_writes_for_scope(
    config: CatalogStorageConfig,
    scope: CatalogScope,
) -> Result<OpenedCatalog, StorageConfigError> {
    match config {
        CatalogStorageConfig::Inline { .. } => Err(StorageConfigError::Invalid(
            "profiling/ingestion requires a durable catalog backend; data_environment.catalog kind=inline is read-only"
                .to_string(),
        )),
        CatalogStorageConfig::Empty => Err(StorageConfigError::Invalid(
            "profiling/ingestion requires a durable catalog backend; data_environment.catalog kind=empty is read-only"
                .to_string(),
        )),
        CatalogStorageConfig::Sqlite { url, .. } => {
            let path = sqlite_path(&url);
            let catalog = Arc::new(
                SqliteCatalog::open(path)
                    .map_err(|err| {
                        StorageConfigError::Open(format!(
                            "failed to open writable catalog sqlite store '{}': {err}",
                            redact_url(&url)
                        ))
                    })?
                    .with_scope(scope),
            );
            Ok(OpenedCatalog {
                reader: catalog.clone(),
                writer: catalog,
            })
        }
        CatalogStorageConfig::Postgres {
            url,
            url_env,
            ensure_schema,
        } => {
            let url = resolve_url("catalog", url, url_env)?;
            let catalog = PostgresCatalog::connect(&url).await.map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open writable catalog postgres store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            if ensure_schema {
                catalog.ensure_schema().await.map_err(|err| {
                    StorageConfigError::Open(format!(
                        "failed to initialize writable catalog postgres store '{}': {err}",
                        redact_url(&url)
                    ))
                })?;
            }
            let catalog = Arc::new(catalog.with_scope(scope));
            Ok(OpenedCatalog {
                reader: catalog.clone(),
                writer: catalog,
            })
        }
    }
}

/// Derive the catalog scope used by runtime-attached catalog dependencies.
///
/// Runtime tenant identity is authoritative. A tenant id inside
/// `data_environment` is accepted only when it matches the runtime tenant so a
/// Python or host-language descriptor cannot rebind catalog authorization.
pub fn catalog_scope_for_runtime_deps(
    deps: &RuntimeDeps,
    config: &DataEnvironmentConfig,
) -> Result<CatalogScope, StorageConfigError> {
    let runtime_tenant = deps.tenant.resource_id().clone();
    if let Some(config_tenant) = &config.tenant_id {
        if config_tenant != &runtime_tenant {
            return Err(StorageConfigError::Invalid(format!(
                "data_environment.tenant_id '{}' must match runtime tenant '{}'",
                config_tenant.as_str(),
                runtime_tenant.as_str()
            )));
        }
    }
    Ok(CatalogScope::new(
        runtime_tenant,
        config
            .workspace_id
            .clone()
            .unwrap_or_else(WorkspaceId::default_workspace),
    ))
}

/// Derive a catalog scope for data commands that do not already have a
/// `RuntimeDeps` tenant context.
pub fn catalog_scope_from_data_environment(
    config: &DataEnvironmentConfig,
    default_tenant_id: TenantId,
) -> CatalogScope {
    CatalogScope::new(
        config.tenant_id.clone().unwrap_or(default_tenant_id),
        config
            .workspace_id
            .clone()
            .unwrap_or_else(WorkspaceId::default_workspace),
    )
}

/// Build a target database from a descriptor.
pub async fn build_target_database(
    config: TargetDatabaseStorageConfig,
) -> Result<Arc<dyn TargetDatabase>, StorageConfigError> {
    match config {
        TargetDatabaseStorageConfig::Sqlite { url } => {
            let path = sqlite_path(&url);
            let db = SqliteTargetDatabase::open(path).map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open target_database sqlite store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            Ok(Arc::new(db))
        }
        TargetDatabaseStorageConfig::Postgres {
            url,
            url_env,
            schema,
        } => {
            if let Some(schema) = schema.as_deref() {
                validate_sql_identifier("data_environment.target_database.schema", schema)?;
            }
            let url = resolve_url("target_database", url, url_env)?;
            let schema = schema.unwrap_or_else(|| "public".to_string());
            let db = SqlxTargetDatabase::connect(&url).await.map_err(|err| {
                StorageConfigError::Open(format!(
                    "failed to open target_database postgres store '{}': {err}",
                    redact_url(&url)
                ))
            })?;
            Ok(Arc::new(db.with_schema(schema)))
        }
    }
}

async fn build_legacy_target_database(
    url: &str,
    schema: &str,
) -> Result<Arc<dyn TargetDatabase>, StorageConfigError> {
    if url.starts_with("sqlite:") {
        return build_target_database(TargetDatabaseStorageConfig::Sqlite {
            url: url.to_string(),
        })
        .await;
    }
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        return build_target_database(TargetDatabaseStorageConfig::Postgres {
            url: Some(url.to_string()),
            url_env: None,
            schema: Some(schema.to_string()),
        })
        .await;
    }
    Err(StorageConfigError::Invalid(format!(
        "data_environment.target_database_url supports sqlite:, postgres://, or postgresql:// URLs; got '{}'",
        redact_url(url)
    )))
}

fn validate_sql_identifier(field: &str, value: &str) -> Result<(), StorageConfigError> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StorageConfigError::Invalid(format!(
            "{field} must be non-empty and contain only [a-zA-Z0-9_]"
        )));
    }
    Ok(())
}

fn resolve_url(
    scope: &str,
    url: Option<String>,
    url_env: Option<String>,
) -> Result<String, StorageConfigError> {
    match (url, url_env) {
        (Some(_), Some(_)) => Err(StorageConfigError::Invalid(format!(
            "data_environment.{scope} accepts either url or url_env, not both"
        ))),
        (Some(url), None) => {
            if url.is_empty() {
                Err(StorageConfigError::Invalid(format!(
                    "data_environment.{scope}.url must not be empty"
                )))
            } else {
                Ok(url)
            }
        }
        (None, Some(var)) => env::var(&var).map_err(|_| {
            StorageConfigError::Invalid(format!(
                "environment variable {var} is not set for data_environment.{scope}.url_env"
            ))
        }),
        (None, None) => Err(StorageConfigError::Invalid(format!(
            "data_environment.{scope} requires url_env or url"
        ))),
    }
}

fn sqlite_path(url: &str) -> &str {
    url.strip_prefix("sqlite:").unwrap_or(url)
}

/// Redact credentials and common secret query parameters in connection strings.
pub fn redact_url(input: &str) -> String {
    let mut redacted = redact_userinfo(input);
    redacted = redact_query_param(&redacted, "password");
    redacted = redact_query_param(&redacted, "apikey");
    redacted = redact_query_param(&redacted, "api_key");
    redacted = redact_query_param(&redacted, "token");
    redacted = redact_query_param(&redacted, "access_token");
    redacted
}

fn redact_userinfo(input: &str) -> String {
    let Some(scheme_idx) = input.find("://") else {
        return input.to_string();
    };
    let authority_start = scheme_idx + 3;
    let authority_end = input[authority_start..]
        .find(['/', '?', '#'])
        .map(|idx| authority_start + idx)
        .unwrap_or(input.len());
    let authority = &input[authority_start..authority_end];
    let Some(at_idx) = authority.rfind('@') else {
        return input.to_string();
    };
    let userinfo = &authority[..at_idx];
    let redacted_userinfo = match userinfo.find(':') {
        Some(colon_idx) => format!("{}:***", &userinfo[..colon_idx]),
        None => "***".to_string(),
    };
    format!(
        "{}{}{}",
        &input[..authority_start],
        redacted_userinfo,
        &input[authority_start + at_idx..]
    )
}

fn redact_query_param(input: &str, param: &str) -> String {
    let Some(query_start) = input.find('?') else {
        return input.to_string();
    };
    let mut output = String::with_capacity(input.len());
    output.push_str(&input[..query_start + 1]);
    let query_and_fragment = &input[query_start + 1..];
    let (query, fragment) = match query_and_fragment.find('#') {
        Some(idx) => (&query_and_fragment[..idx], &query_and_fragment[idx..]),
        None => (query_and_fragment, ""),
    };
    for (idx, part) in query.split('&').enumerate() {
        if idx > 0 {
            output.push('&');
        }
        let key_end = part.find('=');
        if key_end
            .map(|end| part[..end].eq_ignore_ascii_case(param))
            .unwrap_or(false)
        {
            output.push_str(&part[..key_end.expect("checked above")]);
            output.push_str("=***");
        } else {
            output.push_str(part);
        }
    }
    output.push_str(fragment);
    output
}
