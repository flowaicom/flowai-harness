//! Knowledge document ingestion pipeline — walk directories, dedup by content
//! hash, persist via KV, optionally trigger extraction.
//!
//! # Three-Phase Pipeline
//!
//! 1. **Scan** — Recursive directory walk, compute SHA-256 per file. Pure IO, no KV.
//! 2. **Dedup** — Load existing document hashes from KV, skip known content hashes.
//! 3. **Store** — Persist new documents via KV with `DocumentItem` type.
//! 4. **Extract** (optional) — Trigger `KnowledgeExtractionService` for each new document.
//!
//! Cancellation token checked between documents. Event sender reports progress.
//!
//! # Design Principles
//!
//! - Content-addressed idempotency: same content → same hash → skip on re-ingest.
//! - Follows the gold-standard pattern from `KnowledgeExtractionService`.
//! - Structured concurrency, cancellation discipline, progress events.
//! - Single-load, accumulate, single-write for the hash index (no per-doc TOCTOU).
//!
//! # Extraction optionality
//!
//! The extraction service is `Option<Arc<KnowledgeExtractionService>>` — a concrete
//! interpreter composing over algebraic traits. Introducing a `DocumentExtractor`
//! trait for a binary present/absent choice would be premature abstraction.
//! Revisit when a second extraction strategy appears.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use agent_fw_algebra::cancellation::CancellationToken;
use agent_fw_algebra::kv_store::KVStore;
use agent_fw_catalog::knowledge::{DocumentItem, ExtractionStatus};

use crate::knowledge_extraction::KnowledgeExtractionService;

const MAX_SCAN_DEPTH: usize = 16;
const MAX_SCAN_FILES: usize = 1_024;
const MAX_SCAN_FILE_BYTES: u64 = 1_048_576;
const MAX_SCAN_TOTAL_BYTES: u64 = 16 * 1_048_576;

// =============================================================================
// Types
// =============================================================================

/// Source specification for document ingestion.
#[derive(Debug, Clone)]
pub enum KnowledgeSourceSpec {
    /// Scan a local directory for documents.
    LocalDirectory {
        path: PathBuf,
        /// File extensions to include (e.g., `["md", "txt"]`).
        /// Empty means include all files.
        extensions: Vec<String>,
    },
}

/// A file discovered during the scan phase.
#[derive(Debug, Clone)]
pub struct DiscoveredDocument {
    pub path: PathBuf,
    pub name: String,
    pub content_hash: String,
    pub size_bytes: u64,
    pub content: String,
}

/// Summary of an ingestion run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSummary {
    pub scanned: usize,
    pub new: usize,
    pub skipped_duplicate: usize,
    pub errors: Vec<String>,
}

/// Progress events emitted during ingestion.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum IngestEvent {
    /// Discovery phase found documents.
    #[serde(rename_all = "camelCase")]
    Discovered { total: usize },
    /// Currently ingesting a document.
    #[serde(rename_all = "camelCase")]
    Ingesting {
        current: usize,
        total: usize,
        name: String,
    },
    /// Ingestion pipeline completed.
    #[serde(rename_all = "camelCase")]
    Completed(IngestSummary),
}

// =============================================================================
// Error Type
// =============================================================================

/// Errors from knowledge ingestion operations.
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeIngestionError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("KV store error: {0}")]
    KVStore(String),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("Corrupt hash index: {0}")]
    CorruptIndex(String),

    #[error("Knowledge source limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("Operation cancelled")]
    Cancelled,
}

// =============================================================================
// KV Keys
// =============================================================================

/// KV key for the set of known document content hashes.
const HASH_INDEX_KEY: &str = "knowledge:content_hashes";

/// KV key prefix for document items.
const DOC_KEY_PREFIX: &str = "knowledge:doc:";

// =============================================================================
// Persistence Policy
// =============================================================================

/// KV persistence policy for knowledge ingestion artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeIngestionPersistencePolicy {
    hash_index_key: String,
    document_key_prefix: String,
    document_index_key: Option<String>,
    prepend_document_index_entries: bool,
}

impl Default for KnowledgeIngestionPersistencePolicy {
    fn default() -> Self {
        Self {
            hash_index_key: HASH_INDEX_KEY.to_string(),
            document_key_prefix: DOC_KEY_PREFIX.to_string(),
            document_index_key: None,
            prepend_document_index_entries: false,
        }
    }
}

impl KnowledgeIngestionPersistencePolicy {
    /// Override the hash-index key.
    pub fn with_hash_index_key(mut self, key: impl Into<String>) -> Self {
        self.hash_index_key = key.into();
        self
    }

    /// Override the document key prefix.
    pub fn with_document_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.document_key_prefix = prefix.into();
        self
    }

    /// Maintain a JSON `{ ids: [...] }` document index for ingested documents.
    pub fn with_document_index_key(mut self, key: impl Into<String>) -> Self {
        self.document_index_key = Some(key.into());
        self
    }

    /// Insert new document IDs at the front of the document index.
    pub fn prepend_document_index_entries(mut self) -> Self {
        self.prepend_document_index_entries = true;
        self
    }

    pub fn hash_index_key(&self) -> &str {
        &self.hash_index_key
    }

    pub fn document_index_key(&self) -> Option<&str> {
        self.document_index_key.as_deref()
    }

    pub fn document_key(&self, id: &str) -> String {
        format!("{}{}", self.document_key_prefix, id)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdIndex {
    ids: Vec<String>,
}

// =============================================================================
// Service
// =============================================================================

/// Orchestrates document discovery, dedup, and persistence.
///
/// Assumes single-writer discipline for the hash index (same as `IndexedEntity`).
pub struct KnowledgeIngestionService {
    kv: Arc<dyn KVStore>,
    extraction: Option<Arc<KnowledgeExtractionService>>,
    persistence: KnowledgeIngestionPersistencePolicy,
}

impl KnowledgeIngestionService {
    /// Create a new ingestion service with the default framework persistence policy.
    pub fn new(kv: Arc<dyn KVStore>, extraction: Option<Arc<KnowledgeExtractionService>>) -> Self {
        Self::new_with_persistence(
            kv,
            extraction,
            KnowledgeIngestionPersistencePolicy::default(),
        )
    }

    /// Create a new ingestion service with an explicit persistence policy.
    pub fn new_with_persistence(
        kv: Arc<dyn KVStore>,
        extraction: Option<Arc<KnowledgeExtractionService>>,
        persistence: KnowledgeIngestionPersistencePolicy,
    ) -> Self {
        Self {
            kv,
            extraction,
            persistence,
        }
    }

    /// Run the full ingestion pipeline.
    ///
    /// Single-load, accumulate, single-write pattern for the hash index:
    /// loads the index once before the document loop, accumulates new hashes
    /// in memory, and flushes once at the end. This eliminates the per-document
    /// TOCTOU that existed when each `store_document` reloaded the index.
    pub async fn ingest(
        &self,
        tenant_id: &str,
        source: &KnowledgeSourceSpec,
        cancel: &CancellationToken,
        progress: Option<&mpsc::Sender<IngestEvent>>,
    ) -> Result<IngestSummary, KnowledgeIngestionError> {
        // Phase 1: Scan
        let discovered = match source {
            KnowledgeSourceSpec::LocalDirectory { path, extensions } => {
                scan_local_directory(path, extensions)?
            }
        };

        if let Some(tx) = progress {
            let _ = tx
                .send(IngestEvent::Discovered {
                    total: discovered.len(),
                })
                .await;
        }

        if cancel.is_cancelled() {
            return Err(KnowledgeIngestionError::Cancelled);
        }

        // Phase 2: Dedup — single load of known hashes
        let mut known_hashes = self.load_hash_index(tenant_id).await?;
        let scanned = discovered.len();
        let mut new_docs = Vec::new();
        let mut skipped_count = 0;
        let mut hash_index_dirty = false;
        for doc in discovered {
            if !known_hashes.contains(&doc.content_hash) {
                new_docs.push(doc);
                continue;
            }

            if self
                .should_reprocess_known_document(tenant_id, &doc)
                .await?
            {
                known_hashes.remove(&doc.content_hash);
                hash_index_dirty = true;
                new_docs.push(doc);
            } else {
                skipped_count += 1;
            }
        }
        let mut new_count = 0;
        let mut errors = Vec::new();

        // Phase 3: Store new documents, accumulate hashes in memory
        let total = new_docs.len();
        for (i, doc) in new_docs.iter().enumerate() {
            if cancel.is_cancelled() {
                // Best-effort flush of accumulated hashes so partially-completed
                // work is not re-ingested on retry.
                let _ = self.save_hash_index(tenant_id, &known_hashes).await;
                return Err(KnowledgeIngestionError::Cancelled);
            }

            if let Some(tx) = progress {
                let _ = tx
                    .send(IngestEvent::Ingesting {
                        current: i + 1,
                        total,
                        name: doc.name.clone(),
                    })
                    .await;
            }

            match self.persist_document(tenant_id, doc).await {
                Ok(doc_item) => {
                    // Phase 4: Optional extraction
                    if let Some(ref extraction) = self.extraction {
                        if let Err(e) = extraction
                            .extract_from_document(tenant_id, &doc_item, cancel, None)
                            .await
                        {
                            warn!(
                                document = %doc.name,
                                error = %e,
                                "Extraction failed for ingested document"
                            );
                            if let Err(rollback_err) =
                                self.rollback_document(tenant_id, &doc_item.id).await
                            {
                                warn!(
                                    document = %doc.name,
                                    document_id = %doc_item.id,
                                    error = %rollback_err,
                                    "Failed to roll back document after extraction failure"
                                );
                                errors.push(format!(
                                    "Rollback failed for {} after extraction error: {}",
                                    doc.name, rollback_err
                                ));
                            }
                            errors.push(format!("Extraction failed for {}: {}", doc.name, e));
                            continue;
                        }
                    }

                    new_count += 1;
                    if known_hashes.insert(doc.content_hash.clone()) {
                        hash_index_dirty = true;
                    }
                }
                Err(e) => {
                    warn!(document = %doc.name, error = %e, "Failed to store document");
                    errors.push(format!("Store failed for {}: {}", doc.name, e));
                }
            }
        }

        // Single write: flush accumulated hashes
        if hash_index_dirty {
            self.save_hash_index(tenant_id, &known_hashes).await?;
        }

        let summary = IngestSummary {
            scanned,
            new: new_count,
            skipped_duplicate: skipped_count,
            errors,
        };

        if let Some(tx) = progress {
            let _ = tx.send(IngestEvent::Completed(summary.clone())).await;
        }

        info!(
            scanned = summary.scanned,
            new = summary.new,
            skipped = summary.skipped_duplicate,
            "Knowledge ingestion completed"
        );

        Ok(summary)
    }

    /// Load the hash index from KV. Returns `HashSet` for O(1) membership.
    ///
    /// Propagates corrupt index errors instead of silently defaulting.
    async fn load_hash_index(
        &self,
        tenant_id: &str,
    ) -> Result<HashSet<String>, KnowledgeIngestionError> {
        let val = self
            .kv
            .get_json(tenant_id, self.persistence.hash_index_key())
            .await
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;

        let mut hashes = match val {
            Some(v) => {
                let hashes: Vec<String> = serde_json::from_value(v)
                    .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))?;
                hashes.into_iter().collect::<HashSet<_>>()
            }
            None => HashSet::new(),
        };

        if let Some(index_key) = self.persistence.document_index_key() {
            let maybe_index = self
                .kv
                .get_json(tenant_id, index_key)
                .await
                .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;

            if let Some(index_value) = maybe_index {
                let index: IdIndex = serde_json::from_value(index_value)
                    .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))?;
                for doc_id in index.ids {
                    let doc_key = self.persistence.document_key(&doc_id);
                    if let Some(doc_value) = self
                        .kv
                        .get_json(tenant_id, &doc_key)
                        .await
                        .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?
                    {
                        let doc: DocumentItem = serde_json::from_value(doc_value)
                            .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))?;
                        hashes.insert(hex::encode(Sha256::digest(doc.content.as_bytes())));
                    }
                }
            }
        }

        Ok(hashes)
    }

    /// Save the hash index to KV. Serializes as a sorted `Vec` for deterministic JSON.
    async fn save_hash_index(
        &self,
        tenant_id: &str,
        hashes: &HashSet<String>,
    ) -> Result<(), KnowledgeIngestionError> {
        let mut sorted: Vec<&String> = hashes.iter().collect();
        sorted.sort();
        let idx_value = serde_json::to_value(&sorted)
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
        self.kv
            .put_json(
                tenant_id,
                self.persistence.hash_index_key(),
                idx_value,
                None,
            )
            .await
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
        Ok(())
    }

    /// Persist a single document to KV. Does NOT touch the hash index.
    ///
    /// Uses the full 64-char hex SHA-256 for the doc ID, eliminating
    /// birthday-paradox collision risk from the previous 12-char truncation.
    async fn persist_document(
        &self,
        tenant_id: &str,
        doc: &DiscoveredDocument,
    ) -> Result<DocumentItem, KnowledgeIngestionError> {
        let doc_id = format!("doc-{}", doc.content_hash);
        let item = DocumentItem {
            id: doc_id.clone(),
            name: doc.name.clone(),
            content: doc.content.clone(),
            target_database_id: None,
            extraction_status: ExtractionStatus::Pending,
            extracted_knowledge_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Store document.
        let key = self.persistence.document_key(&doc_id);
        let value = serde_json::to_value(&item)
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
        self.kv
            .put_json(tenant_id, &key, value, None)
            .await
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;

        if let Some(index_key) = self.persistence.document_index_key() {
            let mut index: IdIndex = self
                .kv
                .get_json(tenant_id, index_key)
                .await
                .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?
                .map(|value| {
                    serde_json::from_value(value)
                        .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))
                })
                .transpose()?
                .unwrap_or_default();

            if self.persistence.prepend_document_index_entries {
                if !index.ids.iter().any(|existing| existing == &doc_id) {
                    index.ids.insert(0, doc_id.clone());
                }
            } else if !index.ids.iter().any(|existing| existing == &doc_id) {
                index.ids.push(doc_id.clone());
            }

            let index_value = serde_json::to_value(&index)
                .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
            self.kv
                .put_json(tenant_id, index_key, index_value, None)
                .await
                .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
        }

        debug!(document_id = %doc_id, name = %doc.name, "Document stored");
        Ok(item)
    }

    async fn should_reprocess_known_document(
        &self,
        tenant_id: &str,
        doc: &DiscoveredDocument,
    ) -> Result<bool, KnowledgeIngestionError> {
        let doc_id = format!("doc-{}", doc.content_hash);
        let key = self.persistence.document_key(&doc_id);
        let maybe_document = self
            .kv
            .get_json(tenant_id, &key)
            .await
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
        let Some(document_value) = maybe_document else {
            return Ok(true);
        };

        let document: DocumentItem = serde_json::from_value(document_value)
            .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))?;
        Ok(self.extraction.is_some() && document.extraction_status != ExtractionStatus::Processed)
    }

    async fn rollback_document(
        &self,
        tenant_id: &str,
        doc_id: &str,
    ) -> Result<(), KnowledgeIngestionError> {
        let key = self.persistence.document_key(doc_id);
        self.kv
            .delete(tenant_id, &key)
            .await
            .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;

        if let Some(index_key) = self.persistence.document_index_key() {
            let maybe_index = self
                .kv
                .get_json(tenant_id, index_key)
                .await
                .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
            if let Some(index_value) = maybe_index {
                let mut index: IdIndex = serde_json::from_value(index_value)
                    .map_err(|e| KnowledgeIngestionError::CorruptIndex(e.to_string()))?;
                let original_len = index.ids.len();
                index.ids.retain(|existing| existing != doc_id);
                if index.ids.len() != original_len {
                    let index_value = serde_json::to_value(&index)
                        .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
                    self.kv
                        .put_json(tenant_id, index_key, index_value, None)
                        .await
                        .map_err(|e| KnowledgeIngestionError::KVStore(e.to_string()))?;
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// Scan
// =============================================================================

/// Recursively scan a local directory, computing SHA-256 for each file.
///
/// Filters by extension if `extensions` is non-empty. Returns all discovered
/// documents with their content and hash.
pub fn scan_local_directory(
    path: &std::path::Path,
    extensions: &[String],
) -> Result<Vec<DiscoveredDocument>, KnowledgeIngestionError> {
    let root = path.canonicalize()?;
    let mut state = ScanState::default();
    scan_recursive(&root, &root, extensions, 0, &mut state)?;
    Ok(state.results)
}

#[derive(Default)]
struct ScanState {
    results: Vec<DiscoveredDocument>,
    matched_files: usize,
    total_bytes: u64,
}

fn scan_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    extensions: &[String],
    depth: usize,
    state: &mut ScanState,
) -> Result<(), KnowledgeIngestionError> {
    if depth > MAX_SCAN_DEPTH {
        return Err(KnowledgeIngestionError::LimitExceeded(format!(
            "directory depth exceeded {MAX_SCAN_DEPTH}: {}",
            dir.display()
        )));
    }
    let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(root) {
            return Err(KnowledgeIngestionError::LimitExceeded(format!(
                "path escapes knowledge root: {}",
                path.display()
            )));
        }

        if file_type.is_dir() {
            scan_recursive(root, &canonical, extensions, depth + 1, state)?;
        } else if file_type.is_file() {
            // Extension filter
            if !extensions.is_empty() {
                let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !extensions.iter().any(|e| e == ext) {
                    continue;
                }
            }
            if state.matched_files >= MAX_SCAN_FILES {
                return Err(KnowledgeIngestionError::LimitExceeded(format!(
                    "file count exceeded {MAX_SCAN_FILES}"
                )));
            }
            state.matched_files += 1;

            let metadata = std::fs::metadata(&canonical)?;
            if metadata.len() > MAX_SCAN_FILE_BYTES {
                return Err(KnowledgeIngestionError::LimitExceeded(format!(
                    "file exceeds {MAX_SCAN_FILE_BYTES} bytes: {}",
                    canonical.display()
                )));
            }
            let next_total = state.total_bytes.saturating_add(metadata.len());
            if next_total > MAX_SCAN_TOTAL_BYTES {
                return Err(KnowledgeIngestionError::LimitExceeded(format!(
                    "total file bytes exceeded {MAX_SCAN_TOTAL_BYTES}"
                )));
            }
            state.total_bytes = next_total;

            let content = match std::fs::read_to_string(&canonical) {
                Ok(c) => c,
                Err(e) => {
                    debug!(path = %canonical.display(), error = %e, "Skipping unreadable file");
                    continue;
                }
            };

            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            let hash = hex::encode(hasher.finalize());

            let name = canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            state.results.push(DiscoveredDocument {
                path: canonical,
                name,
                content_hash: hash,
                size_bytes: metadata.len(),
                content,
            });
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::KVStoreExt;
    use agent_fw_catalog::{
        EnrichmentError, EnrichmentResult, KnowledgeExtractionRequest, KnowledgeItem,
        SemanticEnricher, TableEnrichmentRequest,
    };
    use agent_fw_interpreter::{DashMapKVStore, MockEnricher, MockTargetDatabase};
    use tempfile::TempDir;

    struct FailingKnowledgeEnricher;

    #[async_trait::async_trait]
    impl SemanticEnricher for FailingKnowledgeEnricher {
        async fn enrich_table(
            &self,
            _request: TableEnrichmentRequest,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            unreachable!("knowledge ingestion test should not enrich tables")
        }

        async fn extract_knowledge(
            &self,
            _request: KnowledgeExtractionRequest,
        ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
            Err(EnrichmentError::ParseFailed(
                "missing field `knowledgeType`".to_string(),
            ))
        }
    }

    fn make_service() -> (KnowledgeIngestionService, Arc<dyn KVStore>) {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let svc = KnowledgeIngestionService::new(Arc::clone(&kv), None);
        (svc, kv)
    }

    fn create_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("doc1.md"), "# Document 1\nSome content.").unwrap();
        std::fs::write(dir.path().join("doc2.txt"), "Plain text document.").unwrap();
        std::fs::write(dir.path().join("ignored.rs"), "fn main() {}").unwrap();
        dir
    }

    #[test]
    fn scan_finds_files_with_extension_filter() {
        let dir = create_test_dir();
        let docs = scan_local_directory(dir.path(), &["md".into(), "txt".into()]).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs
            .iter()
            .all(|d| d.name.ends_with(".md") || d.name.ends_with(".txt")));
    }

    #[test]
    fn scan_finds_all_files_without_filter() {
        let dir = create_test_dir();
        let docs = scan_local_directory(dir.path(), &[]).unwrap();
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let docs = scan_local_directory(dir.path(), &[]).unwrap();
        assert!(docs.is_empty());
    }

    #[test]
    fn scan_computes_sha256() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.md"), "hello world").unwrap();
        let docs = scan_local_directory(dir.path(), &[]).unwrap();
        assert_eq!(docs.len(), 1);
        // SHA-256 of "hello world"
        assert_eq!(
            docs[0].content_hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn scan_rejects_too_many_files() {
        let dir = TempDir::new().unwrap();
        for index in 0..=MAX_SCAN_FILES {
            std::fs::write(dir.path().join(format!("doc-{index}.txt")), "x").unwrap();
        }

        let error = scan_local_directory(dir.path(), &[]).unwrap_err();

        assert!(matches!(error, KnowledgeIngestionError::LimitExceeded(_)));
    }

    #[test]
    fn scan_rejects_too_many_unreadable_matching_files() {
        let dir = TempDir::new().unwrap();
        for index in 0..=MAX_SCAN_FILES {
            std::fs::write(dir.path().join(format!("doc-{index}.txt")), [0xFF, 0xFE]).unwrap();
        }

        let error = scan_local_directory(dir.path(), &["txt".into()]).unwrap_err();

        assert!(matches!(error, KnowledgeIngestionError::LimitExceeded(_)));
    }

    #[test]
    fn scan_rejects_oversized_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("large.txt"),
            vec![b'x'; MAX_SCAN_FILE_BYTES as usize + 1],
        )
        .unwrap();

        let error = scan_local_directory(dir.path(), &[]).unwrap_err();

        assert!(matches!(error, KnowledgeIngestionError::LimitExceeded(_)));
    }

    #[cfg(unix)]
    #[test]
    fn scan_rejects_symlink_escape() {
        let root = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let target = outside.path().join("secret.txt");
        std::fs::write(&target, "secret").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("escape.txt")).unwrap();

        let error = scan_local_directory(root.path(), &[]).unwrap_err();

        assert!(matches!(error, KnowledgeIngestionError::LimitExceeded(_)));
    }

    #[tokio::test]
    async fn ingest_stores_new_documents() {
        let (svc, _kv) = make_service();
        let dir = create_test_dir();
        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into(), "txt".into()],
        };

        let summary = svc
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();
        assert_eq!(summary.scanned, 2);
        assert_eq!(summary.new, 2);
        assert_eq!(summary.skipped_duplicate, 0);
    }

    #[tokio::test]
    async fn ingest_dedup_on_second_run() {
        let (svc, _kv) = make_service();
        let dir = create_test_dir();
        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into(), "txt".into()],
        };

        // First ingest
        let summary1 = svc
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();
        assert_eq!(summary1.new, 2);

        // Second ingest — same content, should skip all
        let summary2 = svc
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();
        assert_eq!(summary2.new, 0);
        assert_eq!(summary2.skipped_duplicate, 2);
    }

    #[tokio::test]
    async fn failed_extraction_does_not_mark_document_duplicate_on_retry() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("business.md"), "Revenue is net sales.").unwrap();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into()],
        };
        let cancel = CancellationToken::new();

        let failing_extraction = Arc::new(KnowledgeExtractionService::new(
            Arc::new(MockTargetDatabase::new()),
            Arc::new(FailingKnowledgeEnricher),
            Arc::clone(&kv),
        ));
        let failing_service =
            KnowledgeIngestionService::new(Arc::clone(&kv), Some(failing_extraction));

        let failed_summary = failing_service
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();

        assert_eq!(failed_summary.scanned, 1);
        assert_eq!(failed_summary.new, 0);
        assert_eq!(failed_summary.skipped_duplicate, 0);
        assert_eq!(failed_summary.errors.len(), 1);
        assert!(kv
            .list_keys("tenant-1", DOC_KEY_PREFIX)
            .await
            .unwrap()
            .is_empty());
        assert!(!kv.exists("tenant-1", HASH_INDEX_KEY).await.unwrap());

        let successful_extraction = Arc::new(KnowledgeExtractionService::new(
            Arc::new(MockTargetDatabase::new()),
            Arc::new(MockEnricher::new()),
            Arc::clone(&kv),
        ));
        let successful_service =
            KnowledgeIngestionService::new(Arc::clone(&kv), Some(successful_extraction));

        let retry_summary = successful_service
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();

        assert_eq!(retry_summary.new, 1);
        assert_eq!(retry_summary.skipped_duplicate, 0);
        assert!(retry_summary.errors.is_empty());
        assert_eq!(
            kv.list_keys("tenant-1", DOC_KEY_PREFIX)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(kv.exists("tenant-1", HASH_INDEX_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn extract_ingest_repairs_existing_pending_duplicate() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("business.md"), "Revenue is net sales.").unwrap();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into()],
        };
        let discovered = scan_local_directory(dir.path(), &["md".into()]).unwrap();
        let doc = discovered.first().unwrap();
        let doc_id = format!("doc-{}", doc.content_hash);
        let pending_document = DocumentItem {
            id: doc_id.clone(),
            name: doc.name.clone(),
            content: doc.content.clone(),
            target_database_id: None,
            extraction_status: ExtractionStatus::Pending,
            extracted_knowledge_ids: vec![],
            created_at: "2026-05-26T00:00:00Z".to_string(),
        };
        kv.put_json(
            "tenant-1",
            HASH_INDEX_KEY,
            serde_json::json!([doc.content_hash.clone()]),
            None,
        )
        .await
        .unwrap();
        kv.put_json(
            "tenant-1",
            &format!("{DOC_KEY_PREFIX}{doc_id}"),
            serde_json::to_value(&pending_document).unwrap(),
            None,
        )
        .await
        .unwrap();

        let extraction_policy = crate::knowledge_extraction::KnowledgePersistencePolicy::default()
            .with_document_key_prefix(DOC_KEY_PREFIX)
            .with_embedded_document_knowledge_ids();
        let extraction = Arc::new(KnowledgeExtractionService::new_with_persistence(
            Arc::new(MockTargetDatabase::new()),
            Arc::new(MockEnricher::new()),
            Arc::clone(&kv),
            extraction_policy,
        ));
        let service = KnowledgeIngestionService::new(Arc::clone(&kv), Some(extraction));

        let summary = service
            .ingest("tenant-1", &source, &CancellationToken::new(), None)
            .await
            .unwrap();

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.new, 1);
        assert_eq!(summary.skipped_duplicate, 0);
        assert!(summary.errors.is_empty());
        let repaired: DocumentItem = kv
            .get("tenant-1", &format!("{DOC_KEY_PREFIX}{doc_id}"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repaired.extraction_status, ExtractionStatus::Processed);
        assert_eq!(repaired.extracted_knowledge_ids, vec!["mock-k-0"]);
    }

    #[tokio::test]
    async fn ingest_respects_cancellation() {
        let (svc, _kv) = make_service();
        let dir = create_test_dir();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec![],
        };

        let result = svc.ingest("tenant-1", &source, &cancel, None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KnowledgeIngestionError::Cancelled
        ));
    }

    #[tokio::test]
    async fn ingest_emits_progress_events() {
        let (svc, _kv) = make_service();
        let dir = create_test_dir();
        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into()],
        };

        let (tx, mut rx) = mpsc::channel(16);
        svc.ingest("tenant-1", &source, &cancel, Some(&tx))
            .await
            .unwrap();

        let mut events = vec![];
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Should have: Discovered, Ingesting, Completed
        assert!(events.len() >= 2);
        assert!(matches!(&events[0], IngestEvent::Discovered { .. }));
        assert!(matches!(events.last().unwrap(), IngestEvent::Completed(_)));
    }

    #[tokio::test]
    async fn ingest_rejects_corrupt_hash_index() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        // Poison the hash index with a non-array JSON value
        kv.put_json(
            "tenant-1",
            HASH_INDEX_KEY,
            serde_json::json!({"not": "an array"}),
            None,
        )
        .await
        .unwrap();

        let svc = KnowledgeIngestionService::new(Arc::clone(&kv), None);
        let dir = create_test_dir();
        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into()],
        };

        let result = svc.ingest("tenant-1", &source, &cancel, None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KnowledgeIngestionError::CorruptIndex(_)
        ));
    }

    #[tokio::test]
    async fn custom_persistence_policy_writes_configured_document_and_hash_keys() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let policy = KnowledgeIngestionPersistencePolicy::default()
            .with_hash_index_key("data:knowledge:content_hashes")
            .with_document_key_prefix("data:document:")
            .with_document_index_key("data:documents:index")
            .prepend_document_index_entries();
        let svc = KnowledgeIngestionService::new_with_persistence(Arc::clone(&kv), None, policy);

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.md"), "alpha").unwrap();
        std::fs::write(dir.path().join("b.md"), "beta").unwrap();

        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec!["md".into()],
        };

        let summary = svc
            .ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();
        assert_eq!(summary.new, 2);

        let alpha_hash = hex::encode(Sha256::digest(b"alpha"));
        let beta_hash = hex::encode(Sha256::digest(b"beta"));

        let stored_hashes = kv
            .get_json("tenant-1", "data:knowledge:content_hashes")
            .await
            .unwrap()
            .unwrap();
        let stored_hashes: Vec<String> = serde_json::from_value(stored_hashes).unwrap();
        assert_eq!(stored_hashes.len(), 2);
        assert!(stored_hashes.contains(&alpha_hash));
        assert!(stored_hashes.contains(&beta_hash));

        let alpha_key = format!("data:document:doc-{alpha_hash}");
        let beta_key = format!("data:document:doc-{beta_hash}");
        assert!(kv.get_json("tenant-1", &alpha_key).await.unwrap().is_some());
        assert!(kv.get_json("tenant-1", &beta_key).await.unwrap().is_some());

        let index = kv
            .get_json("tenant-1", "data:documents:index")
            .await
            .unwrap()
            .unwrap();
        let index: IdIndex = serde_json::from_value(index).unwrap();
        assert_eq!(
            index.ids,
            vec![format!("doc-{beta_hash}"), format!("doc-{alpha_hash}")]
        );
    }

    #[tokio::test]
    async fn doc_id_uses_full_content_hash() {
        let (svc, kv) = make_service();
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.md"), "hello world").unwrap();
        let cancel = CancellationToken::new();
        let source = KnowledgeSourceSpec::LocalDirectory {
            path: dir.path().to_path_buf(),
            extensions: vec![],
        };

        svc.ingest("tenant-1", &source, &cancel, None)
            .await
            .unwrap();

        // Full 64-char SHA-256 of "hello world"
        let full_hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let doc_key = format!("{}doc-{}", DOC_KEY_PREFIX, full_hash);
        let stored = kv.get_json("tenant-1", &doc_key).await.unwrap();
        assert!(
            stored.is_some(),
            "Document should be stored with full hash key"
        );
    }
}
