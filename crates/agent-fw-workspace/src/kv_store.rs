//! KVWorkspaceStore — KV-blob interpreter for workspace store algebra.
//!
//! Stores all workspace artifacts as JSON blobs in a [`KVStore`], using
//! index keys per entity type per tenant for list operations.
//!
//! This is the fallback interpreter when no relational database is available.
//! It trades query flexibility for simplicity and zero-config operation.
//!
//! # Architecture
//!
//! Each entity is stored as a JSON blob under its own key. An `EntityIndex`
//! per entity type per tenant tracks which IDs exist, enabling list operations
//! without key scanning.
//!
//! ```text
//!   thread:{id}        → Thread JSON
//!   thread:{id}:messages → Vec<Message> JSON
//!   threads:index      → EntityIndex { ids: ["id1", "id2", ...] }
//! ```

use std::sync::Arc;

use agent_fw_algebra::KVStore;
use agent_fw_core::{TenantId, TestCaseId};
use agent_fw_eval::test_case::{AuthoredTestCase, TestCaseStatus};
use agent_fw_eval::types::{EvalRun, EvalStatus, EvalThreadFork, TestCaseResult, TestCaseSet};

use crate::data_source::DataSource;
use crate::file_store;
use crate::indexed_entity::IdIndex;
use crate::kv_keys;
use crate::store::{
    DataSourceStore, EvalStore, MessageStore, TestCaseStore, ThreadStore, WorkspaceEntityStore,
    WorkspaceError,
};
use crate::thread::{Message, Thread};
use crate::workspace::Workspace;

type ThreadAuxKeysFn = dyn Fn(&str) -> Vec<String> + Send + Sync;

// =============================================================================
// KVWorkspaceStore
// =============================================================================

/// KV-blob workspace store.
///
/// Wraps any [`KVStore`] implementation and stores all workspace artifacts
/// as JSON blobs. Suitable for development, testing, and as a fallback
/// when no relational database is configured.
pub struct KVWorkspaceStore {
    kv: Arc<dyn KVStore>,
    key_policy: Arc<dyn WorkspaceKvKeyPolicy>,
}

impl KVWorkspaceStore {
    /// Create a new KV workspace store wrapping the given KV backend.
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self::new_with_policy(kv, Arc::new(DefaultWorkspaceKvKeyPolicy))
    }

    /// Create a new KV workspace store with an explicit key policy.
    pub fn new_with_policy(
        kv: Arc<dyn KVStore>,
        key_policy: Arc<dyn WorkspaceKvKeyPolicy>,
    ) -> Self {
        Self { kv, key_policy }
    }

    /// Create a new KV workspace store using default framework keys plus
    /// caller-provided auxiliary thread cleanup keys.
    pub fn new_with_thread_aux_keys<F>(kv: Arc<dyn KVStore>, thread_aux_keys: F) -> Self
    where
        F: Fn(&str) -> Vec<String> + Send + Sync + 'static,
    {
        Self::new_with_policy(
            kv,
            Arc::new(ThreadAuxKeysWorkspaceKvKeyPolicy {
                thread_aux_keys: Arc::new(thread_aux_keys),
            }),
        )
    }
}

/// Configurable KV vocabulary for workspace persistence.
///
/// This allows consuming applications to keep domain-specific key contracts
/// while reusing the framework KV workspace interpreter unchanged.
pub trait WorkspaceKvKeyPolicy: Send + Sync {
    fn thread(&self, id: &str) -> String {
        kv_keys::thread(id)
    }

    fn thread_messages(&self, id: &str) -> String {
        kv_keys::thread_messages(id)
    }

    fn thread_aux_keys(&self, _id: &str) -> Vec<String> {
        Vec::new()
    }

    fn threads_index(&self) -> &'static str {
        kv_keys::THREADS_INDEX
    }

    fn test_case(&self, id: &str) -> String {
        kv_keys::test_case(id)
    }

    fn test_cases_index(&self) -> &'static str {
        kv_keys::TEST_CASES_INDEX
    }

    fn eval_run(&self, id: &str) -> String {
        kv_keys::eval_run(id)
    }

    fn eval_run_results(&self, id: &str) -> String {
        kv_keys::eval_run_results(id)
    }

    fn eval_runs_index(&self) -> &'static str {
        kv_keys::EVAL_RUNS_INDEX
    }

    fn eval_test_case_set(&self, id: &str) -> String {
        kv_keys::eval_test_case_set(id)
    }

    fn eval_test_case_sets_index(&self) -> &'static str {
        kv_keys::EVAL_TEST_CASE_SETS_INDEX
    }

    fn eval_forks(&self, eval_run_id: &str, test_case_id: &str) -> String {
        kv_keys::eval_forks(eval_run_id, test_case_id)
    }

    fn data_source(&self, id: &str) -> String {
        kv_keys::data_source(id)
    }

    fn data_sources_index(&self) -> &'static str {
        kv_keys::DATA_SOURCES_INDEX
    }

    fn workspace(&self, id: &str) -> String {
        kv_keys::workspace(id)
    }

    fn workspaces_index(&self) -> &'static str {
        kv_keys::WORKSPACES_INDEX
    }
}

#[derive(Debug, Default)]
pub struct DefaultWorkspaceKvKeyPolicy;

impl WorkspaceKvKeyPolicy for DefaultWorkspaceKvKeyPolicy {}

struct ThreadAuxKeysWorkspaceKvKeyPolicy {
    thread_aux_keys: Arc<ThreadAuxKeysFn>,
}

impl WorkspaceKvKeyPolicy for ThreadAuxKeysWorkspaceKvKeyPolicy {
    fn thread_aux_keys(&self, id: &str) -> Vec<String> {
        (self.thread_aux_keys)(id)
    }
}

// =============================================================================
// EntityIndex — unified index type for all entity types
// =============================================================================

/// Index tracking entity IDs per tenant per entity type.
///
/// All entity types use identical `{ ids: Vec<String> }` structures.
/// Serde aliases allow reading legacy keys written with older field names.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct EntityIndex {
    #[serde(
        default,
        alias = "thread_ids",
        alias = "caseIds",
        alias = "run_ids",
        alias = "set_ids",
        alias = "source_ids",
        alias = "workspace_ids"
    )]
    ids: Vec<String>,
}

// =============================================================================
// Error bridging
// =============================================================================

fn kv_err(e: agent_fw_algebra::KVError) -> WorkspaceError {
    WorkspaceError::KV(e.to_string())
}

fn serde_err(e: serde_json::Error) -> WorkspaceError {
    WorkspaceError::Serde(e.to_string())
}

// =============================================================================
// Generic KV helpers
// =============================================================================

async fn load_index<I: serde::de::DeserializeOwned + Default>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
) -> Result<I, WorkspaceError> {
    let val = kv.get_json(tenant, key).await.map_err(kv_err)?;
    match val {
        Some(v) => serde_json::from_value(v).map_err(serde_err),
        None => Ok(I::default()),
    }
}

async fn save_index<I: serde::Serialize>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
    index: &I,
) -> Result<(), WorkspaceError> {
    let v = serde_json::to_value(index).map_err(serde_err)?;
    kv.put_json(tenant, key, v, None).await.map_err(kv_err)
}

async fn put_entity<T: serde::Serialize>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
    entity: &T,
) -> Result<(), WorkspaceError> {
    let v = serde_json::to_value(entity).map_err(serde_err)?;
    kv.put_json(tenant, key, v, None).await.map_err(kv_err)
}

async fn get_entity<T: serde::de::DeserializeOwned>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
) -> Result<Option<T>, WorkspaceError> {
    let val = kv.get_json(tenant, key).await.map_err(kv_err)?;
    match val {
        Some(v) => Ok(Some(serde_json::from_value(v).map_err(serde_err)?)),
        None => Ok(None),
    }
}

// =============================================================================
// ThreadStore
// =============================================================================

#[async_trait::async_trait]
impl ThreadStore for KVWorkspaceStore {
    async fn list_threads(&self, tenant: &TenantId) -> Result<Vec<Thread>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.threads_index()).await?;
        let mut threads = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(th) =
                get_entity::<Thread>(self.kv.as_ref(), t, &self.key_policy.thread(id)).await?
            {
                threads.push(th);
            }
        }
        Ok(threads)
    }

    async fn get_thread(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<Thread>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.thread(id),
        )
        .await
    }

    async fn upsert_thread(
        &self,
        tenant: &TenantId,
        thread: &Thread,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        put_entity(
            self.kv.as_ref(),
            t,
            &self.key_policy.thread(&thread.id),
            thread,
        )
        .await?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.threads_index()).await?;
        if !idx.ids.contains(&thread.id) {
            idx.ids.insert(0, thread.id.clone());
            save_index(self.kv.as_ref(), t, self.key_policy.threads_index(), &idx).await?;
        }
        Ok(())
    }

    async fn delete_thread(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.thread(id))
            .await
            .map_err(kv_err)?;
        self.kv
            .delete(t, &self.key_policy.thread_messages(id))
            .await
            .map_err(kv_err)?;
        file_store::delete_thread_files(self.kv.as_ref(), t, id).await?;
        for key in self.key_policy.thread_aux_keys(id) {
            let _ = self.kv.delete(t, &key).await;
        }
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.threads_index()).await?;
        idx.ids.retain(|tid| tid != id);
        save_index(self.kv.as_ref(), t, self.key_policy.threads_index(), &idx).await?;
        Ok(())
    }
}

// =============================================================================
// MessageStore
// =============================================================================

#[async_trait::async_trait]
impl MessageStore for KVWorkspaceStore {
    async fn insert_message(
        &self,
        tenant: &TenantId,
        message: &Message,
        thread_id: &str,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let key = self.key_policy.thread_messages(thread_id);
        let mut messages: Vec<Message> = get_entity(self.kv.as_ref(), t, &key)
            .await?
            .unwrap_or_default();
        messages.push(message.clone());
        put_entity(self.kv.as_ref(), t, &key, &messages).await
    }

    async fn get_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let key = self.key_policy.thread_messages(thread_id);
        let messages: Vec<Message> = get_entity(self.kv.as_ref(), tenant.as_str(), &key)
            .await?
            .unwrap_or_default();
        Ok(messages.into_iter().skip(offset).take(limit).collect())
    }

    async fn get_all_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let key = self.key_policy.thread_messages(thread_id);
        let messages: Vec<Message> = get_entity(self.kv.as_ref(), tenant.as_str(), &key)
            .await?
            .unwrap_or_default();
        Ok(messages)
    }

    async fn get_recent_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let key = self.key_policy.thread_messages(thread_id);
        let messages: Vec<Message> = get_entity(self.kv.as_ref(), tenant.as_str(), &key)
            .await?
            .unwrap_or_default();
        if messages.len() <= limit {
            return Ok(messages);
        }
        let skip = messages.len() - limit;
        Ok(messages.into_iter().skip(skip).collect())
    }

    async fn get_all_messages_batch(
        &self,
        tenant: &TenantId,
        thread_ids: &[String],
    ) -> Result<Vec<Vec<Message>>, WorkspaceError> {
        let t = tenant.as_str();
        let keys: Vec<String> = thread_ids
            .iter()
            .map(|thread_id| self.key_policy.thread_messages(thread_id))
            .collect();
        let batch = self.kv.get_many_json(t, &keys).await.map_err(kv_err)?;

        let mut messages_by_thread = Vec::with_capacity(thread_ids.len());
        for key in keys {
            let messages = batch
                .get(&key)
                .map(|value| serde_json::from_value::<Vec<Message>>(value.clone()))
                .transpose()
                .map_err(serde_err)?
                .unwrap_or_default();
            messages_by_thread.push(messages);
        }
        Ok(messages_by_thread)
    }
}

// =============================================================================
// TestCaseStore
// =============================================================================

#[async_trait::async_trait]
impl TestCaseStore for KVWorkspaceStore {
    async fn list_test_cases(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<AuthoredTestCase>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.test_cases_index()).await?;
        let mut cases = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(tc) =
                get_entity::<AuthoredTestCase>(self.kv.as_ref(), t, &self.key_policy.test_case(id))
                    .await?
            {
                cases.push(tc);
            }
        }
        Ok(cases)
    }

    async fn get_test_case(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<AuthoredTestCase>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.test_case(id),
        )
        .await
    }

    async fn get_test_cases_by_id(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<Vec<Option<AuthoredTestCase>>, WorkspaceError> {
        let t = tenant.as_str();
        let keys: Vec<String> = ids.iter().map(|id| self.key_policy.test_case(id)).collect();
        let batch = self.kv.get_many_json(t, &keys).await.map_err(kv_err)?;

        let mut cases = Vec::with_capacity(ids.len());
        for key in keys {
            let test_case = batch
                .get(&key)
                .map(|value| serde_json::from_value::<AuthoredTestCase>(value.clone()))
                .transpose()
                .map_err(serde_err)?;
            cases.push(test_case);
        }
        Ok(cases)
    }

    async fn rebuild_test_case_index(
        &self,
        tenant: &TenantId,
    ) -> Result<IdIndex<TestCaseId>, WorkspaceError> {
        let t = tenant.as_str();
        let key_prefix = self.key_policy.test_case("");
        let keys = self.kv.list_keys(t, &key_prefix).await.map_err(kv_err)?;
        let batch = self.kv.get_many_json(t, &keys).await.map_err(kv_err)?;

        let total = keys.len();
        let mut skipped = 0u32;
        let mut dated_ids: Vec<(TestCaseId, String)> = keys
            .into_iter()
            .filter_map(|key| {
                let id = match key.strip_prefix(&key_prefix).and_then(TestCaseId::new) {
                    Some(id) => id,
                    None => {
                        skipped += 1;
                        tracing::warn!(key = %key, "Skipping malformed test case key during index rebuild");
                        return None;
                    }
                };
                let created_at = batch
                    .get(&key)
                    .and_then(|value| value.get("createdAt").and_then(|date| date.as_str()))
                    .map(str::to_string);
                if created_at.is_none() {
                    skipped += 1;
                    tracing::warn!(case_id = %id, "Skipping test case without createdAt during index rebuild");
                }
                created_at.map(|created_at| (id, created_at))
            })
            .collect();
        dated_ids.sort_by(|a, b| b.1.cmp(&a.1));

        let index = IdIndex {
            ids: dated_ids.into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
        };
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.test_cases_index(),
            &EntityIndex {
                ids: index.ids.iter().map(|id| id.to_string()).collect(),
            },
        )
        .await?;

        if skipped > 0 {
            tracing::warn!(
                indexed = index.ids.len(),
                skipped,
                total,
                "Rebuilt test case index with gaps"
            );
        } else {
            tracing::info!(count = index.ids.len(), "Rebuilt test case index");
        }

        Ok(index)
    }

    async fn insert_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let id = tc.id.as_str();
        put_entity(self.kv.as_ref(), t, &self.key_policy.test_case(id), tc).await?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.test_cases_index()).await?;
        if !idx.ids.contains(&id.to_string()) {
            idx.ids.insert(0, id.to_string());
            save_index(
                self.kv.as_ref(),
                t,
                self.key_policy.test_cases_index(),
                &idx,
            )
            .await?;
        }
        Ok(())
    }

    async fn update_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError> {
        put_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.test_case(tc.id.as_str()),
            tc,
        )
        .await
    }

    async fn delete_test_case(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.test_case(id))
            .await
            .map_err(kv_err)?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.test_cases_index()).await?;
        idx.ids.retain(|cid| cid != id);
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.test_cases_index(),
            &idx,
        )
        .await?;
        Ok(())
    }

    async fn batch_update_status(
        &self,
        tenant: &TenantId,
        ids: &[String],
        status: TestCaseStatus,
    ) -> Result<u64, WorkspaceError> {
        let t = tenant.as_str();
        let mut count = 0u64;
        for id in ids {
            if let Some(mut tc) =
                get_entity::<AuthoredTestCase>(self.kv.as_ref(), t, &self.key_policy.test_case(id))
                    .await?
            {
                tc.status = status;
                tc.updated_at = chrono::Utc::now().to_rfc3339();
                put_entity(self.kv.as_ref(), t, &self.key_policy.test_case(id), &tc).await?;
                count += 1;
            }
        }
        Ok(count)
    }

    async fn batch_delete_test_cases(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<u64, WorkspaceError> {
        let t = tenant.as_str();
        let mut count = 0u64;
        for id in ids {
            if self
                .kv
                .delete(t, &self.key_policy.test_case(id))
                .await
                .map_err(kv_err)?
            {
                count += 1;
            }
        }
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.test_cases_index()).await?;
        idx.ids.retain(|cid| !ids.contains(cid));
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.test_cases_index(),
            &idx,
        )
        .await?;
        Ok(count)
    }
}

// =============================================================================
// EvalStore
// =============================================================================

#[async_trait::async_trait]
impl EvalStore for KVWorkspaceStore {
    async fn insert_eval_run(
        &self,
        tenant: &TenantId,
        run: &EvalRun,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let id = run.id.as_str();
        put_entity(self.kv.as_ref(), t, &self.key_policy.eval_run(id), run).await?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.eval_runs_index()).await?;
        if !idx.ids.contains(&id.to_string()) {
            idx.ids.insert(0, id.to_string());
            save_index(self.kv.as_ref(), t, self.key_policy.eval_runs_index(), &idx).await?;
        }
        Ok(())
    }

    async fn get_eval_run(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<EvalRun>, WorkspaceError> {
        let t = tenant.as_str();
        let mut run: Option<EvalRun> =
            get_entity(self.kv.as_ref(), t, &self.key_policy.eval_run(id)).await?;
        // Load results separately (stored in their own key)
        if let Some(ref mut r) = run {
            let results: Vec<TestCaseResult> =
                get_entity(self.kv.as_ref(), t, &self.key_policy.eval_run_results(id))
                    .await?
                    .unwrap_or_default();
            r.results = results;
        }
        Ok(run)
    }

    async fn update_eval_status(
        &self,
        tenant: &TenantId,
        id: &str,
        status: &EvalStatus,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        if let Some(mut run) =
            get_entity::<EvalRun>(self.kv.as_ref(), t, &self.key_policy.eval_run(id)).await?
        {
            run.status = status.clone();
            run.updated_at = chrono::Utc::now().to_rfc3339();
            put_entity(self.kv.as_ref(), t, &self.key_policy.eval_run(id), &run).await?;
        }
        Ok(())
    }

    async fn list_eval_runs(&self, tenant: &TenantId) -> Result<Vec<EvalRun>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.eval_runs_index()).await?;
        let mut runs = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(r) =
                get_entity::<EvalRun>(self.kv.as_ref(), t, &self.key_policy.eval_run(id)).await?
            {
                runs.push(r);
            }
        }
        Ok(runs)
    }

    async fn delete_eval_run(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.eval_run(id))
            .await
            .map_err(kv_err)?;
        self.kv
            .delete(t, &self.key_policy.eval_run_results(id))
            .await
            .map_err(kv_err)?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.eval_runs_index()).await?;
        idx.ids.retain(|rid| rid != id);
        save_index(self.kv.as_ref(), t, self.key_policy.eval_runs_index(), &idx).await?;
        Ok(())
    }

    async fn insert_eval_result(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        _test_case_id: &str,
        result: &TestCaseResult,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let key = self.key_policy.eval_run_results(eval_run_id);
        let mut results: Vec<TestCaseResult> = get_entity(self.kv.as_ref(), t, &key)
            .await?
            .unwrap_or_default();
        results.push(result.clone());
        put_entity(self.kv.as_ref(), t, &key, &results).await
    }

    async fn get_eval_results(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
    ) -> Result<Vec<TestCaseResult>, WorkspaceError> {
        let results: Vec<TestCaseResult> = get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.eval_run_results(eval_run_id),
        )
        .await?
        .unwrap_or_default();
        Ok(results)
    }

    async fn upsert_eval_test_case_set(
        &self,
        tenant: &TenantId,
        set: &TestCaseSet,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let set_id = set.id.clone();
        put_entity(
            self.kv.as_ref(),
            t,
            &self.key_policy.eval_test_case_set(&set_id),
            set,
        )
        .await?;
        let mut idx: EntityIndex = load_index(
            self.kv.as_ref(),
            t,
            self.key_policy.eval_test_case_sets_index(),
        )
        .await?;
        if !idx.ids.contains(&set_id) {
            idx.ids.insert(0, set_id);
            save_index(
                self.kv.as_ref(),
                t,
                self.key_policy.eval_test_case_sets_index(),
                &idx,
            )
            .await?;
        }
        Ok(())
    }

    async fn get_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<TestCaseSet>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.eval_test_case_set(id),
        )
        .await
    }

    async fn list_eval_test_case_sets(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<TestCaseSet>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex = load_index(
            self.kv.as_ref(),
            t,
            self.key_policy.eval_test_case_sets_index(),
        )
        .await?;
        let mut sets = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(set) = get_entity::<TestCaseSet>(
                self.kv.as_ref(),
                t,
                &self.key_policy.eval_test_case_set(id),
            )
            .await?
            {
                sets.push(set);
            }
        }
        Ok(sets)
    }

    async fn delete_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.eval_test_case_set(id))
            .await
            .map_err(kv_err)?;
        let mut idx: EntityIndex = load_index(
            self.kv.as_ref(),
            t,
            self.key_policy.eval_test_case_sets_index(),
        )
        .await?;
        idx.ids.retain(|set_id| set_id != id);
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.eval_test_case_sets_index(),
            &idx,
        )
        .await?;
        Ok(())
    }

    async fn append_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork: &EvalThreadFork,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let key = self.key_policy.eval_forks(eval_run_id, test_case_id);
        let mut forks: Vec<EvalThreadFork> = get_entity(self.kv.as_ref(), t, &key)
            .await?
            .unwrap_or_default();
        forks.push(fork.clone());
        put_entity(self.kv.as_ref(), t, &key, &forks).await
    }

    async fn list_eval_thread_forks(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
    ) -> Result<Vec<EvalThreadFork>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.eval_forks(eval_run_id, test_case_id),
        )
        .await
        .map(|forks: Option<Vec<EvalThreadFork>>| forks.unwrap_or_default())
    }

    async fn delete_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork_id: &str,
    ) -> Result<bool, WorkspaceError> {
        let t = tenant.as_str();
        let key = self.key_policy.eval_forks(eval_run_id, test_case_id);
        let mut forks: Vec<EvalThreadFork> = get_entity(self.kv.as_ref(), t, &key)
            .await?
            .unwrap_or_default();
        let before = forks.len();
        forks.retain(|fork| fork.id != fork_id);
        if forks.len() == before {
            return Ok(false);
        }
        put_entity(self.kv.as_ref(), t, &key, &forks).await?;
        Ok(true)
    }
}

// =============================================================================
// DataSourceStore
// =============================================================================

#[async_trait::async_trait]
impl DataSourceStore for KVWorkspaceStore {
    async fn list_data_sources(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<DataSource>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.data_sources_index()).await?;
        let mut sources = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(ds) =
                get_entity::<DataSource>(self.kv.as_ref(), t, &self.key_policy.data_source(id))
                    .await?
            {
                sources.push(ds);
            }
        }
        Ok(sources)
    }

    async fn get_data_source(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<DataSource>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.data_source(id),
        )
        .await
    }

    async fn upsert_data_source(
        &self,
        tenant: &TenantId,
        ds: &DataSource,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        put_entity(
            self.kv.as_ref(),
            t,
            &self.key_policy.data_source(&ds.id),
            ds,
        )
        .await?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.data_sources_index()).await?;
        if !idx.ids.contains(&ds.id) {
            idx.ids.insert(0, ds.id.clone());
            save_index(
                self.kv.as_ref(),
                t,
                self.key_policy.data_sources_index(),
                &idx,
            )
            .await?;
        }
        Ok(())
    }

    async fn delete_data_source(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.data_source(id))
            .await
            .map_err(kv_err)?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.data_sources_index()).await?;
        idx.ids.retain(|sid| sid != id);
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.data_sources_index(),
            &idx,
        )
        .await?;
        Ok(())
    }
}

// =============================================================================
// WorkspaceEntityStore
// =============================================================================

#[async_trait::async_trait]
impl WorkspaceEntityStore for KVWorkspaceStore {
    async fn list_workspaces(&self, tenant: &TenantId) -> Result<Vec<Workspace>, WorkspaceError> {
        let t = tenant.as_str();
        let idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.workspaces_index()).await?;
        let mut workspaces = Vec::with_capacity(idx.ids.len());
        for id in &idx.ids {
            if let Some(ws) =
                get_entity::<Workspace>(self.kv.as_ref(), t, &self.key_policy.workspace(id)).await?
            {
                workspaces.push(ws);
            }
        }
        Ok(workspaces)
    }

    async fn get_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<Option<Workspace>, WorkspaceError> {
        get_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.workspace(workspace_id),
        )
        .await
    }

    async fn create_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        let id = workspace.id.as_str();
        put_entity(
            self.kv.as_ref(),
            t,
            &self.key_policy.workspace(id),
            workspace,
        )
        .await?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.workspaces_index()).await?;
        if !idx.ids.contains(&id.to_string()) {
            idx.ids.insert(0, id.to_string());
            save_index(
                self.kv.as_ref(),
                t,
                self.key_policy.workspaces_index(),
                &idx,
            )
            .await?;
        }
        Ok(())
    }

    async fn update_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError> {
        put_entity(
            self.kv.as_ref(),
            tenant.as_str(),
            &self.key_policy.workspace(workspace.id.as_str()),
            workspace,
        )
        .await
    }

    async fn delete_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<(), WorkspaceError> {
        let t = tenant.as_str();
        self.kv
            .delete(t, &self.key_policy.workspace(workspace_id))
            .await
            .map_err(kv_err)?;
        let mut idx: EntityIndex =
            load_index(self.kv.as_ref(), t, self.key_policy.workspaces_index()).await?;
        idx.ids.retain(|wid| wid != workspace_id);
        save_index(
            self.kv.as_ref(),
            t,
            self.key_policy.workspaces_index(),
            &idx,
        )
        .await?;
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::TestCaseId;
    use agent_fw_eval::test_case::AuthoredTestCase;
    use agent_fw_eval::types::{
        EvalConfig, EvalProgress, EvalStatus, EvalTestCase, EvalThreadFork,
    };
    use agent_fw_interpreter::DashMapKVStore;
    use serde_json::json;

    struct LegacyPolicy;

    impl WorkspaceKvKeyPolicy for LegacyPolicy {
        fn thread(&self, id: &str) -> String {
            format!("legacy:thread:{id}")
        }

        fn thread_messages(&self, id: &str) -> String {
            format!("legacy:thread:{id}:messages")
        }

        fn thread_aux_keys(&self, id: &str) -> Vec<String> {
            vec![format!("legacy:thread:{id}:cost")]
        }

        fn threads_index(&self) -> &'static str {
            "legacy:threads:index"
        }

        fn test_case(&self, id: &str) -> String {
            format!("legacy:test:{id}")
        }

        fn test_cases_index(&self) -> &'static str {
            "legacy:tests:index"
        }

        fn eval_run(&self, id: &str) -> String {
            format!("legacy:eval:{id}")
        }

        fn eval_run_results(&self, id: &str) -> String {
            format!("legacy:eval:{id}:results")
        }

        fn eval_runs_index(&self) -> &'static str {
            "legacy:evals:index"
        }

        fn data_source(&self, id: &str) -> String {
            format!("legacy:data-source:{id}")
        }

        fn data_sources_index(&self) -> &'static str {
            "legacy:data-sources:index"
        }

        fn workspace(&self, id: &str) -> String {
            format!("legacy:workspace:{id}")
        }

        fn workspaces_index(&self) -> &'static str {
            "legacy:workspaces:index"
        }
    }

    fn make_store() -> KVWorkspaceStore {
        KVWorkspaceStore::new(Arc::new(DashMapKVStore::new()))
    }

    fn make_store_with_policy() -> (Arc<DashMapKVStore>, KVWorkspaceStore) {
        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new_with_policy(kv.clone(), Arc::new(LegacyPolicy));
        (kv, store)
    }

    fn make_store_with_thread_aux_keys() -> (Arc<DashMapKVStore>, KVWorkspaceStore) {
        let kv = Arc::new(DashMapKVStore::new());
        let store = KVWorkspaceStore::new_with_thread_aux_keys(kv.clone(), |id| {
            vec![format!("thread:{id}:cost"), format!("thread:{id}:latency")]
        });
        (kv, store)
    }

    fn tenant() -> TenantId {
        TenantId::new_unchecked("test-tenant")
    }

    // =========================================================================
    // ThreadStore laws
    // =========================================================================

    #[tokio::test]
    async fn thread_roundtrip() {
        let store = make_store();
        let t = tenant();
        let thread = Thread::with_id("t-1", Some("Test".to_string())).with_source_id("source-1");

        store.upsert_thread(&t, &thread).await.unwrap();
        let got = store.get_thread(&t, "t-1").await.unwrap().unwrap();
        assert_eq!(got.id, "t-1");
        assert_eq!(got.title, Some("Test".to_string()));
        assert_eq!(got.source_id.as_deref(), Some("source-1"));
    }

    #[tokio::test]
    async fn thread_delete_get() {
        let store = make_store();
        let t = tenant();
        let thread = Thread::with_id("t-2", None);

        store.upsert_thread(&t, &thread).await.unwrap();
        store.delete_thread(&t, "t-2").await.unwrap();
        assert!(store.get_thread(&t, "t-2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn thread_list_consistency() {
        let store = make_store();
        let t = tenant();
        let thread = Thread::with_id("t-3", Some("Listed".to_string()));

        store.upsert_thread(&t, &thread).await.unwrap();
        let list = store.list_threads(&t).await.unwrap();
        assert!(list.iter().any(|th| th.id == "t-3"));
    }

    #[tokio::test]
    async fn thread_upsert_idempotent() {
        let store = make_store();
        let t = tenant();
        let thread = Thread::with_id("t-4", Some("Idem".to_string()));

        store.upsert_thread(&t, &thread).await.unwrap();
        store.upsert_thread(&t, &thread).await.unwrap();
        let list = store.list_threads(&t).await.unwrap();
        assert_eq!(list.iter().filter(|th| th.id == "t-4").count(), 1);
    }

    #[tokio::test]
    async fn custom_key_policy_controls_persistence_and_aux_cleanup() {
        let (kv, store) = make_store_with_policy();
        let t = tenant();
        let thread = Thread::with_id("legacy-1", None);

        store.upsert_thread(&t, &thread).await.unwrap();
        store
            .insert_message(&t, &Message::new("user", "hello"), "legacy-1")
            .await
            .unwrap();
        kv.put_json(
            t.as_str(),
            "legacy:thread:legacy-1:cost",
            json!({"usd": 1.0}),
            None,
        )
        .await
        .unwrap();

        assert!(kv
            .get_json(t.as_str(), "legacy:thread:legacy-1")
            .await
            .unwrap()
            .is_some());
        assert!(kv
            .get_json(t.as_str(), "legacy:threads:index")
            .await
            .unwrap()
            .is_some());

        store.delete_thread(&t, "legacy-1").await.unwrap();

        assert!(kv
            .get_json(t.as_str(), "legacy:thread:legacy-1")
            .await
            .unwrap()
            .is_none());
        assert!(kv
            .get_json(t.as_str(), "legacy:thread:legacy-1:messages")
            .await
            .unwrap()
            .is_none());
        assert!(kv
            .get_json(t.as_str(), "legacy:thread:legacy-1:cost")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn thread_aux_key_builder_cleans_default_thread_auxiliary_keys() {
        let (kv, store) = make_store_with_thread_aux_keys();
        let t = tenant();
        let thread = Thread::with_id("t-aux", None);

        store.upsert_thread(&t, &thread).await.unwrap();
        kv.put_json(t.as_str(), "thread:t-aux:cost", json!({"usd": 1.0}), None)
            .await
            .unwrap();
        kv.put_json(
            t.as_str(),
            "thread:t-aux:latency",
            json!({"totalDurationMs": 42}),
            None,
        )
        .await
        .unwrap();

        store.delete_thread(&t, "t-aux").await.unwrap();

        assert!(kv
            .get_json(t.as_str(), "thread:t-aux:cost")
            .await
            .unwrap()
            .is_none());
        assert!(kv
            .get_json(t.as_str(), "thread:t-aux:latency")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn thread_delete_cascades_messages() {
        let store = make_store();
        let t = tenant();
        let thread = Thread::with_id("t-5", None);
        let msg = Message::new("user", "hello");

        store.upsert_thread(&t, &thread).await.unwrap();
        store.insert_message(&t, &msg, "t-5").await.unwrap();
        store.delete_thread(&t, "t-5").await.unwrap();
        let messages = store.get_all_messages(&t, "t-5").await.unwrap();
        assert!(messages.is_empty());
    }

    // =========================================================================
    // MessageStore laws
    // =========================================================================

    #[tokio::test]
    async fn message_insert_and_get_all() {
        let store = make_store();
        let t = tenant();
        let m1 = Message::new("user", "first");
        let m2 = Message::new("assistant", "second");

        store.insert_message(&t, &m1, "thread-a").await.unwrap();
        store.insert_message(&t, &m2, "thread-a").await.unwrap();
        let all = store.get_all_messages(&t, "thread-a").await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].content, "first");
        assert_eq!(all[1].content, "second");
    }

    #[tokio::test]
    async fn message_pagination() {
        let store = make_store();
        let t = tenant();
        for i in 0..5 {
            let msg = Message::new("user", format!("msg-{}", i));
            store.insert_message(&t, &msg, "thread-b").await.unwrap();
        }

        let page = store.get_messages(&t, "thread-b", 2, 1).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].content, "msg-1");
        assert_eq!(page[1].content, "msg-2");
    }

    #[tokio::test]
    async fn message_batch_load_preserves_input_order() {
        let store = make_store();
        let t = tenant();
        store
            .insert_message(&t, &Message::new("user", "thread-a-1"), "thread-a")
            .await
            .unwrap();
        store
            .insert_message(&t, &Message::new("assistant", "thread-b-1"), "thread-b")
            .await
            .unwrap();
        store
            .insert_message(&t, &Message::new("user", "thread-a-2"), "thread-a")
            .await
            .unwrap();

        let batches = store
            .get_all_messages_batch(&t, &["thread-b".to_string(), "thread-a".to_string()])
            .await
            .unwrap();

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].content, "thread-b-1");
        assert_eq!(batches[1].len(), 2);
        assert_eq!(batches[1][0].content, "thread-a-1");
        assert_eq!(batches[1][1].content, "thread-a-2");
    }

    #[tokio::test]
    async fn recent_messages_returns_tail_in_chronological_order() {
        let store = make_store();
        let t = tenant();
        for content in ["msg-0", "msg-1", "msg-2", "msg-3"] {
            store
                .insert_message(&t, &Message::new("user", content), "thread-c")
                .await
                .unwrap();
        }

        let recent = store.get_recent_messages(&t, "thread-c", 2).await.unwrap();

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "msg-2");
        assert_eq!(recent[1].content, "msg-3");
    }

    // =========================================================================
    // DataSourceStore laws
    // =========================================================================

    #[tokio::test]
    async fn data_source_roundtrip() {
        let store = make_store();
        let t = tenant();
        let ds = DataSource {
            id: "ds-1".to_string(),
            name: "Test DB".to_string(),
            database_type: crate::data_source::DatabaseType::PostgreSQL,
            host: "localhost".to_string(),
            port: 5432,
            database_name: "testdb".to_string(),
            schema_name: "public".to_string(),
            encrypted_credentials: None,
            is_active: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        store.upsert_data_source(&t, &ds).await.unwrap();
        let got = store.get_data_source(&t, "ds-1").await.unwrap().unwrap();
        assert_eq!(got.id, "ds-1");
        assert_eq!(got.name, "Test DB");
    }

    #[tokio::test]
    async fn data_source_delete_get() {
        let store = make_store();
        let t = tenant();
        let ds = DataSource {
            id: "ds-2".to_string(),
            name: "Del DB".to_string(),
            database_type: crate::data_source::DatabaseType::MySQL,
            host: "localhost".to_string(),
            port: 3306,
            database_name: "deldb".to_string(),
            schema_name: "public".to_string(),
            encrypted_credentials: None,
            is_active: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        store.upsert_data_source(&t, &ds).await.unwrap();
        store.delete_data_source(&t, "ds-2").await.unwrap();
        assert!(store.get_data_source(&t, "ds-2").await.unwrap().is_none());
    }

    // =========================================================================
    // WorkspaceEntityStore laws
    // =========================================================================

    #[tokio::test]
    async fn workspace_entity_roundtrip() {
        let store = make_store();
        let t = tenant();
        let ws = Workspace::default_workspace();

        store.create_workspace(&t, &ws).await.unwrap();
        let got = store.get_workspace(&t, "default").await.unwrap().unwrap();
        assert_eq!(got.id, ws.id);
        assert_eq!(got.slug, "default");
    }

    #[tokio::test]
    async fn workspace_entity_delete_get() {
        let store = make_store();
        let t = tenant();
        let ws = Workspace::default_workspace();

        store.create_workspace(&t, &ws).await.unwrap();
        store.delete_workspace(&t, "default").await.unwrap();
        assert!(store.get_workspace(&t, "default").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn workspace_entity_list_consistency() {
        let store = make_store();
        let t = tenant();
        let ws = Workspace::default_workspace();

        store.create_workspace(&t, &ws).await.unwrap();
        let list = store.list_workspaces(&t).await.unwrap();
        assert!(list.iter().any(|w| w.id.is_default()));
    }

    // =========================================================================
    // TestCaseStore laws
    // =========================================================================

    #[tokio::test]
    async fn test_case_roundtrip() {
        let store = make_store();
        let t = tenant();
        let tc = AuthoredTestCase::new(TestCaseId::new_unchecked("tc-1"), "test input".to_string());

        store.insert_test_case(&t, &tc).await.unwrap();
        let got = store.get_test_case(&t, "tc-1").await.unwrap().unwrap();
        assert_eq!(got.id.as_str(), "tc-1");
        assert_eq!(got.input, "test input");
    }

    #[tokio::test]
    async fn test_case_delete_get() {
        let store = make_store();
        let t = tenant();
        let tc = AuthoredTestCase::new(TestCaseId::new_unchecked("tc-2"), "delete me".to_string());

        store.insert_test_case(&t, &tc).await.unwrap();
        store.delete_test_case(&t, "tc-2").await.unwrap();
        assert!(store.get_test_case(&t, "tc-2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_case_batch_delete() {
        let store = make_store();
        let t = tenant();
        for i in 0..3 {
            let tc = AuthoredTestCase::new(
                TestCaseId::new_unchecked(format!("bd-{}", i)),
                format!("input {}", i),
            );
            store.insert_test_case(&t, &tc).await.unwrap();
        }

        let count = store
            .batch_delete_test_cases(&t, &["bd-0".to_string(), "bd-1".to_string()])
            .await
            .unwrap();
        assert_eq!(count, 2);
        assert!(store.get_test_case(&t, "bd-0").await.unwrap().is_none());
        assert!(store.get_test_case(&t, "bd-1").await.unwrap().is_none());
        assert!(store.get_test_case(&t, "bd-2").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_case_batch_update_status() {
        let store = make_store();
        let t = tenant();
        for i in 0..2 {
            let tc = AuthoredTestCase::new(
                TestCaseId::new_unchecked(format!("bs-{}", i)),
                format!("input {}", i),
            );
            store.insert_test_case(&t, &tc).await.unwrap();
        }

        let count = store
            .batch_update_status(
                &t,
                &["bs-0".to_string(), "bs-1".to_string()],
                TestCaseStatus::Active,
            )
            .await
            .unwrap();
        assert_eq!(count, 2);

        let tc0 = store.get_test_case(&t, "bs-0").await.unwrap().unwrap();
        assert_eq!(tc0.status, TestCaseStatus::Active);
    }

    #[tokio::test]
    async fn test_case_batch_get_preserves_input_order() {
        let store = make_store();
        let t = tenant();
        for id in ["tc-a", "tc-b"] {
            let tc = AuthoredTestCase::new(TestCaseId::new_unchecked(id), format!("input {id}"));
            store.insert_test_case(&t, &tc).await.unwrap();
        }

        let cases = store
            .get_test_cases_by_id(
                &t,
                &[
                    "tc-b".to_string(),
                    "missing".to_string(),
                    "tc-a".to_string(),
                ],
            )
            .await
            .unwrap();

        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0].as_ref().map(|tc| tc.id.as_str()), Some("tc-b"));
        assert!(cases[1].is_none());
        assert_eq!(cases[2].as_ref().map(|tc| tc.id.as_str()), Some("tc-a"));
    }

    #[tokio::test]
    async fn rebuild_test_case_index_repairs_orphaned_case_entries() {
        let store = make_store();
        let t = tenant();
        let tc = AuthoredTestCase::new(
            TestCaseId::new_unchecked("tc-orphan"),
            "repair me".to_string(),
        );

        put_entity(
            store.kv.as_ref(),
            t.as_str(),
            &store.key_policy.test_case("tc-orphan"),
            &tc,
        )
        .await
        .unwrap();

        let rebuilt = store.rebuild_test_case_index(&t).await.unwrap();
        assert_eq!(rebuilt.ids, vec![TestCaseId::new_unchecked("tc-orphan")]);

        let listed = store.list_test_cases(&t).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id.as_str(), "tc-orphan");
    }

    // =========================================================================
    // EvalStore laws
    // =========================================================================

    #[tokio::test]
    async fn eval_run_roundtrip() {
        let store = make_store();
        let t = tenant();
        let run = EvalRun::new(EvalConfig::default());

        let id = run.id.as_str().to_string();
        store.insert_eval_run(&t, &run).await.unwrap();
        let got = store.get_eval_run(&t, &id).await.unwrap().unwrap();
        assert_eq!(got.id, run.id);
        assert_eq!(got.status, EvalStatus::Queued);
    }

    #[tokio::test]
    async fn eval_run_delete_get() {
        let store = make_store();
        let t = tenant();
        let run = EvalRun::new(EvalConfig::default());

        let id = run.id.as_str().to_string();
        store.insert_eval_run(&t, &run).await.unwrap();
        store.delete_eval_run(&t, &id).await.unwrap();
        assert!(store.get_eval_run(&t, &id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn eval_status_update() {
        let store = make_store();
        let t = tenant();
        let run = EvalRun::new(EvalConfig::default());

        let id = run.id.as_str().to_string();
        store.insert_eval_run(&t, &run).await.unwrap();
        let running_status = EvalStatus::Running {
            progress: EvalProgress {
                completed_samples: 1,
                total_samples: 10,
                completed_test_cases: 0,
                total_test_cases: 5,
                current_test_case_id: None,
                elapsed_ms: 100,
                estimated_remaining_ms: None,
                test_case_states: Vec::new(),
            },
        };
        store
            .update_eval_status(&t, &id, &running_status)
            .await
            .unwrap();
        let got = store.get_eval_run(&t, &id).await.unwrap().unwrap();
        assert!(matches!(got.status, EvalStatus::Running { .. }));
    }

    #[tokio::test]
    async fn eval_results_roundtrip() {
        let store = make_store();
        let t = tenant();
        let run = EvalRun::new(EvalConfig::default());
        let run_id = run.id.as_str().to_string();

        store.insert_eval_run(&t, &run).await.unwrap();

        let result = TestCaseResult {
            test_case_id: TestCaseId::new_unchecked("tc-r1"),
            input: Some("test input".to_string()),
            samples: vec![],
            pass_at_k: vec![],
            aggregate_score: 0.0,
        };

        store
            .insert_eval_result(&t, &run_id, "tc-r1", &result)
            .await
            .unwrap();
        let results = store.get_eval_results(&t, &run_id).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].test_case_id.as_str(), "tc-r1");
    }

    #[tokio::test]
    async fn eval_test_case_sets_roundtrip() {
        let store = make_store();
        let t = tenant();
        let set = TestCaseSet {
            id: "set-1".to_string(),
            name: "Regression".to_string(),
            description: "smoke suite".to_string(),
            test_cases: vec![EvalTestCase {
                id: TestCaseId::new_unchecked("tc-1"),
                tags: vec!["smoke".to_string()],
                input: "find products".to_string(),
                expected_trajectory: vec!["searchProducts".to_string()],
                trajectory_mode: agent_fw_eval::TrajectoryMode::Unordered,
                ground_truth: None,
                final_response: None,
                source_thread_id: Some("thread-1".to_string()),
            }],
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        store.upsert_eval_test_case_set(&t, &set).await.unwrap();

        let listed = store.list_eval_test_case_sets(&t).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], set);

        let loaded = store
            .get_eval_test_case_set(&t, "set-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded, set);

        store.delete_eval_test_case_set(&t, "set-1").await.unwrap();
        assert!(store
            .get_eval_test_case_set(&t, "set-1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn eval_thread_forks_roundtrip() {
        let store = make_store();
        let t = tenant();
        let fork = EvalThreadFork {
            id: "fork-1".to_string(),
            thread_id: "thread-1".to_string(),
            parent_thread_id: Some("thread-0".to_string()),
            fork_at_message_index: Some(3),
            edited_content: Some("retry with filters".to_string()),
            label: Some("Fork".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        store
            .append_eval_thread_fork(&t, "eval-1", "tc-1", &fork)
            .await
            .unwrap();

        let listed = store
            .list_eval_thread_forks(&t, "eval-1", "tc-1")
            .await
            .unwrap();
        assert_eq!(listed, vec![fork.clone()]);

        assert!(store
            .delete_eval_thread_fork(&t, "eval-1", "tc-1", "fork-1")
            .await
            .unwrap());
        assert!(store
            .list_eval_thread_forks(&t, "eval-1", "tc-1")
            .await
            .unwrap()
            .is_empty());
    }

    // =========================================================================
    // Tenant isolation
    // =========================================================================

    #[tokio::test]
    async fn tenant_isolation() {
        let store = make_store();
        let t1 = TenantId::new_unchecked("tenant-a");
        let t2 = TenantId::new_unchecked("tenant-b");

        let thread = Thread::with_id("shared-id", Some("T1 thread".to_string()));
        store.upsert_thread(&t1, &thread).await.unwrap();

        // Tenant B cannot see tenant A's thread
        assert!(store.get_thread(&t2, "shared-id").await.unwrap().is_none());
    }
}
