//! KV-backed document and knowledge-item persistence helpers.
//!
//! These helpers centralize the framework-owned corpus layout for workspace
//! knowledge/documents so consuming apps do not have to duplicate:
//! - key prefixes
//! - index keys
//! - list/get/upsert/delete plumbing

use crate::knowledge_extraction::KnowledgePersistencePolicy;
use crate::knowledge_ingestion::KnowledgeIngestionPersistencePolicy;
use agent_fw_algebra::KVStore;
use agent_fw_catalog::{DocumentItem, KnowledgeItem};
use agent_fw_workspace::{EntityConfig, IndexedEntity, IndexedEntityError};

/// KV layout for workspace-backed documents.
pub const DOCUMENT_CONFIG: EntityConfig = EntityConfig {
    key_prefix: "data:document:",
    index_key: "data:documents:index",
};

/// KV layout for workspace-backed knowledge items.
pub const KNOWLEDGE_CONFIG: EntityConfig = EntityConfig {
    key_prefix: "data:knowledge:",
    index_key: "data:knowledge:index",
};

/// KV key for the workspace-local document content-hash index.
pub const WORKSPACE_DOCUMENT_HASH_INDEX_KEY: &str = "data:knowledge:content_hashes";

/// Resolve the canonical KV key for a document.
pub fn document_key(id: &str) -> String {
    DOCUMENT_CONFIG.entity_key(id)
}

/// Resolve the canonical KV key for a knowledge item.
pub fn knowledge_key(id: &str) -> String {
    KNOWLEDGE_CONFIG.entity_key(id)
}

/// Standard persistence policy for workspace-local knowledge extraction.
pub fn workspace_extraction_persistence() -> KnowledgePersistencePolicy {
    KnowledgePersistencePolicy::default()
        .with_knowledge_key_prefix(KNOWLEDGE_CONFIG.key_prefix)
        .with_knowledge_index_key(KNOWLEDGE_CONFIG.index_key)
        .prepend_knowledge_index_entries()
        .with_document_key_prefix(DOCUMENT_CONFIG.key_prefix)
        .with_embedded_document_knowledge_ids()
}

/// Standard persistence policy for workspace-local knowledge ingestion.
pub fn workspace_ingestion_persistence() -> KnowledgeIngestionPersistencePolicy {
    KnowledgeIngestionPersistencePolicy::default()
        .with_hash_index_key(WORKSPACE_DOCUMENT_HASH_INDEX_KEY)
        .with_document_key_prefix(DOCUMENT_CONFIG.key_prefix)
        .with_document_index_key(DOCUMENT_CONFIG.index_key)
        .prepend_document_index_entries()
}

/// List all KV-backed documents for a tenant.
pub async fn list_documents<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
) -> Result<Vec<DocumentItem>, IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &DOCUMENT_CONFIG)
        .list()
        .await
}

/// List all KV-backed knowledge items for a tenant.
pub async fn list_knowledge_items<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
) -> Result<Vec<KnowledgeItem>, IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &KNOWLEDGE_CONFIG)
        .list()
        .await
}

/// Get an optional KV-backed document by ID.
pub async fn get_document<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    document_id: &str,
) -> Result<Option<DocumentItem>, IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &DOCUMENT_CONFIG)
        .get(document_id)
        .await
}

/// Get an optional KV-backed knowledge item by ID.
pub async fn get_knowledge_item<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    item_id: &str,
) -> Result<Option<KnowledgeItem>, IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &KNOWLEDGE_CONFIG)
        .get(item_id)
        .await
}

/// Insert or update a KV-backed document.
pub async fn upsert_document<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    document: &DocumentItem,
) -> Result<(), IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &DOCUMENT_CONFIG)
        .put(&document.id, document)
        .await
}

/// Insert or update a KV-backed knowledge item.
pub async fn upsert_knowledge_item<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    item: &KnowledgeItem,
) -> Result<(), IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &KNOWLEDGE_CONFIG)
        .put(&item.id, item)
        .await
}

/// Delete a KV-backed document by ID.
pub async fn delete_document<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    document_id: &str,
) -> Result<(), IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &DOCUMENT_CONFIG)
        .delete(document_id)
        .await
        .map(|_| ())
}

/// Delete a KV-backed knowledge item by ID.
pub async fn delete_knowledge_item<K: KVStore + ?Sized>(
    kv: &K,
    tenant_id: &str,
    item_id: &str,
) -> Result<(), IndexedEntityError> {
    IndexedEntity::new(kv, tenant_id, &KNOWLEDGE_CONFIG)
        .delete(item_id)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{ExtractionStatus, KnowledgeType};
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn document_helpers_roundtrip() {
        let kv = DashMapKVStore::new();
        let tenant = "tenant-1";
        let document = DocumentItem {
            id: "doc-1".to_string(),
            name: "Doc".to_string(),
            content: "Content".to_string(),
            target_database_id: Some("db-1".to_string()),
            extraction_status: ExtractionStatus::Pending,
            extracted_knowledge_ids: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        upsert_document(&kv, tenant, &document).await.unwrap();

        let listed = list_documents(&kv, tenant).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "doc-1");
        assert_eq!(document_key("doc-1"), "data:document:doc-1");

        let fetched = get_document(&kv, tenant, "doc-1").await.unwrap();
        assert_eq!(fetched.as_ref().map(|doc| doc.name.as_str()), Some("Doc"));

        delete_document(&kv, tenant, "doc-1").await.unwrap();
        assert!(get_document(&kv, tenant, "doc-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn knowledge_helpers_roundtrip() {
        let kv = DashMapKVStore::new();
        let tenant = "tenant-1";
        let item = KnowledgeItem {
            id: "k-1".to_string(),
            name: "Revenue Rule".to_string(),
            description: "Revenue equals price times quantity.".to_string(),
            knowledge_type: KnowledgeType::BusinessRule,
            scope_tables: vec!["fact_sales".to_string()],
            scope_columns: vec!["price".to_string()],
            sql_expression: None,
            synonyms: vec!["sales".to_string()],
            source_document_id: Some("doc-1".to_string()),
        };

        upsert_knowledge_item(&kv, tenant, &item).await.unwrap();

        let listed = list_knowledge_items(&kv, tenant).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "k-1");
        assert_eq!(knowledge_key("k-1"), "data:knowledge:k-1");

        let fetched = get_knowledge_item(&kv, tenant, "k-1").await.unwrap();
        assert_eq!(
            fetched.as_ref().map(|knowledge| knowledge.name.as_str()),
            Some("Revenue Rule")
        );

        delete_knowledge_item(&kv, tenant, "k-1").await.unwrap();
        assert!(get_knowledge_item(&kv, tenant, "k-1")
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn workspace_persistence_presets_match_workspace_corpus_layout() {
        let extraction = workspace_extraction_persistence();
        assert_eq!(extraction.knowledge_key("k-1"), knowledge_key("k-1"));
        assert_eq!(extraction.document_key("doc-1"), document_key("doc-1"));
        assert!(extraction.document_knowledge_ids_key("doc-1").is_none());

        let ingestion = workspace_ingestion_persistence();
        assert_eq!(
            ingestion.hash_index_key(),
            WORKSPACE_DOCUMENT_HASH_INDEX_KEY
        );
        assert_eq!(ingestion.document_key("doc-1"), document_key("doc-1"));
        assert_eq!(
            ingestion.document_index_key(),
            Some(DOCUMENT_CONFIG.index_key)
        );
    }
}
