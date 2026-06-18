use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tantivy::collector::{Count, DocSetCollector, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, BoostQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, TantivyDocument, Term, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{TextAnalyzer, TokenStream};
use tantivy::{Index, IndexWriter, ReloadPolicy};

use agent_fw_catalog::{
    CatalogError, CatalogScope, CatalogSearchBackend, CatalogSearchFacets, CatalogSearchFilters,
    CatalogSearchHealth, CatalogSearchHitRef, CatalogSearchRequest, CatalogSearchResults,
};

use crate::cursor::{decode_offset, encode_offset, request_signature};
use crate::error::CatalogIndexError;
use crate::facets::FacetAccumulator;
use crate::path::CatalogIndexPaths;
use crate::projection::{CatalogDocumentProjection, PROJECTED_CATALOG_SCHEMA_VERSION};
use crate::schema::{build_catalog_schema, CatalogIndexFields};

const WRITER_MEMORY_BYTES: usize = 50_000_000;
const MAX_FACET_DOCS: usize = 1_000;

#[derive(Debug, Clone)]
pub struct TantivyCatalogIndex {
    paths: CatalogIndexPaths,
    handles: Arc<Mutex<IndexHandlePool>>,
    write_locks: Arc<Mutex<WriteLockPool>>,
    max_handles: usize,
    max_write_locks: usize,
}

impl TantivyCatalogIndex {
    const DEFAULT_MAX_HANDLES: usize = 64;
    const DEFAULT_MAX_WRITE_LOCKS: usize = 128;

    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            paths: CatalogIndexPaths::new(root),
            handles: Arc::new(Mutex::new(IndexHandlePool::default())),
            write_locks: Arc::new(Mutex::new(WriteLockPool::default())),
            max_handles: Self::DEFAULT_MAX_HANDLES,
            max_write_locks: Self::DEFAULT_MAX_WRITE_LOCKS,
        }
    }

    pub fn paths(&self) -> &CatalogIndexPaths {
        &self.paths
    }

    pub fn cached_scope_count(&self) -> usize {
        self.handles
            .lock()
            .map(|handles| handles.len())
            .unwrap_or(0)
    }

    pub fn rebuild<I>(&self, scope: &CatalogScope, projections: I) -> Result<(), CatalogIndexError>
    where
        I: IntoIterator<Item = CatalogDocumentProjection>,
    {
        let lock = self.write_lock_for_scope(scope)?;
        let _guard = lock.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index write lock poisoned: {error}"))
        })?;
        let index_path = self.paths.scope_path(scope);

        let catalog_schema = build_catalog_schema();
        if index_path.exists() {
            self.drop_cached_handle(scope)?;
            std::fs::remove_dir_all(&index_path)?;
        }
        std::fs::create_dir_all(&index_path)?;
        let index = Index::create_in_dir(&index_path, catalog_schema.schema.clone())?;
        let mut writer: IndexWriter<TantivyDocument> = index.writer(WRITER_MEMORY_BYTES)?;
        writer.delete_all_documents()?;
        for projection in projections {
            writer.add_document(projection_document(
                &catalog_schema.fields,
                scope,
                &projection,
            ))?;
        }
        writer.commit()?;
        self.cache_handle(scope, index)?;
        self.clear_stale(scope)?;
        Ok(())
    }

    pub fn upsert(
        &self,
        scope: &CatalogScope,
        projection: CatalogDocumentProjection,
    ) -> Result<(), CatalogIndexError> {
        let lock = self.write_lock_for_scope(scope)?;
        let _guard = lock.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index write lock poisoned: {error}"))
        })?;
        let (index, fields) = self.open_or_create(scope)?;
        let mut writer: IndexWriter<TantivyDocument> = index.writer(WRITER_MEMORY_BYTES)?;
        writer.delete_term(Term::from_field_text(fields.entry_id, &projection.entry_id));
        writer.add_document(projection_document(&fields, scope, &projection))?;
        writer.commit()?;
        Ok(())
    }

    pub fn delete(&self, scope: &CatalogScope, entry_id: &str) -> Result<(), CatalogIndexError> {
        let lock = self.write_lock_for_scope(scope)?;
        let _guard = lock.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index write lock poisoned: {error}"))
        })?;
        let (index, fields) = self.open_or_create(scope)?;
        let mut writer: IndexWriter<TantivyDocument> = index.writer(WRITER_MEMORY_BYTES)?;
        writer.delete_term(Term::from_field_text(fields.entry_id, entry_id));
        writer.commit()?;
        Ok(())
    }

    pub fn mark_stale(&self, scope: &CatalogScope, reason: &str) -> Result<(), CatalogIndexError> {
        let marker = self.paths.stale_marker_path(scope);
        if let Some(parent) = marker.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(marker, reason)?;
        Ok(())
    }

    fn write_lock_for_scope(
        &self,
        scope: &CatalogScope,
    ) -> Result<Arc<Mutex<()>>, CatalogIndexError> {
        let mut locks = self.write_locks.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index write-lock pool: {error}"))
        })?;
        Ok(locks.get_or_insert(scope, self.max_write_locks))
    }

    fn clear_stale(&self, scope: &CatalogScope) -> Result<(), CatalogIndexError> {
        let marker = self.paths.stale_marker_path(scope);
        if marker.exists() {
            std::fs::remove_file(marker)?;
        }
        Ok(())
    }

    fn open_or_create(
        &self,
        scope: &CatalogScope,
    ) -> Result<(Index, CatalogIndexFields), CatalogIndexError> {
        let catalog_schema = build_catalog_schema();
        let index_path = self.paths.scope_path(scope);
        std::fs::create_dir_all(&index_path)?;
        let meta_path = index_path.join("meta.json");
        let index = if meta_path.exists() {
            self.cached_or_open(scope, &index_path)?
        } else {
            let index = Index::create_in_dir(&index_path, catalog_schema.schema.clone())?;
            self.cache_handle(scope, index.clone())?;
            index
        };
        ensure_schema_current(&index, &catalog_schema.schema)?;
        Ok((index, catalog_schema.fields))
    }

    fn open_existing(
        &self,
        scope: &CatalogScope,
    ) -> Result<(Index, CatalogIndexFields), CatalogIndexError> {
        let catalog_schema = build_catalog_schema();
        let index_path = self.paths.scope_path(scope);
        let meta_path = index_path.join("meta.json");
        if !meta_path.exists() {
            return Err(CatalogIndexError::Unavailable(format!(
                "catalog index is missing for tenant/workspace scope at {}",
                index_path.display()
            )));
        }
        let index = self.cached_or_open(scope, &index_path)?;
        Ok((index, catalog_schema.fields))
    }

    fn cached_or_open(
        &self,
        scope: &CatalogScope,
        index_path: &Path,
    ) -> Result<Index, CatalogIndexError> {
        if let Ok(mut handles) = self.handles.lock() {
            if let Some(index) = handles.get(scope) {
                return Ok(index.clone());
            }
        }
        let index = Index::open_in_dir(index_path)?;
        self.cache_handle(scope, index.clone())?;
        Ok(index)
    }

    fn cache_handle(&self, scope: &CatalogScope, index: Index) -> Result<(), CatalogIndexError> {
        let mut handles = self.handles.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index handle pool lock: {error}"))
        })?;
        handles.insert(scope.clone(), index, self.max_handles);
        Ok(())
    }

    fn drop_cached_handle(&self, scope: &CatalogScope) -> Result<(), CatalogIndexError> {
        let mut handles = self.handles.lock().map_err(|error| {
            CatalogIndexError::Unavailable(format!("catalog index handle pool lock: {error}"))
        })?;
        handles.remove(scope);
        Ok(())
    }

    fn read_stale_reason(&self, scope: &CatalogScope) -> Result<Option<String>, CatalogIndexError> {
        let marker = self.paths.stale_marker_path(scope);
        match std::fs::read_to_string(marker) {
            Ok(reason) => Ok(Some(reason)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn search_sync(
        &self,
        scope: &CatalogScope,
        request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogIndexError> {
        if request.query.trim().is_empty() {
            return Err(CatalogIndexError::InvalidQuery(
                "catalog search query must not be empty".to_string(),
            ));
        }
        let limit = request.limit.max(1);
        let cursor_signature = request_signature(scope, &request);
        let offset = decode_offset(request.cursor.as_ref(), &cursor_signature)?;
        let (index, fields) = self.open_existing(scope)?;
        let current_schema = build_catalog_schema().schema;
        if index.schema() != current_schema {
            return Err(CatalogIndexError::Unavailable(
                "catalog index schema is stale; rebuild the catalog search index before retrying search_catalog".to_string(),
            ));
        }
        if let Some(reason) = self.read_stale_reason(scope)? {
            return Err(CatalogIndexError::Unavailable(format!(
                "catalog index is stale: {reason}"
            )));
        }
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        reader.reload()?;
        let searcher = reader.searcher();
        let indexed_entries = searcher.search(&AllQuery, &Count)?;
        let projection_version = index_projection_version(&searcher, fields, indexed_entries)?;
        if projection_version != PROJECTED_CATALOG_SCHEMA_VERSION {
            return Err(CatalogIndexError::Unavailable(format!(
                "catalog index projection_version {projection_version} is stale; expected {PROJECTED_CATALOG_SCHEMA_VERSION}"
            )));
        }
        let query = build_query(&index, fields, scope, &request)?;
        let text_analyzer = index.tokenizers().get("default").ok_or_else(|| {
            CatalogIndexError::Unavailable(
                "catalog index default tokenizer is not registered".to_string(),
            )
        })?;

        let candidate_count = searcher.search(&*query, &Count)?;
        let top_docs = searcher.search(
            &*query,
            &TopDocs::with_limit(limit.saturating_add(1))
                .and_offset(offset)
                .tweak_score(entry_id_tie_break_tweaker(fields.entry_id)),
        )?;
        let has_more = top_docs.len() > limit;
        let page_docs = top_docs.into_iter().take(limit).collect::<Vec<_>>();
        let max_raw = page_docs
            .iter()
            .map(|(key, _)| key.0 as f64)
            .fold(0.0_f64, f64::max);

        let mut hits = Vec::with_capacity(page_docs.len());
        let mut warnings = Vec::new();
        for (page_index, ((raw_score, _entry_id_key), doc_address)) in
            page_docs.into_iter().enumerate()
        {
            let document = searcher.doc::<TantivyDocument>(doc_address)?;
            let Some(entry_id) = first_string(&document, fields.entry_id)
                .filter(|entry_id| !entry_id.trim().is_empty())
            else {
                warnings.push(format!(
                    "catalog index document at rank {} is missing entry_id and was skipped",
                    offset + page_index + 1
                ));
                continue;
            };
            let rank = offset + page_index + 1;
            let (matched_fields, match_signals) =
                matched_fields_and_signals(&document, fields, &request.query, &text_analyzer);
            let snippet = best_snippet(&searcher, &*query, &document, fields);
            hits.push(CatalogSearchHitRef {
                entry_id,
                score: normalize_score(raw_score as f64, max_raw),
                rank,
                match_signals,
                matched_fields,
                raw_score: Some(raw_score as f64),
                snippet,
                // `rank` is the 1-based absolute position of this hit in the
                // candidate ordering, so it is exactly the 0-based offset of the
                // candidate immediately after it. Resuming the same logical
                // request from this offset continues strictly past this hit.
                resume_cursor: Some(encode_offset(rank, &cursor_signature)),
            });
        }

        Ok(CatalogSearchResults {
            facets: collect_facets(&searcher, &*query, fields, candidate_count)?,
            has_more,
            next_cursor: has_more.then(|| encode_offset(offset + limit, &cursor_signature)),
            candidate_count,
            hits,
            warnings: {
                warnings.extend(facet_warnings(candidate_count));
                warnings
            },
        })
    }

    fn health_sync(&self, scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogIndexError> {
        let (index, fields) = match self.open_existing(scope) {
            Ok(opened) => opened,
            Err(CatalogIndexError::Unavailable(reason)) => {
                return Ok(CatalogSearchHealth::Unavailable { reason });
            }
            Err(error) => return Err(error),
        };
        let reader = index.reader()?;
        let searcher = reader.searcher();
        let indexed_entries = searcher.search(&AllQuery, &Count)?;
        let current_schema = build_catalog_schema().schema;
        if index.schema() != current_schema {
            return Ok(CatalogSearchHealth::Stale {
                indexed_entries,
                projection_version: 0,
                reason: "catalog index schema is stale; rebuild the catalog search index"
                    .to_string(),
            });
        }
        let projection_version = index_projection_version(&searcher, fields, indexed_entries)?;
        if let Some(reason) = self.read_stale_reason(scope)? {
            return Ok(CatalogSearchHealth::Stale {
                indexed_entries,
                projection_version,
                reason,
            });
        }
        if projection_version != PROJECTED_CATALOG_SCHEMA_VERSION {
            return Ok(CatalogSearchHealth::Stale {
                indexed_entries,
                projection_version,
                reason: format!(
                    "catalog index projection_version {projection_version} is stale; expected {PROJECTED_CATALOG_SCHEMA_VERSION}"
                ),
            });
        }
        Ok(CatalogSearchHealth::Ready {
            indexed_entries,
            projection_version,
        })
    }
}

#[derive(Debug, Default)]
struct IndexHandlePool {
    entries: HashMap<CatalogScope, CachedIndex>,
    clock: u64,
}

#[derive(Debug)]
struct CachedIndex {
    index: Index,
    last_used: u64,
}

impl IndexHandlePool {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&mut self, scope: &CatalogScope) -> Option<Index> {
        let entry = self.entries.get_mut(scope)?;
        self.clock = self.clock.saturating_add(1);
        entry.last_used = self.clock;
        Some(entry.index.clone())
    }

    fn insert(&mut self, scope: CatalogScope, index: Index, max_handles: usize) {
        self.clock = self.clock.saturating_add(1);
        if !self.entries.contains_key(&scope) && self.entries.len() >= max_handles {
            if let Some(evict_scope) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(scope, _)| scope.clone())
            {
                self.entries.remove(&evict_scope);
            }
        }
        self.entries.insert(
            scope,
            CachedIndex {
                index,
                last_used: self.clock,
            },
        );
    }

    fn remove(&mut self, scope: &CatalogScope) {
        self.entries.remove(scope);
    }
}

#[derive(Debug, Default)]
struct WriteLockPool {
    entries: HashMap<CatalogScope, CachedWriteLock>,
    clock: u64,
}

#[derive(Debug)]
struct CachedWriteLock {
    lock: Arc<Mutex<()>>,
    last_used: u64,
}

impl WriteLockPool {
    fn get_or_insert(&mut self, scope: &CatalogScope, max_locks: usize) -> Arc<Mutex<()>> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(scope) {
            entry.last_used = self.clock;
            return entry.lock.clone();
        }
        self.evict_idle_until_below(max_locks);
        let lock = Arc::new(Mutex::new(()));
        self.entries.insert(
            scope.clone(),
            CachedWriteLock {
                lock: lock.clone(),
                last_used: self.clock,
            },
        );
        lock
    }

    fn evict_idle_until_below(&mut self, max_locks: usize) {
        while self.entries.len() >= max_locks {
            let Some(evict_scope) = self
                .entries
                .iter()
                .filter(|(_, entry)| Arc::strong_count(&entry.lock) == 1)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(scope, _)| scope.clone())
            else {
                break;
            };
            self.entries.remove(&evict_scope);
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use agent_fw_core::{TenantId, WorkspaceId};

    fn scope(name: &str) -> CatalogScope {
        CatalogScope::new(
            TenantId::new_unchecked(format!("tenant-{name}")),
            WorkspaceId::new_unchecked(format!("workspace-{name}")),
        )
    }

    fn in_memory_index() -> Index {
        Index::create_in_ram(build_catalog_schema().schema)
    }

    #[test]
    fn handle_pool_evicts_least_recently_used_scope() {
        let mut pool = IndexHandlePool::default();
        let scope_a = scope("a");
        let scope_b = scope("b");
        let scope_c = scope("c");

        pool.insert(scope_a.clone(), in_memory_index(), 2);
        pool.insert(scope_b.clone(), in_memory_index(), 2);
        assert!(pool.get(&scope_a).is_some(), "scope_a should be cached");
        assert!(pool.entries.contains_key(&scope_b));
        pool.insert(scope_c.clone(), in_memory_index(), 2);

        assert!(pool.entries.contains_key(&scope_a));
        assert!(!pool.entries.contains_key(&scope_b));
        assert!(pool.entries.contains_key(&scope_c));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn write_lock_pool_does_not_evict_live_lock_guards() {
        let mut pool = WriteLockPool::default();
        let scope_a = scope("a");
        let scope_b = scope("b");
        let scope_c = scope("c");

        let active = pool.get_or_insert(&scope_a, 2);
        let _guard = active.lock().unwrap();
        pool.get_or_insert(&scope_b, 2);
        pool.get_or_insert(&scope_c, 2);

        assert!(pool.entries.contains_key(&scope_a));
        assert!(
            !pool.entries.contains_key(&scope_b),
            "idle scope_b should be evicted before the live scope_a lock"
        );
        assert!(pool.entries.contains_key(&scope_c));
    }

    #[test]
    fn write_lock_pool_serializes_same_scope_guards() {
        let mut pool = WriteLockPool::default();
        let scope_a = scope("a");
        let scope_b = scope("b");

        let first = pool.get_or_insert(&scope_a, 4);
        let same_scope = pool.get_or_insert(&scope_a, 4);
        let other_scope = pool.get_or_insert(&scope_b, 4);

        assert!(Arc::ptr_eq(&first, &same_scope));
        assert!(!Arc::ptr_eq(&first, &other_scope));

        let first_guard = first.lock().unwrap();
        let _other_guard = other_scope
            .try_lock()
            .expect("different scopes should be independently lockable");

        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _same_scope_guard = same_scope.lock().unwrap();
            tx.send(()).unwrap();
        });

        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "same-scope lock acquisition must wait while a guard is live"
        );
        drop(first_guard);
        rx.recv_timeout(std::time::Duration::from_secs(1))
            .expect("same-scope waiter should acquire lock after guard is released");
        waiter.join().unwrap();
    }
}

#[async_trait]
impl CatalogSearchBackend for TantivyCatalogIndex {
    async fn search(
        &self,
        scope: &CatalogScope,
        request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogError> {
        // `search_sync` does blocking mmap reads and reader reloads. Offload it
        // to the blocking pool so it never stalls the async reactor on a small
        // or current-thread runtime. `TantivyCatalogIndex` is `Clone` over
        // shared `Arc` internals, so the clone is cheap and gives the closure a
        // `Send + 'static` handle without borrowing `&self`.
        let backend = self.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || backend.search_sync(&scope, request))
            .await
            .map_err(|join| {
                CatalogError::Unavailable(format!("catalog search task failed: {join}"))
            })?
            .map_err(CatalogError::from)
    }

    async fn health(&self, scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogError> {
        let backend = self.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || backend.health_sync(&scope))
            .await
            .map_err(|join| {
                CatalogError::Unavailable(format!("catalog health task failed: {join}"))
            })?
            .map_err(CatalogError::from)
    }
}

fn projection_document(
    fields: &CatalogIndexFields,
    scope: &CatalogScope,
    projection: &CatalogDocumentProjection,
) -> TantivyDocument {
    let mut document = TantivyDocument::default();
    add_text(&mut document, fields.scope_tenant, scope.tenant_id.as_str());
    add_text(
        &mut document,
        fields.scope_workspace,
        scope.workspace_id.as_str(),
    );
    add_text(&mut document, fields.entry_id, &projection.entry_id);
    add_text(&mut document, fields.kind, &projection.kind_name);
    add_exact(&mut document, fields.name_exact, &projection.name);
    add_text(&mut document, fields.name_text, &projection.name);
    if let Some(qualified_name) = &projection.qualified_name {
        add_exact(&mut document, fields.qualified_name_exact, qualified_name);
        add_text(&mut document, fields.qualified_name_text, qualified_name);
    }
    add_text(
        &mut document,
        fields.description_text,
        &projection.description,
    );
    for tag in &projection.tags {
        add_exact(&mut document, fields.tags, tag);
        add_text(&mut document, fields.tags_text, tag);
    }
    add_optional_exact(
        &mut document,
        fields.database_id,
        projection.database_id.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.schema_name,
        projection.schema_name.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.table_name,
        projection.table_name.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.column_name,
        projection.column_name.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.data_type,
        projection.data_type.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.semantic_type,
        projection.semantic_type.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.knowledge_type,
        projection.knowledge_type.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.relation_kind,
        projection.relation_kind.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.updated_at,
        projection.updated_at.as_deref(),
    );
    add_optional_text(
        &mut document,
        fields.relation_type,
        projection.relation_type.as_deref(),
    );
    add_optional_bool(&mut document, fields.nullable, projection.nullable);
    add_optional_bool(&mut document, fields.primary_key, projection.primary_key);
    add_optional_text(
        &mut document,
        fields.foreign_key_text,
        projection.foreign_key_text.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.cardinality,
        projection.source_cardinality.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.cardinality,
        projection.target_cardinality.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.confidence,
        projection.confidence.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.source_table,
        projection.source_table.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.source_column,
        projection.source_column.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.target_table,
        projection.target_table.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.target_column,
        projection.target_column.as_deref(),
    );
    for table in &projection.table_filter_values {
        add_exact(&mut document, fields.table_name, table);
    }
    for column in &projection.column_filter_values {
        add_exact(&mut document, fields.column_name, column);
    }
    for table in &projection.source_table_filter_values {
        add_exact(&mut document, fields.source_table, table);
    }
    for column in &projection.source_column_filter_values {
        add_exact(&mut document, fields.source_column, column);
    }
    for table in &projection.target_table_filter_values {
        add_exact(&mut document, fields.target_table, table);
    }
    for column in &projection.target_column_filter_values {
        add_exact(&mut document, fields.target_column, column);
    }
    add_optional_bool(
        &mut document,
        fields.preferred_query_surface,
        projection.preferred_query_surface,
    );
    add_optional_bool(
        &mut document,
        fields.low_cardinality_enum,
        projection.low_cardinality_enum,
    );
    let synonyms_text = projection.synonyms_text();
    add_text(&mut document, fields.synonyms_text, &synonyms_text);
    let context_text = projection.context_text();
    add_text(&mut document, fields.context_text, &context_text);
    add_optional_text(
        &mut document,
        fields.formula_text,
        projection.formula_text.as_deref(),
    );
    add_optional_text(
        &mut document,
        fields.sql_expression_text,
        projection.sql_expression_text.as_deref(),
    );
    let validation_rules_text = projection.validation_rules_text();
    add_text(
        &mut document,
        fields.validation_rules_text,
        &validation_rules_text,
    );
    add_optional_text(
        &mut document,
        fields.document_body_text,
        projection.document_body.as_deref(),
    );
    add_optional_exact(
        &mut document,
        fields.enum_value_exact,
        projection.enum_value.as_deref(),
    );
    add_optional_text(
        &mut document,
        fields.enum_value_text,
        projection.enum_value.as_deref(),
    );
    add_optional_text(
        &mut document,
        fields.enum_value_text,
        projection.enum_normalized_value.as_deref(),
    );
    add_optional_text(
        &mut document,
        fields.enum_display_value_text,
        projection.enum_display_value.as_deref(),
    );
    add_text(
        &mut document,
        fields.projection_version,
        &projection.projection_version.to_string(),
    );
    document
}

fn build_query(
    index: &Index,
    fields: CatalogIndexFields,
    scope: &CatalogScope,
    request: &CatalogSearchRequest,
) -> Result<Box<dyn Query>, CatalogIndexError> {
    let mut parser = QueryParser::for_index(index, fields.text_search_fields());
    parser.set_field_boost(fields.name_text, 4.0);
    parser.set_field_boost(fields.qualified_name_text, 4.0);
    parser.set_field_boost(fields.synonyms_text, 3.5);
    parser.set_field_boost(fields.enum_value_text, 3.5);
    parser.set_field_boost(fields.enum_display_value_text, 3.5);
    parser.set_field_boost(fields.description_text, 1.5);
    parser.set_field_boost(fields.relation_type, 1.2);
    parser.set_field_boost(fields.foreign_key_text, 1.2);
    parser.set_field_boost(fields.formula_text, 1.2);
    parser.set_field_boost(fields.sql_expression_text, 1.2);
    parser.set_field_boost(fields.validation_rules_text, 1.2);
    parser.set_field_boost(fields.document_body_text, 0.7);
    parser.set_field_boost(fields.context_text, 0.5);

    let (text_query, _errors) = parser.parse_query_lenient(&request.query);
    let query_norm = normalize_exact(&request.query);
    let exact_shortcuts: Vec<Box<dyn Query>> = vec![
        exact_term(fields.entry_id, request.query.trim()),
        exact_term(fields.name_exact, &query_norm),
        exact_term(fields.qualified_name_exact, &query_norm),
        exact_term(fields.enum_value_exact, &query_norm),
    ];
    let exact_query: Box<dyn Query> = Box::new(BoostQuery::new(
        Box::new(BooleanQuery::union(exact_shortcuts)),
        20.0,
    ));
    let lexical_query: Box<dyn Query> =
        Box::new(BooleanQuery::union(vec![text_query, exact_query]));
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
        (
            Occur::Must,
            exact_term(fields.scope_tenant, scope.tenant_id.as_str()),
        ),
        (
            Occur::Must,
            exact_term(fields.scope_workspace, scope.workspace_id.as_str()),
        ),
        (Occur::Must, lexical_query),
    ];

    if !request.kinds.is_empty() {
        let kind_queries = request
            .kinds
            .iter()
            .map(|kind| exact_term(fields.kind, kind.public_name()))
            .collect();
        clauses.push((Occur::Must, Box::new(BooleanQuery::union(kind_queries))));
    }

    push_filter_clauses(&mut clauses, fields, &request.filters);

    Ok(Box::new(BooleanQuery::new(clauses)))
}

fn push_filter_clauses(
    clauses: &mut Vec<(Occur, Box<dyn Query>)>,
    fields: CatalogIndexFields,
    filters: &CatalogSearchFilters,
) {
    push_optional_filter(clauses, fields.database_id, filters.database_id.as_deref());
    push_optional_filter(clauses, fields.schema_name, filters.schema.as_deref());
    push_optional_filter(clauses, fields.table_name, filters.table.as_deref());
    push_optional_filter(clauses, fields.column_name, filters.column.as_deref());
    push_optional_filter(clauses, fields.data_type, filters.data_type.as_deref());
    push_optional_filter(
        clauses,
        fields.semantic_type,
        filters.semantic_type.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.knowledge_type,
        filters.knowledge_type.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.relation_kind,
        filters.relation_kind.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.source_table,
        filters.source_table.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.source_column,
        filters.source_column.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.target_table,
        filters.target_table.as_deref(),
    );
    push_optional_filter(
        clauses,
        fields.target_column,
        filters.target_column.as_deref(),
    );
    for tag in &filters.tags {
        clauses.push((Occur::Must, exact_term(fields.tags, &normalize_exact(tag))));
    }
    if let Some(preferred) = filters.preferred_query_surface {
        clauses.push((
            Occur::Must,
            exact_term(fields.preferred_query_surface, bool_exact(preferred)),
        ));
    }
    if let Some(low_cardinality) = filters.low_cardinality_enum {
        clauses.push((
            Occur::Must,
            exact_term(fields.low_cardinality_enum, bool_exact(low_cardinality)),
        ));
    }
}

fn push_optional_filter(
    clauses: &mut Vec<(Occur, Box<dyn Query>)>,
    field: tantivy::schema::Field,
    value: Option<&str>,
) {
    if let Some(value) = value {
        clauses.push((Occur::Must, exact_term(field, &normalize_exact(value))));
    }
}

fn exact_term(field: tantivy::schema::Field, value: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

fn collect_facets(
    searcher: &tantivy::Searcher,
    query: &dyn Query,
    fields: CatalogIndexFields,
    candidate_count: usize,
) -> Result<CatalogSearchFacets, CatalogIndexError> {
    if candidate_count == 0 {
        return Ok(CatalogSearchFacets::default());
    }
    let facet_docs = searcher.search(
        query,
        &TopDocs::with_limit(candidate_count.min(MAX_FACET_DOCS))
            .tweak_score(entry_id_tie_break_tweaker(fields.entry_id)),
    )?;
    let mut facets = FacetAccumulator::default();
    for (_key, doc_address) in facet_docs {
        let document = searcher.doc::<TantivyDocument>(doc_address)?;
        facets.add_kind(first_string(&document, fields.kind));
        facets.add_schema(first_string(&document, fields.schema_name));
        facets.add_table(first_string(&document, fields.table_name));
        for tag in all_strings(&document, fields.tags) {
            facets.add_tag(Some(tag));
        }
    }
    Ok(facets.finish())
}

fn index_projection_version(
    searcher: &tantivy::Searcher,
    fields: CatalogIndexFields,
    indexed_entries: usize,
) -> Result<u32, CatalogIndexError> {
    if indexed_entries == 0 {
        return Ok(PROJECTED_CATALOG_SCHEMA_VERSION);
    }
    let docs = searcher.search(&AllQuery, &DocSetCollector)?;
    let mut version: Option<u32> = None;
    for doc_address in docs {
        let document = searcher.doc::<TantivyDocument>(doc_address)?;
        let Some(raw_version) = first_string(&document, fields.projection_version) else {
            return Ok(0);
        };
        let parsed = raw_version.parse::<u32>().unwrap_or(0);
        match version {
            Some(existing) if existing != parsed => return Ok(existing.min(parsed)),
            Some(_) => {}
            None => version = Some(parsed),
        }
    }
    Ok(version.unwrap_or(0))
}

type EntryIdSortKey = (tantivy::Score, std::cmp::Reverse<String>);

/// Build a `tweak_score` closure that produces a composite sort key
/// `(score, Reverse(entry_id))` so equal-score hits break ties on ascending
/// `entry_id`.
///
/// `TopDocs` keeps the LARGEST keys: a higher score wins the primary
/// comparison (descending score, unchanged from `order_by_score`), and on a
/// score tie `Reverse(entry_id)` makes the lexicographically-smaller `entry_id`
/// compare larger so it surfaces first (ascending `entry_id`). The `entry_id`
/// string is the unique catalog primary key, so this ordering is stable across
/// rebuilds, segment merges, and SQLite- vs Postgres-built indexes — unlike the
/// `DocAddress` tie-break `order_by_score` falls back to, which reflects only
/// physical insertion/segment layout.
///
/// Term ordinals are per-segment, so the string is resolved via `ord_to_str`
/// rather than compared as raw ordinals across segments. A missing column or
/// missing value (which cannot happen in practice because `entry_id` is always
/// populated) degrades to the empty string, which still sorts deterministically.
fn entry_id_tie_break_tweaker(
    entry_id_field: tantivy::schema::Field,
) -> impl Fn(&tantivy::SegmentReader) -> Box<dyn Fn(tantivy::DocId, tantivy::Score) -> EntryIdSortKey>
{
    move |segment_reader: &tantivy::SegmentReader| {
        let field_name = segment_reader.schema().get_field_name(entry_id_field);
        let entry_id_column = segment_reader.fast_fields().str(field_name).ok().flatten();
        Box::new(
            move |doc_id: tantivy::DocId, original_score: tantivy::Score| {
                let mut entry_id = String::new();
                if let Some(column) = &entry_id_column {
                    if let Some(ord) = column.term_ords(doc_id).next() {
                        let _ = column.ord_to_str(ord, &mut entry_id);
                    }
                }
                (original_score, std::cmp::Reverse(entry_id))
            },
        )
    }
}

fn facet_warnings(candidate_count: usize) -> Vec<String> {
    if candidate_count <= MAX_FACET_DOCS {
        return Vec::new();
    }
    vec![format!(
        "facet counts are approximate over the top {MAX_FACET_DOCS} of {candidate_count} matching catalog entries"
    )]
}

fn matched_fields_and_signals(
    document: &TantivyDocument,
    fields: CatalogIndexFields,
    query: &str,
    analyzer: &TextAnalyzer,
) -> (Vec<String>, Vec<String>) {
    let tokens = analyzed_tokens(analyzer, query);
    let query_norm = normalize_exact(query);
    let mut matched_fields = Vec::new();
    let mut signals = Vec::new();

    if first_string(document, fields.name_exact).as_deref() == Some(query_norm.as_str()) {
        push_unique(&mut matched_fields, "name");
        push_unique(&mut signals, "exact_name");
    }
    if first_string(document, fields.qualified_name_exact).as_deref() == Some(query_norm.as_str()) {
        push_unique(&mut matched_fields, "qualified_name");
        push_unique(&mut signals, "qualified_name");
    }
    if any_text_matches(document, fields.name_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "name");
        push_unique(&mut signals, "fts");
    }
    if any_text_matches(document, fields.qualified_name_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "qualified_name");
        push_unique(&mut signals, "qualified_name");
    }
    if any_text_matches(document, fields.description_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "description");
        push_unique(&mut signals, "fts");
    }
    if any_text_matches(document, fields.tags_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "tags");
        push_unique(&mut signals, "metadata");
    }
    if any_text_matches(document, fields.synonyms_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "synonyms");
        push_unique(&mut signals, "synonym");
    }
    if any_text_matches(document, fields.enum_value_text, &tokens, analyzer)
        || any_text_matches(document, fields.enum_display_value_text, &tokens, analyzer)
        || first_string(document, fields.enum_value_exact).as_deref() == Some(query_norm.as_str())
    {
        push_unique(&mut matched_fields, "enum_value");
        push_unique(&mut signals, "metadata");
    }
    if any_text_matches(document, fields.document_body_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "document_body");
        push_unique(&mut signals, "fts");
    }
    if any_text_matches(document, fields.context_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "context");
        push_unique(&mut signals, "relationship");
    }
    if any_text_matches(document, fields.relation_type, &tokens, analyzer) {
        push_unique(&mut matched_fields, "relation_type");
        push_unique(&mut signals, "metadata");
    }
    if any_text_matches(document, fields.foreign_key_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "foreign_key");
        push_unique(&mut signals, "relationship");
    }
    if any_text_matches(document, fields.formula_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "formula");
        push_unique(&mut signals, "metadata");
    }
    if any_text_matches(document, fields.sql_expression_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "sql_expression");
        push_unique(&mut signals, "metadata");
    }
    if any_text_matches(document, fields.validation_rules_text, &tokens, analyzer) {
        push_unique(&mut matched_fields, "validation_rules");
        push_unique(&mut signals, "metadata");
    }

    if matched_fields.is_empty() {
        push_unique(&mut signals, "fts");
    }

    (matched_fields, signals)
}

fn best_snippet(
    searcher: &tantivy::Searcher,
    query: &dyn Query,
    document: &TantivyDocument,
    fields: CatalogIndexFields,
) -> Option<String> {
    for field in [
        fields.document_body_text,
        fields.description_text,
        fields.context_text,
        fields.synonyms_text,
    ] {
        if first_string(document, field).is_none() {
            continue;
        }
        if let Ok(generator) = SnippetGenerator::create(searcher, query, field) {
            let snippet = generator.snippet_from_doc(document).to_html();
            if !snippet.trim().is_empty() {
                return Some(snippet);
            }
        }
    }
    None
}

fn any_text_matches(
    document: &TantivyDocument,
    field: tantivy::schema::Field,
    tokens: &HashSet<String>,
    analyzer: &TextAnalyzer,
) -> bool {
    if tokens.is_empty() {
        return false;
    }
    all_strings(document, field).into_iter().any(|value| {
        let value_tokens = analyzed_tokens(analyzer, &value);
        !value_tokens.is_disjoint(tokens)
    })
}

fn analyzed_tokens(analyzer: &TextAnalyzer, text: &str) -> HashSet<String> {
    let mut analyzer = analyzer.clone();
    let mut stream = analyzer.token_stream(text);
    let mut tokens = HashSet::new();
    stream.process(&mut |token| {
        if !token.text.trim().is_empty() {
            tokens.insert(token.text.clone());
        }
    });
    tokens
}

fn normalize_score(raw_score: f64, max_raw_score: f64) -> f64 {
    if max_raw_score <= f64::EPSILON {
        return 0.0;
    }
    (raw_score / max_raw_score).clamp(0.0, 1.0)
}

fn add_text(document: &mut TantivyDocument, field: tantivy::schema::Field, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        document.add_text(field, trimmed);
    }
}

fn add_exact(document: &mut TantivyDocument, field: tantivy::schema::Field, value: &str) {
    let normalized = normalize_exact(value);
    if !normalized.is_empty() {
        document.add_text(field, normalized);
    }
}

fn add_optional_text(
    document: &mut TantivyDocument,
    field: tantivy::schema::Field,
    value: Option<&str>,
) {
    if let Some(value) = value {
        add_text(document, field, value);
    }
}

fn add_optional_exact(
    document: &mut TantivyDocument,
    field: tantivy::schema::Field,
    value: Option<&str>,
) {
    if let Some(value) = value {
        add_exact(document, field, value);
    }
}

fn add_optional_bool(
    document: &mut TantivyDocument,
    field: tantivy::schema::Field,
    value: Option<bool>,
) {
    if let Some(value) = value {
        add_text(document, field, bool_exact(value));
    }
}

fn bool_exact(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn normalize_exact(value: &str) -> String {
    value.trim().to_lowercase()
}

fn first_string(document: &TantivyDocument, field: tantivy::schema::Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str().map(str::to_string))
}

fn all_strings(document: &TantivyDocument, field: tantivy::schema::Field) -> Vec<String> {
    document
        .get_all(field)
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn ensure_schema_current(
    index: &Index,
    current_schema: &tantivy::schema::Schema,
) -> Result<(), CatalogIndexError> {
    if index.schema() == *current_schema {
        return Ok(());
    }
    Err(CatalogIndexError::Unavailable(
        "catalog index schema is stale; rebuild the catalog search index before applying direct writes".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::{TenantId, WorkspaceId};

    fn scope(index: usize) -> CatalogScope {
        CatalogScope::new(
            TenantId::new_unchecked(format!("tenant-{index}")),
            WorkspaceId::new_unchecked("workspace"),
        )
    }

    #[test]
    fn write_lock_pool_trims_idle_overflow_after_contention() {
        let mut pool = WriteLockPool::default();

        let held_locks: Vec<_> = (0..3)
            .map(|index| pool.get_or_insert(&scope(index), 2))
            .collect();
        assert_eq!(pool.entries.len(), 3);

        drop(held_locks);
        let _new_lock = pool.get_or_insert(&scope(99), 2);

        assert_eq!(pool.entries.len(), 2);
        assert!(pool.entries.contains_key(&scope(99)));
    }
}
