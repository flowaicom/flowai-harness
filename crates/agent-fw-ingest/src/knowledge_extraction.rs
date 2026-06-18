//! Knowledge extraction service — orchestrates LLM-based knowledge extraction
//! from documents using structured concurrency, retry, timeout, and cancellation.
//!
//! Composes over:
//! - [`SemanticEnricher::extract_knowledge`] — LLM extraction
//! - [`IntrospectionService`] — table/column context discovery
//! - [`KVStore`] — document and knowledge persistence
//! - [`CancellationToken`] — cooperative cancellation
//! - [`Nursery`] — parallel introspection with structured concurrency
//! - [`RetryPolicy`] + [`with_timeout`] — resilient LLM calls
//!
//! # Three-Phase Narrative
//!
//! 1. **Discover** — introspect database for table/column context (parallel via Nursery)
//! 2. **Extract** — call enricher.extract_knowledge() with retry + timeout + cancellation
//! 3. **Persist** — save extracted KnowledgeItems to KV, update document status
//!
//! # Example
//!
//! ```ignore
//! use agent_fw_ingest::knowledge_extraction::KnowledgeExtractionService;
//!
//! let svc = KnowledgeExtractionService::new(db, enricher, kv);
//! let items = svc.extract_from_document(
//!     "tenant-1",
//!     &document,
//!     &cancel,
//!     Some(&progress_tx),
//! ).await?;
//! ```

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use agent_fw_algebra::cancellation::CancellationToken;
use agent_fw_algebra::kv_store::{KVStore, KVStoreExt};
use agent_fw_algebra::nursery::with_nursery;
use agent_fw_algebra::pipeline::PipelineCtx;
use agent_fw_algebra::retry::{retry_when, RetryPolicy};
use agent_fw_algebra::target_db::TargetDatabase;
use agent_fw_algebra::timeout::with_timeout;
use agent_fw_catalog::enrichment::{EnrichmentError, SemanticEnricher};
use agent_fw_catalog::knowledge::{DocumentItem, ExtractionStatus, KnowledgeItem};
use agent_fw_catalog::KnowledgeExtractionRequest;

use crate::introspection::IntrospectionService;

// =============================================================================
// Constants
// =============================================================================

/// Timeout for a single LLM knowledge extraction call.
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(90);

/// Retry policy: exponential backoff (500ms base, 2x, max 10s, 2 retries).
fn extraction_retry_policy() -> RetryPolicy {
    RetryPolicy::exponential_backoff_jitter(2, Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(10))
}

// =============================================================================
// Persistence Policy
// =============================================================================

/// How extracted knowledge IDs are persisted for a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentKnowledgeIdStorage {
    /// Store document metadata and extracted IDs under separate keys.
    SplitKey { suffix: String },
    /// Store extracted IDs inside the persisted `DocumentItem` itself.
    EmbeddedInDocument,
}

/// KV persistence policy for knowledge extraction outputs.
///
/// The default preserves the framework's split-key contract:
/// - `knowledge:{id}` for knowledge items
/// - `doc:{id}` for documents
/// - `doc:{id}:knowledge_ids` for extracted knowledge-id lists
///
/// Consumers can override key prefixes, embed extracted IDs directly in the
/// document payload, and maintain a knowledge index when their application's
/// workspace store expects one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgePersistencePolicy {
    knowledge_key_prefix: String,
    knowledge_index_key: Option<String>,
    prepend_knowledge_index_entries: bool,
    document_key_prefix: String,
    document_knowledge_id_storage: DocumentKnowledgeIdStorage,
}

impl Default for KnowledgePersistencePolicy {
    fn default() -> Self {
        Self {
            knowledge_key_prefix: "knowledge:".to_string(),
            knowledge_index_key: None,
            prepend_knowledge_index_entries: false,
            document_key_prefix: "doc:".to_string(),
            document_knowledge_id_storage: DocumentKnowledgeIdStorage::SplitKey {
                suffix: ":knowledge_ids".to_string(),
            },
        }
    }
}

impl KnowledgePersistencePolicy {
    /// Override the knowledge-item key prefix.
    pub fn with_knowledge_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.knowledge_key_prefix = prefix.into();
        self
    }

    /// Maintain a JSON `{ ids: [...] }` index for extracted knowledge items.
    pub fn with_knowledge_index_key(mut self, key: impl Into<String>) -> Self {
        self.knowledge_index_key = Some(key.into());
        self
    }

    /// Insert new knowledge IDs at the front of the index while preserving the
    /// batch order provided by the extraction result.
    pub fn prepend_knowledge_index_entries(mut self) -> Self {
        self.prepend_knowledge_index_entries = true;
        self
    }

    /// Override the document key prefix.
    pub fn with_document_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.document_key_prefix = prefix.into();
        self
    }

    /// Persist extracted knowledge IDs inside the document payload.
    pub fn with_embedded_document_knowledge_ids(mut self) -> Self {
        self.document_knowledge_id_storage = DocumentKnowledgeIdStorage::EmbeddedInDocument;
        self
    }

    /// Persist extracted knowledge IDs under a separate document-scoped key.
    pub fn with_split_document_knowledge_ids(mut self, suffix: impl Into<String>) -> Self {
        self.document_knowledge_id_storage = DocumentKnowledgeIdStorage::SplitKey {
            suffix: suffix.into(),
        };
        self
    }

    /// Build the persisted knowledge-item key.
    pub fn knowledge_key(&self, id: &str) -> String {
        format!("{}{}", self.knowledge_key_prefix, id)
    }

    /// Build the persisted document key.
    pub fn document_key(&self, id: &str) -> String {
        format!("{}{}", self.document_key_prefix, id)
    }

    /// Build the extracted-knowledge-id key when split-key storage is enabled.
    pub fn document_knowledge_ids_key(&self, id: &str) -> Option<String> {
        match &self.document_knowledge_id_storage {
            DocumentKnowledgeIdStorage::SplitKey { suffix } => {
                Some(format!("{}{}{}", self.document_key_prefix, id, suffix))
            }
            DocumentKnowledgeIdStorage::EmbeddedInDocument => None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdIndex {
    ids: Vec<String>,
}

// =============================================================================
// Error Type
// =============================================================================

/// Errors from knowledge extraction operations.
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeExtractionError {
    #[error("Enrichment failed: {0}")]
    Enrichment(#[from] EnrichmentError),

    #[error("Database introspection failed: {0}")]
    Introspection(String),

    #[error("KV store error: {0}")]
    KVStore(String),

    #[error("Extraction timed out after {0}ms")]
    Timeout(u64),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Nursery error: {0}")]
    Nursery(String),
}

// =============================================================================
// Progress Events
// =============================================================================

/// Progress events emitted during knowledge extraction.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtractionEvent {
    /// Discovery phase started.
    #[serde(rename_all = "camelCase")]
    DiscoveryStarted { schemas_found: usize },
    /// Discovery phase completed with table/column context.
    #[serde(rename_all = "camelCase")]
    DiscoveryCompleted {
        tables_found: usize,
        columns_found: usize,
    },
    /// Extraction started for a document.
    #[serde(rename_all = "camelCase")]
    ExtractionStarted {
        document_id: String,
        document_name: String,
    },
    /// Extraction completed for a document.
    #[serde(rename_all = "camelCase")]
    ExtractionCompleted {
        document_id: String,
        items_extracted: usize,
    },
    /// Extraction failed for a document.
    #[serde(rename_all = "camelCase")]
    ExtractionFailed { document_id: String, error: String },
    /// All extractions complete.
    #[serde(rename_all = "camelCase")]
    AllCompleted {
        total_items: usize,
        total_documents: usize,
    },
    /// Error during the extraction pipeline.
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// Send an event, returning `false` if the receiver has disconnected.
async fn emit(tx: &mpsc::Sender<ExtractionEvent>, event: ExtractionEvent) -> bool {
    tx.send(event).await.is_ok()
}

// =============================================================================
// KnowledgeExtractionService
// =============================================================================

/// Orchestrates LLM-based knowledge extraction from documents.
///
/// Composes `SemanticEnricher`, `IntrospectionService`, and `KVStore` with
/// structured concurrency (`Nursery`), retry, timeout, and cancellation.
pub struct KnowledgeExtractionService {
    enricher: Arc<dyn SemanticEnricher>,
    kv: Arc<dyn KVStore>,
    introspection: Arc<IntrospectionService>,
    persistence: KnowledgePersistencePolicy,
}

impl KnowledgeExtractionService {
    /// Create a new extraction service with the framework default persistence policy.
    pub fn new(
        db: Arc<dyn TargetDatabase>,
        enricher: Arc<dyn SemanticEnricher>,
        kv: Arc<dyn KVStore>,
    ) -> Self {
        Self::new_with_persistence(db, enricher, kv, KnowledgePersistencePolicy::default())
    }

    /// Create a new extraction service with an explicit KV persistence policy.
    pub fn new_with_persistence(
        db: Arc<dyn TargetDatabase>,
        enricher: Arc<dyn SemanticEnricher>,
        kv: Arc<dyn KVStore>,
        persistence: KnowledgePersistencePolicy,
    ) -> Self {
        Self {
            enricher,
            kv,
            introspection: Arc::new(IntrospectionService::new(db)),
            persistence,
        }
    }

    /// Discover available tables and columns for extraction context.
    ///
    /// Uses `Nursery` for parallel schema introspection — the first
    /// production callsite for structured concurrency in the framework.
    /// Each schema is introspected in parallel via a nursery child task,
    /// collecting both qualified table names and column names.
    pub async fn discover_context(
        &self,
        cancel: &CancellationToken,
    ) -> Result<DatabaseContext, KnowledgeExtractionError> {
        let schemas = self
            .introspection
            .list_schemas()
            .await
            .map_err(|e| KnowledgeExtractionError::Introspection(e.to_string()))?;

        if cancel.is_cancelled() {
            return Err(KnowledgeExtractionError::Cancelled);
        }

        // Nursery collects (tables, columns) per schema via mpsc channel.
        // Channel avoids Arc<Mutex<Vec>> — each child sends results, parent collects.
        let (tx, mut rx) = mpsc::channel::<(Vec<String>, Vec<String>)>(schemas.len().max(1));

        use agent_fw_algebra::nursery::Nursery;
        let result = with_nursery(cancel, |nursery: &mut Nursery<KnowledgeExtractionError>| {
            for schema in &schemas {
                let introspection = Arc::clone(&self.introspection);
                let tx = tx.clone();
                let schema = schema.clone();

                nursery.spawn(move |child_cancel| async move {
                    if child_cancel.is_cancelled() {
                        return Ok(());
                    }

                    let tables = introspection
                        .list_tables(&schema)
                        .await
                        .map_err(|e| KnowledgeExtractionError::Introspection(e.to_string()))?;

                    let mut table_names = Vec::new();
                    let mut col_names = Vec::new();

                    for table_info in &tables {
                        let qualified = format!("{}.{}", schema, table_info.table_name);

                        // Introspect each table to get column details
                        match introspection
                            .introspect_table(&schema, &table_info.table_name)
                            .await
                        {
                            Ok(physical) => {
                                for col in &physical.columns {
                                    col_names.push(format!(
                                        "{}.{}.{}",
                                        schema, table_info.table_name, col.column_name
                                    ));
                                }
                            }
                            Err(e) => {
                                // Non-fatal: we still have the table name
                                tracing::debug!(
                                    schema = %schema,
                                    table = %table_info.table_name,
                                    error = %e,
                                    "Column introspection failed, skipping columns"
                                );
                            }
                        }

                        table_names.push(qualified);
                    }

                    // Send results back — ignore error if receiver dropped
                    let _ = tx.send((table_names, col_names)).await;
                    Ok(())
                });
            }
            // Body completes immediately; nursery waits for all spawned tasks.
            async { Ok(()) }
        })
        .await;

        // Drop our sender so rx knows all senders are gone
        drop(tx);

        match result {
            Ok(()) => {}
            Err(e) => {
                return Err(KnowledgeExtractionError::Nursery(format!("{e:?}")));
            }
        }

        // Collect all results from the channel
        let mut tables = Vec::new();
        let mut columns = Vec::new();
        while let Some((t, c)) = rx.recv().await {
            tables.extend(t);
            columns.extend(c);
        }

        debug!(
            tables = tables.len(),
            columns = columns.len(),
            "Knowledge extraction context discovered"
        );

        Ok(DatabaseContext { tables, columns })
    }

    /// Extract knowledge from a single document.
    ///
    /// Discovers database context, then calls `enricher.extract_knowledge()`
    /// with retry + timeout + cancellation.
    pub async fn extract_from_document(
        &self,
        tenant_id: &str,
        document: &DocumentItem,
        cancel: &CancellationToken,
        progress: Option<&mpsc::Sender<ExtractionEvent>>,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeExtractionError> {
        let context = self.discover_context(cancel).await?;

        if let Some(tx) = progress {
            let _ = emit(
                tx,
                ExtractionEvent::DiscoveryCompleted {
                    tables_found: context.tables.len(),
                    columns_found: context.columns.len(),
                },
            )
            .await;
        }

        self.extract_document_with_context(tenant_id, document, &context, cancel, progress)
            .await
    }

    /// Extract knowledge from multiple documents.
    ///
    /// Discovers database context **once**, then processes documents sequentially.
    /// (LLM calls are expensive; parallelism is at the introspection level via Nursery.)
    pub async fn extract_from_documents(
        &self,
        tenant_id: &str,
        documents: &[DocumentItem],
        cancel: &CancellationToken,
        progress: Option<&mpsc::Sender<ExtractionEvent>>,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeExtractionError> {
        // Discover context once — database schema doesn't change between documents
        let context = self.discover_context(cancel).await?;

        if let Some(tx) = progress {
            let _ = emit(
                tx,
                ExtractionEvent::DiscoveryCompleted {
                    tables_found: context.tables.len(),
                    columns_found: context.columns.len(),
                },
            )
            .await;
        }

        let mut all_items = Vec::new();

        for doc in documents {
            if cancel.is_cancelled() {
                return Err(KnowledgeExtractionError::Cancelled);
            }

            match self
                .extract_document_with_context(tenant_id, doc, &context, cancel, progress)
                .await
            {
                Ok(items) => {
                    all_items.extend(items);
                }
                Err(e) => {
                    warn!(document_id = %doc.id, error = %e, "Document extraction failed");
                    if let Some(tx) = progress {
                        let _ = emit(
                            tx,
                            ExtractionEvent::ExtractionFailed {
                                document_id: doc.id.clone(),
                                error: e.to_string(),
                            },
                        )
                        .await;
                    }
                    // Update document status to Failed
                    let _ = self
                        .update_document_status(tenant_id, doc, ExtractionStatus::Failed)
                        .await;
                }
            }
        }

        if let Some(tx) = progress {
            let _ = emit(
                tx,
                ExtractionEvent::AllCompleted {
                    total_items: all_items.len(),
                    total_documents: documents.len(),
                },
            )
            .await;
        }

        Ok(all_items)
    }

    /// Extract knowledge from documents using PipelineCtx for cancel + progress.
    ///
    /// Convenience entry point for studio routes. Uses the PipelineCtx's cancel
    /// token for cooperative cancellation and emits ExtractionEvent progress.
    /// Discovers database context **once**, then processes documents sequentially.
    pub async fn extract_from_documents_with_ctx(
        &self,
        tenant_id: &str,
        documents: &[DocumentItem],
        ctx: &PipelineCtx<ExtractionEvent>,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeExtractionError> {
        let context = self.discover_context(ctx.cancel_token()).await?;

        let _ = ctx
            .emit_progress(ExtractionEvent::DiscoveryCompleted {
                tables_found: context.tables.len(),
                columns_found: context.columns.len(),
            })
            .await;

        let mut all_items = Vec::new();

        for doc in documents {
            if ctx.cancel_token().is_cancelled() {
                return Err(KnowledgeExtractionError::Cancelled);
            }

            let _ = ctx
                .emit_progress(ExtractionEvent::ExtractionStarted {
                    document_id: doc.id.clone(),
                    document_name: doc.name.clone(),
                })
                .await;

            let request = KnowledgeExtractionRequest {
                document_content: doc.content.clone(),
                document_name: doc.name.clone(),
                database_context: doc.target_database_id.clone(),
                available_tables: context.tables.clone(),
                available_columns: context.columns.clone(),
            };

            match self.extract_with_retry(request, ctx.cancel_token()).await {
                Ok(items) => {
                    self.persist_results(tenant_id, doc, &items).await?;
                    let _ = ctx
                        .emit_progress(ExtractionEvent::ExtractionCompleted {
                            document_id: doc.id.clone(),
                            items_extracted: items.len(),
                        })
                        .await;
                    info!(
                        document_id = %doc.id,
                        items = items.len(),
                        "Knowledge extraction completed"
                    );
                    all_items.extend(items);
                }
                Err(e) => {
                    warn!(document_id = %doc.id, error = %e, "Document extraction failed");
                    let _ = ctx
                        .emit_progress(ExtractionEvent::ExtractionFailed {
                            document_id: doc.id.clone(),
                            error: e.to_string(),
                        })
                        .await;
                    let _ = self
                        .update_document_status(tenant_id, doc, ExtractionStatus::Failed)
                        .await;
                }
            }
        }

        let _ = ctx
            .emit_progress(ExtractionEvent::AllCompleted {
                total_items: all_items.len(),
                total_documents: documents.len(),
            })
            .await;

        Ok(all_items)
    }

    /// Extract knowledge from a single document using pre-discovered context.
    ///
    /// Internal workhorse: skips discovery, uses the provided `DatabaseContext`.
    async fn extract_document_with_context(
        &self,
        tenant_id: &str,
        document: &DocumentItem,
        context: &DatabaseContext,
        cancel: &CancellationToken,
        progress: Option<&mpsc::Sender<ExtractionEvent>>,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeExtractionError> {
        if let Some(tx) = progress {
            let _ = emit(
                tx,
                ExtractionEvent::ExtractionStarted {
                    document_id: document.id.clone(),
                    document_name: document.name.clone(),
                },
            )
            .await;
        }

        let request = KnowledgeExtractionRequest {
            document_content: document.content.clone(),
            document_name: document.name.clone(),
            database_context: document.target_database_id.clone(),
            available_tables: context.tables.clone(),
            available_columns: context.columns.clone(),
        };

        let items = self.extract_with_retry(request, cancel).await?;

        self.persist_results(tenant_id, document, &items).await?;

        if let Some(tx) = progress {
            let _ = emit(
                tx,
                ExtractionEvent::ExtractionCompleted {
                    document_id: document.id.clone(),
                    items_extracted: items.len(),
                },
            )
            .await;
        }

        info!(
            document_id = %document.id,
            items = items.len(),
            "Knowledge extraction completed"
        );

        Ok(items)
    }

    /// Call enricher.extract_knowledge() with retry + timeout + cancellation.
    async fn extract_with_retry(
        &self,
        request: KnowledgeExtractionRequest,
        cancel: &CancellationToken,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeExtractionError> {
        let enricher = Arc::clone(&self.enricher);
        let policy = extraction_retry_policy();

        let result = retry_when(
            &policy,
            || {
                let e = enricher.clone();
                let r = request.clone();
                let cancel = cancel.clone();
                async move {
                    let extraction_fut = e.extract_knowledge(r);
                    let timed = with_timeout(EXTRACTION_TIMEOUT, extraction_fut).await;
                    match timed {
                        Ok(Ok(items)) => {
                            if cancel.is_cancelled() {
                                Err(EnrichmentError::Cancelled)
                            } else {
                                Ok(items)
                            }
                        }
                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(EnrichmentError::Timeout {
                            duration_ms: EXTRACTION_TIMEOUT.as_millis() as u64,
                        }),
                    }
                }
            },
            |e: &EnrichmentError| e.is_retryable(),
        )
        .await?;

        Ok(result)
    }

    /// Persist extracted knowledge items to KV and update document status.
    async fn persist_results(
        &self,
        tenant_id: &str,
        document: &DocumentItem,
        items: &[KnowledgeItem],
    ) -> Result<(), KnowledgeExtractionError> {
        let knowledge_ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();

        // Save each knowledge item to KV.
        for item in items {
            let key = self.persistence.knowledge_key(&item.id);
            self.kv
                .put(tenant_id, &key, item, None)
                .await
                .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?;
        }

        // Optionally maintain a JSON `{ ids: [...] }` index for workspace-style listing.
        if let Some(index_key) = self.persistence.knowledge_index_key.as_deref() {
            let mut index: IdIndex = self
                .kv
                .get(tenant_id, index_key)
                .await
                .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?
                .unwrap_or_default();

            if self.persistence.prepend_knowledge_index_entries {
                for knowledge_id in knowledge_ids.iter().rev() {
                    if !index.ids.iter().any(|existing| existing == knowledge_id) {
                        index.ids.insert(0, knowledge_id.clone());
                    }
                }
            } else {
                for knowledge_id in &knowledge_ids {
                    if !index.ids.iter().any(|existing| existing == knowledge_id) {
                        index.ids.push(knowledge_id.clone());
                    }
                }
            }

            self.kv
                .put(tenant_id, index_key, &index, None)
                .await
                .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?;
        }

        match self.persistence.document_knowledge_ids_key(&document.id) {
            Some(ids_key) => {
                self.kv
                    .put(tenant_id, &ids_key, &knowledge_ids, None)
                    .await
                    .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?;

                self.update_document_status(tenant_id, document, ExtractionStatus::Processed)
                    .await?;
            }
            None => {
                let mut updated = document.clone();
                updated.extraction_status = ExtractionStatus::Processed;
                updated.extracted_knowledge_ids = knowledge_ids;
                let document_key = self.persistence.document_key(&document.id);
                self.kv
                    .put(tenant_id, &document_key, &updated, None)
                    .await
                    .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?;
            }
        }

        debug!(
            document_id = %document.id,
            knowledge_items = items.len(),
            "Persisted extraction results"
        );

        Ok(())
    }

    /// Update the extraction status of a document in KV.
    async fn update_document_status(
        &self,
        tenant_id: &str,
        document: &DocumentItem,
        status: ExtractionStatus,
    ) -> Result<(), KnowledgeExtractionError> {
        let mut updated = document.clone();
        updated.extraction_status = status;
        let key = self.persistence.document_key(&document.id);
        self.kv
            .put(tenant_id, &key, &updated, None)
            .await
            .map_err(|e| KnowledgeExtractionError::KVStore(e.to_string()))?;
        Ok(())
    }
}

// =============================================================================
// Supporting Types
// =============================================================================

/// Database context for knowledge extraction requests.
#[derive(Debug, Clone)]
pub struct DatabaseContext {
    /// Qualified table names (schema.table).
    pub tables: Vec<String>,
    /// Column names found across all tables.
    pub columns: Vec<String>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::{DashMapKVStore, MockEnricher, MockTargetDatabase};

    fn make_service() -> KnowledgeExtractionService {
        let db = Arc::new(MockTargetDatabase::new());
        let enricher = Arc::new(MockEnricher::new());
        let kv = Arc::new(DashMapKVStore::new());
        KnowledgeExtractionService::new(db, enricher, kv)
    }

    fn make_document() -> DocumentItem {
        DocumentItem {
            id: "doc-1".into(),
            name: "test-doc.md".into(),
            content: "Revenue is calculated as quantity * price".into(),
            target_database_id: None,
            extraction_status: ExtractionStatus::Pending,
            extracted_knowledge_ids: vec![],
            created_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn extract_from_document_completes() {
        let svc = make_service();
        let cancel = CancellationToken::new();
        let doc = make_document();

        // MockEnricher returns empty knowledge items and
        // MockTargetDatabase returns empty query results (no tables).
        // The service should handle this gracefully.
        let result = svc
            .extract_from_document("tenant-1", &doc, &cancel, None)
            .await;

        // MockTargetDatabase's query returns empty rows, so introspection
        // finds no schemas/tables. Extraction still succeeds with empty context.
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn respects_cancellation() {
        let svc = make_service();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let doc = make_document();
        let result = svc
            .extract_from_document("tenant-1", &doc, &cancel, None)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KnowledgeExtractionError::Cancelled
        ));
    }

    #[tokio::test]
    async fn emits_progress_events() {
        let svc = make_service();
        let cancel = CancellationToken::new();
        let doc = make_document();

        let (tx, mut rx) = mpsc::channel(16);

        let result = svc
            .extract_from_document("tenant-1", &doc, &cancel, Some(&tx))
            .await;

        assert!(result.is_ok());

        // Should have received at least ExtractionStarted and ExtractionCompleted
        let mut events = vec![];
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(
            events.len() >= 2,
            "Expected at least 2 events, got {}",
            events.len()
        );
    }
}
