//! Vector ingestion pipeline — populates VectorStore from catalog entries.
//!
//! Composes `VectorStore` with `CatalogWriter` output to create searchable
//! embeddings for schema knowledge.
//!
//! # Architecture
//!
//! ```text
//! CatalogEntry ──► build_embedding_content() ──► EmbeddingItem
//!                  (pure: no IO)                      │
//!                                                     ▼
//!                                    VectorStore::upsert_batch()
//! ```
//!
//! # Content-addressed idempotency
//!
//! Each embedding item uses a deterministic ID derived from the catalog entry ID,
//! so re-running ingestion upserts (overwrites) rather than duplicates.
//!
//! The `SchemaFingerprint` provides a SHA-256 sentinel: if the schema hasn't
//! changed since last ingestion, the entire pipeline can be skipped.
//!
//! # Parametricity
//!
//! `VectorIngestionService<V>` and `KnowledgeManager<E, W, V>` are parametric
//! in their algebra dependencies. The caller decides the concrete types and
//! allocation strategy — no `Arc<dyn Trait>` baked in.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

use agent_fw_algebra::{EmbeddingService, VectorStore, VectorStoreError};
use agent_fw_catalog::{
    CatalogEntry, CatalogError, CatalogKind, KnowledgeItem, SemanticTableProfile, TableProfile,
};

// =============================================================================
// EmbeddingKind — classification of embedding items
// =============================================================================

/// Classification of embedding items.
///
/// Forms a finite set whose cardinality equals the number of ingestion phases.
/// Each kind corresponds to a distinct phase in the ingestion pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingKind {
    /// Table-level embeddings for navigation.
    Table,
    /// Column-level metadata embeddings.
    Column,
    /// Concrete product examples.
    ProductSample,
    /// Categorical value embeddings.
    Tag,
    /// Usage guidance patterns.
    FilterPattern,
    /// Business rules & terminology.
    Knowledge,
    /// FK constraint embeddings for JOIN graph.
    Relationship,
}

impl EmbeddingKind {
    /// Get the string representation for IDs and types.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Column => "column",
            Self::ProductSample => "product_sample",
            Self::Tag => "tag",
            Self::FilterPattern => "filter_pattern",
            Self::Knowledge => "knowledge",
            Self::Relationship => "relationship",
        }
    }

    /// All embedding kinds in the recommended ingestion order.
    pub const ALL: &'static [Self] = &[
        Self::Table,
        Self::Column,
        Self::ProductSample,
        Self::Tag,
        Self::FilterPattern,
        Self::Knowledge,
        Self::Relationship,
    ];
}

impl std::fmt::Display for EmbeddingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// =============================================================================
// IngestionSummary — tracking ingestion progress (Monoid)
// =============================================================================

/// Summary of an ingestion run.
///
/// # Algebra
///
/// `IngestionSummary` forms a **Monoid** under `combine`:
///
/// - **Identity**: `IngestionSummary::default()` is the identity element
/// - **Associativity**: `(a.combine(b)).combine(c) == a.combine(b.combine(c))`
///
/// This allows accumulating summaries from parallel ingestion phases:
///
/// ```ignore
/// let table_summary = ingest_tables().await?;
/// let column_summary = ingest_columns().await?;
/// let total = table_summary.combine(column_summary);
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionSummary {
    /// Items ingested per kind.
    pub by_kind: std::collections::HashMap<String, usize>,
    /// Total items ingested.
    pub total: usize,
    /// Whether ingestion was skipped (content hash unchanged).
    pub skipped: bool,
}

impl IngestionSummary {
    /// Create an empty summary (Monoid identity).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add count for a kind.
    pub fn add_kind(&mut self, kind: EmbeddingKind, count: usize) {
        *self.by_kind.entry(kind.as_str().to_string()).or_insert(0) += count;
        self.total += count;
    }

    /// Mark as skipped.
    pub fn mark_skipped(&mut self) {
        self.skipped = true;
    }

    /// Monoid combine — accumulate two summaries into one.
    ///
    /// # Laws
    ///
    /// - L1 (Identity): `Self::default().combine(x) == x`
    /// - L2 (Identity): `x.combine(Self::default()) == x`
    /// - L3 (Associativity): `(a.combine(b)).combine(c) == a.combine(b.combine(c))`
    pub fn combine(self, other: Self) -> Self {
        let mut combined = self;
        for (kind, count) in other.by_kind {
            *combined.by_kind.entry(kind).or_default() += count;
        }
        combined.total += other.total;
        // If either was skipped, the combined result is "skipped"
        // (semantic: we didn't ingest new data in at least one phase)
        combined.skipped = combined.skipped || other.skipped;
        combined
    }
}

// =============================================================================
// Typed metadata — replace serde_json::json!({}) with compile-time checked structs
// =============================================================================

/// Metadata for a catalog entry's vector embedding.
///
/// Typed struct — misspelled fields or wrong types are caught at compile time,
/// unlike raw `serde_json::json!({})`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEmbeddingMetadata {
    pub catalog_id: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
}

/// Metadata for a knowledge item's vector embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEmbeddingMetadata {
    pub knowledge_id: String,
    pub knowledge_type: String,
    pub name: String,
    pub scope_tables: Vec<String>,
}

// =============================================================================
// SchemaFingerprint — content-addressed idempotency sentinel
// =============================================================================

/// SHA-256 fingerprint of a schema's structural identity.
///
/// When stored in KV, allows the ingestion pipeline to skip re-processing
/// if the schema hasn't changed.
///
/// # Law — Determinism
///
/// ```text
/// fingerprint(schema_a) == fingerprint(schema_a)
/// schema_a ≠ schema_b  ⟹  fingerprint(schema_a) ≠ fingerprint(schema_b)
///                           (with overwhelming probability)
/// ```
///
/// # Law — Order Independence
///
/// ```text
/// fingerprint(permute(tables)) == fingerprint(tables)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaFingerprint(pub String);

impl SchemaFingerprint {
    fn compare_column_signature(a: &ColumnSignature, b: &ColumnSignature) -> Ordering {
        (
            a.name.as_str(),
            a.data_type.as_str(),
            a.is_nullable,
            a.is_primary_key,
        )
            .cmp(&(
                b.name.as_str(),
                b.data_type.as_str(),
                b.is_nullable,
                b.is_primary_key,
            ))
    }

    /// Compute fingerprint from table names and their column signatures.
    ///
    /// The fingerprint captures: table names, column names, column types,
    /// nullability, and primary key status — the structural identity of
    /// the schema that affects profiling output.
    pub fn from_tables(tables: &[(String, Vec<ColumnSignature>)]) -> Self {
        let mut hasher = Sha256::new();
        // Sort tables by their full canonical representation for determinism.
        // This keeps the order-independence law valid even for duplicate names.
        let mut sorted: Vec<_> = tables
            .iter()
            .map(|(name, columns)| {
                let mut sorted_cols: Vec<_> = columns.iter().collect();
                sorted_cols.sort_by(|a, b| Self::compare_column_signature(a, b));
                (name, sorted_cols)
            })
            .collect();
        sorted.sort_by(|(a_name, a_cols), (b_name, b_cols)| {
            a_name.as_str().cmp(b_name.as_str()).then_with(|| {
                a_cols
                    .iter()
                    .map(|col| {
                        (
                            col.name.as_str(),
                            col.data_type.as_str(),
                            col.is_nullable,
                            col.is_primary_key,
                        )
                    })
                    .cmp(b_cols.iter().map(|col| {
                        (
                            col.name.as_str(),
                            col.data_type.as_str(),
                            col.is_nullable,
                            col.is_primary_key,
                        )
                    }))
            })
        });
        for (table_name, columns) in sorted {
            hasher.update(table_name.as_bytes());
            hasher.update(b"\n");
            for col in columns {
                hasher.update(col.name.as_bytes());
                hasher.update(b":");
                hasher.update(col.data_type.as_bytes());
                hasher.update(b":");
                hasher.update(if col.is_nullable { b"NL" } else { b"NN" });
                hasher.update(b":");
                hasher.update(if col.is_primary_key { b"PK" } else { b"NK" });
                hasher.update(b"\n");
            }
        }
        Self(hex::encode(hasher.finalize()))
    }

    /// Build from introspection results.
    pub fn from_physical_tables(tables: &[agent_fw_catalog::PhysicalTable]) -> Self {
        let entries: Vec<(String, Vec<ColumnSignature>)> = tables
            .iter()
            .map(|t| {
                let cols = t
                    .columns
                    .iter()
                    .map(|c| ColumnSignature {
                        name: c.column_name.clone(),
                        data_type: c.data_type.clone(),
                        is_nullable: c.is_nullable,
                        is_primary_key: c.is_primary_key,
                    })
                    .collect();
                (format!("{}.{}", t.schema_name, t.table_name), cols)
            })
            .collect();
        Self::from_tables(&entries)
    }
}

/// Column signature for fingerprinting.
#[derive(Debug, Clone)]
pub struct ColumnSignature {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
}

// =============================================================================
// EmbeddingContent — pure content builders
// =============================================================================

/// Build searchable text content from a catalog entry.
///
/// Pure function — no IO. The content is designed for semantic search:
/// table/column descriptions, relationships, and quality notes are
/// formatted into a single text block.
pub fn build_embedding_content(entry: &CatalogEntry) -> String {
    let mut parts = Vec::new();
    parts.push(format!("[{}] {}", entry.kind.as_str(), entry.name));
    if !entry.content.is_empty() {
        parts.push(entry.content.clone());
    }
    if let Some(ref qn) = entry.qualified_name {
        parts.push(format!("Path: {}", qn));
    }
    parts.join("\n")
}

/// Build embedding content for a table with its semantic profile.
pub fn build_table_embedding(
    schema: &str,
    table: &str,
    semantic: &SemanticTableProfile,
    profile: &TableProfile,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("[table] {}.{}", schema, table));
    parts.push(semantic.description.clone());
    if !semantic.short_description.is_empty() {
        parts.push(format!("Summary: {}", semantic.short_description));
    }
    // Include column descriptions for richer semantic search
    for (col_name, desc) in semantic.column_descriptions.iter() {
        parts.push(format!("  {}: {}", col_name, desc));
    }
    // Include relationships
    for rel in &semantic.relationships {
        parts.push(format!(
            "  {} -> {} ({})",
            rel.source_table, rel.target_table, rel.description
        ));
    }
    // Include quality notes
    for note in &semantic.quality_notes {
        parts.push(format!("  [quality:{}] {}", note.column_name, note.notes));
    }
    // Include profiling summary
    parts.push(format!("Columns: {}", profile.columns.len()));
    parts.join("\n")
}

/// Build embedding content for a knowledge item.
pub fn build_knowledge_embedding(item: &KnowledgeItem) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "[knowledge:{}] {}",
        item.knowledge_type.as_str(),
        item.name
    ));
    parts.push(item.description.clone());
    if !item.scope_tables.is_empty() {
        parts.push(format!("Tables: {}", item.scope_tables.join(", ")));
    }
    if !item.scope_columns.is_empty() {
        parts.push(format!("Columns: {}", item.scope_columns.join(", ")));
    }
    if let Some(ref sql) = item.sql_expression {
        parts.push(format!("SQL: {}", sql));
    }
    if !item.synonyms.is_empty() {
        parts.push(format!("Synonyms: {}", item.synonyms.join(", ")));
    }
    parts.join("\n")
}

// =============================================================================
// VectorIngestionService<V> — parametric over VectorStore
// =============================================================================

/// Vector ingestion service: populates VectorStore from catalog entries.
///
/// Parametric in `V: VectorStore` — the caller decides the concrete
/// implementation and allocation strategy. The embedder uses `Arc<dyn>`
/// since it's an optional external service behind a network call.
///
/// When an `EmbeddingService` is provided via `with_embedder`, content is
/// batch-embedded before upserting. Without an embedder, items are stored
/// with empty embedding vectors (sentinel rows for metadata-only indexing).
///
/// Content-addressed IDs ensure idempotent re-ingestion.
pub struct VectorIngestionService<V> {
    vector_store: V,
    embedder: Option<std::sync::Arc<dyn EmbeddingService>>,
}

impl<V: VectorStore> VectorIngestionService<V> {
    pub fn new(vector_store: V) -> Self {
        Self {
            vector_store,
            embedder: None,
        }
    }

    /// Attach an embedding service for computing vectors before upserting.
    pub fn with_embedder(mut self, embedder: std::sync::Arc<dyn EmbeddingService>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Access the underlying vector store.
    pub fn store(&self) -> &V {
        &self.vector_store
    }

    /// Batch-compute embeddings for a list of content strings.
    ///
    /// When no embedder is configured, returns empty vectors (sentinel).
    async fn compute_embeddings(
        &self,
        contents: &[String],
    ) -> Result<Vec<Vec<f32>>, VectorStoreError> {
        match &self.embedder {
            Some(embedder) => {
                let text_refs: Vec<&str> = contents.iter().map(|s| s.as_str()).collect();
                embedder
                    .embed_batch(&text_refs)
                    .await
                    .map_err(|e| VectorStoreError::Execution(format!("embedding failed: {e}")))
            }
            None => Ok(contents.iter().map(|_| vec![]).collect()),
        }
    }

    /// Ingest catalog entries into the vector store.
    ///
    /// Each entry is converted to searchable text, embedded (if an embedder
    /// is configured), and upserted with a deterministic ID (`vec:{catalog_id}`).
    ///
    /// Returns the number of items upserted.
    pub async fn ingest_catalog_entries(
        &self,
        entries: &[CatalogEntry],
    ) -> Result<usize, VectorStoreError> {
        if entries.is_empty() {
            return Ok(0);
        }

        // Build content + metadata in one pass
        let prepared: Vec<(String, CatalogEmbeddingMetadata, &CatalogEntry)> = entries
            .iter()
            .map(|entry| {
                let content = build_embedding_content(entry);
                let metadata = CatalogEmbeddingMetadata {
                    catalog_id: entry.id.clone(),
                    kind: entry.kind.as_str().to_string(),
                    name: entry.name.clone(),
                    qualified_name: entry.qualified_name.clone(),
                };
                (content, metadata, entry)
            })
            .collect();

        // Batch-compute embeddings
        let contents: Vec<String> = prepared.iter().map(|(c, _, _)| c.clone()).collect();
        let embeddings = self.compute_embeddings(&contents).await?;

        // Zip into EmbeddingItems
        let items: Vec<agent_fw_algebra::EmbeddingItem> = prepared
            .into_iter()
            .zip(embeddings)
            .map(
                |((content, metadata, entry), embedding)| agent_fw_algebra::EmbeddingItem {
                    id: format!("vec:{}", entry.id),
                    content,
                    item_type: entry.kind.as_str().to_string(),
                    metadata: serde_json::to_value(metadata).unwrap_or_default(),
                    embedding,
                },
            )
            .collect();

        self.vector_store.upsert_batch(&items).await
    }

    /// Ingest knowledge items into the vector store.
    ///
    /// Returns the number of items upserted.
    pub async fn ingest_knowledge_items(
        &self,
        items: &[KnowledgeItem],
    ) -> Result<usize, VectorStoreError> {
        if items.is_empty() {
            return Ok(0);
        }

        // Build content + metadata
        let prepared: Vec<(String, KnowledgeEmbeddingMetadata, &KnowledgeItem)> = items
            .iter()
            .map(|item| {
                let content = build_knowledge_embedding(item);
                let metadata = KnowledgeEmbeddingMetadata {
                    knowledge_id: item.id.clone(),
                    knowledge_type: item.knowledge_type.as_str().to_string(),
                    name: item.name.clone(),
                    scope_tables: item.scope_tables.clone(),
                };
                (content, metadata, item)
            })
            .collect();

        // Batch-compute embeddings
        let contents: Vec<String> = prepared.iter().map(|(c, _, _)| c.clone()).collect();
        let embeddings = self.compute_embeddings(&contents).await?;

        // Zip into EmbeddingItems
        let embedding_items: Vec<agent_fw_algebra::EmbeddingItem> = prepared
            .into_iter()
            .zip(embeddings)
            .map(
                |((content, metadata, item), embedding)| agent_fw_algebra::EmbeddingItem {
                    id: format!("vec:knowledge:{}", item.id),
                    content,
                    item_type: "knowledge".to_string(),
                    metadata: serde_json::to_value(metadata).unwrap_or_default(),
                    embedding,
                },
            )
            .collect();

        self.vector_store.upsert_batch(&embedding_items).await
    }

    /// Clean up vector entries for a database that's been re-profiled.
    ///
    /// Deletes all vector entries with the given catalog ID prefix,
    /// then re-ingests the new entries. Atomic at the semantic level
    /// (not transactional, but content-addressed IDs prevent orphans).
    pub async fn replace_for_database(
        &self,
        database_id_prefix: &str,
        entries: &[CatalogEntry],
    ) -> Result<usize, VectorStoreError> {
        // Delete old entries
        self.vector_store
            .delete_by_prefix(&format!("vec:{}", database_id_prefix))
            .await?;
        // Ingest new entries
        self.ingest_catalog_entries(entries).await
    }
}

// =============================================================================
// KnowledgeManager<E, W, V> — parametric over all three algebra traits
// =============================================================================

/// Orchestrates the knowledge management pipeline:
///
/// ```text
/// Document ──► SemanticEnricher::extract_knowledge()
///                    │
///                    ▼
///              Vec<KnowledgeItem>
///                    │
///                    ├──► CatalogWriter::save_items() (persistence)
///                    │
///                    └──► VectorIngestionService::ingest_knowledge_items() (search)
/// ```
///
/// Parametric in `E`, `W`, `V` — the caller provides concrete implementations.
/// No `Arc<dyn Trait>` baked in; no allocation strategy imposed.
///
/// # Partial failure
///
/// Steps 3 (persist) and 4 (index) use different backends and cannot share
/// a transaction. `IngestionOutcome` reports exactly what succeeded so the
/// caller can compensate on partial failure.
pub struct KnowledgeManager<E, W, V> {
    enricher: E,
    writer: W,
    vector_ingest: VectorIngestionService<V>,
}

impl<E, W, V> KnowledgeManager<E, W, V>
where
    E: agent_fw_catalog::SemanticEnricher,
    W: agent_fw_catalog::CatalogWriter,
    V: VectorStore,
{
    pub fn new(enricher: E, writer: W, vector_store: V) -> Self {
        Self {
            enricher,
            writer,
            vector_ingest: VectorIngestionService::new(vector_store),
        }
    }

    /// Attach an embedding service to the inner `VectorIngestionService`.
    ///
    /// Without this, knowledge items are stored with empty embeddings
    /// (metadata-only indexing, no semantic search).
    pub fn with_embedder(mut self, embedder: std::sync::Arc<dyn EmbeddingService>) -> Self {
        self.vector_ingest = self.vector_ingest.with_embedder(embedder);
        self
    }

    /// Access the underlying vector ingestion service.
    pub fn vector_ingest(&self) -> &VectorIngestionService<V> {
        &self.vector_ingest
    }

    /// Extract knowledge from a document and persist + index it.
    ///
    /// Returns `IngestionOutcome` which reports:
    /// - The extracted items
    /// - Whether persistence succeeded
    /// - Whether vector indexing succeeded
    ///
    /// On full success, both flags are `true`. On partial failure,
    /// the caller can inspect which step failed and compensate.
    pub async fn process_document(
        &self,
        document: &agent_fw_catalog::DocumentItem,
        database_context: Option<&str>,
        available_tables: &[String],
        available_columns: &[String],
    ) -> Result<IngestionOutcome, KnowledgeManagerError> {
        // 1. Extract knowledge via LLM
        let request = agent_fw_catalog::KnowledgeExtractionRequest {
            document_content: document.content.clone(),
            document_name: document.name.clone(),
            database_context: database_context.map(String::from),
            available_tables: available_tables.to_vec(),
            available_columns: available_columns.to_vec(),
        };

        let items = self
            .enricher
            .extract_knowledge(request)
            .await
            .map_err(KnowledgeManagerError::Extraction)?;

        if items.is_empty() {
            return Ok(IngestionOutcome {
                items,
                persisted: true,
                indexed: true,
            });
        }

        // 2. Build catalog entries for persistence
        let catalog_entries: Vec<CatalogEntry> = items
            .iter()
            .map(|item| CatalogEntry {
                id: format!("knowledge:{}", item.id),
                kind: CatalogKind::Knowledge,
                name: item.name.clone(),
                qualified_name: None,
                content: item.description.clone(),
                tags: vec![
                    "knowledge".to_string(),
                    item.knowledge_type.as_str().to_string(),
                ],
                links: vec![],
                metadata: serde_json::json!({
                    "knowledge_type": item.knowledge_type.as_str(),
                    "scope_tables": item.scope_tables,
                    "scope_columns": item.scope_columns,
                }),
            })
            .collect();

        // 3. Persist to catalog
        let persisted = match self.writer.save_items(catalog_entries).await {
            Ok(_) => true,
            Err(e) => {
                return Err(KnowledgeManagerError::Persistence {
                    source: e,
                    items_extracted: items,
                });
            }
        };

        // 4. Index into vector store for semantic search
        let indexed = match self.vector_ingest.ingest_knowledge_items(&items).await {
            Ok(_) => true,
            Err(_indexing_err) => {
                // Partial failure: items were persisted but not indexed.
                // Return outcome so the caller can compensate.
                return Ok(IngestionOutcome {
                    items,
                    persisted,
                    indexed: false,
                });
            }
        };

        Ok(IngestionOutcome {
            items,
            persisted,
            indexed,
        })
    }
}

/// Outcome of knowledge ingestion — reports partial success.
///
/// When `persisted` is true but `indexed` is false, catalog entries
/// exist without corresponding vector embeddings. The caller should
/// retry indexing or alert the user.
#[derive(Debug)]
pub struct IngestionOutcome {
    /// The extracted knowledge items.
    pub items: Vec<KnowledgeItem>,
    /// Whether catalog persistence succeeded.
    pub persisted: bool,
    /// Whether vector indexing succeeded.
    pub indexed: bool,
}

impl IngestionOutcome {
    /// True if both persistence and indexing succeeded.
    pub fn is_complete(&self) -> bool {
        self.persisted && self.indexed
    }
}

/// Errors from the knowledge management pipeline.
///
/// Preserves structured error types from the algebra layer —
/// callers can pattern-match on the *reason* for failure.
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeManagerError {
    #[error("knowledge extraction failed: {0}")]
    Extraction(agent_fw_catalog::EnrichmentError),

    /// Catalog persistence failed. Contains the items that were extracted
    /// but not yet persisted, allowing the caller to retry.
    #[error("catalog persistence failed: {source}")]
    Persistence {
        source: CatalogError,
        /// Items that were extracted but failed to persist.
        items_extracted: Vec<KnowledgeItem>,
    },
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_fingerprint_deterministic() {
        let tables = vec![(
            "public.orders".to_string(),
            vec![
                ColumnSignature {
                    name: "id".into(),
                    data_type: "integer".into(),
                    is_nullable: false,
                    is_primary_key: true,
                },
                ColumnSignature {
                    name: "amount".into(),
                    data_type: "numeric".into(),
                    is_nullable: false,
                    is_primary_key: false,
                },
            ],
        )];
        let fp1 = SchemaFingerprint::from_tables(&tables);
        let fp2 = SchemaFingerprint::from_tables(&tables);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn schema_fingerprint_changes_on_schema_change() {
        let tables_v1 = vec![(
            "public.orders".to_string(),
            vec![ColumnSignature {
                name: "id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                is_primary_key: true,
            }],
        )];
        let tables_v2 = vec![(
            "public.orders".to_string(),
            vec![
                ColumnSignature {
                    name: "id".into(),
                    data_type: "integer".into(),
                    is_nullable: false,
                    is_primary_key: true,
                },
                ColumnSignature {
                    name: "status".into(),
                    data_type: "text".into(),
                    is_nullable: true,
                    is_primary_key: false,
                },
            ],
        )];
        let fp1 = SchemaFingerprint::from_tables(&tables_v1);
        let fp2 = SchemaFingerprint::from_tables(&tables_v2);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn schema_fingerprint_order_independent() {
        let tables_a = vec![
            ("public.a".to_string(), vec![]),
            ("public.b".to_string(), vec![]),
        ];
        let tables_b = vec![
            ("public.b".to_string(), vec![]),
            ("public.a".to_string(), vec![]),
        ];
        let fp_a = SchemaFingerprint::from_tables(&tables_a);
        let fp_b = SchemaFingerprint::from_tables(&tables_b);
        assert_eq!(fp_a, fp_b, "Fingerprint must be order-independent");
    }

    #[test]
    fn schema_fingerprint_column_order_independent() {
        let tables_a = vec![(
            "public.t".to_string(),
            vec![
                ColumnSignature {
                    name: "a".into(),
                    data_type: "int".into(),
                    is_nullable: false,
                    is_primary_key: true,
                },
                ColumnSignature {
                    name: "b".into(),
                    data_type: "text".into(),
                    is_nullable: true,
                    is_primary_key: false,
                },
            ],
        )];
        let tables_b = vec![(
            "public.t".to_string(),
            vec![
                ColumnSignature {
                    name: "b".into(),
                    data_type: "text".into(),
                    is_nullable: true,
                    is_primary_key: false,
                },
                ColumnSignature {
                    name: "a".into(),
                    data_type: "int".into(),
                    is_nullable: false,
                    is_primary_key: true,
                },
            ],
        )];
        let fp_a = SchemaFingerprint::from_tables(&tables_a);
        let fp_b = SchemaFingerprint::from_tables(&tables_b);
        assert_eq!(fp_a, fp_b, "Fingerprint must be column-order-independent");
    }

    #[test]
    fn schema_fingerprint_nullable_vs_not_nullable_differ() {
        let tables_a = vec![(
            "t".to_string(),
            vec![ColumnSignature {
                name: "c".into(),
                data_type: "int".into(),
                is_nullable: true,
                is_primary_key: false,
            }],
        )];
        let tables_b = vec![(
            "t".to_string(),
            vec![ColumnSignature {
                name: "c".into(),
                data_type: "int".into(),
                is_nullable: false,
                is_primary_key: false,
            }],
        )];
        assert_ne!(
            SchemaFingerprint::from_tables(&tables_a),
            SchemaFingerprint::from_tables(&tables_b),
            "Nullable vs not-nullable must produce different fingerprints"
        );
    }

    #[test]
    fn schema_fingerprint_pk_vs_non_pk_differ() {
        let tables_a = vec![(
            "t".to_string(),
            vec![ColumnSignature {
                name: "c".into(),
                data_type: "int".into(),
                is_nullable: false,
                is_primary_key: true,
            }],
        )];
        let tables_b = vec![(
            "t".to_string(),
            vec![ColumnSignature {
                name: "c".into(),
                data_type: "int".into(),
                is_nullable: false,
                is_primary_key: false,
            }],
        )];
        assert_ne!(
            SchemaFingerprint::from_tables(&tables_a),
            SchemaFingerprint::from_tables(&tables_b),
            "PK vs non-PK must produce different fingerprints"
        );
    }

    #[test]
    fn build_embedding_content_table() {
        let entry = CatalogEntry {
            id: "test-id".into(),
            kind: CatalogKind::Table,
            name: "orders".into(),
            qualified_name: Some("public.orders".into()),
            content: "Customer orders table".into(),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({}),
        };
        let content = build_embedding_content(&entry);
        assert!(content.contains("[table] orders"));
        assert!(content.contains("Customer orders table"));
        assert!(content.contains("Path: public.orders"));
    }

    #[test]
    fn build_knowledge_embedding_includes_all_fields() {
        let item = KnowledgeItem {
            id: "k1".into(),
            name: "ARR Definition".into(),
            description: "Annual Recurring Revenue".into(),
            knowledge_type: agent_fw_catalog::KnowledgeType::Terminology,
            scope_tables: vec!["fact_revenue".into()],
            scope_columns: vec!["revenue_amount".into()],
            sql_expression: Some("SUM(revenue_amount)".into()),
            synonyms: vec!["Annual Revenue".into()],
            source_document_id: None,
        };
        let content = build_knowledge_embedding(&item);
        assert!(content.contains("[knowledge:terminology]"));
        assert!(content.contains("ARR Definition"));
        assert!(content.contains("fact_revenue"));
        assert!(content.contains("SUM(revenue_amount)"));
        assert!(content.contains("Annual Revenue"));
    }

    #[test]
    fn catalog_embedding_metadata_serializes() {
        let meta = CatalogEmbeddingMetadata {
            catalog_id: "id-1".into(),
            kind: "table".into(),
            name: "orders".into(),
            qualified_name: Some("public.orders".into()),
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["catalogId"], "id-1");
        assert_eq!(json["kind"], "table");
        assert_eq!(json["qualifiedName"], "public.orders");
    }

    #[test]
    fn catalog_embedding_metadata_omits_none_qualified_name() {
        let meta = CatalogEmbeddingMetadata {
            catalog_id: "id-1".into(),
            kind: "table".into(),
            name: "orders".into(),
            qualified_name: None,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert!(!json.as_object().unwrap().contains_key("qualifiedName"));
    }

    #[test]
    fn knowledge_embedding_metadata_serializes() {
        let meta = KnowledgeEmbeddingMetadata {
            knowledge_id: "k1".into(),
            knowledge_type: "terminology".into(),
            name: "ARR".into(),
            scope_tables: vec!["revenue".into()],
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["knowledgeId"], "k1");
        assert_eq!(json["knowledgeType"], "terminology");
    }

    #[test]
    fn ingestion_outcome_complete_vs_partial() {
        let complete = IngestionOutcome {
            items: vec![],
            persisted: true,
            indexed: true,
        };
        assert!(complete.is_complete());

        let partial = IngestionOutcome {
            items: vec![],
            persisted: true,
            indexed: false,
        };
        assert!(!partial.is_complete());
    }

    // =========================================================================
    // EmbeddingKind tests
    // =========================================================================

    #[test]
    fn embedding_kind_as_str() {
        assert_eq!(EmbeddingKind::Table.as_str(), "table");
        assert_eq!(EmbeddingKind::Column.as_str(), "column");
        assert_eq!(EmbeddingKind::ProductSample.as_str(), "product_sample");
        assert_eq!(EmbeddingKind::Tag.as_str(), "tag");
        assert_eq!(EmbeddingKind::FilterPattern.as_str(), "filter_pattern");
        assert_eq!(EmbeddingKind::Knowledge.as_str(), "knowledge");
        assert_eq!(EmbeddingKind::Relationship.as_str(), "relationship");
    }

    #[test]
    fn embedding_kind_all_contains_all() {
        assert_eq!(EmbeddingKind::ALL.len(), 7);
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::Table));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::Column));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::ProductSample));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::Tag));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::FilterPattern));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::Knowledge));
        assert!(EmbeddingKind::ALL.contains(&EmbeddingKind::Relationship));
    }

    #[test]
    fn embedding_kind_display() {
        assert_eq!(format!("{}", EmbeddingKind::Table), "table");
        assert_eq!(
            format!("{}", EmbeddingKind::ProductSample),
            "product_sample"
        );
    }

    #[test]
    fn embedding_kind_serialization() {
        let kind = EmbeddingKind::FilterPattern;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"filter_pattern\"");

        let parsed: EmbeddingKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, EmbeddingKind::FilterPattern);
    }

    // =========================================================================
    // IngestionSummary tests
    // =========================================================================

    #[test]
    fn ingestion_summary_empty() {
        let summary = IngestionSummary::new();
        assert_eq!(summary.total, 0);
        assert!(!summary.skipped);
        assert!(summary.by_kind.is_empty());
    }

    #[test]
    fn ingestion_summary_add_kind() {
        let mut summary = IngestionSummary::new();
        summary.add_kind(EmbeddingKind::Table, 10);
        summary.add_kind(EmbeddingKind::Column, 50);
        summary.add_kind(EmbeddingKind::Table, 5); // Additional

        assert_eq!(summary.total, 65);
        assert_eq!(*summary.by_kind.get("table").unwrap(), 15);
        assert_eq!(*summary.by_kind.get("column").unwrap(), 50);
    }

    #[test]
    fn ingestion_summary_skipped() {
        let mut summary = IngestionSummary::new();
        summary.mark_skipped();
        assert!(summary.skipped);
    }

    // =========================================================================
    // IngestionSummary Monoid Laws
    // =========================================================================

    /// L1 (Left Identity): mempty.combine(x) == x
    #[test]
    fn ingestion_summary_monoid_left_identity() {
        let mut x = IngestionSummary::new();
        x.add_kind(EmbeddingKind::Table, 10);
        x.add_kind(EmbeddingKind::Column, 20);

        let identity = IngestionSummary::default();
        let combined = identity.combine(x.clone());

        assert_eq!(combined.total, x.total);
        assert_eq!(combined.by_kind, x.by_kind);
        assert_eq!(combined.skipped, x.skipped);
    }

    /// L2 (Right Identity): x.combine(mempty) == x
    #[test]
    fn ingestion_summary_monoid_right_identity() {
        let mut x = IngestionSummary::new();
        x.add_kind(EmbeddingKind::Table, 10);
        x.add_kind(EmbeddingKind::Column, 20);

        let identity = IngestionSummary::default();
        let combined = x.clone().combine(identity);

        assert_eq!(combined.total, x.total);
        assert_eq!(combined.by_kind, x.by_kind);
        assert_eq!(combined.skipped, x.skipped);
    }

    /// L3 (Associativity): (a.combine(b)).combine(c) == a.combine(b.combine(c))
    #[test]
    fn ingestion_summary_monoid_associativity() {
        let mut a = IngestionSummary::new();
        a.add_kind(EmbeddingKind::Table, 10);
        a.add_kind(EmbeddingKind::Column, 20);

        let mut b = IngestionSummary::new();
        b.add_kind(EmbeddingKind::Table, 5);
        b.add_kind(EmbeddingKind::Knowledge, 100);

        let mut c = IngestionSummary::new();
        c.add_kind(EmbeddingKind::Column, 30);
        c.mark_skipped();

        // (a.combine(b)).combine(c)
        let left = a.clone().combine(b.clone()).combine(c.clone());

        // a.combine(b.combine(c))
        let right = a.clone().combine(b.clone().combine(c.clone()));

        assert_eq!(left.total, right.total, "Monoid associativity: total");
        assert_eq!(left.by_kind, right.by_kind, "Monoid associativity: by_kind");
        assert_eq!(left.skipped, right.skipped, "Monoid associativity: skipped");
    }

    /// Skipped propagation: skipped.combine(non-skipped) is skipped
    #[test]
    fn ingestion_summary_skipped_propagation() {
        let mut skipped = IngestionSummary::new();
        skipped.mark_skipped();

        let mut non_skipped = IngestionSummary::new();
        non_skipped.add_kind(EmbeddingKind::Table, 5);

        let combined = skipped.combine(non_skipped);
        assert!(combined.skipped, "Skipped OR non-skipped should be skipped");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use hegel::generators;

    fn draw_column_signature(tc: &hegel::TestCase) -> ColumnSignature {
        let name: String = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));
        let data_type: String = tc.draw(generators::sampled_from(vec![
            "integer".to_string(),
            "text".to_string(),
            "numeric".to_string(),
            "boolean".to_string(),
            "uuid".to_string(),
        ]));
        let is_nullable: bool = tc.draw(generators::booleans());
        let is_primary_key: bool = tc.draw(generators::booleans());
        ColumnSignature {
            name,
            data_type,
            is_nullable,
            is_primary_key,
        }
    }

    fn draw_table(tc: &hegel::TestCase) -> (String, Vec<ColumnSignature>) {
        let name: String =
            tc.draw(generators::from_regex(r"[a-z]{1,8}\.[a-z]{1,8}").fullmatch(true));
        let col_count: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(4));
        let cols: Vec<ColumnSignature> =
            (0..col_count).map(|_| draw_column_signature(tc)).collect();
        (name, cols)
    }

    fn draw_tables(tc: &hegel::TestCase) -> Vec<(String, Vec<ColumnSignature>)> {
        let table_count: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(5));
        (0..table_count).map(|_| draw_table(tc)).collect()
    }

    /// Law: fingerprint(schema) == fingerprint(schema)  (determinism)
    #[hegel::test]
    fn fingerprint_determinism(tc: hegel::TestCase) {
        let tables = draw_tables(&tc);
        let fp1 = SchemaFingerprint::from_tables(&tables);
        let fp2 = SchemaFingerprint::from_tables(&tables);
        assert_eq!(fp1, fp2);
    }

    /// Law: fingerprint(permute(tables)) == fingerprint(tables)  (order independence)
    #[hegel::test]
    fn fingerprint_table_order_independence(tc: hegel::TestCase) {
        let tables = draw_tables(&tc);
        if tables.len() < 2 {
            tc.assume(false);
            return;
        }
        let mut reversed = tables.clone();
        reversed.reverse();
        assert_eq!(
            SchemaFingerprint::from_tables(&tables),
            SchemaFingerprint::from_tables(&reversed),
        );
    }

    /// Law: fingerprint(permute_cols(table)) == fingerprint(table)
    #[hegel::test]
    fn fingerprint_column_order_independence(tc: hegel::TestCase) {
        let tables = draw_tables(&tc);
        let reversed_cols: Vec<_> = tables
            .iter()
            .map(|(name, cols)| {
                let mut rc = cols.clone();
                rc.reverse();
                (name.clone(), rc)
            })
            .collect();
        assert_eq!(
            SchemaFingerprint::from_tables(&tables),
            SchemaFingerprint::from_tables(&reversed_cols),
        );
    }
}
