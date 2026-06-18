//! Reusable algebraic law test harnesses.
//!
//! Each module provides a `test_all` function that verifies an implementation
//! satisfies the documented algebraic laws for a given trait.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_kv_store_satisfies_laws() {
//!     let store = MyKVStore::new();
//!     agent_fw_test::kv_laws::test_all(&store).await;
//! }
//!
//! #[tokio::test]
//! async fn cancellation_laws() {
//!     agent_fw_test::cancellation_laws::test_all().await;
//! }
//!
//! #[test]
//! fn my_data_source_pool_satisfies_laws() {
//!     let pool = MyDataSourcePool::new();
//!     agent_fw_test::data_source_pool_laws::test_all(&pool);
//! }
//! ```

pub mod agent_memory_laws;
pub mod approval_laws;
pub mod cancellation_laws;
pub mod chat_interpreter_laws;
pub mod crud_service_laws;
pub mod data_source_pool_laws;
pub mod embedding_service_laws;
pub mod encryption_laws;
pub mod error_accumulator_laws;
pub mod error_enricher_laws;
pub mod eval_scorer_laws;
pub mod event_log_laws;
pub mod event_sink_laws;
pub mod fallback_laws;
pub mod filter_laws;
pub mod fixtures;
pub mod indexed_entity_laws;
pub mod job_registry_laws;
pub mod kv_laws;
pub mod metric_point_laws;
pub mod non_empty_laws;
pub mod nursery_laws;
pub mod pipeline_ctx_laws;
pub mod plan_context_laws;
pub mod plan_laws;
pub mod provisioner_laws;
pub mod resource_laws;
pub mod sample_executor_laws;
pub mod schedule_laws;
pub mod semantic_laws;
pub mod semilattice_laws;
pub mod stream_builder_laws;
pub mod sub_agent_laws;
pub mod table_role_laws;
pub mod target_db_laws;
pub mod test_case_index_laws;
pub mod tool_handler_laws;
pub mod tool_output_laws;
pub mod validated_laws;
pub mod vector_store_laws;
pub mod workspace_store_laws;
pub mod writable_db_laws;
