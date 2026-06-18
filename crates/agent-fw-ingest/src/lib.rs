//! `agent-fw-ingest` — Service layer for database ingestion pipelines.
//!
//! This crate composes over algebra traits (`TargetDatabase`, `KVStore`,
//! `SemanticEnricher`, `CatalogWriter`, `VectorStore`) to provide:
//!
//! - **introspection** — PostgreSQL schema discovery via `information_schema`
//! - **profiling** — Column-level statistics and semantic type inference
//! - **builder** — Pure catalog entry construction (no IO)
//! - **ingestion** — 7-stage orchestrator with cancellation + SSE progress
//!
//! ```text
//! ┌───────────────────────────────────────────────────────┐
//! │                  agent-fw-ingest                       │
//! │                                                       │
//! │  introspection ── information_schema queries          │
//! │  profiling ──── column stats + semantic inference     │
//! │  builder ──── pure: PhysicalTable → CatalogEntry      │
//! │  ingestion ── orchestrator composing all above        │
//! │                                                       │
//! │  Composes: TargetDatabase, KVStore, SemanticEnricher, │
//! │            CatalogWriter, VectorStore, EventSink      │
//! └───────────────────────────────────────────────────────┘
//! ```

pub mod builder;
pub mod etl;
pub mod ingestion;
pub mod introspection;
pub mod job_store;
pub mod knowledge_extraction;
pub mod knowledge_ingestion;
pub mod knowledge_store;
pub mod profiling;
pub mod vector_ingest;

pub use job_store::{
    append_import_event, append_profiling_event, data_job_key, data_jobs_index_key,
    delete_data_job, get_data_job, get_data_job_ids, get_import_events, get_import_status,
    get_profiling_events, get_profiling_status, import_events_key, import_status_key,
    ingestion_events_key, ingestion_status_key, list_data_jobs, put_import_status,
    put_profiling_status, register_data_job, update_data_job_phase,
    workspace_ingestion_persistence, DataJobKind, DataJobRecord, DataJobStore,
};
