//! CachedEnricher — KV-cached semantic enrichment combinator.
//!
//! Wraps any [`SemanticEnricher`] and caches results in a [`KVStore`].
//! On cache hit, returns immediately without calling the inner enricher.
//! On cache miss, delegates to the inner enricher and stores the result.
//!
//! # Laws (extending SemanticEnricher L1-L4)
//!
//! - **L5 (Cache-hit)**: If KV has entry for table, return Cached (no LLM call)
//! - **L6 (Cache-miss-then-store)**: On miss, call inner, store result, return Fresh
//! - **L7 (TTL delegation)**: TTL enforcement is delegated to the KVStore.
//!   CachedEnricher passes `Some(self.ttl)` to `put_json`; the store is
//!   responsible for honoring it. `DashMapKVStore` enforces via lazy expiry.
//!   Stores that ignore TTL will cache indefinitely.
//! - **L8 (Source fidelity)**: Returns `EnrichmentSource::Cached` on successful
//!   cache hits; cached fallback entries remain `Fallback` so degradation is
//!   visible to callers.

use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::KVStore;
use agent_fw_catalog::{
    CachedEnrichmentEntry, EnrichmentError, EnrichmentResult, EnrichmentSource,
    KnowledgeExtractionRequest, KnowledgeItem, SemanticEnricher, TableEnrichmentRequest,
};

/// Default cache TTL: 24 hours.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Key prefix for cached enrichment entries.
const CACHE_KEY_PREFIX: &str = "enrichment:cache";

/// A [`SemanticEnricher`] that caches results in KV, falling back to inner on miss.
pub struct CachedEnricher<E> {
    inner: E,
    kv: Arc<dyn KVStore>,
    tenant: String,
    ttl: Duration,
}

impl<E: SemanticEnricher> CachedEnricher<E> {
    /// Create a cached enricher with the default 24-hour TTL.
    pub fn new(inner: E, kv: Arc<dyn KVStore>, tenant: impl Into<String>) -> Self {
        Self {
            inner,
            kv,
            tenant: tenant.into(),
            ttl: DEFAULT_CACHE_TTL,
        }
    }

    /// Create a cached enricher with a custom TTL.
    pub fn with_ttl(
        inner: E,
        kv: Arc<dyn KVStore>,
        tenant: impl Into<String>,
        ttl: Duration,
    ) -> Self {
        Self {
            inner,
            kv,
            tenant: tenant.into(),
            ttl,
        }
    }

    /// Generate a deterministic cache key for a table enrichment request.
    ///
    /// Includes the table identity, column set, row count, database context,
    /// and FK-edge context so semantically different enrichment inputs do not
    /// collide in cache.
    fn cache_key(request: &TableEnrichmentRequest) -> String {
        let mut hasher = Sha256::new();
        hasher.update(request.table.schema_name.as_bytes());
        hasher.update(b"|");
        hasher.update(request.table.table_name.as_bytes());
        hasher.update(b"|");
        for column in &request.table.columns {
            hasher.update(column.column_name.as_bytes());
            hasher.update(b",");
        }
        hasher.update(b"|");
        hasher.update(request.table.row_count.to_string().as_bytes());
        hasher.update(b"|ctx:");
        if let Some(ref context) = request.database_context {
            hasher.update(context.as_bytes());
        }
        hasher.update(b"|fk:");
        let mut fk_edges: Vec<(&str, &str, &str, &str)> = request
            .fk_edges
            .iter()
            .map(|edge| {
                (
                    edge.source_table.as_str(),
                    edge.source_column.as_str(),
                    edge.target_table.as_str(),
                    edge.target_column.as_str(),
                )
            })
            .collect();
        fk_edges.sort_unstable();
        for (source_table, source_column, target_table, target_column) in fk_edges {
            hasher.update(source_table.as_bytes());
            hasher.update(b".");
            hasher.update(source_column.as_bytes());
            hasher.update(b"->");
            hasher.update(target_table.as_bytes());
            hasher.update(b".");
            hasher.update(target_column.as_bytes());
            hasher.update(b";");
        }
        format!("{}:{}", CACHE_KEY_PREFIX, hex::encode(hasher.finalize()))
    }
}

#[async_trait::async_trait]
impl<E: SemanticEnricher> SemanticEnricher for CachedEnricher<E> {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        let key = Self::cache_key(&request);

        // Check cache (Law L5)
        if let Ok(Some(cached_json)) = self.kv.get_json(&self.tenant, &key).await {
            if let Ok(entry) = serde_json::from_value::<CachedEnrichmentEntry>(cached_json) {
                let mut result = if entry.source == EnrichmentSource::Fallback {
                    EnrichmentResult::fallback(entry.profile)
                } else {
                    EnrichmentResult::cached(entry.profile)
                };
                result.model_id = entry.model_id;
                result.fallback_reason = if result.source == EnrichmentSource::Fallback {
                    entry
                        .fallback_reason
                        .or_else(|| Some("cached fallback enrichment result".to_string()))
                } else {
                    None
                };
                return Ok(result);
            }
        }

        // Cache miss — delegate to inner enricher (Law L6)
        let result = self.inner.enrich_table(request).await?;
        if result.source == EnrichmentSource::Fallback {
            return Ok(result);
        }

        // Store in cache
        let entry = CachedEnrichmentEntry {
            profile: result.profile.clone(),
            source: result.source,
            model_id: result.model_id.clone(),
            fallback_reason: result.fallback_reason.clone(),
        };
        match serde_json::to_value(&entry) {
            Ok(value) => {
                if let Err(e) = self
                    .kv
                    .put_json(&self.tenant, &key, value, Some(self.ttl))
                    .await
                {
                    tracing::warn!(error = %e, key = %key, "failed to cache enrichment result");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, key = %key, "failed to serialize enrichment entry for cache");
            }
        }

        Ok(result)
    }

    async fn extract_knowledge(
        &self,
        request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        // Knowledge extraction is not cached — it depends on document content
        // which is not content-addressable in the same way as table schema.
        self.inner.extract_knowledge(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DashMapKVStore, MockEnricher};
    use agent_fw_catalog::{
        ColumnDescriptions, ColumnInfo, EnrichmentSource, PhysicalTable, SemanticTableProfile,
        TableEnrichmentRequest,
    };

    fn make_request() -> TableEnrichmentRequest {
        TableEnrichmentRequest {
            table: PhysicalTable {
                schema_name: "public".into(),
                table_name: "users".into(),
                columns: vec![ColumnInfo {
                    column_name: "id".into(),
                    data_type: "integer".into(),
                    is_nullable: false,
                    is_primary_key: true,
                    column_default: None,
                    ordinal_position: 1,
                    foreign_key: None,
                }],
                constraints: vec![],
                indexes: vec![],
                row_count: 100,
            },
            sample_rows: vec![],
            profile: agent_fw_catalog::TableProfile {
                table_name: "users".into(),
                columns: vec![],
            },
            database_context: None,
            fk_edges: vec![],
        }
    }

    #[test]
    fn cache_key_changes_with_fk_context() {
        let mut left = make_request();
        left.database_context = Some("users join accounts".into());
        left.fk_edges.push(agent_fw_catalog::ForeignKeyEdge {
            source_table: "users".into(),
            source_column: "account_id".into(),
            target_table: "accounts".into(),
            target_column: "id".into(),
        });

        let right = make_request();

        assert_ne!(
            CachedEnricher::<MockEnricher>::cache_key(&left),
            CachedEnricher::<MockEnricher>::cache_key(&right)
        );
    }

    #[test]
    fn cache_key_is_stable_across_fk_order() {
        let mut left = make_request();
        left.fk_edges = vec![
            agent_fw_catalog::ForeignKeyEdge {
                source_table: "users".into(),
                source_column: "account_id".into(),
                target_table: "accounts".into(),
                target_column: "id".into(),
            },
            agent_fw_catalog::ForeignKeyEdge {
                source_table: "users".into(),
                source_column: "manager_id".into(),
                target_table: "users".into(),
                target_column: "id".into(),
            },
        ];

        let mut right = make_request();
        right.fk_edges = left.fk_edges.iter().cloned().rev().collect();

        assert_eq!(
            CachedEnricher::<MockEnricher>::cache_key(&left),
            CachedEnricher::<MockEnricher>::cache_key(&right)
        );
    }

    // =========================================================================
    // L5: Cache hit returns Cached
    // =========================================================================

    #[tokio::test]
    async fn l5_cache_hit_returns_cached() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let inner = MockEnricher::new();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        // First call: cache miss → Fresh
        let req = make_request();
        let result1 = enricher.enrich_table(req.clone()).await.unwrap();
        assert_eq!(result1.source, EnrichmentSource::Fresh);

        // Second call: cache hit → Cached
        let result2 = enricher.enrich_table(req).await.unwrap();
        assert_eq!(result2.source, EnrichmentSource::Cached);
    }

    // =========================================================================
    // L6: Cache miss delegates to inner
    // =========================================================================

    #[tokio::test]
    async fn l6_cache_miss_delegates_and_stores() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let inner = MockEnricher::new();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        let req = make_request();
        let result = enricher.enrich_table(req).await.unwrap();

        // Should have gotten a result from the inner enricher
        assert!(!result.profile.description.is_empty());

        // Should have stored it in the cache
        let cache_key = CachedEnricher::<MockEnricher>::cache_key(&make_request());
        let cached = kv.get_json("test-tenant", &cache_key).await.unwrap();
        assert!(cached.is_some());
    }

    // =========================================================================
    // L8: Source fidelity
    // =========================================================================

    #[tokio::test]
    async fn l8_source_fidelity() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());

        // Pre-populate cache
        let profile = SemanticTableProfile {
            description: "Pre-cached".into(),
            short_description: "cached".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };
        let entry = CachedEnrichmentEntry {
            profile: profile.clone(),
            source: EnrichmentSource::Fresh,
            model_id: Some("cached-model".into()),
            fallback_reason: None,
        };
        let val = serde_json::to_value(&entry).unwrap();
        let cache_key = CachedEnricher::<MockEnricher>::cache_key(&make_request());
        kv.put_json("test-tenant", &cache_key, val, None)
            .await
            .unwrap();

        let inner = MockEnricher::new();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        let result = enricher.enrich_table(make_request()).await.unwrap();
        assert_eq!(result.source, EnrichmentSource::Cached);
        assert_eq!(result.profile.description, "Pre-cached");
        assert_eq!(result.model_id.as_deref(), Some("cached-model"));
        assert_eq!(result.fallback_reason, None);
    }

    #[tokio::test]
    async fn l8_cached_fallback_remains_visible() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());

        let profile = SemanticTableProfile {
            description: "Pre-cached fallback".into(),
            short_description: "fallback".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };
        let entry = CachedEnrichmentEntry {
            profile,
            source: EnrichmentSource::Fallback,
            model_id: None,
            fallback_reason: Some("cached fallback reason".into()),
        };
        let val = serde_json::to_value(&entry).unwrap();
        let cache_key = CachedEnricher::<MockEnricher>::cache_key(&make_request());
        kv.put_json("test-tenant", &cache_key, val, None)
            .await
            .unwrap();

        let inner = MockEnricher::new();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        let result = enricher.enrich_table(make_request()).await.unwrap();
        assert_eq!(result.source, EnrichmentSource::Fallback);
        assert_eq!(
            result.fallback_reason.as_deref(),
            Some("cached fallback reason")
        );
    }

    #[tokio::test]
    async fn l8_cache_miss_does_not_store_fallback() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let inner = MockEnricher::fallback();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        let result = enricher.enrich_table(make_request()).await.unwrap();
        assert_eq!(result.source, EnrichmentSource::Fallback);

        let cache_key = CachedEnricher::<MockEnricher>::cache_key(&make_request());
        let cached = kv.get_json("test-tenant", &cache_key).await.unwrap();
        assert!(cached.is_none());
    }

    // =========================================================================
    // Knowledge extraction passes through (no caching)
    // =========================================================================

    #[tokio::test]
    async fn knowledge_extraction_passes_through() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let inner = MockEnricher::new();
        let enricher = CachedEnricher::new(inner, Arc::clone(&kv), "test-tenant");

        let req = KnowledgeExtractionRequest {
            document_content: "test content".into(),
            document_name: "test.md".into(),
            database_context: None,
            available_tables: vec![],
            available_columns: vec![],
        };

        let items = enricher.extract_knowledge(req).await.unwrap();
        // MockEnricher returns a single knowledge item derived from the document
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Knowledge from test.md");
    }
}
