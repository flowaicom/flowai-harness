//! Workspace Store Algebra — persistent storage for user-authored artifacts.
//!
//! # Motivation
//!
//! Threads, test cases, eval runs, and data sources are user-authored artifacts
//! that must outlive individual requests. This module defines **focused sub-traits**
//! (one per resource type), enabling each consumer to declare exactly what it needs:
//!
//! - [`ThreadStore`] — thread CRUD (4 methods)
//! - [`MessageStore`] — message insert/query (3 methods)
//! - [`TestCaseStore`] — test case CRUD + batch ops (7 methods)
//! - [`EvalStore`] — eval runs + results (7 methods)
//! - [`DataSourceStore`] — data source CRUD (4 methods)
//! - [`WorkspaceEntityStore`] — workspace entity CRUD (5 methods)
//!
//! The composed [`WorkspaceStore`] supertrait bundles all six for consumers that
//! need the full vocabulary.
//!
//! # Interpreters
//!
//! - [`crate::kv_store::KVWorkspaceStore`] — stores artifacts as KV blobs (development/fallback)
//! - Production interpreters (e.g. SQLite, PostgreSQL) are provided by consuming crates.
//!
//! # Laws (apply to each sub-trait independently)
//!
//! 1. **Roundtrip**: `insert(x); get(x.id)` ≡ `Some(x)`
//! 2. **Delete-Get**: `delete(id); get(id)` ≡ `None`
//! 3. **Upsert idempotence**: `upsert(x); upsert(x)` ≡ `upsert(x)`
//! 4. **List consistency**: after `insert(x)`, `list()` contains `x`

use agent_fw_core::{TenantId, TestCaseId};
use agent_fw_eval::test_case::{AuthoredTestCase, TestCaseStatus};
use agent_fw_eval::types::{EvalRun, EvalStatus, EvalThreadFork, TestCaseResult, TestCaseSet};

use crate::data_source::DataSource;
use crate::indexed_entity::IdIndex;
use crate::thread::{Message, Thread};
use crate::workspace::Workspace;

// =============================================================================
// Error Type
// =============================================================================

/// Error type for workspace store operations.
///
/// Uses `String` wrappers to avoid leaking interpreter-specific types
/// (`sqlx::Error`, `serde_json::Error`) into the algebra layer.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Database error: {0}")]
    Db(String),
    #[error("Serialization error: {0}")]
    Serde(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("KV error: {0}")]
    KV(String),
}

impl From<agent_fw_algebra::KVError> for WorkspaceError {
    fn from(e: agent_fw_algebra::KVError) -> Self {
        WorkspaceError::KV(e.to_string())
    }
}

// =============================================================================
// Focused Sub-Traits
// =============================================================================

/// Thread persistence — list, get, upsert, delete.
///
/// # Laws
///
/// 1. `upsert(t); get(t.id)` ≡ `Some(t)`
/// 2. `delete(id); get(id)` ≡ `None`
/// 3. `upsert(t); upsert(t)` ≡ `upsert(t)`
/// 4. after `upsert(t)`, `list()` contains `t`
#[async_trait::async_trait]
pub trait ThreadStore: Send + Sync {
    /// List threads for a tenant, ordered by most recent first.
    async fn list_threads(&self, tenant: &TenantId) -> Result<Vec<Thread>, WorkspaceError>;

    /// Get a single thread by ID.
    async fn get_thread(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<Thread>, WorkspaceError>;

    /// Insert or update a thread.
    async fn upsert_thread(&self, tenant: &TenantId, thread: &Thread)
        -> Result<(), WorkspaceError>;

    /// Delete a thread (and its messages, if cascading).
    async fn delete_thread(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError>;
}

/// Message persistence — insert and query within threads.
///
/// # Laws
///
/// 1. `insert(m, tid); get_all(tid)` contains `m`
/// 2. `get_messages(tid, limit, 0).len()` ≤ `limit`
#[async_trait::async_trait]
pub trait MessageStore: Send + Sync {
    /// Insert a message into a thread.
    async fn insert_message(
        &self,
        tenant: &TenantId,
        message: &Message,
        thread_id: &str,
    ) -> Result<(), WorkspaceError>;

    /// Get messages for a thread with pagination.
    async fn get_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Message>, WorkspaceError>;

    /// Get ALL messages for a thread (no pagination).
    async fn get_all_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
    ) -> Result<Vec<Message>, WorkspaceError>;

    /// Get the most recent messages for a thread, preserving chronological order.
    async fn get_recent_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let messages = self.get_all_messages(tenant, thread_id).await?;
        if messages.len() <= limit {
            return Ok(messages);
        }
        let skip = messages.len() - limit;
        Ok(messages.into_iter().skip(skip).collect())
    }

    /// Get all messages for multiple threads, preserving input order.
    async fn get_all_messages_batch(
        &self,
        tenant: &TenantId,
        thread_ids: &[String],
    ) -> Result<Vec<Vec<Message>>, WorkspaceError> {
        let mut batches = Vec::with_capacity(thread_ids.len());
        for thread_id in thread_ids {
            batches.push(self.get_all_messages(tenant, thread_id).await?);
        }
        Ok(batches)
    }
}

/// Test case persistence — CRUD + batch operations.
///
/// # Laws
///
/// 1. `insert(tc); get(tc.id)` ≡ `Some(tc)`
/// 2. `delete(id); get(id)` ≡ `None`
/// 3. after `insert(tc)`, `list()` contains `tc`
/// 4. `batch_delete(ids)` count ≤ `ids.len()`
#[async_trait::async_trait]
pub trait TestCaseStore: Send + Sync {
    /// List test cases for a tenant, ordered by most recent first.
    async fn list_test_cases(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<AuthoredTestCase>, WorkspaceError>;

    /// Get a single test case by ID.
    async fn get_test_case(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<AuthoredTestCase>, WorkspaceError>;

    /// Get multiple test cases by ID, preserving input order.
    async fn get_test_cases_by_id(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<Vec<Option<AuthoredTestCase>>, WorkspaceError> {
        let mut cases = Vec::with_capacity(ids.len());
        for id in ids {
            cases.push(self.get_test_case(tenant, id).await?);
        }
        Ok(cases)
    }

    /// Rebuild or rederive the test-case index for this backend.
    ///
    /// KV-backed implementations may need a true repair pass. Relational
    /// backends can usually satisfy this by re-listing the authoritative rows.
    async fn rebuild_test_case_index(
        &self,
        tenant: &TenantId,
    ) -> Result<IdIndex<TestCaseId>, WorkspaceError> {
        let cases = self.list_test_cases(tenant).await?;
        Ok(IdIndex {
            ids: cases.into_iter().map(|test_case| test_case.id).collect(),
        })
    }

    /// Insert a new test case (upsert on conflict).
    async fn insert_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError>;

    /// Update an existing test case.
    async fn update_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError>;

    /// Delete a test case by ID.
    async fn delete_test_case(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError>;

    /// Batch update status for multiple test cases. Returns count of affected rows.
    async fn batch_update_status(
        &self,
        tenant: &TenantId,
        ids: &[String],
        status: TestCaseStatus,
    ) -> Result<u64, WorkspaceError>;

    /// Batch delete test cases. Returns count of deleted rows.
    async fn batch_delete_test_cases(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<u64, WorkspaceError>;
}

/// Eval persistence — eval runs, uploaded test-case sets, and per-test-case results.
///
/// # Laws
///
/// 1. `insert_run(r); get_run(r.id)` ≡ `Some(r)`
/// 2. `delete_run(id); get_run(id)` ≡ `None`
/// 3. `upsert_set(s); get_set(s.id)` ≡ `Some(s)`
/// 4. `insert_result(rid, tcid, res); get_results(rid)` contains `res`
#[async_trait::async_trait]
pub trait EvalStore: Send + Sync {
    /// Insert a new eval run.
    async fn insert_eval_run(&self, tenant: &TenantId, run: &EvalRun)
        -> Result<(), WorkspaceError>;

    /// Get an eval run by ID (results loaded separately via `get_eval_results`).
    async fn get_eval_run(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<EvalRun>, WorkspaceError>;

    /// Update eval run status.
    async fn update_eval_status(
        &self,
        tenant: &TenantId,
        id: &str,
        status: &EvalStatus,
    ) -> Result<(), WorkspaceError>;

    /// List eval runs for a tenant, ordered by most recent first.
    async fn list_eval_runs(&self, tenant: &TenantId) -> Result<Vec<EvalRun>, WorkspaceError>;

    /// Delete an eval run (and its results, if cascading).
    async fn delete_eval_run(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError>;

    /// Insert a single eval result.
    async fn insert_eval_result(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        result: &TestCaseResult,
    ) -> Result<(), WorkspaceError>;

    /// Get all results for an eval run.
    async fn get_eval_results(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
    ) -> Result<Vec<TestCaseResult>, WorkspaceError>;

    /// Insert or update an uploaded eval test-case set.
    async fn upsert_eval_test_case_set(
        &self,
        tenant: &TenantId,
        set: &TestCaseSet,
    ) -> Result<(), WorkspaceError>;

    /// Get a single uploaded eval test-case set by ID.
    async fn get_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<TestCaseSet>, WorkspaceError>;

    /// List uploaded eval test-case sets for a tenant, ordered by newest first.
    async fn list_eval_test_case_sets(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<TestCaseSet>, WorkspaceError>;

    /// Delete an uploaded eval test-case set by ID.
    async fn delete_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<(), WorkspaceError>;

    /// Append a persisted thread fork record for an eval test case.
    async fn append_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork: &EvalThreadFork,
    ) -> Result<(), WorkspaceError>;

    /// List persisted thread fork records for an eval test case.
    async fn list_eval_thread_forks(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
    ) -> Result<Vec<EvalThreadFork>, WorkspaceError>;

    /// Delete a persisted thread fork record by ID.
    ///
    /// Returns `true` when a record was removed and `false` when no matching
    /// fork existed for the given eval/test-case pair.
    async fn delete_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork_id: &str,
    ) -> Result<bool, WorkspaceError>;
}

/// Data source persistence — CRUD for database connection configs.
///
/// # Laws
///
/// 1. `upsert(ds); get(ds.id)` ≡ `Some(ds)`
/// 2. `delete(id); get(id)` ≡ `None`
/// 3. `upsert(ds); upsert(ds)` ≡ `upsert(ds)`
/// 4. after `upsert(ds)`, `list()` contains `ds`
#[async_trait::async_trait]
pub trait DataSourceStore: Send + Sync {
    /// List data sources for a tenant.
    async fn list_data_sources(&self, tenant: &TenantId)
        -> Result<Vec<DataSource>, WorkspaceError>;

    /// Get a single data source by ID.
    async fn get_data_source(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<DataSource>, WorkspaceError>;

    /// Insert or update a data source.
    async fn upsert_data_source(
        &self,
        tenant: &TenantId,
        ds: &DataSource,
    ) -> Result<(), WorkspaceError>;

    /// Delete a data source by ID.
    async fn delete_data_source(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError>;
}

/// Workspace entity persistence — CRUD for workspace metadata.
///
/// # Laws
///
/// 1. `create(ws); get(ws.id)` ≡ `Some(ws)`
/// 2. `delete(id); get(id)` ≡ `None`
/// 3. after `create(ws)`, `list()` contains `ws`
#[async_trait::async_trait]
pub trait WorkspaceEntityStore: Send + Sync {
    /// List workspaces for a tenant.
    async fn list_workspaces(&self, tenant: &TenantId) -> Result<Vec<Workspace>, WorkspaceError>;

    /// Get a workspace by ID.
    async fn get_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<Option<Workspace>, WorkspaceError>;

    /// Create a new workspace.
    async fn create_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError>;

    /// Update an existing workspace.
    async fn update_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError>;

    /// Delete a workspace by ID.
    async fn delete_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<(), WorkspaceError>;
}

// =============================================================================
// Composed Supertrait
// =============================================================================

/// Full workspace store — bundles all six focused sub-traits.
///
/// Use this when a consumer genuinely needs all resource types (e.g. `AppState`).
/// Prefer the narrower sub-traits when possible.
pub trait WorkspaceStore:
    ThreadStore + MessageStore + TestCaseStore + EvalStore + DataSourceStore + WorkspaceEntityStore
{
}

/// Blanket impl: any type implementing all sub-traits is a WorkspaceStore.
impl<T> WorkspaceStore for T where
    T: ThreadStore
        + MessageStore
        + TestCaseStore
        + EvalStore
        + DataSourceStore
        + WorkspaceEntityStore
{
}
