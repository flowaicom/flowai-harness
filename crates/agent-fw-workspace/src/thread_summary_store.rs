//! KV-backed thread summary helpers for cost and latency attachments.
//!
//! These helpers own the generic per-thread summary vocabulary used by chat and
//! replay flows, so consuming applications do not hand-roll `thread:{id}:cost`
//! or `thread:{id}:latency` persistence.

use std::sync::Arc;

use agent_fw_algebra::{KVStore, KVStoreExt};
use agent_fw_core::{CostSummary, LatencySummary};

use crate::{kv_keys, WorkspaceError};

/// Bound thread-summary store for callers that already own a KV backend.
#[derive(Clone)]
pub struct ThreadSummaryStore {
    kv: Arc<dyn KVStore>,
}

impl ThreadSummaryStore {
    pub fn new(kv: Arc<dyn KVStore>) -> Self {
        Self { kv }
    }

    pub fn kv(&self) -> &Arc<dyn KVStore> {
        &self.kv
    }

    pub async fn put_cost_summary(
        &self,
        tenant: &str,
        thread_id: &str,
        summary: &CostSummary,
    ) -> Result<(), WorkspaceError> {
        put_thread_cost_summary(self.kv.as_ref(), tenant, thread_id, summary).await
    }

    pub async fn get_cost_summary(
        &self,
        tenant: &str,
        thread_id: &str,
    ) -> Result<Option<CostSummary>, WorkspaceError> {
        get_thread_cost_summary(self.kv.as_ref(), tenant, thread_id).await
    }

    pub async fn put_summaries(
        &self,
        tenant: &str,
        thread_id: &str,
        cost: Option<&CostSummary>,
        latency: Option<&LatencySummary>,
    ) -> Result<(), WorkspaceError> {
        put_thread_summaries(self.kv.as_ref(), tenant, thread_id, cost, latency).await
    }

    pub async fn put_latency_summary(
        &self,
        tenant: &str,
        thread_id: &str,
        summary: &LatencySummary,
    ) -> Result<(), WorkspaceError> {
        put_thread_latency_summary(self.kv.as_ref(), tenant, thread_id, summary).await
    }

    pub async fn get_latency_summary(
        &self,
        tenant: &str,
        thread_id: &str,
    ) -> Result<Option<LatencySummary>, WorkspaceError> {
        get_thread_latency_summary(self.kv.as_ref(), tenant, thread_id).await
    }

    pub async fn delete_thread_summaries(
        &self,
        tenant: &str,
        thread_id: &str,
    ) -> Result<(), WorkspaceError> {
        delete_thread_summaries(self.kv.as_ref(), tenant, thread_id).await
    }
}

pub async fn put_thread_cost_summary<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
    summary: &CostSummary,
) -> Result<(), WorkspaceError> {
    kv.put(tenant, &kv_keys::thread_cost(thread_id), summary, None)
        .await
        .map_err(Into::into)
}

pub async fn get_thread_cost_summary<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<Option<CostSummary>, WorkspaceError> {
    kv.get(tenant, &kv_keys::thread_cost(thread_id))
        .await
        .map_err(Into::into)
}

pub async fn put_thread_latency_summary<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
    summary: &LatencySummary,
) -> Result<(), WorkspaceError> {
    kv.put(tenant, &kv_keys::thread_latency(thread_id), summary, None)
        .await
        .map_err(Into::into)
}

pub async fn get_thread_latency_summary<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<Option<LatencySummary>, WorkspaceError> {
    kv.get(tenant, &kv_keys::thread_latency(thread_id))
        .await
        .map_err(Into::into)
}

pub async fn put_thread_summaries<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
    cost: Option<&CostSummary>,
    latency: Option<&LatencySummary>,
) -> Result<(), WorkspaceError> {
    if let Some(cost) = cost {
        put_thread_cost_summary(kv, tenant, thread_id, cost).await?;
    }
    if let Some(latency) = latency {
        put_thread_latency_summary(kv, tenant, thread_id, latency).await?;
    }
    Ok(())
}

pub async fn delete_thread_summaries<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<(), WorkspaceError> {
    let cost_key = kv_keys::thread_cost(thread_id);
    let latency_key = kv_keys::thread_latency(thread_id);
    let _ = tokio::join!(
        kv.delete(tenant, &cost_key),
        kv.delete(tenant, &latency_key),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::ToolTiming;
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn thread_summary_store_roundtrips_cost_and_latency() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = ThreadSummaryStore::new(Arc::clone(&kv));
        let cost = CostSummary::new(vec![]);
        let latency = LatencySummary {
            total_duration_ms: 120,
            tool_timings: vec![ToolTiming::completed("searchProducts", "call-1", 35)],
            ..LatencySummary::zero()
        };

        store
            .put_cost_summary("tenant-a", "thread-1", &cost)
            .await
            .unwrap();
        store
            .put_latency_summary("tenant-a", "thread-1", &latency)
            .await
            .unwrap();

        assert_eq!(
            store
                .get_cost_summary("tenant-a", "thread-1")
                .await
                .unwrap(),
            Some(cost)
        );
        assert_eq!(
            store
                .get_latency_summary("tenant-a", "thread-1")
                .await
                .unwrap(),
            Some(latency)
        );
    }

    #[tokio::test]
    async fn delete_thread_summaries_removes_both_entries() {
        let kv = DashMapKVStore::new();
        let cost = CostSummary::new(vec![]);
        let latency = LatencySummary::zero();
        put_thread_cost_summary(&kv, "tenant-a", "thread-1", &cost)
            .await
            .unwrap();
        put_thread_latency_summary(&kv, "tenant-a", "thread-1", &latency)
            .await
            .unwrap();

        delete_thread_summaries(&kv, "tenant-a", "thread-1")
            .await
            .unwrap();

        assert!(get_thread_cost_summary(&kv, "tenant-a", "thread-1")
            .await
            .unwrap()
            .is_none());
        assert!(get_thread_latency_summary(&kv, "tenant-a", "thread-1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn put_summaries_persists_optional_values() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = ThreadSummaryStore::new(Arc::clone(&kv));
        let cost = CostSummary::new(vec![]);
        let latency = LatencySummary::zero();

        store
            .put_summaries("tenant-a", "thread-1", Some(&cost), None)
            .await
            .unwrap();
        store
            .put_summaries("tenant-a", "thread-1", None, Some(&latency))
            .await
            .unwrap();

        assert_eq!(
            store
                .get_cost_summary("tenant-a", "thread-1")
                .await
                .unwrap(),
            Some(cost)
        );
        assert_eq!(
            store
                .get_latency_summary("tenant-a", "thread-1")
                .await
                .unwrap(),
            Some(latency)
        );
    }
}
