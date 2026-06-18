//! Ingestion state machine — progress tracking for database ingestion.
//!
//! `IngestionStatus` is a state machine with compile-time valid transitions:
//! Queued → Discovering → Profiling → Enriching → Extracting → Indexing → Completed/Failed
//!
//! `IngestionSummary` is a commutative monoid under `combine`.

use std::ops::Not;

use serde::{Deserialize, Serialize};

use crate::enrichment::EnrichmentSource;

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Ingestion pipeline status (state machine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum IngestionStatus {
    Queued,
    #[serde(rename_all = "camelCase")]
    Discovering {
        tables_found: u32,
    },
    #[serde(rename_all = "camelCase")]
    Profiling {
        tables_found: u32,
        columns_profiled: u32,
        total_columns: u32,
    },
    #[serde(rename_all = "camelCase")]
    Enriching {
        tables_enriched: u32,
        total_tables: u32,
    },
    #[serde(rename_all = "camelCase")]
    Extracting {
        enums_extracted: u32,
    },
    #[serde(rename_all = "camelCase")]
    Indexing {
        items_indexed: u32,
    },
    #[serde(rename_all = "camelCase")]
    Completed {
        summary: IngestionSummary,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        error: String,
        partial: Option<IngestionSummary>,
    },
}

/// Error for invalid state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionTransitionError {
    pub from: &'static str,
    pub to: &'static str,
}

impl std::fmt::Display for IngestionTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid ingestion transition: {} → {}",
            self.from, self.to
        )
    }
}

impl std::error::Error for IngestionTransitionError {}

impl IngestionStatus {
    fn variant_name(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Discovering { .. } => "discovering",
            Self::Profiling { .. } => "profiling",
            Self::Enriching { .. } => "enriching",
            Self::Extracting { .. } => "extracting",
            Self::Indexing { .. } => "indexing",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
        }
    }

    /// Transition: Queued → Discovering.
    pub fn start_discovering(self, tables_found: u32) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Queued => Ok(Self::Discovering { tables_found }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "discovering",
            }),
        }
    }

    /// Transition: Discovering → Profiling.
    pub fn start_profiling(self, total_columns: u32) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Discovering { tables_found } => Ok(Self::Profiling {
                tables_found,
                columns_profiled: 0,
                total_columns,
            }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "profiling",
            }),
        }
    }

    /// Transition: Profiling → Enriching.
    pub fn start_enriching(self, total_tables: u32) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Profiling { .. } => Ok(Self::Enriching {
                tables_enriched: 0,
                total_tables,
            }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "enriching",
            }),
        }
    }

    /// Transition: Enriching → Extracting.
    pub fn start_extracting(self) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Enriching { .. } => Ok(Self::Extracting { enums_extracted: 0 }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "extracting",
            }),
        }
    }

    /// Transition: Extracting → Indexing.
    pub fn start_indexing(self) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Extracting { .. } => Ok(Self::Indexing { items_indexed: 0 }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "indexing",
            }),
        }
    }

    /// Transition: Indexing → Completed.
    pub fn complete(self, summary: IngestionSummary) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Indexing { .. } => Ok(Self::Completed { summary }),
            _ => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "completed",
            }),
        }
    }

    /// Transition: Any non-terminal → Failed.
    pub fn fail(self, error: String) -> Result<Self, IngestionTransitionError> {
        match self {
            Self::Completed { .. } | Self::Failed { .. } => Err(IngestionTransitionError {
                from: self.variant_name(),
                to: "failed",
            }),
            _ => Ok(Self::Failed {
                error,
                partial: None,
            }),
        }
    }
}

/// Summary of an ingestion run.
///
/// Forms a commutative monoid under `combine` with `ZERO` as identity.
/// `enrichment_degraded` is a join-semilattice under OR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionSummary {
    pub tables_discovered: u32,
    pub columns_profiled: u32,
    pub enums_extracted: u32,
    pub relationships_found: u32,
    pub catalog_items_indexed: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Not::not")]
    pub enrichment_degraded: bool,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub enrichment_cache_hits: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub enrichment_fallbacks: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub enrichment_fresh: u32,
}

impl IngestionSummary {
    /// Monoid identity.
    pub const ZERO: Self = Self {
        tables_discovered: 0,
        columns_profiled: 0,
        enums_extracted: 0,
        relationships_found: 0,
        catalog_items_indexed: 0,
        duration_ms: 0,
        enrichment_degraded: false,
        enrichment_cache_hits: 0,
        enrichment_fallbacks: 0,
        enrichment_fresh: 0,
    };

    /// Monoid operation (commutative, associative).
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            tables_discovered: self.tables_discovered + other.tables_discovered,
            columns_profiled: self.columns_profiled + other.columns_profiled,
            enums_extracted: self.enums_extracted + other.enums_extracted,
            relationships_found: self.relationships_found + other.relationships_found,
            catalog_items_indexed: self.catalog_items_indexed + other.catalog_items_indexed,
            duration_ms: self.duration_ms + other.duration_ms,
            enrichment_degraded: self.enrichment_degraded || other.enrichment_degraded,
            enrichment_cache_hits: self.enrichment_cache_hits + other.enrichment_cache_hits,
            enrichment_fallbacks: self.enrichment_fallbacks + other.enrichment_fallbacks,
            enrichment_fresh: self.enrichment_fresh + other.enrichment_fresh,
        }
    }
}

/// Events emitted during ingestion for SSE progress streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum IngestionEvent {
    #[serde(rename_all = "camelCase")]
    Started { job_id: String },
    #[serde(rename_all = "camelCase")]
    Progress { status: IngestionStatus },
    #[serde(rename_all = "camelCase")]
    TableProfiled {
        table_name: String,
        columns: u32,
        duration_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    TableEnriched {
        table_name: String,
        source: EnrichmentSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback_reason: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TableCompleted {
        table_name: String,
        summary: IngestionSummary,
    },
    #[serde(rename_all = "camelCase")]
    TableFailed { table_name: String, error: String },
    #[serde(rename_all = "camelCase")]
    Completed { summary: IngestionSummary },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_state_transitions() {
        let s = IngestionStatus::Queued;
        let s = s.start_discovering(10).unwrap();
        let s = s.start_profiling(50).unwrap();
        let s = s.start_enriching(10).unwrap();
        let s = s.start_extracting().unwrap();
        let s = s.start_indexing().unwrap();
        let s = s.complete(IngestionSummary::ZERO).unwrap();
        match s {
            IngestionStatus::Completed { .. } => {}
            _ => panic!("Expected Completed"),
        }
    }

    #[test]
    fn invalid_transitions_rejected() {
        let s = IngestionStatus::Queued;
        // Can't skip to Profiling
        assert!(s.clone().start_profiling(50).is_err());
        // Can't skip to Enriching
        assert!(s.start_enriching(10).is_err());
    }

    #[test]
    fn fail_from_any_non_terminal() {
        assert!(IngestionStatus::Queued.fail("err".into()).is_ok());
        assert!(IngestionStatus::Discovering { tables_found: 5 }
            .fail("err".into())
            .is_ok());
    }

    #[test]
    fn fail_from_terminal_rejected() {
        let completed = IngestionStatus::Completed {
            summary: IngestionSummary::ZERO,
        };
        assert!(completed.fail("err".into()).is_err());
    }

    #[test]
    fn summary_monoid_identity() {
        let s = IngestionSummary {
            tables_discovered: 5,
            columns_profiled: 20,
            enums_extracted: 3,
            relationships_found: 2,
            catalog_items_indexed: 30,
            duration_ms: 1000,
            enrichment_degraded: false,
            enrichment_cache_hits: 1,
            enrichment_fallbacks: 0,
            enrichment_fresh: 4,
        };
        let combined = s.combine(&IngestionSummary::ZERO);
        assert_eq!(combined.tables_discovered, 5);
        assert_eq!(combined.duration_ms, 1000);
    }

    #[test]
    fn summary_monoid_combine() {
        let a = IngestionSummary {
            tables_discovered: 3,
            columns_profiled: 10,
            enums_extracted: 1,
            relationships_found: 1,
            catalog_items_indexed: 15,
            duration_ms: 500,
            enrichment_degraded: false,
            enrichment_cache_hits: 1,
            enrichment_fallbacks: 0,
            enrichment_fresh: 2,
        };
        let b = IngestionSummary {
            tables_discovered: 2,
            columns_profiled: 8,
            enums_extracted: 2,
            relationships_found: 1,
            catalog_items_indexed: 12,
            duration_ms: 300,
            enrichment_degraded: true,
            enrichment_cache_hits: 0,
            enrichment_fallbacks: 1,
            enrichment_fresh: 1,
        };
        let c = a.combine(&b);
        assert_eq!(c.tables_discovered, 5);
        assert_eq!(c.columns_profiled, 18);
        assert_eq!(c.duration_ms, 800);
        assert!(c.enrichment_degraded); // OR semilattice
    }

    #[test]
    fn ingestion_status_serde() {
        let status = IngestionStatus::Profiling {
            tables_found: 10,
            columns_profiled: 25,
            total_columns: 50,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"status\":\"profiling\""));
        let parsed: IngestionStatus = serde_json::from_str(&json).unwrap();
        match parsed {
            IngestionStatus::Profiling {
                columns_profiled, ..
            } => assert_eq!(columns_profiled, 25),
            _ => panic!("Expected Profiling"),
        }
    }

    #[test]
    fn ingestion_event_serde() {
        let event = IngestionEvent::TableProfiled {
            table_name: "users".into(),
            columns: 12,
            duration_ms: 150,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tableProfiled\""));
    }
}
