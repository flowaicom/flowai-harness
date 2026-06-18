//! Stock implementations of algebra traits.
//!
//! # Production Implementations
//!
//! - [`DashMapKVStore`] — In-memory KV with TTL (default)
//! - [`ChannelEventSink`] — Async channel-backed EventSink (default)
//! - [`SqlxTargetDatabase`] — PostgreSQL via sqlx (feature `postgres`)
//! - [`PgVectorStore`] — PostgreSQL + pgvector semantic search (feature `postgres`)
//! - [`PostgresKVStore`] — PostgreSQL-backed KV with TTL (feature `postgres`)
//! - [`PostgresCatalog`] / [`ScopedPostgresCatalog`] — PostgreSQL catalog handle and scoped reader/writer (feature `postgres`)
//! - [`RedisKVStore`] — Redis-backed KV with native TTL (feature `redis`)
//! - [`AesEncryptionService`] — AES-256-GCM encryption (feature `encryption`)
//! - [`NoOpEncryptionService`] — Plaintext development/test encryption
//! - [`LanceDbVectorStore`] — Embedded vector search via LanceDB (feature `lancedb`)
//! - [`NetworkSubAgentInvoker`] — HTTP-based multi-agent invocation (feature `http-clients`)
//! - [`NeonProvisioner`] — Neon Management API DatabaseProvisioner (feature `http-clients`)
//!
//! # Combinators
//!
//! - [`RetryKVStore`] — Algebra-level retry wrapper for any KVStore
//! - [`RetryTargetDatabase`] — Algebra-level retry wrapper for any TargetDatabase
//! - [`RetryWritableDatabase`] — Algebra-level retry wrapper for any WritableDatabase
//! - [`RetryCatalog`] — Algebra-level retry wrapper for any DataCatalog + CatalogWriter
//! - [`RetryVectorStore`] — Algebra-level retry wrapper for any VectorStore
//! - [`DurableEventSink`] — Bridges EventSink + EventLog for durable streaming
//! - [`InstrumentedKVStore`] — Decorates any KV store with canonical timing events
//!
//! # Error Sentinels
//!
//! Used when a subsystem is not configured — provides clear error messages instead of panics.
//!
//! - [`ErrorTargetDatabase`] — TargetDatabase that always errors
//! - [`ErrorSubAgentInvoker`] — SubAgentInvoker that always errors
//! - [`ErrorCatalog`] — DataCatalog + CatalogWriter that always errors
//!
//! # Mock/Test Implementations
//!
//! - [`MockTargetDatabase`] — In-memory pattern-matched SQL mock
//! - [`MockWritableDatabase`] — In-memory WritableDatabase for ETL testing
//! - [`MockVectorStore`] — In-memory cosine similarity search
//! - [`MockCatalog`] — In-memory DataCatalog + CatalogWriter
//! - [`MockSubAgentInvoker`] — In-memory configurable SubAgentInvoker
//! - [`MockEnricher`] — Fixed-output SemanticEnricher
//! - [`MockProvisioner`] — In-memory DatabaseProvisioner

pub mod approval_store;
pub mod cached_enricher;
pub mod channel_sink;
pub mod column_signature_cache;
pub mod dashmap_kv;
pub mod dual_catalog_writer;
pub mod durable_event_sink;
pub mod error_catalog;
pub mod error_sub_agent;
pub mod error_target_db;
pub mod instrumented_kv;
pub mod memory_event_log;
pub mod mock_catalog;
pub mod mock_enricher;
pub mod mock_provisioner;
pub mod mock_sub_agent;
pub mod mock_target_db;
pub mod mock_vector_store;
pub mod mock_writable_db;
pub mod noop_encryption;
pub mod noop_vector_store;
pub mod retry_catalog;
pub mod retry_defaults;
pub mod retry_kv;
pub mod retry_target_db;
pub mod retry_vector_store;
pub mod retry_writable_db;
pub mod sqlite_catalog;
pub mod sqlite_data_source_pool;
pub mod sqlite_kv;
pub mod sqlite_target_db;
pub mod sqlite_writable_db;

#[cfg(feature = "postgres")]
pub mod local_provisioner;
#[cfg(feature = "postgres")]
pub mod pg_data_source_pool;
#[cfg(feature = "postgres")]
pub mod pgvector_store;
#[cfg(feature = "postgres")]
pub mod postgres_catalog;
#[cfg(feature = "postgres")]
pub mod postgres_kv;
#[cfg(feature = "postgres")]
pub mod sqlx_target_db;
#[cfg(feature = "postgres")]
pub mod sqlx_writable_db;

#[cfg(feature = "redis")]
pub mod redis_kv;

#[cfg(feature = "lancedb")]
pub mod lancedb_vector_store;

#[cfg(feature = "encryption")]
pub mod aes_encryption;

#[cfg(feature = "http-clients")]
pub mod anthropic;

#[cfg(feature = "http-clients")]
pub mod anthropic_enricher;

#[cfg(feature = "rig-chat")]
pub mod rig_chat;

#[cfg(feature = "rig-chat")]
pub mod rig_provider;

#[cfg(feature = "http-clients")]
pub mod network_sub_agent;

#[cfg(feature = "http-clients")]
pub mod neon_provisioner;

#[cfg(feature = "http-clients")]
pub mod openai_compatible;

#[cfg(feature = "http-clients")]
pub mod openrouter_embeddings;

pub use agent_fw_core::KVTimingEvent;
pub use approval_store::{KvPendingApprovalStore, DEFAULT_APPROVAL_NAMESPACE};
pub use cached_enricher::CachedEnricher;
pub use channel_sink::ChannelEventSink;
pub use column_signature_cache::{
    CardinalityBucket, ColumnSignature, ColumnSignatureCachedEnricher,
};
pub use dashmap_kv::DashMapKVStore;
pub use dual_catalog_writer::DualCatalogWriter;
pub use durable_event_sink::DurableEventSink;
pub use error_catalog::ErrorCatalog;
pub use error_sub_agent::ErrorSubAgentInvoker;
pub use error_target_db::ErrorTargetDatabase;
pub use instrumented_kv::InstrumentedKVStore;
pub use memory_event_log::MemoryEventLog;
pub use mock_catalog::MockCatalog;
pub use mock_enricher::MockEnricher;
pub use mock_provisioner::MockProvisioner;
pub use mock_sub_agent::{InvocationRecord, MockAgentResponse, MockSubAgentInvoker};
pub use mock_target_db::{MockQueryResult, MockTargetDatabase};
pub use mock_vector_store::MockVectorStore;
pub use mock_writable_db::MockWritableDatabase;
pub use noop_encryption::NoOpEncryptionService;
pub use noop_vector_store::NoOpVectorStore;
pub use retry_catalog::RetryCatalog;
pub use retry_kv::RetryKVStore;
pub use retry_target_db::RetryTargetDatabase;
pub use retry_vector_store::RetryVectorStore;
pub use retry_writable_db::RetryWritableDatabase;
pub use sqlite_catalog::{ScopedSqliteCatalog, SqliteCatalog};
pub use sqlite_data_source_pool::{SqliteDataSourcePool, SqliteDataSourcePoolConfig};
pub use sqlite_kv::SqliteKVStore;
pub use sqlite_target_db::SqliteTargetDatabase;
pub use sqlite_writable_db::SqliteWritableDatabase;

#[cfg(feature = "postgres")]
pub use local_provisioner::LocalPostgresProvisioner;
#[cfg(feature = "postgres")]
pub use pg_data_source_pool::PgDataSourcePool;
#[cfg(feature = "postgres")]
pub use pgvector_store::PgVectorStore;
#[cfg(feature = "postgres")]
pub use postgres_catalog::{PostgresCatalog, ScopedPostgresCatalog};
#[cfg(feature = "postgres")]
pub use postgres_kv::PostgresKVStore;
#[cfg(feature = "postgres")]
pub use sqlx_target_db::{
    query_table_by_ids, query_table_with_filters, search_table_text, SqlxTargetDatabase,
    TableFilterQuerySpec,
};
#[cfg(feature = "postgres")]
pub use sqlx_writable_db::SqlxWritableDatabase;

#[cfg(feature = "redis")]
pub use redis_kv::RedisKVStore;

#[cfg(feature = "lancedb")]
pub use lancedb_vector_store::LanceDbVectorStore;

#[cfg(feature = "encryption")]
pub use aes_encryption::AesEncryptionService;

#[cfg(feature = "http-clients")]
pub use anthropic::AnthropicInterpreter;

#[cfg(feature = "http-clients")]
pub use anthropic_enricher::AnthropicEnricher;

#[cfg(feature = "rig-chat")]
pub use rig_chat::{
    stock_chat_interpreter_from_settings, MockChatInterpreter, RigAnthropicChatInterpreter,
    RigBedrockChatInterpreter, RigChatInterpreterError, RigOpenAiCompatibleChatInterpreter,
};

#[cfg(feature = "rig-chat")]
pub use rig_provider::{
    stock_openai_compatible_base_url, stock_provider_entries, stock_provider_factory,
    stock_provider_factory_from_settings, stock_provider_spec_from_settings,
    stock_provider_specs_from_settings, AnthropicCompletionProvider, BedrockCompletionProvider,
    OpenAiCompatibleCompletionProvider, StockProviderFactoryError, StockProviderSettingsOptions,
    StockProviderSpec,
};

#[cfg(feature = "http-clients")]
pub use network_sub_agent::{AgentEndpoint, NetworkSubAgentInvoker};

#[cfg(feature = "http-clients")]
pub use neon_provisioner::NeonProvisioner;

#[cfg(feature = "http-clients")]
pub use openai_compatible::{
    parse_models_response, validate_remote_url, verify_openai_compatible_endpoint,
    OpenAiCompatibleProbeResult, RemoteModel,
};

#[cfg(feature = "http-clients")]
pub use openrouter_embeddings::OpenRouterEmbeddings;
