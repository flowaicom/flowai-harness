//! Capability traits with algebraic laws.
//!
//! Each trait in this crate specifies properties that implementations must satisfy,
//! tested via property-based tests in `agent-fw-test`.
//!
//! # Traits
//!
//! - [`KVStore`] — Key-value storage with TTL
//! - [`AgentMemoryStore`] — Stateful agent conversation memory
//! - [`EventSink`] — Push-based event emission (synchronous, non-blocking)
//! - [`SubAgentInvoker`] — Multi-agent delegation
//! - [`CancellationToken`] — Cooperative cancellation
//! - [`TargetDatabase`] — Async read-only SQL database access
//! - [`WritableDatabase`] — Async DDL + DML for ETL pipelines
//! - [`VectorStore`] — Async semantic similarity search
//! - [`EncryptionService`] — Async encryption/decryption
//! - [`PauseToken`] — Cooperative pause for flow control
//! - [`DataSourcePool`] — Connection configuration caching for data sources
//!
//! # Combinators
//!
//! - [`parallel`] — race, zip_par, zip_par_result, race_all, race_success, zip_all
//! - [`retry`] — RetryPolicy, RetryOutcome, with_retry, retry_when, retry_until, retry_pausable
//! - [`schedule`] — Schedule, Decision, pausable_sleep, repeat_on_schedule, retry_on_schedule
//! - [`timeout`] — with_timeout, TimeoutExt, TimeoutOutcome, timeout_or_cancel
//! - [`resource`] — bracket, bracket_simple, bracket_result, with_resource, with_finally
//! - [`error`] — Either, AgentError, ResultExt, ErrorAccumulator
//!
//! # Modules
//!
//! - [`nursery`] — Structured task concurrency with guaranteed containment
//! - [`scope`] — Structured concurrency with LIFO cleanup
//! - [`timing`] — Per-request timing context and async timing helpers

// ── Capability traits ───────────────────────────────────────────────────

/// Stateful agent conversation memory.
/// Tested in `agent-fw-test::agent_memory_laws`.
pub mod agent_memory;

/// Pending approval store algebra (pre-dispatch approval): `PendingApprovalStore` trait,
/// `ApprovalAwait` future, `ApprovalError`, `ExpireReason`. Pure-data types
/// live in `agent-fw-core::approval`; the closure-bearing rule/policy and
/// the `ApprovalLayer` consumer live in `agent-fw-agent::approval`.
/// Tested in `agent-fw-test::approval_laws`.
pub mod approval;

/// Cooperative cancellation token — law: cancel is idempotent.
/// Tested in `agent-fw-test::cancellation_laws`.
pub mod cancellation;

/// Connection configuration caching for heterogeneous data sources.
/// Tested in `agent-fw-test::data_source_pool_laws`.
pub mod data_source_pool;

/// Async encryption / decryption service.
pub mod encryption;

/// Either, AgentError, ErrorAccumulator — structured error handling.
pub mod error;

/// Push-based event emission (synchronous, non-blocking).
/// Laws: emit never blocks the caller; tee distributes.
/// Tested in `agent-fw-test::event_sink_laws`.
pub mod event_sink;

/// Key-value storage with TTL — laws: get-after-put, delete-after-put, TTL expiry.
/// Tested in `agent-fw-test::kv_laws`.
pub mod kv_store;

/// Cooperative pause for flow control (complement to CancellationToken).
pub mod pause;

/// Multi-agent delegation (invoke sub-agent, collect result).
/// Tested in `agent-fw-test::sub_agent_laws`.
pub mod sub_agent;

/// Test case metadata index (CRUD + listing).
/// Tested in `agent-fw-test::test_case_index_laws`.
pub mod test_case_index;

/// Job lifecycle registry — states: Pending → Running → {Completed, Failed, Cancelled}.
/// Tested in `agent-fw-test::job_registry_laws`.
pub mod job_registry;

/// Async read-only SQL database access.
/// Tested in `agent-fw-test::target_db_laws`.
pub mod target_db;

/// Async semantic similarity search (embeddings + nearest-neighbor).
/// Tested in `agent-fw-test::vector_store_laws`.
pub mod vector_store;

/// Async DDL + DML for ETL pipelines.
/// Tested in `agent-fw-test::writable_db_laws`.
pub mod writable_db;

/// Full event log algebra (append + read) — laws: append-read roundtrip, ordering.
/// Tested in `agent-fw-test::event_log_laws`.
pub mod event_log;

// ── Combinators ────────────────────────────────────────────────────────

/// Race, zip_par, race_all — parallel composition of async effects.
pub mod parallel;

/// Bracket, with_resource, with_finally — resource safety (acquire/use/release).
pub mod resource;

/// RetryPolicy with exponential backoff, jitter, and pause-awareness.
pub mod retry;

/// Schedule algebra for repeat / retry — laws: associativity, identity.
/// Tested in `agent-fw-test::schedule_laws`.
pub mod schedule;

/// with_timeout, timeout_or_cancel — deadline enforcement.
pub mod timeout;

// ── Structured concurrency ─────────────────────────────────────────────

/// Task nursery with guaranteed containment (all children finish before parent).
/// Tested in `agent-fw-test::nursery_laws`.
pub mod nursery;

/// Structured concurrency scope with LIFO cleanup ordering.
pub mod scope;

/// Per-request timing context and async timing helpers.
pub mod timing;

// ── Compositional building blocks ──────────────────────────────────────

/// Fallback source combinator (try primary, fall back to secondary).
/// Tested in `agent-fw-test::fallback_laws`.
pub mod fallback;

/// Pipeline context threading for multi-stage transformations.
/// Tested in `agent-fw-test::pipeline_ctx_laws`.
pub mod pipeline;

/// Validated applicative — accumulate all errors, don't short-circuit.
/// Tested in `agent-fw-test::validated_laws`.
pub mod validated;

/// Canonical null interpreters for testing: NullKVStore, NullEventSink, etc.
pub mod testing;

// Re-exports
pub use agent_memory::{AgentMemoryError, AgentMemoryStore};
pub use approval::{
    ApprovalAwait, ApprovalError, ExpireReason, InMemoryPendingApprovalStore, PendingApprovalStore,
};
pub use cancellation::CancellationToken;
pub use data_source_pool::{
    parse_sqlite_path, parse_sqlite_path_with_root, resolve_sqlite_url_with_root, DataSourceConfig,
    DataSourcePool, DataSourcePoolError, DatabaseType, PooledConnection, ResolvedSource,
};
pub use encryption::{EncryptedPayload, EncryptionError, EncryptionService};
pub use error::{AgentError, Either, ErrorAccumulator};
pub use event_log::{EventEntry, EventLog, EventLogError, EventLogExt};
pub use event_sink::{
    BroadcastEventChannel, EventSink, EventSinkExt, EventSource, TeeEventSink, ValidatingEventSink,
};
pub use fallback::{with_fallback, with_fallback_sync, FallbackError, FallbackSource};
pub use job_registry::{
    CancelError, JobKind, JobPhase, JobRegistryError, JobState, JobView, PauseState,
};
pub use kv_store::{
    prefixed_key, KVError, KVMetricsAccumulator, KVOperationMetrics, KVStore, KVStoreExt,
    PrefixedRecordStore, PrefixedRecordStoreError,
};
pub use pause::{ComposedPauseToken, PauseToken};
pub use pipeline::PipelineCtx;
pub use retry::{
    retry_pausable, retry_until, retry_when, retry_when_observed, retry_when_observed_hinted,
    retry_when_pausable, retry_with_observer, with_retry, RetryContext, RetryOutcome,
    RetryPausableOutcome, RetryPolicy,
};
pub use sub_agent::{SubAgentError, SubAgentInvoker, SubAgentRequest, SubAgentResult};
pub use target_db::{
    escape_identifier, escape_literal, validate_read_only, validate_read_only_for, DbError, DbRow,
    QueryParam, ReadOnlyQuery, TargetDatabase, TargetDatabaseExt, DEFAULT_TIMEOUT,
};
pub use test_case_index::{TestCaseIndex, TestCaseIndexError, TestCaseMeta};
pub use timeout::{
    timeout_cancellable, timeout_map, timeout_or, timeout_or_cancel, with_timeout, TimeoutConfig,
    TimeoutError, TimeoutExt, TimeoutOutcome,
};
pub use validated::{ensure, validate_all, Validated};
pub use vector_store::{
    EmbeddingError, EmbeddingItem, EmbeddingService, VectorHit, VectorStore, VectorStoreError,
};
pub use writable_db::{
    DdlKind, DdlStatement, DmlKind, DmlStatement, InsertBatch, TableName, WritableDatabase,
    WritableDatabaseExt, WriteDbError,
};
