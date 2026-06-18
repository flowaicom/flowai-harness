use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_fw_algebra::KVStoreExt;
use agent_fw_catalog::{
    decode_metadata,
    knowledge::{DocumentItem, ExtractionStatus, KnowledgeItem},
    relation_kind, CatalogEntry, CatalogKind, CatalogScope, DocumentMetadata, EnrichmentError,
    EnrichmentResult, KnowledgeExtractionRequest, KnowledgeMetadata, SemanticEnricher,
    TableEnrichmentRequest,
};
use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_ingest::builder::generate_catalog_id;
use agent_fw_interpreter::MockEnricher;
use flowai_runtime::data::{
    ingest_knowledge, IngestKnowledgeCommand, KnowledgeCommandDeps, KnowledgeIngestEvent,
    KnowledgeSourceSpec,
};
use flowai_runtime::storage::{
    build_catalog_for_scope, build_kv_store_from_environment,
    open_writable_catalog_from_environment_for_scope, CatalogStorageConfig, DataEnvironmentConfig,
    KvStorageConfig, TargetDatabaseStorageConfig,
};
use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

struct FailingKnowledgeEnricher;

#[async_trait::async_trait]
impl SemanticEnricher for FailingKnowledgeEnricher {
    async fn enrich_table(
        &self,
        _request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        unreachable!("knowledge ingest test should not enrich tables")
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Err(EnrichmentError::ParseFailed(
            "missing field `knowledgeType`".to_string(),
        ))
    }
}

struct RichKnowledgeEnricher;

#[async_trait::async_trait]
impl SemanticEnricher for RichKnowledgeEnricher {
    async fn enrich_table(
        &self,
        _request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        unreachable!("knowledge ingest test should not enrich tables")
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Ok(vec![KnowledgeItem {
            id: "rich-k-0".to_string(),
            name: "Slow mover threshold".to_string(),
            description: "Slow movers use a velocity ratio below 0.25.".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::Constraint,
            scope_tables: vec!["public.fact_scenario".to_string()],
            scope_columns: vec!["public.fact_scenario.velocity_ratio".to_string()],
            sql_expression: Some("velocity_ratio < 0.25".to_string()),
            synonyms: vec![
                "slow mover cutoff".to_string(),
                "velocity threshold".to_string(),
            ],
            source_document_id: None,
        }])
    }
}

struct UnqualifiedKnowledgeEnricher;

#[async_trait::async_trait]
impl SemanticEnricher for UnqualifiedKnowledgeEnricher {
    async fn enrich_table(
        &self,
        _request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        unreachable!("knowledge ingest test should not enrich tables")
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Ok(vec![KnowledgeItem {
            id: "unqualified-k-0".to_string(),
            name: "Slow mover threshold".to_string(),
            description: "Slow movers use a velocity ratio below 0.25.".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::Constraint,
            scope_tables: vec!["fact_scenario".to_string()],
            scope_columns: vec!["fact_scenario.velocity_ratio".to_string()],
            sql_expression: Some("velocity_ratio < 0.25".to_string()),
            synonyms: vec!["slow mover cutoff".to_string()],
            source_document_id: None,
        }])
    }
}

struct DynamicUnqualifiedKnowledgeEnricher {
    table_name: String,
    column_name: String,
}

#[async_trait::async_trait]
impl SemanticEnricher for DynamicUnqualifiedKnowledgeEnricher {
    async fn enrich_table(
        &self,
        _request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        unreachable!("knowledge ingest test should not enrich tables")
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Ok(vec![KnowledgeItem {
            id: "dynamic-unqualified-k-0".to_string(),
            name: "Unqualified slow mover threshold".to_string(),
            description: "Slow movers use a velocity ratio below 0.25.".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::Constraint,
            scope_tables: vec![self.table_name.clone()],
            scope_columns: vec![format!("{}.{}", self.table_name, self.column_name)],
            sql_expression: Some(format!("{} < 0.25", self.column_name)),
            synonyms: vec!["slow mover cutoff".to_string()],
            source_document_id: None,
        }])
    }
}

#[tokio::test]
async fn knowledge_ingest_persists_documents_and_hash_index_into_sqlite_kv() {
    let kv_path = temp_sqlite_path("knowledge-kv");
    let dir = temp_directory("knowledge-docs");
    fs::write(dir.join("doc1.md"), "# Document 1\nRevenue is sum(amount).").unwrap();
    fs::write(dir.join("doc2.txt"), "Orders drive revenue.").unwrap();
    fs::write(dir.join("ignored.rs"), "fn main() {}").unwrap();

    let command = IngestKnowledgeCommand {
        data_environment: data_environment_with_sqlite_kv(&kv_path),
        tenant_id: "acme".to_string(),
        workspace_id: None,
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string(), "txt".to_string()],
        },
        extract_knowledge: false,
    };

    let mut handle = ingest_knowledge(command.clone(), KnowledgeCommandDeps::new())
        .await
        .unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(events
        .iter()
        .any(|event| matches!(event, KnowledgeIngestEvent::Discovered { total: 2 })));
    let summary = completed_summary(&events);
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.new, 2);
    assert_eq!(summary.skipped_duplicate, 0);
    assert!(summary.errors.is_empty());

    let kv = build_kv_store_from_environment(&command.data_environment)
        .await
        .unwrap();
    let document_keys = kv.list_keys("acme", "data:document:").await.unwrap();
    assert_eq!(document_keys.len(), 2);
    assert!(kv
        .exists("acme", "data:knowledge:content_hashes")
        .await
        .unwrap());
    assert!(kv
        .list_keys("acme", "knowledge:doc:")
        .await
        .unwrap()
        .is_empty());

    cleanup_files(&[kv_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_with_extraction_embeds_knowledge_ids_into_documents() {
    let kv_path = temp_sqlite_path("knowledge-extract-kv");
    let target_path = temp_sqlite_path("knowledge-extract-target");
    let dir = temp_directory("knowledge-extract-docs");
    seed_target_database(&target_path);
    fs::write(
        dir.join("metrics.md"),
        "# Metrics\nRevenue is calculated as SUM(orders.quantity).",
    )
    .unwrap();

    let command = IngestKnowledgeCommand {
        data_environment: DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: None,
            kv: Some(KvStorageConfig::Sqlite {
                url: sqlite_url(&kv_path),
                ensure_schema: true,
            }),
            catalog: None,
            catalog_search: None,
            target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                url: sqlite_url(&target_path),
            }),
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
        tenant_id: "acme".to_string(),
        workspace_id: None,
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(MockEnricher::new()));

    let mut handle = ingest_knowledge(command.clone(), deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Error { .. })),
        "unexpected knowledge ingest errors: {events:?}"
    );
    let summary = completed_summary(&events);
    assert_eq!(summary.new, 1);
    assert!(summary.errors.is_empty());

    let kv = build_kv_store_from_environment(&command.data_environment)
        .await
        .unwrap();
    let stored_knowledge: KnowledgeItem = kv
        .get("acme", "data:knowledge:mock-k-0")
        .await
        .unwrap()
        .unwrap();
    assert!(stored_knowledge.name.contains("metrics.md"));

    let document_keys = kv.list_keys("acme", "data:document:").await.unwrap();
    assert_eq!(document_keys.len(), 1);
    let stored_document: DocumentItem = kv.get("acme", &document_keys[0]).await.unwrap().unwrap();
    assert_eq!(
        stored_document.extraction_status,
        ExtractionStatus::Processed
    );
    assert_eq!(stored_document.extracted_knowledge_ids, vec!["mock-k-0"]);

    cleanup_files(&[kv_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_failure_does_not_project_partial_document_to_catalog() {
    let kv_path = temp_sqlite_path("knowledge-atomic-error-kv");
    let catalog_path = temp_sqlite_path("knowledge-atomic-error-catalog");
    let target_path = temp_sqlite_path("knowledge-atomic-error-target");
    let dir = temp_directory("knowledge-atomic-error-docs");
    seed_target_database(&target_path);
    fs::write(dir.join("metrics.md"), "# Metrics\nRevenue guidance.").unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(&target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(FailingKnowledgeEnricher));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Completed(_))),
        "failed extraction must not emit a completed event: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            KnowledgeIngestEvent::Error { message } if message.contains("Extraction failed")
        )),
        "expected extraction error event, got {events:?}"
    );

    let kv = build_kv_store_from_environment(&data_environment)
        .await
        .unwrap();
    assert!(kv
        .list_keys("acme::workspace:analytics", "data:document:")
        .await
        .unwrap()
        .is_empty());
    assert!(!kv
        .exists("acme::workspace:analytics", "data:knowledge:content_hashes")
        .await
        .unwrap());

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("analytics"),
        ),
    )
    .await
    .unwrap();
    assert!(catalog
        .list_by_type(CatalogKind::Document, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap()
        .is_empty());

    cleanup_files(&[kv_path, catalog_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_projects_documents_and_extracted_items_into_catalog() {
    let kv_path = temp_sqlite_path("knowledge-project-kv");
    let catalog_path = temp_sqlite_path("knowledge-project-catalog");
    let target_path = temp_sqlite_path("knowledge-project-target");
    let dir = temp_directory("knowledge-project-docs");
    seed_target_database(&target_path);
    fs::write(
        dir.join("metrics.md"),
        "# Metrics\nRevenue is calculated as SUM(orders.quantity).",
    )
    .unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(&target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    seed_fact_scenario_catalog_targets(&data_environment).await;
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(RichKnowledgeEnricher));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Error { .. })),
        "unexpected knowledge ingest errors: {events:?}"
    );
    assert_eq!(completed_summary(&events).new, 1);

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("analytics"),
        ),
    )
    .await
    .unwrap();
    let documents = catalog
        .list_by_type(CatalogKind::Document, 10)
        .await
        .unwrap();
    assert_eq!(documents.len(), 1);
    assert!(
        documents[0].content.is_empty(),
        "catalog Document content should remain empty until a real summary is projected"
    );
    let kv = build_kv_store_from_environment(&data_environment)
        .await
        .unwrap();
    let document_keys = kv
        .list_keys("acme::workspace:analytics", "data:document:")
        .await
        .unwrap();
    assert_eq!(document_keys.len(), 1);
    let stored_document: DocumentItem = kv
        .get("acme::workspace:analytics", &document_keys[0])
        .await
        .unwrap()
        .unwrap();
    let document_metadata = decode_metadata::<DocumentMetadata>(&documents[0]).unwrap();
    assert_eq!(document_metadata.source_document_id, stored_document.id);
    assert!(document_metadata.content_available);
    assert_eq!(document_metadata.content_source.as_deref(), Some("kv"));
    assert_eq!(
        document_metadata.extraction_status.as_deref(),
        Some("processed")
    );
    assert_eq!(document_metadata.extracted_knowledge_ids, vec!["rich-k-0"]);
    assert_eq!(
        stored_document.content,
        "# Metrics\nRevenue is calculated as SUM(orders.quantity)."
    );

    let knowledge = catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap();
    assert_eq!(knowledge.len(), 1);
    let knowledge_metadata = decode_metadata::<KnowledgeMetadata>(&knowledge[0]).unwrap();
    assert_eq!(
        knowledge_metadata.source_knowledge_id.as_deref(),
        Some("rich-k-0")
    );
    assert_eq!(
        knowledge_metadata.source_document_id,
        Some(document_metadata.source_document_id.clone())
    );
    assert_eq!(
        knowledge_metadata.knowledge_type.as_deref(),
        Some("constraint")
    );
    assert_eq!(
        knowledge_metadata.scope_tables,
        vec!["public.fact_scenario".to_string()]
    );
    assert_eq!(
        knowledge_metadata.scope_columns,
        vec!["public.fact_scenario.velocity_ratio".to_string()]
    );
    assert_eq!(
        knowledge_metadata.sql_expression.as_deref(),
        Some("velocity_ratio < 0.25")
    );
    assert_eq!(
        knowledge_metadata.synonyms,
        vec![
            "slow mover cutoff".to_string(),
            "velocity threshold".to_string()
        ]
    );
    let stored_knowledge: KnowledgeItem = kv
        .get("acme::workspace:analytics", "data:knowledge:rich-k-0")
        .await
        .unwrap()
        .unwrap();
    assert!(
        knowledge[0].content == stored_knowledge.description,
        "catalog Knowledge content should carry agent-facing knowledge details"
    );
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::EXTRACTED_FROM && relation.target_id == documents[0].id
    }));
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(
                    CatalogKind::Table,
                    "warehouse",
                    &["public", "fact_scenario"],
                )
    }));
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(
                    CatalogKind::Column,
                    "warehouse",
                    &["public", "fact_scenario", "velocity_ratio"],
                )
    }));

    cleanup_files(&[kv_path, catalog_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_projects_unqualified_sqlite_scopes_to_unique_catalog_targets() {
    let kv_path = temp_sqlite_path("knowledge-project-unqualified-kv");
    let catalog_path = temp_sqlite_path("knowledge-project-unqualified-catalog");
    let target_path = temp_sqlite_path("knowledge-project-unqualified-target");
    let dir = temp_directory("knowledge-project-unqualified-docs");
    seed_target_database(&target_path);
    fs::write(dir.join("metrics.md"), "# Metrics\nRevenue guidance.").unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(&target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    seed_fact_scenario_catalog_targets_for_schema(&data_environment, "main").await;
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(UnqualifiedKnowledgeEnricher));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Error { .. })),
        "unexpected knowledge ingest errors: {events:?}"
    );
    assert_eq!(completed_summary(&events).new, 1);

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("analytics"),
        ),
    )
    .await
    .unwrap();
    let knowledge = catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap();
    assert_eq!(knowledge.len(), 1);
    let knowledge_metadata = decode_metadata::<KnowledgeMetadata>(&knowledge[0]).unwrap();
    assert_eq!(knowledge_metadata.scope_tables, vec!["fact_scenario"]);
    assert_eq!(
        knowledge_metadata.scope_columns,
        vec!["fact_scenario.velocity_ratio"]
    );
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(CatalogKind::Table, "warehouse", &["main", "fact_scenario"])
    }));
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(
                    CatalogKind::Column,
                    "warehouse",
                    &["main", "fact_scenario", "velocity_ratio"],
                )
    }));

    cleanup_files(&[kv_path, catalog_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_rejects_ambiguous_unqualified_sqlite_scope_targets() {
    let kv_path = temp_sqlite_path("knowledge-project-ambiguous-kv");
    let catalog_path = temp_sqlite_path("knowledge-project-ambiguous-catalog");
    let target_path = temp_sqlite_path("knowledge-project-ambiguous-target");
    let dir = temp_directory("knowledge-project-ambiguous-docs");
    seed_target_database(&target_path);
    fs::write(dir.join("metrics.md"), "# Metrics\nRevenue guidance.").unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(&target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    seed_fact_scenario_catalog_targets_for_schema(&data_environment, "main").await;
    seed_fact_scenario_catalog_targets_for_schema(&data_environment, "analytics").await;
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(UnqualifiedKnowledgeEnricher));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Completed(_))),
        "ambiguous schema targets must not emit completed: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            KnowledgeIngestEvent::Error { message } if message.contains("ambiguous scope target")
        )),
        "expected ambiguous scope target error, got {events:?}"
    );

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("analytics"),
        ),
    )
    .await
    .unwrap();
    assert!(catalog
        .list_by_type(CatalogKind::Document, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap()
        .is_empty());

    cleanup_files(&[kv_path, catalog_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn env_gated_postgres_knowledge_ingest_projects_unqualified_scope_relations() {
    let Some(kv_url) = env_url("FLOWAI_TEST_POSTGRES_KV_URL") else {
        return;
    };
    let Some(catalog_url) = env_url("FLOWAI_TEST_POSTGRES_CATALOG_URL") else {
        return;
    };
    let Some(target_url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
        return;
    };

    let schema_name = unique_name("flowai_knowledge");
    let table_name = unique_name("fact_scenario");
    let kv_table = unique_name("flowai_kv");
    let column_name = "velocity_ratio".to_string();
    let tenant_id = unique_name("acme");
    let workspace_id = unique_name("analytics");
    let dir = temp_directory("knowledge-project-postgres-docs");
    fs::write(dir.join("metrics.md"), "# Metrics\nRevenue guidance.").unwrap();

    let target_pool = sqlx::PgPool::connect(&target_url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema_name}"))
        .execute(&target_pool)
        .await
        .unwrap();
    sqlx::query(&format!(
        "CREATE TABLE {schema_name}.{table_name} (id INTEGER PRIMARY KEY, {column_name} DOUBLE PRECISION NOT NULL)"
    ))
    .execute(&target_pool)
    .await
    .unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked(tenant_id.clone())),
        workspace_id: Some(WorkspaceId::new_unchecked(workspace_id.clone())),
        kv: Some(KvStorageConfig::Postgres {
            url: Some(kv_url.clone()),
            url_env: None,
            table: Some(kv_table.clone()),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Postgres {
            url: Some(catalog_url.clone()),
            url_env: None,
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Postgres {
            url: Some(target_url.clone()),
            url_env: None,
            schema: Some(schema_name.clone()),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    seed_catalog_targets(&data_environment, &schema_name, &table_name, &column_name).await;
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: tenant_id.clone(),
        workspace_id: Some(WorkspaceId::new_unchecked(workspace_id.clone())),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps =
        KnowledgeCommandDeps::new().with_enricher(Arc::new(DynamicUnqualifiedKnowledgeEnricher {
            table_name: table_name.clone(),
            column_name: column_name.clone(),
        }));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Error { .. })),
        "unexpected postgres knowledge ingest errors: {events:?}"
    );
    assert_eq!(completed_summary(&events).new, 1);

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked(tenant_id.clone()),
            WorkspaceId::new_unchecked(workspace_id.clone()),
        ),
    )
    .await
    .unwrap();
    let knowledge = catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap();
    assert_eq!(knowledge.len(), 1);
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(
                    CatalogKind::Table,
                    "warehouse",
                    &[schema_name.as_str(), table_name.as_str()],
                )
    }));
    assert!(knowledge[0].links.iter().any(|relation| {
        relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
            && relation.target_id
                == generate_catalog_id(
                    CatalogKind::Column,
                    "warehouse",
                    &[
                        schema_name.as_str(),
                        table_name.as_str(),
                        column_name.as_str(),
                    ],
                )
    }));

    let catalog_pool = sqlx::PgPool::connect(&catalog_url).await.unwrap();
    sqlx::query("DELETE FROM catalog_relations WHERE tenant_id = $1 AND workspace_id = $2")
        .bind(&tenant_id)
        .bind(&workspace_id)
        .execute(&catalog_pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM catalog_entries WHERE tenant_id = $1 AND workspace_id = $2")
        .bind(&tenant_id)
        .bind(&workspace_id)
        .execute(&catalog_pool)
        .await
        .unwrap();
    let kv_pool = sqlx::PgPool::connect(&kv_url).await.unwrap();
    sqlx::query(&format!("DROP TABLE IF EXISTS {kv_table}"))
        .execute(&kv_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE"))
        .execute(&target_pool)
        .await
        .unwrap();
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_does_not_complete_when_scope_targets_are_missing_from_catalog() {
    let kv_path = temp_sqlite_path("knowledge-project-missing-target-kv");
    let catalog_path = temp_sqlite_path("knowledge-project-missing-target-catalog");
    let target_path = temp_sqlite_path("knowledge-project-missing-target-target");
    let dir = temp_directory("knowledge-project-missing-target-docs");
    seed_target_database(&target_path);
    fs::write(
        dir.join("metrics.md"),
        "# Metrics\nRevenue is calculated as SUM(orders.quantity).",
    )
    .unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(&target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let command = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: true,
    };
    let deps = KnowledgeCommandDeps::new().with_enricher(Arc::new(RichKnowledgeEnricher));

    let mut handle = ingest_knowledge(command, deps).await.unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Completed(_))),
        "missing schema targets must not emit completed: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            KnowledgeIngestEvent::Error { message } if message.contains("missing scope targets")
        )),
        "expected missing scope target error, got {events:?}"
    );

    let catalog = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("analytics"),
        ),
    )
    .await
    .unwrap();
    assert!(catalog
        .list_by_type(CatalogKind::Document, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(catalog
        .list_by_type(CatalogKind::Knowledge, 10)
        .await
        .unwrap()
        .is_empty());

    cleanup_files(&[kv_path, catalog_path, target_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_does_not_complete_when_catalog_projection_fails() {
    let kv_path = temp_sqlite_path("knowledge-project-error-kv");
    let catalog_path = temp_sqlite_path("knowledge-project-error-catalog");
    let dir = temp_directory("knowledge-project-error-docs");
    fs::write(dir.join("doc.md"), "# Metrics\nRevenue guidance.").unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: None,
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let kv = build_kv_store_from_environment(&data_environment)
        .await
        .unwrap();
    kv.put_json(
        "acme",
        "data:documents:index",
        json!({"ids": "not-a-list"}),
        None,
    )
    .await
    .unwrap();

    let command = IngestKnowledgeCommand {
        data_environment,
        tenant_id: "acme".to_string(),
        workspace_id: None,
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: false,
    };

    let mut handle = ingest_knowledge(command, KnowledgeCommandDeps::new())
        .await
        .unwrap();
    let events = drain_events(&mut handle.events).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, KnowledgeIngestEvent::Completed(_))),
        "projection failure must not be reported after a completed event: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            KnowledgeIngestEvent::Error { message } if message.contains("Corrupt hash index")
        )),
        "expected corrupt canonical index error, got {events:?}"
    );

    cleanup_files(&[kv_path, catalog_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_catalog_projection_ids_are_scoped_by_workspace() {
    let kv_path = temp_sqlite_path("knowledge-scope-id-kv");
    let catalog_path = temp_sqlite_path("knowledge-scope-id-catalog");
    let dir = temp_directory("knowledge-scope-id-docs");
    fs::write(dir.join("doc.md"), "# Shared\nWorkspace scoped content.").unwrap();

    let data_environment = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("acme")),
        workspace_id: None,
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(&kv_path),
            ensure_schema: true,
        }),
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let command_a = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: false,
    };
    let mut handle_a = ingest_knowledge(command_a, KnowledgeCommandDeps::new())
        .await
        .unwrap();
    assert_eq!(
        completed_summary(&drain_events(&mut handle_a.events).await).new,
        1
    );

    let command_b = IngestKnowledgeCommand {
        data_environment: data_environment.clone(),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-b")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: false,
    };
    let mut handle_b = ingest_knowledge(command_b, KnowledgeCommandDeps::new())
        .await
        .unwrap();
    assert_eq!(
        completed_summary(&drain_events(&mut handle_b.events).await).new,
        1
    );

    let catalog_a = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("workspace-a"),
        ),
    )
    .await
    .unwrap();
    let catalog_b = build_catalog_for_scope(
        data_environment.catalog.clone().unwrap(),
        CatalogScope::new(
            TenantId::new_unchecked("acme"),
            WorkspaceId::new_unchecked("workspace-b"),
        ),
    )
    .await
    .unwrap();
    let document_a = catalog_a
        .list_by_type(CatalogKind::Document, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let document_b = catalog_b
        .list_by_type(CatalogKind::Document, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();

    assert_ne!(
        document_a.id, document_b.id,
        "projected document ids should include the catalog tenant/workspace scope"
    );

    cleanup_files(&[kv_path, catalog_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_scopes_persistence_by_workspace() {
    let kv_path = temp_sqlite_path("knowledge-workspace-kv");
    let dir = temp_directory("knowledge-workspace-docs");
    fs::write(dir.join("doc.md"), "# Shared\nWorkspace scoped content.").unwrap();

    let command_a = IngestKnowledgeCommand {
        data_environment: data_environment_with_sqlite_kv(&kv_path),
        tenant_id: "acme".to_string(),
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
        database_id: "warehouse".to_string(),
        source: KnowledgeSourceSpec::LocalDirectory {
            path: dir.clone(),
            extensions: vec!["md".to_string()],
        },
        extract_knowledge: false,
    };

    let mut handle_a = ingest_knowledge(command_a.clone(), KnowledgeCommandDeps::new())
        .await
        .unwrap();
    let events_a = drain_events(&mut handle_a.events).await;
    assert_eq!(completed_summary(&events_a).new, 1);

    let command_b = IngestKnowledgeCommand {
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-b")),
        ..command_a.clone()
    };
    let mut handle_b = ingest_knowledge(command_b.clone(), KnowledgeCommandDeps::new())
        .await
        .unwrap();
    let events_b = drain_events(&mut handle_b.events).await;
    assert_eq!(
        completed_summary(&events_b).new,
        1,
        "same document content should not dedupe across workspaces"
    );

    let kv = build_kv_store_from_environment(&command_a.data_environment)
        .await
        .unwrap();
    assert!(kv
        .list_keys("acme", "data:document:")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        kv.list_keys("acme::workspace:workspace-a", "data:document:")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        kv.list_keys("acme::workspace:workspace-b", "data:document:")
            .await
            .unwrap()
            .len(),
        1
    );

    cleanup_files(&[kv_path]);
    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_requires_nonblank_database_id() {
    let dir = temp_directory("knowledge-blank-database");
    fs::write(dir.join("doc1.md"), "hello").unwrap();

    let err = ingest_knowledge(
        IngestKnowledgeCommand {
            data_environment: DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: None,
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
            tenant_id: "acme".to_string(),
            workspace_id: None,
            database_id: "   ".to_string(),
            source: KnowledgeSourceSpec::LocalDirectory {
                path: dir.clone(),
                extensions: vec!["md".to_string()],
            },
            extract_knowledge: false,
        },
        KnowledgeCommandDeps::new(),
    )
    .await
    .err()
    .expect("blank database id should fail");

    assert!(err
        .to_string()
        .contains("knowledge ingestion database_id must not be blank"));

    cleanup_dir(&dir);
}

#[tokio::test]
async fn knowledge_ingest_requires_kv_configuration() {
    let dir = temp_directory("knowledge-no-kv");
    fs::write(dir.join("doc1.md"), "hello").unwrap();

    let err = ingest_knowledge(
        IngestKnowledgeCommand {
            data_environment: DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: None,
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
            tenant_id: "acme".to_string(),
            workspace_id: None,
            database_id: "warehouse".to_string(),
            source: KnowledgeSourceSpec::LocalDirectory {
                path: dir.clone(),
                extensions: vec!["md".to_string()],
            },
            extract_knowledge: false,
        },
        KnowledgeCommandDeps::new(),
    )
    .await
    .err()
    .expect("missing kv config should fail");

    assert!(err
        .to_string()
        .contains("knowledge ingestion requires data_environment.kv"));

    cleanup_dir(&dir);
}

async fn seed_fact_scenario_catalog_targets(data_environment: &DataEnvironmentConfig) {
    seed_fact_scenario_catalog_targets_for_schema(data_environment, "public").await;
}

async fn seed_fact_scenario_catalog_targets_for_schema(
    data_environment: &DataEnvironmentConfig,
    schema_name: &str,
) {
    seed_catalog_targets(
        data_environment,
        schema_name,
        "fact_scenario",
        "velocity_ratio",
    )
    .await;
}

async fn seed_catalog_targets(
    data_environment: &DataEnvironmentConfig,
    schema_name: &str,
    table_name: &str,
    column_name: &str,
) {
    let tenant_id = data_environment
        .tenant_id
        .clone()
        .unwrap_or_else(|| TenantId::new_unchecked("acme"));
    let workspace_id = data_environment
        .workspace_id
        .clone()
        .unwrap_or_else(|| WorkspaceId::new_unchecked("analytics"));
    let opened = open_writable_catalog_from_environment_for_scope(
        data_environment,
        CatalogScope::new(tenant_id, workspace_id),
    )
    .await
    .unwrap();
    opened
        .writer
        .save_items(vec![
            CatalogEntry {
                id: generate_catalog_id(
                    CatalogKind::Table,
                    "warehouse",
                    &[schema_name, table_name],
                ),
                kind: CatalogKind::Table,
                name: table_name.to_string(),
                qualified_name: Some(format!("{schema_name}.{table_name}")),
                content: "Scenario fact table".to_string(),
                tags: vec![],
                links: vec![],
                metadata: json!({
                    "databaseId": "warehouse",
                    "schemaName": schema_name,
                    "tableName": table_name,
                    "relationType": "base_table",
                    "rowCount": null,
                    "columnCount": 1,
                    "preferredQuerySurface": true,
                    "source": {},
                }),
            },
            CatalogEntry {
                id: generate_catalog_id(
                    CatalogKind::Column,
                    "warehouse",
                    &[schema_name, table_name, column_name],
                ),
                kind: CatalogKind::Column,
                name: column_name.to_string(),
                qualified_name: Some(format!("{schema_name}.{table_name}.{column_name}")),
                content: "Velocity ratio".to_string(),
                tags: vec![],
                links: vec![],
                metadata: json!({
                    "databaseId": "warehouse",
                    "schemaName": schema_name,
                    "tableName": table_name,
                    "columnName": column_name,
                    "dataType": "numeric",
                    "nullable": false,
                    "primaryKey": false,
                    "foreignKey": null,
                    "semanticType": null,
                    "distinctCount": null,
                    "nullCount": null,
                    "totalCount": null,
                    "lowCardinalityEnum": false,
                }),
            },
        ])
        .await
        .unwrap();
}

async fn drain_events(
    events: &mut tokio::sync::mpsc::Receiver<KnowledgeIngestEvent>,
) -> Vec<KnowledgeIngestEvent> {
    let mut collected = Vec::new();
    while let Some(event) = events.recv().await {
        collected.push(event);
    }
    collected
}

fn completed_summary(
    events: &[KnowledgeIngestEvent],
) -> &flowai_runtime::data::KnowledgeIngestSummary {
    events
        .iter()
        .find_map(|event| match event {
            KnowledgeIngestEvent::Completed(summary) => Some(summary),
            _ => None,
        })
        .expect("knowledge ingest should emit a completed summary")
}

fn data_environment_with_sqlite_kv(kv_path: &Path) -> DataEnvironmentConfig {
    DataEnvironmentConfig {
        tenant_id: None,
        workspace_id: None,
        kv: Some(KvStorageConfig::Sqlite {
            url: sqlite_url(kv_path),
            ensure_schema: true,
        }),
        catalog: None,
        catalog_search: None,
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    }
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite:{}", path.display())
}

fn temp_sqlite_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.db", Uuid::new_v4()))
}

fn temp_directory(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn env_url(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn unique_name(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}

fn cleanup_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn seed_target_database(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            quantity INTEGER NOT NULL
        );
        INSERT INTO orders (id, quantity) VALUES
            (1, 2),
            (2, 1);
        "#,
    )
    .unwrap();
}
