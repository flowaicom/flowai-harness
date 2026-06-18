//! Thread-store ergonomics for runtime continuity.
//!
//! These helpers lift common "ensure a thread exists" and "persist explicit
//! source selection" logic out of consuming applications so thread continuity
//! can be handled consistently across chat, eval replay, and similar runtimes.

use agent_fw_core::TenantId;

use crate::{Thread, ThreadStore, WorkspaceError};

/// Result of reconciling thread metadata in the workspace store.
#[derive(Debug, Clone)]
pub struct EnsuredThread {
    pub thread: Thread,
    pub created: bool,
    pub changed: bool,
}

/// Normalize a candidate explicit source selection.
///
/// Empty strings are treated as absent so downstream stores never persist
/// meaningless empty source identifiers.
pub fn normalize_thread_source_id(source_id: Option<&str>) -> Option<String> {
    source_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Convenience operations over [`ThreadStore`] for source continuity and
/// auto-create behavior.
#[async_trait::async_trait]
pub trait ThreadStoreExt: ThreadStore {
    /// Load the persisted explicit source selection for a thread, if present.
    async fn persisted_thread_source_id(
        &self,
        tenant: &TenantId,
        thread_id: &str,
    ) -> Result<Option<String>, WorkspaceError> {
        Ok(self
            .get_thread(tenant, thread_id)
            .await?
            .and_then(|thread| thread.source_id))
    }

    /// Persist or clear a thread's explicit source selection.
    ///
    /// When the thread does not exist and `source_id` is absent, this is a no-op.
    /// When the thread does not exist and `source_id` is present, a thread record
    /// is created with that source selection.
    async fn reconcile_thread_source_id(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        source_id: Option<&str>,
    ) -> Result<Option<Thread>, WorkspaceError> {
        let normalized_source_id = normalize_thread_source_id(source_id);

        let Some(mut thread) = self.get_thread(tenant, thread_id).await? else {
            let Some(source_id) = normalized_source_id.as_deref() else {
                return Ok(None);
            };
            let thread = Thread::with_id(thread_id, None).with_source_id(source_id);
            self.upsert_thread(tenant, &thread).await?;
            return Ok(Some(thread));
        };

        if thread.source_id == normalized_source_id {
            return Ok(None);
        }

        thread.update_source_id(normalized_source_id);
        self.upsert_thread(tenant, &thread).await?;
        Ok(Some(thread))
    }

    /// Ensure a thread record exists in the workspace store.
    ///
    /// Existing threads are left structurally intact except for optional source
    /// reconciliation. `title` is only applied when the thread is created.
    async fn ensure_thread_record(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        title: Option<String>,
        source_id: Option<&str>,
    ) -> Result<EnsuredThread, WorkspaceError> {
        let normalized_source_id = normalize_thread_source_id(source_id);

        if let Some(mut thread) = self.get_thread(tenant, thread_id).await? {
            let mut changed = false;
            if thread.source_id != normalized_source_id {
                thread.update_source_id(normalized_source_id);
                self.upsert_thread(tenant, &thread).await?;
                changed = true;
            }
            return Ok(EnsuredThread {
                thread,
                created: false,
                changed,
            });
        }

        let mut thread = Thread::with_id(thread_id, title);
        if let Some(source_id) = normalized_source_id.as_deref() {
            thread = thread.with_source_id(source_id);
        }
        self.upsert_thread(tenant, &thread).await?;
        Ok(EnsuredThread {
            thread,
            created: true,
            changed: true,
        })
    }
}

impl<T> ThreadStoreExt for T where T: ThreadStore + ?Sized {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_fw_algebra::KVStore;
    use agent_fw_core::TenantId;
    use agent_fw_interpreter::DashMapKVStore;

    use crate::{KVWorkspaceStore, ThreadStore};

    use super::*;

    fn tenant() -> TenantId {
        TenantId::new_unchecked("test-tenant")
    }

    fn store() -> KVWorkspaceStore {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        KVWorkspaceStore::new(kv)
    }

    #[test]
    fn normalize_thread_source_id_trims_and_drops_empty() {
        assert_eq!(
            normalize_thread_source_id(Some("source-a")),
            Some("source-a".into())
        );
        assert_eq!(
            normalize_thread_source_id(Some("  source-a  ")),
            Some("source-a".into())
        );
        assert_eq!(normalize_thread_source_id(Some("   ")), None);
        assert_eq!(normalize_thread_source_id(None), None);
    }

    #[tokio::test]
    async fn persisted_thread_source_id_reads_existing_thread() {
        let store = store();
        let tenant = tenant();
        store
            .upsert_thread(
                &tenant,
                &Thread::with_id("thread-a", None).with_source_id("source-a"),
            )
            .await
            .unwrap();

        let source_id = store
            .persisted_thread_source_id(&tenant, "thread-a")
            .await
            .unwrap();

        assert_eq!(source_id.as_deref(), Some("source-a"));
    }

    #[tokio::test]
    async fn reconcile_thread_source_id_creates_missing_thread_when_present() {
        let store = store();
        let tenant = tenant();

        let thread = store
            .reconcile_thread_source_id(&tenant, "thread-a", Some("source-a"))
            .await
            .unwrap()
            .expect("thread should be created");

        assert_eq!(thread.source_id.as_deref(), Some("source-a"));
        assert!(store
            .get_thread(&tenant, "thread-a")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn reconcile_thread_source_id_is_noop_when_unchanged() {
        let store = store();
        let tenant = tenant();
        store
            .upsert_thread(
                &tenant,
                &Thread::with_id("thread-a", None).with_source_id("source-a"),
            )
            .await
            .unwrap();

        let thread = store
            .reconcile_thread_source_id(&tenant, "thread-a", Some("source-a"))
            .await
            .unwrap();

        assert!(thread.is_none());
    }

    #[tokio::test]
    async fn ensure_thread_record_creates_with_title_and_source() {
        let store = store();
        let tenant = tenant();

        let ensured = store
            .ensure_thread_record(
                &tenant,
                "thread-a",
                Some("Thread A".to_string()),
                Some("source-a"),
            )
            .await
            .unwrap();

        assert!(ensured.created);
        assert!(ensured.changed);
        assert_eq!(ensured.thread.title.as_deref(), Some("Thread A"));
        assert_eq!(ensured.thread.source_id.as_deref(), Some("source-a"));
    }

    #[tokio::test]
    async fn ensure_thread_record_updates_source_without_replacing_title() {
        let store = store();
        let tenant = tenant();
        store
            .upsert_thread(
                &tenant,
                &Thread::with_id("thread-a", Some("Original".to_string())),
            )
            .await
            .unwrap();

        let ensured = store
            .ensure_thread_record(
                &tenant,
                "thread-a",
                Some("Ignored".to_string()),
                Some("source-a"),
            )
            .await
            .unwrap();

        assert!(!ensured.created);
        assert!(ensured.changed);
        assert_eq!(ensured.thread.title.as_deref(), Some("Original"));
        assert_eq!(ensured.thread.source_id.as_deref(), Some("source-a"));
    }
}
