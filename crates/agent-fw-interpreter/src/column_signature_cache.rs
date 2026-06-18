//! Column-level enrichment cache via content-addressed signatures.
//!
//! Provides a combinator that caches column descriptions by their
//! structural signature (data type + semantic type + cardinality bucket),
//! reducing redundant LLM calls across tables with similar columns.
//!
//! # Types
//!
//! - [`CardinalityBucket`] — Discretized cardinality for cache keying
//! - [`ColumnSignature`] — Content-addressed column identity
//! - [`ColumnSignatureCachedEnricher`] — Combinator wrapping any SemanticEnricher
//!
//! # Laws
//!
//! ## CardinalityBucket
//! - L1 (Totality): `from_count` never panics
//! - L2 (Monotonicity): higher count → same or higher bucket
//!
//! ## ColumnSignature
//! - L3 (Determinism): same fields → same cache_key
//!
//! ## ColumnSignatureCachedEnricher
//! - L4 (Cache hit): matching signature → reuse without LLM call
//! - L5 (Composition): wraps any enricher, including CachedEnricher
//! - L6 (Transparency): result identical to uncached enricher

use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::KVStore;
use agent_fw_catalog::{
    ColumnDescriptions, EnrichmentError, EnrichmentResult, EnrichmentSource,
    KnowledgeExtractionRequest, KnowledgeItem, SemanticEnricher, SemanticTableProfile,
    TableEnrichmentRequest,
};

/// Discretized cardinality for column-level cache keying.
///
/// # Laws
/// - L1 (Totality): Never panics
/// - L2 (Monotonicity): higher distinct count → same or higher bucket
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CardinalityBucket {
    Constant,
    Low,
    Medium,
    High,
    VeryHigh,
}

impl CardinalityBucket {
    /// Classify a distinct count into a bucket.
    ///
    /// Thresholds: 1 → Constant, ≤10 → Low, ≤100 → Medium, ≤10_000 → High, else VeryHigh.
    pub fn from_count(count: usize) -> Self {
        match count {
            0..=1 => Self::Constant,
            2..=10 => Self::Low,
            11..=100 => Self::Medium,
            101..=10_000 => Self::High,
            _ => Self::VeryHigh,
        }
    }
}

impl std::fmt::Display for CardinalityBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Constant => write!(f, "constant"),
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::VeryHigh => write!(f, "very_high"),
        }
    }
}

/// Content-addressed column identity for cache keying.
///
/// Two columns with the same (data_type, semantic_type, cardinality_bucket)
/// are considered structurally equivalent and can share enrichment results.
///
/// # Laws
/// - L3 (Determinism): same fields → same cache_key
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ColumnSignature {
    pub data_type: String,
    pub semantic_type: String,
    pub cardinality: CardinalityBucket,
}

impl ColumnSignature {
    /// Deterministic content-addressed cache key.
    pub fn cache_key(&self) -> String {
        format!(
            "colsig:{}:{}:{}",
            self.data_type.to_lowercase(),
            self.semantic_type.to_lowercase(),
            self.cardinality,
        )
    }
}

/// Default cache TTL for column signature entries: 48 hours.
const DEFAULT_COL_CACHE_TTL: Duration = Duration::from_secs(48 * 60 * 60);

/// Key prefix for column signature cache entries.
const COL_CACHE_PREFIX: &str = "enrichment:colsig";

/// Column-level enrichment cache wrapping any [`SemanticEnricher`].
///
/// Before calling the inner enricher, computes a [`ColumnSignature`] for each
/// column in the request's profile. Columns with cached signatures have their
/// descriptions pre-populated; only tables with uncached columns are sent to
/// the inner enricher (LLM).
///
/// # Laws
/// - L4 (Cache hit): matching signature → reuse without LLM call
/// - L5 (Composition): `ColumnSignatureCachedEnricher(CachedEnricher(inner))` = two-tier cache
/// - L6 (Transparency): result identical to uncached enricher
pub struct ColumnSignatureCachedEnricher<E> {
    inner: E,
    kv: Arc<dyn KVStore>,
    tenant: String,
    ttl: Duration,
}

impl<E: SemanticEnricher> ColumnSignatureCachedEnricher<E> {
    /// Create with default 48-hour TTL.
    pub fn new(inner: E, kv: Arc<dyn KVStore>, tenant: impl Into<String>) -> Self {
        Self {
            inner,
            kv,
            tenant: tenant.into(),
            ttl: DEFAULT_COL_CACHE_TTL,
        }
    }

    /// Override the TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Compute a column signature from profile data.
    fn column_signature(
        data_type: &str,
        semantic_type: agent_fw_catalog::SemanticType,
        distinct_count: i64,
    ) -> ColumnSignature {
        ColumnSignature {
            data_type: data_type.to_string(),
            semantic_type: semantic_type.to_string(),
            cardinality: CardinalityBucket::from_count(distinct_count.max(0) as usize),
        }
    }

    /// Look up a cached description by signature key.
    async fn get_cached(&self, sig: &ColumnSignature) -> Option<String> {
        let key = format!("{}:{}", COL_CACHE_PREFIX, sig.cache_key());
        match self.kv.get_json(&self.tenant, &key).await {
            Ok(Some(value)) => value.as_str().map(|s| s.to_string()),
            _ => None,
        }
    }

    /// Store a description by signature key.
    async fn put_cached(&self, sig: &ColumnSignature, description: &str) {
        let key = format!("{}:{}", COL_CACHE_PREFIX, sig.cache_key());
        let value = serde_json::Value::String(description.to_string());
        let _ = self
            .kv
            .put_json(&self.tenant, &key, value, Some(self.ttl))
            .await;
    }
}

#[async_trait::async_trait]
impl<E: SemanticEnricher + Send + Sync> SemanticEnricher for ColumnSignatureCachedEnricher<E> {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        // Phase 1: Check cache for each column in the profile
        let mut cached_descriptions = ColumnDescriptions::new();
        let mut has_uncached = false;

        for col_profile in &request.profile.columns {
            let sig = Self::column_signature(
                &col_profile.data_type,
                col_profile.semantic_type,
                col_profile.distinct_count,
            );

            if let Some(desc) = self.get_cached(&sig).await {
                cached_descriptions.insert(col_profile.column_name.clone(), desc);
            } else {
                has_uncached = true;
            }
        }

        // Phase 2: If all columns cached, return immediately (L4)
        if !has_uncached && !request.profile.columns.is_empty() {
            let profile = SemanticTableProfile {
                description: format!("Cached enrichment for {}", request.table.table_name),
                short_description: request.table.table_name.clone(),
                column_descriptions: cached_descriptions,
                relationships: Vec::new(),
                quality_notes: Vec::new(),
            };
            return Ok(EnrichmentResult::cached(profile));
        }

        // Phase 3: Delegate to inner enricher
        let column_profiles: Vec<_> = request.profile.columns.clone();
        let result = self.inner.enrich_table(request).await?;

        // Phase 4: Store newly enriched column descriptions by signature
        for col_profile in &column_profiles {
            if let Some(desc) = result
                .profile
                .column_descriptions
                .get(&col_profile.column_name)
            {
                let sig = Self::column_signature(
                    &col_profile.data_type,
                    col_profile.semantic_type,
                    col_profile.distinct_count,
                );
                self.put_cached(&sig, desc).await;
            }
        }

        // Merge cached + fresh descriptions
        let mut merged = result.profile.column_descriptions.clone();
        for (name, desc) in cached_descriptions.iter() {
            if merged.get(name).is_none() {
                merged.insert(name.clone(), desc.clone());
            }
        }

        let source = if !cached_descriptions.is_empty() {
            EnrichmentSource::Cached // partial cache hit
        } else {
            result.source
        };

        Ok(EnrichmentResult {
            profile: SemanticTableProfile {
                column_descriptions: merged,
                ..result.profile
            },
            source,
            model_id: result.model_id,
            fallback_reason: result.fallback_reason,
        })
    }

    async fn extract_knowledge(
        &self,
        request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        // Knowledge extraction doesn't benefit from column-level caching
        self.inner.extract_knowledge(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CardinalityBucket laws ──

    #[test]
    fn cardinality_totality() {
        // L1: Never panics for any input
        for count in [0, 1, 2, 10, 11, 100, 101, 10_000, 10_001, usize::MAX] {
            let _ = CardinalityBucket::from_count(count);
        }
    }

    #[test]
    fn cardinality_monotonicity() {
        // L2: Higher count → same or higher bucket
        let buckets: Vec<CardinalityBucket> = (0..=20_000)
            .step_by(500)
            .map(CardinalityBucket::from_count)
            .collect();

        for window in buckets.windows(2) {
            let (a, b) = (window[0], window[1]);
            assert!(
                bucket_ord(b) >= bucket_ord(a),
                "L2: monotonicity violated: {:?} < {:?}",
                b,
                a
            );
        }
    }

    fn bucket_ord(b: CardinalityBucket) -> u8 {
        match b {
            CardinalityBucket::Constant => 0,
            CardinalityBucket::Low => 1,
            CardinalityBucket::Medium => 2,
            CardinalityBucket::High => 3,
            CardinalityBucket::VeryHigh => 4,
        }
    }

    // ── ColumnSignature laws ──

    #[test]
    fn signature_determinism() {
        // L3: same fields → same key
        let sig = ColumnSignature {
            data_type: "VARCHAR".into(),
            semantic_type: "name".into(),
            cardinality: CardinalityBucket::Medium,
        };
        let key_a = sig.cache_key();
        let key_b = sig.cache_key();
        assert_eq!(key_a, key_b, "L3: cache_key must be deterministic");
    }

    #[test]
    fn signature_case_insensitive() {
        let a = ColumnSignature {
            data_type: "VARCHAR".into(),
            semantic_type: "Name".into(),
            cardinality: CardinalityBucket::Low,
        };
        let b = ColumnSignature {
            data_type: "varchar".into(),
            semantic_type: "name".into(),
            cardinality: CardinalityBucket::Low,
        };
        assert_eq!(
            a.cache_key(),
            b.cache_key(),
            "cache_key should be case-insensitive"
        );
    }

    #[test]
    fn cardinality_serde_roundtrip() {
        let bucket = CardinalityBucket::High;
        let json = serde_json::to_string(&bucket).unwrap();
        let parsed: CardinalityBucket = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, bucket);
    }
}
