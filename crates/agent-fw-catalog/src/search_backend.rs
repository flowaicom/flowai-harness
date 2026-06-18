//! Storage-agnostic catalog search backend contract.
//!
//! `DataCatalog` remains the authoritative storage and hydration API. A
//! `CatalogSearchBackend` returns candidate entry ids, diagnostics, facets, and
//! cursors that callers hydrate through `DataCatalog`.
//!
//! # Laws
//!
//! Implementations must satisfy these retrieval laws:
//!
//! - Scope isolation: `search(scope_a, request)` must not return entry ids
//!   indexed only for `scope_b` when `scope_a != scope_b`.
//! - Limit bound: successful search responses contain at most
//!   `max(request.limit, 1)` hits.
//! - Cursor continuity: a `next_cursor` returned for a logical request continues
//!   the same logical request when supplied back with the same scope, query,
//!   kind set, and filters.
//! - Cursor opacity: cursors are backend-owned tokens. Invalid or mismatched
//!   cursors must fail closed instead of silently changing the logical result
//!   set.
//! - Per-candidate resume: when present, `hit.resume_cursor` resumes the same
//!   logical request strictly after that hit (no skip, no re-return). Callers
//!   that over-fetch a window but emit only a page of survivors carry forward the
//!   resume cursor of the last candidate they consumed.
//! - Candidate consistency: `candidate_count` describes the candidate set used
//!   for pagination before page limiting.
//! - Facet honesty: facets describe the same candidate set, or the backend must
//!   include a warning when counts are approximate.
//! - Health totality: `health(scope)` reports `Ready`, `Stale`, or
//!   `Unavailable` without requiring callers to infer state from a search error.
//! - Stale monotonicity: once a backend reports `Stale`, ordinary incremental
//!   writes must not clear that state unless the backend can prove the index has
//!   been rebuilt or otherwise recovered.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{CatalogError, CatalogScope, SemanticEntityKind};

/// Opaque pagination cursor for a catalog search backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CatalogSearchCursor(String);

impl CatalogSearchCursor {
    pub fn new(cursor: impl Into<String>) -> Self {
        Self(cursor.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Backend-level filters after tool-layer reference canonicalization.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSearchFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_query_surface: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_cardinality_enum: Option<bool>,
}

/// A backend search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<SemanticEntityKind>,
    #[serde(default)]
    pub filters: CatalogSearchFilters,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CatalogSearchCursor>,
}

/// One facet value and its response-local count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogFacetValue {
    pub value: String,
    pub count: usize,
}

/// Facets returned with a catalog search response.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSearchFacets {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<CatalogFacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<CatalogFacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<CatalogFacetValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<CatalogFacetValue>,
}

/// Search hit reference returned by the search backend before hydration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSearchHitRef {
    pub entry_id: String,
    pub score: f64,
    pub rank: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Opaque, backend-owned resume token for this hit.
    ///
    /// When supplied back as the next request's `cursor` for the **same logical
    /// request** (same scope, query, kind set, and filters), retrieval resumes at
    /// the candidate **immediately after** this hit — no skip, no re-return. This
    /// is the per-candidate resume token a paginating caller selects from for the
    /// last candidate it consumed when it over-fetches a window but only emits a
    /// page of survivors. Non-paginating or mock backends may leave it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cursor: Option<CatalogSearchCursor>,
}

/// Search response returned by a backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSearchResults {
    pub hits: Vec<CatalogSearchHitRef>,
    #[serde(default)]
    pub facets: CatalogSearchFacets,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<CatalogSearchCursor>,
    pub candidate_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Health state for a catalog search backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CatalogSearchHealth {
    Ready {
        indexed_entries: usize,
        projection_version: u32,
    },
    Stale {
        indexed_entries: usize,
        projection_version: u32,
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

impl CatalogSearchHealth {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Lexical catalog candidate retrieval.
#[async_trait]
pub trait CatalogSearchBackend: Send + Sync {
    async fn search(
        &self,
        scope: &CatalogScope,
        request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogError>;

    async fn health(&self, scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogError>;
}
