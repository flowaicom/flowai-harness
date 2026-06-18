//! KV-backed helpers for thread file registration and stored file content.
//!
//! These helpers centralize the framework-owned file artifact layout so
//! applications do not need to reconstruct:
//! - `file:{id}` payload keys
//! - `files:{thread_id}` index keys
//! - append / list / cleanup logic

use agent_fw_algebra::KVStore;

use crate::file::{StoredFile, ThreadFileEntry, ThreadFileIndex};
use crate::kv_keys;
use crate::store::WorkspaceError;

fn kv_err(error: agent_fw_algebra::KVError) -> WorkspaceError {
    WorkspaceError::KV(error.to_string())
}

fn serde_err(error: serde_json::Error) -> WorkspaceError {
    WorkspaceError::Serde(error.to_string())
}

async fn load_thread_file_index<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<ThreadFileIndex, WorkspaceError> {
    let key = kv_keys::thread_files(thread_id);
    let value = kv.get_json(tenant, &key).await.map_err(kv_err)?;
    match value {
        Some(value) => serde_json::from_value(value).map_err(serde_err),
        None => Ok(ThreadFileIndex::default()),
    }
}

async fn save_thread_file_index<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
    index: &ThreadFileIndex,
) -> Result<(), WorkspaceError> {
    let key = kv_keys::thread_files(thread_id);
    let value = serde_json::to_value(index).map_err(serde_err)?;
    kv.put_json(tenant, &key, value, None).await.map_err(kv_err)
}

/// Store file content under the canonical framework file key.
pub async fn put_stored_file<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    file_id: &str,
    stored: &StoredFile,
) -> Result<(), WorkspaceError> {
    let key = kv_keys::stored_file(file_id);
    let value = serde_json::to_value(stored).map_err(serde_err)?;
    kv.put_json(tenant, &key, value, None).await.map_err(kv_err)
}

/// Load stored file content by ID.
pub async fn get_stored_file<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    file_id: &str,
) -> Result<Option<StoredFile>, WorkspaceError> {
    let key = kv_keys::stored_file(file_id);
    let value = kv.get_json(tenant, &key).await.map_err(kv_err)?;
    value
        .map(serde_json::from_value::<StoredFile>)
        .transpose()
        .map_err(serde_err)
}

/// List thread-scoped file entries in registration order.
pub async fn list_thread_files<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<Vec<ThreadFileEntry>, WorkspaceError> {
    Ok(load_thread_file_index(kv, tenant, thread_id).await?.files)
}

/// Register a file against a thread using the current timestamp.
pub async fn register_thread_file<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    file_id: &str,
    filename: &str,
    thread_id: &str,
) -> Result<ThreadFileEntry, WorkspaceError> {
    register_thread_file_at(
        kv,
        tenant,
        file_id,
        filename,
        thread_id,
        &chrono::Utc::now().to_rfc3339(),
    )
    .await
}

/// Register a file against a thread with an explicit timestamp.
pub async fn register_thread_file_at<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    file_id: &str,
    filename: &str,
    thread_id: &str,
    created_at: &str,
) -> Result<ThreadFileEntry, WorkspaceError> {
    let mut index = load_thread_file_index(kv, tenant, thread_id).await?;
    index.files.retain(|existing| existing.file_id != file_id);

    let entry = ThreadFileEntry {
        file_id: file_id.to_string(),
        filename: filename.to_string(),
        thread_id: thread_id.to_string(),
        created_at: created_at.to_string(),
    };
    index.files.push(entry.clone());
    save_thread_file_index(kv, tenant, thread_id, &index).await?;
    Ok(entry)
}

/// Delete a thread's file index and any stored files referenced by it.
pub async fn delete_thread_files<K: KVStore + ?Sized>(
    kv: &K,
    tenant: &str,
    thread_id: &str,
) -> Result<(), WorkspaceError> {
    let index = load_thread_file_index(kv, tenant, thread_id).await?;
    for entry in index.files {
        let file_key = kv_keys::stored_file(&entry.file_id);
        let _ = kv.delete(tenant, &file_key).await;
    }

    let index_key = kv_keys::thread_files(thread_id);
    let _ = kv.delete(tenant, &index_key).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn stored_file_roundtrip() {
        let kv = DashMapKVStore::new();
        let tenant = "tenant-1";
        let stored = StoredFile {
            content: "a,b\n1,2\n".to_string(),
            filename: "data.csv".to_string(),
        };

        put_stored_file(&kv, tenant, "file-1", &stored)
            .await
            .unwrap();
        let loaded = get_stored_file(&kv, tenant, "file-1").await.unwrap();
        assert_eq!(loaded, Some(stored));
    }

    #[tokio::test]
    async fn register_thread_file_deduplicates_by_file_id() {
        let kv = DashMapKVStore::new();
        let tenant = "tenant-1";

        register_thread_file_at(
            &kv,
            tenant,
            "file-1",
            "old.csv",
            "thread-1",
            "2026-03-10T00:00:00Z",
        )
        .await
        .unwrap();
        register_thread_file_at(
            &kv,
            tenant,
            "file-1",
            "new.csv",
            "thread-1",
            "2026-03-10T01:00:00Z",
        )
        .await
        .unwrap();

        let files = list_thread_files(&kv, tenant, "thread-1").await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "new.csv");
        assert_eq!(files[0].created_at, "2026-03-10T01:00:00Z");
    }

    #[tokio::test]
    async fn delete_thread_files_removes_index_and_file_payloads() {
        let kv = DashMapKVStore::new();
        let tenant = "tenant-1";

        put_stored_file(
            &kv,
            tenant,
            "file-1",
            &StoredFile {
                content: "hello".to_string(),
                filename: "hello.txt".to_string(),
            },
        )
        .await
        .unwrap();
        register_thread_file_at(
            &kv,
            tenant,
            "file-1",
            "hello.txt",
            "thread-1",
            "2026-03-10T00:00:00Z",
        )
        .await
        .unwrap();

        delete_thread_files(&kv, tenant, "thread-1").await.unwrap();
        assert!(get_stored_file(&kv, tenant, "file-1")
            .await
            .unwrap()
            .is_none());
        assert!(list_thread_files(&kv, tenant, "thread-1")
            .await
            .unwrap()
            .is_empty());
    }
}
