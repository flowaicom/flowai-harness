//! ETL pipeline — Parquet upload, star-schema creation, wave-parallel dimension loading.

pub mod aggregation_parser;
pub mod csv_reader;
pub mod orchestrator;
pub mod parquet_reader;
pub mod schema;
pub mod wave;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::marker::PhantomData;
use thiserror::Error;

/// ETL error types.
#[derive(Debug, Error)]
pub enum EtlError {
    #[error("parsing error: {0}")]
    Parsing(String),
    #[error("schema error: {0}")]
    Schema(String),
    #[error("dimension load error: {0}")]
    DimensionLoad(String),
    #[error("foreign key missing: {0}")]
    ForeignKeyMissing(String),
    #[error("fact load error: {0}")]
    FactLoad(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("date parse error: {0}")]
    DateParse(String),
    #[error("cancelled")]
    Cancelled,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
}

/// ETL processing stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EtlStage {
    Uploading,
    Parsing,
    CreatingSchema,
    LoadingDimensions,
    LoadingFacts,
    Validating,
    Profiling,
    Completed,
    Failed,
}

/// SSE events emitted during ETL processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EtlEvent {
    #[serde(rename_all = "camelCase")]
    Started { job_id: String },
    #[serde(rename_all = "camelCase")]
    StageProgress {
        stage: EtlStage,
        message: String,
        progress_pct: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    DimensionLoaded {
        table_name: String,
        row_count: usize,
    },
    #[serde(rename_all = "camelCase")]
    FactBatchLoaded {
        batch_index: usize,
        rows_in_batch: usize,
        total_loaded: usize,
    },
    #[serde(rename_all = "camelCase")]
    SchemaCreated { tables: Vec<String> },
    #[serde(rename_all = "camelCase")]
    ValidationPassed { checks: Vec<ValidationCheck> },
    #[serde(rename_all = "camelCase")]
    ProfilingEvent {
        table_name: String,
        column_count: usize,
    },
    #[serde(rename_all = "camelCase")]
    Completed { summary: EtlSummary },
    #[serde(rename_all = "camelCase")]
    Error { message: String, stage: EtlStage },
}

/// Summary of ETL results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EtlSummary {
    pub job_id: String,
    pub duration_ms: u64,
    pub table_row_counts: Vec<TableRowCounts>,
    pub validation_checks: Vec<ValidationCheck>,
    pub product_count: usize,
    pub scenario_count: usize,
}

/// Row count for a single table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRowCounts {
    pub table_name: String,
    pub row_count: usize,
}

/// A single validation check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

// =============================================================================
// DimensionLookup<D> — phantom-typed FK safety for wave-parallel loading
// =============================================================================

/// Phantom-typed dimension lookup table.
///
/// Prevents mixing up dimension FK IDs during wave-parallel loading.
/// The type parameter `D` (a zero-sized marker) ensures that a
/// `DimensionLookup<Segment>` cannot be passed where a
/// `DimensionLookup<Channel>` is expected — caught at compile time.
///
/// # Law — Phantom safety
///
/// ```text
/// DimensionLookup<Segment> ≠ DimensionLookup<Channel>  (type-level)
/// ```
#[derive(Debug, Clone)]
pub struct DimensionLookup<D> {
    map: HashMap<String, i64>,
    _phantom: PhantomData<D>,
}

impl<D> DimensionLookup<D> {
    /// Create an empty dimension lookup.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Insert a mapping from dimension code to database ID.
    pub fn insert(&mut self, key: String, id: i64) {
        self.map.insert(key, id);
    }

    /// Look up a dimension database ID by code.
    pub fn get(&self, key: &str) -> Option<i64> {
        self.map.get(key).copied()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the lookup is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterate over all (code, id) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, i64)> + '_ {
        self.map.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Construct from an existing HashMap (e.g., from `insert_batch_returning`).
    pub fn from_map(map: HashMap<String, i64>) -> Self {
        Self {
            map,
            _phantom: PhantomData,
        }
    }

    /// Iterate over all dimension codes (keys).
    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.map.keys().map(|k| k.as_str())
    }

    /// Consume into the inner HashMap (for FFI or legacy interop).
    pub fn into_inner(self) -> HashMap<String, i64> {
        self.map
    }
}

impl<D> Default for DimensionLookup<D> {
    fn default() -> Self {
        Self::new()
    }
}

// Dimension marker types (zero-sized, used only for phantom parameter)
/// Marker type for segment dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct Segment;
/// Marker type for channel dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct Channel;
/// Marker type for time period dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct TimePeriod;
/// Marker type for brand dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct Brand;
/// Marker type for sub-segment dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct SubSegment;
/// Marker type for sub-brand dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct SubBrand;
/// Marker type for product dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct Product;
/// Marker type for coordinate dimension lookups.
#[derive(Debug, Clone, Copy)]
pub struct Coordinate;

/// Maps for lookup during fact loading (dimension ID resolution).
///
/// Each field uses `DimensionLookup<D>` with phantom typing to prevent
/// mixing up dimension FKs at compile time.
#[derive(Debug, Clone, Default)]
pub struct LookupMaps {
    pub segments: DimensionLookup<Segment>,
    pub channels: DimensionLookup<Channel>,
    pub time_periods: DimensionLookup<TimePeriod>,
    pub brands: DimensionLookup<Brand>,
    pub subsegments: DimensionLookup<SubSegment>,
    pub sub_brands: DimensionLookup<SubBrand>,
    pub products: DimensionLookup<Product>,
    pub coordinates: DimensionLookup<Coordinate>,
}

/// Prepared fact row for batch insertion.
#[derive(Debug, Clone)]
pub struct FactInsert {
    pub scenario_name: String,
    pub product_id: i64,
    pub coordinate_id: i64,
    pub value: f64,
    pub period_id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_lookup_insert_and_get() {
        let mut lookup: DimensionLookup<Segment> = DimensionLookup::new();
        lookup.insert("seg_1".into(), 42);
        assert_eq!(lookup.get("seg_1"), Some(42));
        assert_eq!(lookup.get("missing"), None);
        assert_eq!(lookup.len(), 1);
    }

    #[test]
    fn dimension_lookup_from_map() {
        let mut map = HashMap::new();
        map.insert("a".into(), 1);
        map.insert("b".into(), 2);
        let lookup: DimensionLookup<Channel> = DimensionLookup::from_map(map);
        assert_eq!(lookup.get("a"), Some(1));
        assert_eq!(lookup.get("b"), Some(2));
        assert_eq!(lookup.len(), 2);
    }

    #[test]
    fn dimension_lookup_keys_and_iter() {
        let mut lookup: DimensionLookup<Brand> = DimensionLookup::new();
        lookup.insert("x".into(), 10);
        lookup.insert("y".into(), 20);
        let keys: Vec<&str> = lookup.keys().collect();
        assert_eq!(keys.len(), 2);
        let pairs: Vec<(&str, i64)> = lookup.iter().collect();
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn dimension_lookup_default_is_empty() {
        let lookup: DimensionLookup<TimePeriod> = DimensionLookup::default();
        assert!(lookup.is_empty());
        assert_eq!(lookup.len(), 0);
    }

    #[test]
    fn lookup_maps_default_all_empty() {
        let maps = LookupMaps::default();
        assert!(maps.segments.is_empty());
        assert!(maps.channels.is_empty());
        assert!(maps.products.is_empty());
        assert!(maps.coordinates.is_empty());
    }

    /// Phantom typing prevents mixups at compile time.
    /// This is a compile-time-only test — the function signatures enforce the constraint.
    fn _type_safety_check(_segs: &DimensionLookup<Segment>, _chans: &DimensionLookup<Channel>) {
        // The type system prevents passing _segs where _chans is expected.
        // This function exists only to demonstrate the constraint.
    }
}
