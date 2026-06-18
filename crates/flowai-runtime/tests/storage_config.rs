use std::env;
use std::pin::Pin;
use std::sync::Arc;

use agent_fw_agent::{ChatInterpreter, ChatProgram, ToolHandler};
use agent_fw_algebra::testing::NullEventSink;
use agent_fw_algebra::CancellationToken;
use agent_fw_catalog::{
    CatalogEntry, CatalogKind, CatalogScope, CatalogSearchBackend, CatalogSearchFilters,
    CatalogSearchHealth, CatalogSearchRequest, CatalogToolEnvironmentExt,
};
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{StreamPart, TenantId, WorkspaceContext, WorkspaceId};
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_tool::ToolEnvironment;
use flowai_runtime::storage::{
    apply_to_runtime_deps, build_catalog, build_catalog_for_scope, build_kv_store,
    build_target_database, build_target_database_from_environment, open_catalog_for_writes,
    open_catalog_for_writes_for_scope, open_writable_catalog_from_environment, redact_url,
    CatalogSearchConfig, CatalogStorageConfig, DataEnvironmentConfig, KvStorageConfig,
    TargetDatabaseStorageConfig,
};
use flowai_runtime::RuntimeDeps;
use serde_json::json;
use uuid::Uuid;

struct NeverUsedInterpreter;

impl ChatInterpreter for NeverUsedInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
        Box::pin(futures::stream::empty())
    }
}

#[test]
fn parses_all_supported_storage_descriptors() {
    let raw = serde_json::json!({
        "kv": {
            "kind": "postgres",
            "urlEnv": "FLOWAI_KV_DATABASE_URL",
            "table": "runtime_kv",
            "ensureSchema": true
        },
        "catalog": {
            "kind": "sqlite",
            "url": "sqlite:/tmp/catalog.db"
        },
        "targetDatabase": {
            "kind": "postgres",
            "urlEnv": "ACME_TARGET_DATABASE_URL",
            "schema": "customer"
        }
    });

    let config: DataEnvironmentConfig = serde_json::from_value(raw).unwrap();

    assert!(matches!(config.kv, Some(KvStorageConfig::Postgres { .. })));
    assert!(matches!(
        config.catalog,
        Some(CatalogStorageConfig::Sqlite { .. })
    ));
    assert!(matches!(
        config.target_database,
        Some(TargetDatabaseStorageConfig::Postgres { .. })
    ));
}

#[test]
fn accepts_legacy_target_database_url_shape() {
    let raw = serde_json::json!({
        "target_database_url": "sqlite:/tmp/acme.db",
        "catalog": {
            "kind": "inline",
            "entries": []
        }
    });

    let config: DataEnvironmentConfig = serde_json::from_value(raw).unwrap();

    assert_eq!(
        config.legacy_target_database_url.as_deref(),
        Some("sqlite:/tmp/acme.db")
    );
    assert!(matches!(
        config.catalog,
        Some(CatalogStorageConfig::Inline { .. })
    ));
}

#[test]
fn accepts_sqlite_kv_and_catalog_ensure_schema_descriptors() {
    let raw = serde_json::json!({
        "kv": {
            "kind": "sqlite",
            "url": "sqlite:/tmp/runtime-kv.db",
            "ensureSchema": true
        },
        "catalog": {
            "kind": "sqlite",
            "url": "sqlite:/tmp/catalog.db",
            "ensureSchema": true
        }
    });

    let config: DataEnvironmentConfig = serde_json::from_value(raw).unwrap();

    assert!(matches!(
        config.kv,
        Some(KvStorageConfig::Sqlite {
            ensure_schema: true,
            ..
        })
    ));
    assert!(matches!(
        config.catalog,
        Some(CatalogStorageConfig::Sqlite {
            ensure_schema: true,
            ..
        })
    ));
}

#[test]
fn parses_catalog_search_config_without_backend_selector() {
    let raw = serde_json::json!({
        "catalog": {
            "kind": "inline",
            "entries": []
        },
        "catalogSearch": {
            "indexPath": "/tmp/flowai-indexes",
            "rebuildOnStart": true,
            "writeThrough": true
        }
    });

    let config: DataEnvironmentConfig = serde_json::from_value(raw).unwrap();

    let search = config
        .catalog_search
        .expect("catalog search config should parse");
    assert_eq!(search.index_path.to_string_lossy(), "/tmp/flowai-indexes");
    assert!(search.rebuild_on_start);
    assert!(search.write_through);

    let raw_with_selector = serde_json::json!({
        "catalogSearch": {
            "indexPath": "/tmp/flowai-indexes",
            "backend": "sqlite_fts"
        }
    });
    let err = serde_json::from_value::<DataEnvironmentConfig>(raw_with_selector).unwrap_err();
    assert!(err.to_string().contains("unknown field"));
    assert!(err.to_string().contains("backend"));
}

#[test]
fn rejects_blank_data_environment_tenant_id() {
    let raw = serde_json::json!({
        "tenantId": "   ",
        "catalog": {
            "kind": "inline",
            "entries": []
        }
    });

    let err = serde_json::from_value::<DataEnvironmentConfig>(raw).unwrap_err();

    assert!(err.to_string().contains("tenant ID must not be blank"));
}

#[test]
fn rejects_blank_data_environment_workspace_id() {
    let raw = serde_json::json!({
        "workspaceId": "   ",
        "catalog": {
            "kind": "inline",
            "entries": []
        }
    });

    let err = serde_json::from_value::<DataEnvironmentConfig>(raw).unwrap_err();

    assert!(err.to_string().contains("workspace ID must not be blank"));
}

#[test]
fn rejects_redis_catalog_descriptor() {
    let raw = serde_json::json!({
        "catalog": {
            "kind": "redis",
            "urlEnv": "FLOWAI_REDIS_URL"
        }
    });

    let err = serde_json::from_value::<DataEnvironmentConfig>(raw).unwrap_err();

    assert!(err.to_string().contains("unknown variant"));
    assert!(err.to_string().contains("redis"));
}

#[tokio::test]
async fn rejects_invalid_postgres_kv_table_before_opening_connection() {
    let err = build_kv_store(KvStorageConfig::Postgres {
        url: Some("postgresql://user:secret@example.invalid/db".to_string()),
        url_env: None,
        table: Some("kv-store".to_string()),
        ensure_schema: false,
    })
    .await
    .err()
    .expect("invalid table should fail");

    let message = err.to_string();
    assert!(message.contains("data_environment.kv.table"));
    assert!(message.contains("[a-zA-Z0-9_]"));
    assert!(!message.contains("secret"));
    assert!(!message.contains("example.invalid"));
}

#[tokio::test]
async fn rejects_invalid_postgres_target_schema_before_opening_connection() {
    let err = build_target_database(TargetDatabaseStorageConfig::Postgres {
        url: Some("postgresql://user:secret@example.invalid/db".to_string()),
        url_env: None,
        schema: Some("public;drop".to_string()),
    })
    .await
    .err()
    .expect("invalid schema should fail");

    let message = err.to_string();
    assert!(message.contains("data_environment.target_database.schema"));
    assert!(message.contains("[a-zA-Z0-9_]"));
    assert!(!message.contains("secret"));
    assert!(!message.contains("example.invalid"));
}

#[test]
fn redacts_connection_string_credentials_and_secret_query_params() {
    let redacted = redact_url(
        "postgresql://user:p%40ss@host.example.com/db?sslmode=require&password=secret&apikey=abc",
    );

    assert_eq!(
        redacted,
        "postgresql://user:***@host.example.com/db?sslmode=require&password=***&apikey=***"
    );
    assert!(!redacted.contains("p%40ss"));
    assert!(!redacted.contains("secret"));
    assert!(!redacted.contains("abc"));
}

#[test]
fn redacts_secret_query_params_case_insensitively() {
    let redacted = redact_url(
        "postgresql://user:secret@host.example.com/db?Password=secret&Api_Key=abc&TOKEN=xyz",
    );

    assert_eq!(
        redacted,
        "postgresql://user:***@host.example.com/db?Password=***&Api_Key=***&TOKEN=***"
    );
    assert!(!redacted.contains("secret"));
    assert!(!redacted.contains("abc"));
    assert!(!redacted.contains("xyz"));
}

#[tokio::test]
async fn env_gated_postgres_target_database_uses_schema() {
    let Some(url) = env_url("FLOWAI_TEST_POSTGRES_TARGET_URL") else {
        return;
    };
    let schema = unique_name("flowai_target");
    let table = unique_name("items");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!(
        "CREATE TABLE {schema}.{table} (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "INSERT INTO {schema}.{table} (id, name) VALUES (1, 'Tea')"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let db = build_target_database(TargetDatabaseStorageConfig::Postgres {
        url: None,
        url_env: Some("FLOWAI_TEST_POSTGRES_TARGET_URL".to_string()),
        schema: Some(schema.clone()),
    })
    .await
    .unwrap();

    let sample = db.sample_table(&table, 10).await.unwrap();
    assert_eq!(sample, vec![json!({"id": 1, "name": "Tea"})]);
    let rows = db.list_tables().await.unwrap();
    assert!(rows
        .iter()
        .any(|row| row.as_map().get("table_name") == Some(&json!(table))));

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn env_gated_postgres_kv_can_initialize_and_roundtrip() {
    let Some(url) = env_url("FLOWAI_TEST_POSTGRES_KV_URL") else {
        return;
    };
    let table = unique_name("flowai_kv");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let store = build_kv_store(KvStorageConfig::Postgres {
        url: None,
        url_env: Some("FLOWAI_TEST_POSTGRES_KV_URL".to_string()),
        table: Some(table.clone()),
        ensure_schema: true,
    })
    .await
    .unwrap();

    store
        .put_json("tenant-a", "key-a", json!({"value": 42}), None)
        .await
        .unwrap();
    assert_eq!(
        store.get_json("tenant-a", "key-a").await.unwrap(),
        Some(json!({"value": 42}))
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn env_gated_postgres_catalog_can_initialize_and_lookup_exact_names() {
    let Some(url) = env_url("FLOWAI_TEST_POSTGRES_CATALOG_URL") else {
        return;
    };
    let catalog = build_catalog(CatalogStorageConfig::Postgres {
        url: None,
        url_env: Some("FLOWAI_TEST_POSTGRES_CATALOG_URL".to_string()),
        ensure_schema: true,
    })
    .await
    .unwrap();

    let id = format!("table:{}", unique_name("products"));
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(
        r#"
        INSERT INTO catalog_entries (id, kind, name, qualified_name, content, tags, metadata)
        VALUES ($1, 'table', $2, $3, 'Premium tea revenue data', '["sales"]'::jsonb, '{}'::jsonb)
        "#,
    )
    .bind(&id)
    .bind("premium_tea_products")
    .bind("public.premium_tea_products")
    .execute(&pool)
    .await
    .unwrap();

    let result = catalog
        .get_by_qualified_name(CatalogKind::Table, "public.premium_tea_products")
        .await
        .unwrap();
    assert_eq!(result.map(|entry| entry.id), Some(id.clone()));

    sqlx::query("DELETE FROM catalog_entries WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn rejects_inline_and_empty_catalogs_for_writes() {
    // `OpenedCatalog` is not `Debug`, so match rather than `unwrap_err`.
    let inline_error =
        match open_catalog_for_writes(CatalogStorageConfig::Inline { entries: vec![] }).await {
            Ok(_) => panic!("inline catalog must be rejected for profiling/ingestion writes"),
            Err(err) => err.to_string(),
        };
    assert!(
        inline_error.contains("read-only"),
        "unexpected inline rejection error: {inline_error}"
    );

    let empty_error = match open_catalog_for_writes(CatalogStorageConfig::Empty).await {
        Ok(_) => panic!("empty catalog must be rejected for profiling/ingestion writes"),
        Err(err) => err.to_string(),
    };
    assert!(
        empty_error.contains("read-only"),
        "unexpected empty rejection error: {empty_error}"
    );
}

#[tokio::test]
async fn opens_sqlite_catalog_for_writes_and_roundtrips_entries() {
    let path = std::env::temp_dir().join(format!("flowai-catalog-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}", path.display());

    let opened = open_catalog_for_writes(CatalogStorageConfig::Sqlite {
        url: url.clone(),
        ensure_schema: true,
    })
    .await
    .unwrap();

    let entry = CatalogEntry {
        id: format!("table:{}", unique_name("products")),
        kind: CatalogKind::Table,
        name: "products".to_string(),
        qualified_name: Some("public.products".to_string()),
        content: "Product revenue table".to_string(),
        tags: vec!["sales".to_string()],
        links: vec![],
        metadata: json!({"source": "test"}),
    };
    let entry_id = entry.id.clone();

    let ids = opened
        .writer
        .save_in_transaction(vec![entry])
        .await
        .unwrap();
    assert_eq!(ids, vec![entry_id.clone()]);

    let loaded = opened.reader.get_by_id(&entry_id).await.unwrap();
    assert_eq!(loaded.expect("entry should exist").id, entry_id);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn opens_sqlite_catalog_for_writes_under_explicit_scope() {
    let path = std::env::temp_dir().join(format!("flowai-catalog-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}", path.display());
    let scope = CatalogScope::new(
        TenantId::new_unchecked("tenant-a"),
        WorkspaceId::new_unchecked("workspace-a"),
    );

    let opened = open_catalog_for_writes_for_scope(
        CatalogStorageConfig::Sqlite {
            url: url.clone(),
            ensure_schema: true,
        },
        scope.clone(),
    )
    .await
    .unwrap();

    let entry = CatalogEntry {
        id: "table:products".to_string(),
        kind: CatalogKind::Table,
        name: "products".to_string(),
        qualified_name: Some("public.products".to_string()),
        content: "Tenant-scoped product table".to_string(),
        tags: vec!["sales".to_string()],
        links: vec![],
        metadata: json!({}),
    };
    opened
        .writer
        .save_in_transaction(vec![entry.clone()])
        .await
        .unwrap();

    let scoped_reader = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: url.clone(),
            ensure_schema: true,
        },
        scope,
    )
    .await
    .unwrap();
    assert_eq!(
        scoped_reader
            .get_by_id(&entry.id)
            .await
            .unwrap()
            .map(|entry| entry.name),
        Some("products".to_string())
    );

    let other_scope_reader = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: url.clone(),
            ensure_schema: true,
        },
        CatalogScope::new(
            TenantId::new_unchecked("tenant-b"),
            WorkspaceId::new_unchecked("workspace-a"),
        ),
    )
    .await
    .unwrap();
    assert!(other_scope_reader
        .get_by_id(&entry.id)
        .await
        .unwrap()
        .is_none());

    let legacy_reader = build_catalog(CatalogStorageConfig::Sqlite {
        url,
        ensure_schema: true,
    })
    .await
    .unwrap();
    assert!(legacy_reader.get_by_id(&entry.id).await.unwrap().is_none());

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn runtime_data_environment_catalog_uses_runtime_tenant_and_config_workspace() {
    let path = std::env::temp_dir().join(format!("flowai-catalog-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}", path.display());
    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let opened = open_catalog_for_writes_for_scope(
        CatalogStorageConfig::Sqlite {
            url: url.clone(),
            ensure_schema: true,
        },
        scope,
    )
    .await
    .unwrap();
    opened
        .writer
        .save_in_transaction(vec![CatalogEntry {
            id: "table:products".to_string(),
            kind: CatalogKind::Table,
            name: "products".to_string(),
            qualified_name: Some("public.products".to_string()),
            content: "Runtime-scoped product table".to_string(),
            tags: vec![],
            links: vec![],
            metadata: json!({}),
        }])
        .await
        .unwrap();

    let deps = RuntimeDeps::new(
        Arc::new(NeverUsedInterpreter),
        Arc::new(NullEventSink),
        TenantContext::new(TenantId::new_unchecked("acme")),
        Arc::new(DashMapKVStore::new()),
    );
    let deps = apply_to_runtime_deps(
        deps,
        DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
            kv: None,
            catalog: Some(CatalogStorageConfig::Sqlite {
                url,
                ensure_schema: true,
            }),
            catalog_search: None,
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        deps.data_workspace_context
            .as_ref()
            .expect("workspace context should be installed")
            .workspace_tenant_id()
            .as_str(),
        "acme::workspace:analytics"
    );
    let catalog = deps
        .data_catalog
        .expect("runtime catalog should be installed");
    assert!(catalog.get_by_id("table:products").await.unwrap().is_some());

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn rebuild_on_start_builds_catalog_search_index_before_runtime_use() {
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let deps = RuntimeDeps::new(
        Arc::new(NeverUsedInterpreter),
        Arc::new(NullEventSink),
        TenantContext::new(TenantId::new_unchecked("acme")),
        Arc::new(DashMapKVStore::new()),
    );
    let deps = apply_to_runtime_deps(
        deps,
        DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
            kv: None,
            catalog: Some(CatalogStorageConfig::Inline {
                entries: vec![semantic_table_entry("table:products", "products")],
            }),
            catalog_search: Some(CatalogSearchConfig {
                index_path: index_root.clone(),
                rebuild_on_start: true,
                write_through: false,
            }),
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
    )
    .await
    .unwrap();

    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let backend = deps
        .catalog_search_backend
        .expect("catalog search backend should be installed");

    assert!(matches!(
        backend.health(&scope).await.unwrap(),
        CatalogSearchHealth::Ready {
            indexed_entries: 1,
            ..
        }
    ));
    let results = backend
        .search(
            &scope,
            CatalogSearchRequest {
                query: "products".to_string(),
                kinds: vec![],
                filters: CatalogSearchFilters::default(),
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(results.hits[0].entry_id, "table:products");

    let _ = std::fs::remove_dir_all(index_root);
}

#[tokio::test]
async fn catalog_search_config_without_rebuild_reports_missing_index() {
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let deps = RuntimeDeps::new(
        Arc::new(NeverUsedInterpreter),
        Arc::new(NullEventSink),
        TenantContext::new(TenantId::new_unchecked("acme")),
        Arc::new(DashMapKVStore::new()),
    );
    let deps = apply_to_runtime_deps(
        deps,
        DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
            kv: None,
            catalog: Some(CatalogStorageConfig::Inline {
                entries: vec![semantic_table_entry("table:products", "products")],
            }),
            catalog_search: Some(CatalogSearchConfig {
                index_path: index_root.clone(),
                rebuild_on_start: false,
                write_through: false,
            }),
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
    )
    .await
    .unwrap();

    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let backend = deps
        .catalog_search_backend
        .expect("catalog search backend should still be installed");

    assert!(matches!(
        backend.health(&scope).await.unwrap(),
        CatalogSearchHealth::Unavailable { .. }
    ));

    let env = ToolEnvironment::builder()
        .kv_arc(deps.kv.clone())
        .event_sink_arc(deps.event_sink.clone())
        .tenant_context(deps.tenant.clone())
        .build()
        .with_ext::<WorkspaceContext>(Arc::new(WorkspaceContext::from_ids(
            scope.tenant_id.clone(),
            Some(scope.workspace_id.as_str()),
        )))
        .with_catalog(deps.data_catalog.expect("catalog should be installed"))
        .with_catalog_search_backend(backend);
    let result = agent_fw_catalog_tools::surface::handlers::SearchCatalogHandler
        .handle("search", json!({"query": "products"}), &env)
        .await;

    assert!(result.is_error);
    let error = result.content["error"].as_str().unwrap_or_default();
    assert!(error.contains("Catalog search index is unavailable"));
    assert!(error.contains("Rebuild"));

    let _ = std::fs::remove_dir_all(index_root);
}

/// One un-projectable entry must not disable search for the whole scope (H1):
/// a successful rebuild that skips some entries leaves the index `Ready` and
/// searchable against the entries that did project.
#[tokio::test]
async fn rebuild_on_start_keeps_index_ready_and_searchable_when_some_entries_skip_projection() {
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let deps = RuntimeDeps::new(
        Arc::new(NeverUsedInterpreter),
        Arc::new(NullEventSink),
        TenantContext::new(TenantId::new_unchecked("acme")),
        Arc::new(DashMapKVStore::new()),
    );
    // `column:broken` is missing the required column metadata keys, so its
    // projection fails and it is skipped during rebuild.
    let broken_column = CatalogEntry {
        id: "column:broken".to_string(),
        kind: CatalogKind::Column,
        name: "broken".to_string(),
        qualified_name: Some("public.products.broken".to_string()),
        content: "Invalid column metadata".to_string(),
        tags: vec![],
        links: vec![],
        metadata: json!({"databaseId": "warehouse"}),
    };
    let deps = apply_to_runtime_deps(
        deps,
        DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: Some(WorkspaceId::new_unchecked("analytics")),
            kv: None,
            catalog: Some(CatalogStorageConfig::Inline {
                entries: vec![
                    semantic_table_entry("table:products", "products"),
                    broken_column,
                ],
            }),
            catalog_search: Some(CatalogSearchConfig {
                index_path: index_root.clone(),
                rebuild_on_start: true,
                write_through: false,
            }),
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
    )
    .await
    .unwrap();

    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let backend = deps
        .catalog_search_backend
        .expect("catalog search backend should be installed");

    // The valid table projected; the broken column was skipped. The index must
    // be Ready (not Stale) so search_catalog works for the scope.
    assert!(
        matches!(
            backend.health(&scope).await.unwrap(),
            CatalogSearchHealth::Ready {
                indexed_entries: 1,
                ..
            }
        ),
        "index should be Ready with the one projectable entry, not Stale"
    );

    let results = backend
        .search(
            &scope,
            CatalogSearchRequest {
                query: "products".to_string(),
                kinds: vec![],
                filters: CatalogSearchFilters::default(),
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(results.hits[0].entry_id, "table:products");

    let env = ToolEnvironment::builder()
        .kv_arc(deps.kv.clone())
        .event_sink_arc(deps.event_sink.clone())
        .tenant_context(deps.tenant.clone())
        .build()
        .with_ext::<WorkspaceContext>(Arc::new(WorkspaceContext::from_ids(
            scope.tenant_id.clone(),
            Some(scope.workspace_id.as_str()),
        )))
        .with_catalog(deps.data_catalog.expect("catalog should be installed"))
        .with_catalog_search_backend(backend);
    let result = agent_fw_catalog_tools::surface::handlers::SearchCatalogHandler
        .handle("search", json!({"query": "products"}), &env)
        .await;

    assert!(
        !result.is_error,
        "search_catalog should succeed against a partially-rebuilt index, got: {:?}",
        result.content
    );

    let _ = std::fs::remove_dir_all(index_root);
}

/// Skipped entries must remain observable via the rebuild summary so operators
/// can see partial-success details through the rebuild CLI (H1 guard against a
/// regression that "fixes" the issue by dropping skip reporting).
#[tokio::test]
async fn rebuild_summary_reports_skipped_entries_with_reasons() {
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let broken_column = CatalogEntry {
        id: "column:broken".to_string(),
        kind: CatalogKind::Column,
        name: "broken".to_string(),
        qualified_name: Some("public.products.broken".to_string()),
        content: "Invalid column metadata".to_string(),
        tags: vec![],
        links: vec![],
        metadata: json!({"databaseId": "warehouse"}),
    };
    let config = DataEnvironmentConfig {
        tenant_id: Some(scope.tenant_id.clone()),
        workspace_id: Some(scope.workspace_id.clone()),
        kv: None,
        catalog: Some(CatalogStorageConfig::Inline {
            entries: vec![
                semantic_table_entry("table:products", "products"),
                broken_column,
            ],
        }),
        catalog_search: Some(CatalogSearchConfig {
            index_path: index_root.clone(),
            rebuild_on_start: false,
            write_through: false,
        }),
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };

    let summary = flowai_runtime::storage::rebuild_catalog_search_index_from_environment(
        &config,
        scope.clone(),
    )
    .await
    .expect("rebuild should succeed with a partially-projectable catalog");

    assert_eq!(summary.indexed_entries, 1);
    assert_eq!(summary.skipped_entries, 1);
    assert_eq!(summary.warnings.len(), 1);
    assert!(
        summary.warnings[0].contains("column:broken"),
        "skip reason should name the skipped entry, got: {:?}",
        summary.warnings
    );

    let _ = std::fs::remove_dir_all(index_root);
}

#[tokio::test]
async fn rebuild_summary_warns_for_missing_document_body_without_counting_skip() {
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let scope = CatalogScope::new(
        TenantId::new_unchecked("acme"),
        WorkspaceId::new_unchecked("analytics"),
    );
    let config = DataEnvironmentConfig {
        tenant_id: Some(scope.tenant_id.clone()),
        workspace_id: Some(scope.workspace_id.clone()),
        kv: None,
        catalog: Some(CatalogStorageConfig::Inline {
            entries: vec![semantic_document_entry("document:playbook", "doc-playbook")],
        }),
        catalog_search: Some(CatalogSearchConfig {
            index_path: index_root.clone(),
            rebuild_on_start: false,
            write_through: false,
        }),
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };

    let summary = flowai_runtime::storage::rebuild_catalog_search_index_from_environment(
        &config,
        scope.clone(),
    )
    .await
    .expect("document metadata-only indexing should succeed");

    assert_eq!(summary.indexed_entries, 1);
    assert_eq!(summary.skipped_entries, 0);
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("indexing metadata only")),
        "missing document body should warn without skipping the document, got {:?}",
        summary.warnings
    );

    let _ = std::fs::remove_dir_all(index_root);
}

#[tokio::test]
async fn write_through_failure_marks_existing_index_stale_without_rolling_back_catalog_write() {
    let catalog_path = std::env::temp_dir().join(format!("flowai-catalog-{}.db", Uuid::new_v4()));
    let index_root = std::env::temp_dir().join(format!("flowai-index-{}", Uuid::new_v4()));
    let scope = CatalogScope::new(
        TenantId::new_unchecked("tenant-a"),
        WorkspaceId::new_unchecked("workspace-a"),
    );
    let config = DataEnvironmentConfig {
        tenant_id: Some(scope.tenant_id.clone()),
        workspace_id: Some(scope.workspace_id.clone()),
        kv: None,
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: format!("sqlite:{}", catalog_path.display()),
            ensure_schema: true,
        }),
        catalog_search: Some(CatalogSearchConfig {
            index_path: index_root.clone(),
            rebuild_on_start: true,
            write_through: true,
        }),
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let opened = open_writable_catalog_from_environment(&config)
        .await
        .expect("writable catalog should open with write-through wrapper");

    let invalid_for_projection = CatalogEntry {
        id: "column:broken".to_string(),
        kind: CatalogKind::Column,
        name: "broken".to_string(),
        qualified_name: Some("public.products.broken".to_string()),
        content: "Invalid column metadata".to_string(),
        tags: vec![],
        links: vec![],
        metadata: json!({"databaseId": "warehouse"}),
    };

    let ids = opened
        .writer
        .save_items(vec![invalid_for_projection.clone()])
        .await
        .expect("catalog write should not roll back when index write-through fails");
    assert_eq!(ids, vec!["column:broken".to_string()]);
    assert!(opened
        .reader
        .get_by_id("column:broken")
        .await
        .unwrap()
        .is_some());

    let backend = flowai_runtime::storage::build_catalog_search_backend(&config)
        .expect("catalog search backend should open");
    assert!(matches!(
        backend.health(&scope).await.unwrap(),
        CatalogSearchHealth::Stale { .. }
    ));

    let _ = std::fs::remove_file(&catalog_path);
    let _ = std::fs::remove_dir_all(index_root);
}

#[tokio::test]
async fn write_through_requires_existing_ready_index_before_incremental_update() {
    let catalog_path = std::env::temp_dir().join(format!(
        "flowai-catalog-write-through-missing-{}.db",
        Uuid::new_v4()
    ));
    let index_root = std::env::temp_dir().join(format!(
        "flowai-index-write-through-missing-{}",
        Uuid::new_v4()
    ));
    let scope = CatalogScope::new(
        TenantId::new_unchecked("tenant-a"),
        WorkspaceId::new_unchecked("workspace-a"),
    );
    let config = DataEnvironmentConfig {
        tenant_id: Some(TenantId::new_unchecked("tenant-a")),
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
        kv: None,
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: format!("sqlite:{}", catalog_path.display()),
            ensure_schema: true,
        }),
        catalog_search: Some(CatalogSearchConfig {
            index_path: index_root.clone(),
            rebuild_on_start: false,
            write_through: true,
        }),
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };
    let opened = open_writable_catalog_from_environment(&config)
        .await
        .expect("writable catalog should open with write-through wrapper");

    opened
        .writer
        .save_items(vec![semantic_table_entry("table:products", "products")])
        .await
        .expect("catalog write should not roll back when write-through index is missing");
    assert!(opened
        .reader
        .get_by_id("table:products")
        .await
        .unwrap()
        .is_some());

    let handle = flowai_runtime::storage::build_catalog_search_index_handle(&config)
        .expect("catalog search handle should open");
    assert!(!handle
        .index()
        .paths()
        .scope_path(&scope)
        .join("meta.json")
        .exists());
    assert!(matches!(
        handle.health(&scope).await.unwrap(),
        CatalogSearchHealth::Unavailable { .. }
    ));

    let _ = std::fs::remove_file(&catalog_path);
    let _ = std::fs::remove_dir_all(index_root);
}

#[tokio::test]
async fn rejects_inline_catalog_for_writes() {
    let err = open_catalog_for_writes(CatalogStorageConfig::Inline { entries: vec![] })
        .await
        .err()
        .expect("inline catalogs should be rejected");

    let message = err.to_string();
    assert!(message.contains("durable catalog backend"));
    assert!(message.contains("kind=inline"));
}

#[tokio::test]
async fn rejects_empty_catalog_for_writes() {
    let err = open_catalog_for_writes(CatalogStorageConfig::Empty)
        .await
        .err()
        .expect("empty catalogs should be rejected");

    let message = err.to_string();
    assert!(message.contains("durable catalog backend"));
    assert!(message.contains("kind=empty"));
}

#[tokio::test]
async fn rejects_missing_catalog_for_writable_catalog_open() {
    let config = DataEnvironmentConfig {
        tenant_id: None,
        workspace_id: None,
        kv: None,
        catalog: None,
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: "sqlite:/tmp/test.db".to_string(),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };

    let err = open_writable_catalog_from_environment(&config)
        .await
        .err()
        .expect("missing catalog should fail");

    assert!(err
        .to_string()
        .contains("requires data_environment.catalog"));
}

#[tokio::test]
async fn rejects_missing_target_database_for_environment_open() {
    let config = DataEnvironmentConfig {
        tenant_id: None,
        workspace_id: None,
        kv: None,
        catalog: Some(CatalogStorageConfig::Empty),
        catalog_search: None,
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    };

    let err = build_target_database_from_environment(&config)
        .await
        .err()
        .expect("missing target database should fail");

    assert!(err.to_string().contains(
        "requires data_environment.target_database or data_environment.target_database_url"
    ));
}

#[tokio::test]
async fn env_gated_postgres_catalog_can_open_for_writes() {
    let Some(url) = env_url("FLOWAI_TEST_POSTGRES_CATALOG_URL") else {
        return;
    };
    let opened = open_catalog_for_writes(CatalogStorageConfig::Postgres {
        url: Some(url.clone()),
        url_env: None,
        ensure_schema: true,
    })
    .await
    .unwrap();

    let entry = CatalogEntry {
        id: format!("table:{}", unique_name("teams")),
        kind: CatalogKind::Table,
        name: "teams".to_string(),
        qualified_name: Some("public.teams".to_string()),
        content: "Team dimension table".to_string(),
        tags: vec!["org".to_string()],
        links: vec![],
        metadata: json!({}),
    };
    let entry_id = entry.id.clone();

    opened
        .writer
        .save_in_transaction(vec![entry])
        .await
        .unwrap();

    let loaded = opened.reader.get_by_id(&entry_id).await.unwrap();
    assert_eq!(loaded.expect("entry should exist").id, entry_id);

    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DELETE FROM catalog_entries WHERE id = $1")
        .bind(&entry_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn env_gated_redis_kv_can_roundtrip() {
    let Some(_) = env_url("FLOWAI_TEST_REDIS_URL") else {
        return;
    };
    let prefix = format!("flowai-test:{}:", Uuid::new_v4().simple());
    let store = build_kv_store(KvStorageConfig::Redis {
        url: None,
        url_env: Some("FLOWAI_TEST_REDIS_URL".to_string()),
        prefix: Some(prefix),
    })
    .await
    .unwrap();

    store
        .put_json("tenant-a", "key-a", json!({"value": 42}), None)
        .await
        .unwrap();
    assert_eq!(
        store.get_json("tenant-a", "key-a").await.unwrap(),
        Some(json!({"value": 42}))
    );
    assert!(store.delete("tenant-a", "key-a").await.unwrap());
}

fn env_url(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn unique_name(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn semantic_table_entry(id: &str, table: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(format!("public.{table}")),
        content: format!("{table} table"),
        tags: vec!["sales".to_string()],
        links: vec![],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 2,
            "preferredQuerySurface": true
        }),
    }
}

fn semantic_document_entry(id: &str, source_document_id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Document,
        name: "Playbook".to_string(),
        qualified_name: Some("docs.playbook".to_string()),
        content: "Catalog metadata summary.".to_string(),
        tags: vec!["docs".to_string()],
        links: vec![],
        metadata: json!({
            "sourceDocumentId": source_document_id,
            "contentAvailable": true,
            "contentSource": "kv",
            "extractionStatus": "processed",
            "extractedKnowledgeIds": []
        }),
    }
}
