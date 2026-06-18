//! Workspace management: types, store algebra, and stock interpreters.
//!
//! This crate provides the complete workspace layer:
//!
//! - **Domain types**: [`Workspace`], [`DatabaseConfig`], [`Thread`], [`Message`], [`DataSource`]
//! - **Store algebra**: 6 focused sub-traits composed into [`WorkspaceStore`]
//! - **Data source service**: [`DataSourceService`] for encrypted CRUD + connection tests
//! - **KV interpreter**: [`KVWorkspaceStore`] for development/fallback
//! - **PostgreSQL interpreter**: [`PostgresWorkspaceStore`] for canonical relational persistence
//! - **Dynamic source pool**: [`WorkspacePostgresSourcePool`] for workspace-backed
//!   PostgreSQL source acquisition
//! - **Utilities**: [`slugify`] for URL-safe name generation
//!
//! # Architecture
//!
//! The store algebra follows the **composition over monolith** pattern:
//! each consumer declares exactly which sub-trait it needs.
//!
//! ```text
//! ThreadStore ─────┐
//! MessageStore ────┤
//! TestCaseStore ───┤
//! EvalStore ───────┼──► WorkspaceStore (supertrait + blanket impl)
//! DataSourceStore ─┤
//! WorkspaceEntityStore ─┘
//! ```
//!
//! # Laws
//!
//! Each sub-trait has documented algebraic laws (roundtrip, delete-get,
//! upsert idempotence, list consistency) verified in `agent-fw-test`.

pub mod crud_service;
pub mod data_source;
pub mod data_source_service;
pub mod file;
pub mod file_store;
pub mod indexed_entity;
pub mod kv_keys;
pub mod kv_store;
pub mod lifecycle;
#[cfg(feature = "postgres")]
pub mod postgres_store;
pub mod store;
pub mod thread;
pub mod thread_authoring;
pub mod thread_store_ext;
pub mod thread_summary_store;
pub mod tool_env;
pub mod workspace;
#[cfg(feature = "postgres")]
pub mod workspace_source_pool;

// Re-export key types at crate root
pub use agent_fw_eval::{EvalThreadFork, TestCaseSet};
pub use crud_service::{with_predicate, CrudError, CrudService};
pub use data_source::{
    ConnectionTestResult, CreateDataSourceRequest, DataSource, DataSourceStatus, DatabaseType,
    UpdateDataSourceRequest,
};
pub use data_source_service::{
    decrypt_data_source_credentials, encrypt_data_source_credentials,
    resolve_data_source_credentials, DataSourceCredentials, DataSourceService,
    DataSourceServiceError,
};
pub use file::{StoredFile, ThreadFileEntry, ThreadFileIndex};
pub use file_store::{
    delete_thread_files, get_stored_file, list_thread_files, put_stored_file, register_thread_file,
    register_thread_file_at,
};
pub use indexed_entity::{EntityConfig, EntityIndex, IdIndex, IndexedEntity, IndexedEntityError};
pub use kv_store::{DefaultWorkspaceKvKeyPolicy, KVWorkspaceStore, WorkspaceKvKeyPolicy};
#[cfg(feature = "postgres")]
pub use postgres_store::PostgresWorkspaceStore;
pub use store::{
    DataSourceStore, EvalStore, MessageStore, TestCaseStore, ThreadStore, WorkspaceEntityStore,
    WorkspaceError, WorkspaceStore,
};
pub use thread::{Message, PersistedToolInteraction, Thread};
pub use thread_authoring::{
    extract_thread_tool_segment, extract_tool_calls_from_workspace_messages, fork_workspace_thread,
    list_thread_summaries, load_thread_authoring_snapshot, ForkedThread, ThreadAuthoringError,
    ThreadAuthoringSnapshot, ThreadForkError, ThreadSegmentError, ThreadSummary, ThreadSummaryList,
};
pub use thread_store_ext::{normalize_thread_source_id, EnsuredThread, ThreadStoreExt};
pub use thread_summary_store::{
    delete_thread_summaries, get_thread_cost_summary, get_thread_latency_summary,
    put_thread_cost_summary, put_thread_latency_summary, put_thread_summaries, ThreadSummaryStore,
};
pub use tool_env::WorkspaceToolEnvironmentExt;
pub use workspace::{
    explicit_thread_source_id_for_database, resolve_workspace_database_urls,
    resolve_workspace_sqlite_directory, slugify, workspace_target_database_id, DatabaseConfig,
    Workspace, WorkspaceDatabaseUrls, WorkspaceModelConfig,
};
#[cfg(feature = "postgres")]
pub use workspace_source_pool::{
    DatabaseCredentials, WorkspacePostgresSourcePool, WorkspacePostgresSourcePoolError,
};
