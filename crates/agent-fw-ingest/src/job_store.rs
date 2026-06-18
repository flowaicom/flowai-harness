//! KV-backed helpers for import/profiling job persistence.
//!
//! These helpers centralize the framework-owned vocabulary for workspace-local
//! data jobs so consuming apps do not duplicate:
//! - import/profiling status keys
//! - SSE event keys
//! - typed job-record CRUD and workspace indexing

use std::{sync::Arc, time::Duration};

use agent_fw_algebra::{JobPhase, KVError, KVStore, KVStoreExt};
use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::ingestion::IngestionPersistencePolicy;

/// Prefix for ETL import status records.
pub const IMPORT_STATUS_PREFIX: &str = "data:import:";
/// Prefix for profiling/ingestion status records.
pub const INGESTION_STATUS_PREFIX: &str = "data:ingestion:";
/// Prefix for typed non-eval job records.
pub const DATA_JOB_PREFIX: &str = "data:jobs:";

/// Kind of non-eval data job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataJobKind {
    Import,
    Profiling,
}

/// Typed lifecycle snapshot for a workspace-local non-eval job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataJobRecord {
    pub id: String,
    pub workspace_id: String,
    pub kind: DataJobKind,
    pub phase: JobPhase,
    pub created_at: String,
    pub updated_at: String,
}

/// Bound helper for callers that already own a concrete KV backend.
#[derive(Clone)]
pub struct DataJobStore {
    kv: Arc<dyn KVStore>,
}

impl DataJobStore {
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self { kv }
    }

    pub fn kv(&self) -> &Arc<dyn KVStore> {
        &self.kv
    }

    pub async fn register(
        &self,
        tenant_id: &str,
        workspace_id: &str,
        id: &str,
        kind: DataJobKind,
        initial_phase: JobPhase,
    ) -> Result<DataJobRecord, KVError> {
        register_data_job(
            self.kv.as_ref(),
            tenant_id,
            workspace_id,
            id,
            kind,
            initial_phase,
        )
        .await
    }

    pub async fn get(&self, tenant_id: &str, id: &str) -> Result<Option<DataJobRecord>, KVError> {
        get_data_job(self.kv.as_ref(), tenant_id, id).await
    }

    pub async fn advance_phase(
        &self,
        tenant_id: &str,
        id: &str,
        next_phase: JobPhase,
    ) -> Result<Option<DataJobRecord>, KVError> {
        update_data_job_phase(self.kv.as_ref(), tenant_id, id, next_phase).await
    }

    pub async fn list(
        &self,
        tenant_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<DataJobRecord>, KVError> {
        list_data_jobs(self.kv.as_ref(), tenant_id, workspace_id).await
    }

    pub async fn ids(&self, tenant_id: &str, workspace_id: &str) -> Result<Vec<String>, KVError> {
        get_data_job_ids(self.kv.as_ref(), tenant_id, workspace_id).await
    }

    pub async fn delete(
        &self,
        tenant_id: &str,
        workspace_id: &str,
        id: &str,
    ) -> Result<bool, KVError> {
        delete_data_job(self.kv.as_ref(), tenant_id, workspace_id, id).await
    }

    pub async fn put_import_status<V: Serialize + Send + Sync>(
        &self,
        tenant_id: &str,
        job_id: &str,
        status: &V,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        put_import_status(self.kv.as_ref(), tenant_id, job_id, status, ttl).await
    }

    pub async fn get_import_status<V: DeserializeOwned + Send>(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<V>, KVError> {
        get_import_status(self.kv.as_ref(), tenant_id, job_id).await
    }

    pub async fn put_profiling_status<V: Serialize + Send + Sync>(
        &self,
        tenant_id: &str,
        job_id: &str,
        status: &V,
        ttl: Option<Duration>,
    ) -> Result<(), KVError> {
        put_profiling_status(self.kv.as_ref(), tenant_id, job_id, status, ttl).await
    }

    pub async fn get_profiling_status<V: DeserializeOwned + Send>(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<V>, KVError> {
        get_profiling_status(self.kv.as_ref(), tenant_id, job_id).await
    }

    pub async fn append_import_event<V: Serialize + DeserializeOwned + Send + Sync + Clone>(
        &self,
        tenant_id: &str,
        job_id: &str,
        event: &V,
        ttl: Option<Duration>,
    ) -> Result<Vec<V>, KVError> {
        append_import_event(self.kv.as_ref(), tenant_id, job_id, event, ttl).await
    }

    pub async fn import_events<V: DeserializeOwned + Send>(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Vec<V>, KVError> {
        get_import_events(self.kv.as_ref(), tenant_id, job_id).await
    }

    pub async fn append_profiling_event<V: Serialize + DeserializeOwned + Send + Sync + Clone>(
        &self,
        tenant_id: &str,
        job_id: &str,
        event: &V,
        ttl: Option<Duration>,
    ) -> Result<Vec<V>, KVError> {
        append_profiling_event(self.kv.as_ref(), tenant_id, job_id, event, ttl).await
    }

    pub async fn profiling_events<V: DeserializeOwned + Send>(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Vec<V>, KVError> {
        get_profiling_events(self.kv.as_ref(), tenant_id, job_id).await
    }
}

/// Resolve the canonical KV key for an ETL import status record.
pub fn import_status_key(id: &str) -> String {
    format!("{IMPORT_STATUS_PREFIX}{id}")
}

/// Resolve the canonical KV key for ETL import SSE events.
pub fn import_events_key(id: &str) -> String {
    format!("{IMPORT_STATUS_PREFIX}{id}:events")
}

/// Resolve the canonical KV key for a profiling/ingestion status record.
pub fn ingestion_status_key(id: &str) -> String {
    format!("{INGESTION_STATUS_PREFIX}{id}")
}

/// Resolve the canonical KV key for profiling/ingestion SSE events.
pub fn ingestion_events_key(id: &str) -> String {
    format!("{INGESTION_STATUS_PREFIX}{id}:events")
}

/// Resolve the canonical KV key for a typed non-eval job record.
pub fn data_job_key(id: &str) -> String {
    format!("{DATA_JOB_PREFIX}{id}")
}

/// Resolve the canonical KV key for a workspace-scoped data-job index.
pub fn data_jobs_index_key(workspace_id: &str) -> String {
    format!("{DATA_JOB_PREFIX}{workspace_id}:index")
}

/// Standard profiling persistence policy for workspace-local status storage.
pub fn workspace_ingestion_persistence() -> IngestionPersistencePolicy {
    IngestionPersistencePolicy::default().with_status_key_prefix(INGESTION_STATUS_PREFIX)
}

/// Persist an ETL import status snapshot.
pub async fn put_import_status<V: Serialize + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
    status: &V,
    ttl: Option<Duration>,
) -> Result<(), KVError> {
    kv.put(tenant_id, &import_status_key(job_id), status, ttl)
        .await
}

/// Load an ETL import status snapshot.
pub async fn get_import_status<V: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
) -> Result<Option<V>, KVError> {
    kv.get(tenant_id, &import_status_key(job_id)).await
}

/// Persist a profiling/ingestion status snapshot.
pub async fn put_profiling_status<V: Serialize + Send + Sync>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
    status: &V,
    ttl: Option<Duration>,
) -> Result<(), KVError> {
    kv.put(tenant_id, &ingestion_status_key(job_id), status, ttl)
        .await
}

/// Load a profiling/ingestion status snapshot.
pub async fn get_profiling_status<V: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
) -> Result<Option<V>, KVError> {
    kv.get(tenant_id, &ingestion_status_key(job_id)).await
}

/// Load ETL import event history.
pub async fn get_import_events<V: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
) -> Result<Vec<V>, KVError> {
    get_event_history(kv, tenant_id, &import_events_key(job_id)).await
}

/// Append an ETL import event to the stored event history and return the new history.
pub async fn append_import_event<V: Serialize + DeserializeOwned + Send + Sync + Clone>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
    event: &V,
    ttl: Option<Duration>,
) -> Result<Vec<V>, KVError> {
    append_event_history(kv, tenant_id, &import_events_key(job_id), event, ttl).await
}

/// Load profiling/ingestion event history.
pub async fn get_profiling_events<V: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
) -> Result<Vec<V>, KVError> {
    get_event_history(kv, tenant_id, &ingestion_events_key(job_id)).await
}

/// Append a profiling/ingestion event to the stored event history and return the new history.
pub async fn append_profiling_event<V: Serialize + DeserializeOwned + Send + Sync + Clone>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    job_id: &str,
    event: &V,
    ttl: Option<Duration>,
) -> Result<Vec<V>, KVError> {
    append_event_history(kv, tenant_id, &ingestion_events_key(job_id), event, ttl).await
}

/// Read the workspace-scoped data-job index.
pub async fn get_data_job_ids(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    workspace_id: &str,
) -> Result<Vec<String>, KVError> {
    kv.get(tenant_id, &data_jobs_index_key(workspace_id))
        .await
        .map(|ids| ids.unwrap_or_default())
}

/// Prepend an ID to an index, enforcing uniqueness and a bounded size.
pub async fn upsert_index_id(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    index_key: &str,
    id: &str,
) -> Result<(), KVError> {
    let mut ids: Vec<String> = kv
        .get::<Vec<String>>(tenant_id, index_key)
        .await?
        .unwrap_or_default();
    ids.retain(|existing| existing != id);
    ids.insert(0, id.to_string());
    if ids.len() > 256 {
        ids.truncate(256);
    }
    kv.put(tenant_id, index_key, &ids, None).await
}

/// Register a typed non-eval job record and index it for discovery.
pub async fn register_data_job(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    workspace_id: &str,
    id: &str,
    kind: DataJobKind,
    initial_phase: JobPhase,
) -> Result<DataJobRecord, KVError> {
    let now = Utc::now().to_rfc3339();
    let record = DataJobRecord {
        id: id.to_string(),
        workspace_id: workspace_id.to_string(),
        kind,
        phase: initial_phase,
        created_at: now.clone(),
        updated_at: now,
    };
    kv.put(tenant_id, &data_job_key(id), &record, None).await?;
    upsert_index_id(kv, tenant_id, &data_jobs_index_key(workspace_id), id).await?;
    Ok(record)
}

/// Read a typed non-eval job record by ID.
pub async fn get_data_job(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    id: &str,
) -> Result<Option<DataJobRecord>, KVError> {
    kv.get(tenant_id, &data_job_key(id)).await
}

/// Monotonically advance a typed non-eval job phase.
///
/// Returns `Ok(None)` when no record exists for the ID.
pub async fn update_data_job_phase(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    id: &str,
    next_phase: JobPhase,
) -> Result<Option<DataJobRecord>, KVError> {
    let key = data_job_key(id);
    let mut record: Option<DataJobRecord> = kv.get(tenant_id, &key).await?;
    let Some(mut record) = record.take() else {
        return Ok(None);
    };

    let joined = record.phase.join(next_phase);
    if joined != record.phase {
        record.phase = joined;
        record.updated_at = Utc::now().to_rfc3339();
        kv.put(tenant_id, &key, &record, None).await?;
    }
    Ok(Some(record))
}

/// List typed non-eval jobs from the workspace index, newest-first by index order.
pub async fn list_data_jobs(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    workspace_id: &str,
) -> Result<Vec<DataJobRecord>, KVError> {
    let ids = get_data_job_ids(kv, tenant_id, workspace_id).await?;
    let mut jobs = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(record) = get_data_job(kv, tenant_id, &id).await? {
            if record.workspace_id == workspace_id {
                jobs.push(record);
            }
        }
    }
    Ok(jobs)
}

/// Delete a typed non-eval job record and prune its workspace index entry.
///
/// Returns `true` when either the record or the index entry existed.
pub async fn delete_data_job(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    workspace_id: &str,
    id: &str,
) -> Result<bool, KVError> {
    let deleted_record = kv.delete(tenant_id, &data_job_key(id)).await?;
    let mut ids = get_data_job_ids(kv, tenant_id, workspace_id).await?;
    let before = ids.len();
    ids.retain(|existing| existing != id);
    let pruned_index = ids.len() != before;
    if pruned_index {
        kv.put(tenant_id, &data_jobs_index_key(workspace_id), &ids, None)
            .await?;
    }
    Ok(deleted_record || pruned_index)
}

async fn get_event_history<V: DeserializeOwned + Send>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    key: &str,
) -> Result<Vec<V>, KVError> {
    kv.get(tenant_id, key)
        .await
        .map(|events| events.unwrap_or_default())
}

async fn append_event_history<V: Serialize + DeserializeOwned + Send + Sync + Clone>(
    kv: &(impl KVStore + ?Sized),
    tenant_id: &str,
    key: &str,
    event: &V,
    ttl: Option<Duration>,
) -> Result<Vec<V>, KVError> {
    let mut history = get_event_history(kv, tenant_id, key).await?;
    history.push(event.clone());
    kv.put(tenant_id, key, &history, ttl).await?;
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn data_job_store_round_trips_records() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = DataJobStore::new(Arc::clone(&kv));

        let record = store
            .register(
                "tenant-a",
                "ws-a",
                "job-1",
                DataJobKind::Import,
                JobPhase::Queued,
            )
            .await
            .unwrap();

        assert_eq!(record.id, "job-1");
        assert_eq!(import_status_key("job-1"), "data:import:job-1");
        assert_eq!(ingestion_status_key("job-1"), "data:ingestion:job-1");
        assert_eq!(data_jobs_index_key("ws-a"), "data:jobs:ws-a:index");

        let loaded = store.get("tenant-a", "job-1").await.unwrap().unwrap();
        assert_eq!(loaded.workspace_id, "ws-a");

        let advanced = store
            .advance_phase("tenant-a", "job-1", JobPhase::Running)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(advanced.phase, JobPhase::Running);

        let listed = store.list("tenant-a", "ws-a").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, DataJobKind::Import);
    }

    #[tokio::test]
    async fn delete_data_job_prunes_workspace_index() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = DataJobStore::new(Arc::clone(&kv));

        store
            .register(
                "tenant-a",
                "ws-a",
                "job-1",
                DataJobKind::Import,
                JobPhase::Completed,
            )
            .await
            .unwrap();

        assert_eq!(store.ids("tenant-a", "ws-a").await.unwrap(), vec!["job-1"]);
        assert!(store.delete("tenant-a", "ws-a", "job-1").await.unwrap());
        assert!(store.get("tenant-a", "job-1").await.unwrap().is_none());
        assert!(store.ids("tenant-a", "ws-a").await.unwrap().is_empty());
    }

    #[test]
    fn workspace_ingestion_persistence_uses_workspace_prefix() {
        let persistence = workspace_ingestion_persistence();
        assert_eq!(
            persistence.status_key("job-42"),
            ingestion_status_key("job-42")
        );
    }

    #[tokio::test]
    async fn generic_status_and_event_helpers_round_trip() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = DataJobStore::new(Arc::clone(&kv));

        store
            .put_import_status(
                "tenant-a",
                "job-1",
                &serde_json::json!({"stage":"parsing"}),
                None,
            )
            .await
            .unwrap();
        let status: Option<serde_json::Value> =
            store.get_import_status("tenant-a", "job-1").await.unwrap();
        assert_eq!(status, Some(serde_json::json!({"stage":"parsing"})));

        store
            .append_import_event(
                "tenant-a",
                "job-1",
                &serde_json::json!({"type":"progress"}),
                None,
            )
            .await
            .unwrap();
        let events: Vec<serde_json::Value> =
            store.import_events("tenant-a", "job-1").await.unwrap();
        assert_eq!(events, vec![serde_json::json!({"type":"progress"})]);
    }
}
