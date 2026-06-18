use std::path::Path;
use std::sync::Arc;

use agent_fw_catalog::CatalogScope;
use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_eval::{ModelPricing, StaticPricingResolver};
use agent_fw_interpreter::MockEnricher;
use flowai_runtime::data::{
    estimate_profiling, export_catalog, profile_database, profile_table, ExportCatalogCommand,
    ProfileDatabaseCommand, ProfileTableCommand, ProfilingCommandDeps, ProfilingEstimateCommand,
};
use flowai_runtime::storage::{
    build_catalog, build_catalog_for_scope, CatalogStorageConfig, DataEnvironmentConfig,
    TargetDatabaseStorageConfig,
};
use rusqlite::Connection;
use uuid::Uuid;

#[tokio::test]
async fn profiling_estimate_reads_schema_counts_and_pricing() {
    let target_path = temp_sqlite_path("estimate-target");
    seed_target_database(&target_path);

    let command = ProfilingEstimateCommand {
        data_environment: DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: None,
            kv: None,
            catalog: None,
            catalog_search: None,
            target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                url: sqlite_url(&target_path),
            }),
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
        tenant_id: None,
        workspace_id: None,
        database_id: "acme".to_string(),
        schema_name: None,
        tables: vec![],
        model_id: Some("claude-haiku".to_string()),
        sample_size: None,
    };
    let deps =
        ProfilingCommandDeps::new(Arc::new(MockEnricher::new())).with_pricing_resolver(Arc::new(
            StaticPricingResolver::new([ModelPricing::new("claude-haiku", 1.0, 2.0, 0.0)]),
        ));

    let estimate = estimate_profiling(&command, &deps).await.unwrap();
    assert_eq!(estimate.table_count, 2);
    assert_eq!(estimate.column_count, 6);
    assert_eq!(estimate.model_id.as_deref(), Some("claude-haiku"));
    assert!(estimate.estimated_cost_usd.unwrap() > 0.0);
    assert!(estimate.estimated_duration_secs > 0);

    let _ = std::fs::remove_file(&target_path);
}

#[tokio::test]
async fn profile_table_persists_catalog_artifacts_into_sqlite_catalog() {
    let target_path = temp_sqlite_path("table-target");
    let catalog_path = temp_sqlite_path("table-catalog");
    seed_target_database(&target_path);

    let command = ProfileTableCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: None,
        workspace_id: None,
        database_id: "acme".to_string(),
        schema_name: None,
        table_name: "products".to_string(),
        model_id: None,
        sample_size: Some(1),
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let mut handle = profile_table(command, deps).await.unwrap();
    let mut events = Vec::new();
    while let Some(event) = handle.events.recv().await {
        events.push(event);
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, agent_fw_catalog::IngestionEvent::Completed { .. })),
        "expected table profiling to complete, saw events: {events:?}"
    );

    let catalog = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        default_data_scope(),
    )
    .await
    .unwrap();
    let tables = catalog.list_tables().await.unwrap();
    let products = tables
        .iter()
        .find(|entry| entry.name == "products")
        .expect("products table should be indexed");
    let columns = catalog
        .get_columns(products.qualified_name.as_deref().unwrap_or(&products.name))
        .await
        .unwrap();
    assert_eq!(columns.len(), 3);

    let _ = std::fs::remove_file(&target_path);
    let _ = std::fs::remove_file(&catalog_path);
}

#[tokio::test]
async fn profile_table_persists_catalog_under_command_scope() {
    let target_path = temp_sqlite_path("table-scope-target");
    let catalog_path = temp_sqlite_path("table-scope-catalog");
    seed_target_database(&target_path);

    let command = ProfileTableCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: Some(TenantId::new_unchecked("tenant-a")),
        workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
        database_id: "acme".to_string(),
        schema_name: None,
        table_name: "products".to_string(),
        model_id: None,
        sample_size: Some(1),
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let mut handle = profile_table(command, deps).await.unwrap();
    while handle.events.recv().await.is_some() {}

    let scoped_catalog = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        CatalogScope::new(
            TenantId::new_unchecked("tenant-a"),
            WorkspaceId::new_unchecked("workspace-a"),
        ),
    )
    .await
    .unwrap();
    assert!(scoped_catalog
        .list_tables()
        .await
        .unwrap()
        .iter()
        .any(|entry| entry.name == "products"));

    let wrong_tenant_catalog = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        CatalogScope::new(
            TenantId::new_unchecked("tenant-b"),
            WorkspaceId::new_unchecked("workspace-a"),
        ),
    )
    .await
    .unwrap();
    assert!(wrong_tenant_catalog.list_tables().await.unwrap().is_empty());

    let legacy_catalog = build_catalog(CatalogStorageConfig::Sqlite {
        url: sqlite_url(&catalog_path),
        ensure_schema: true,
    })
    .await
    .unwrap();
    assert!(legacy_catalog.list_tables().await.unwrap().is_empty());

    let _ = std::fs::remove_file(&target_path);
    let _ = std::fs::remove_file(&catalog_path);
}

#[tokio::test]
async fn profile_database_persists_multiple_tables_into_sqlite_catalog() {
    let target_path = temp_sqlite_path("db-target");
    let catalog_path = temp_sqlite_path("db-catalog");
    seed_target_database(&target_path);

    let command = ProfileDatabaseCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: None,
        workspace_id: None,
        database_id: "acme".to_string(),
        schema_name: None,
        tables: vec![],
        model_id: None,
        sample_size: Some(1),
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let mut handle = profile_database(command, deps).await.unwrap();
    let mut saw_completed = false;
    while let Some(event) = handle.events.recv().await {
        if matches!(event, agent_fw_catalog::IngestionEvent::Completed { .. }) {
            saw_completed = true;
        }
    }
    assert!(saw_completed, "expected database profiling to complete");

    let catalog = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        default_data_scope(),
    )
    .await
    .unwrap();
    let tables = catalog.list_tables().await.unwrap();
    assert!(tables.iter().any(|entry| entry.name == "products"));
    assert!(tables.iter().any(|entry| entry.name == "orders"));

    let _ = std::fs::remove_file(&target_path);
    let _ = std::fs::remove_file(&catalog_path);
}

#[tokio::test]
async fn profile_database_profiles_requested_table_subset_only() {
    let target_path = temp_sqlite_path("subset-target");
    let catalog_path = temp_sqlite_path("subset-catalog");
    seed_target_database(&target_path);

    let command = ProfileDatabaseCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: None,
        workspace_id: None,
        database_id: "acme".to_string(),
        schema_name: None,
        tables: vec!["products".to_string()],
        model_id: None,
        sample_size: Some(1),
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let mut handle = profile_database(command, deps).await.unwrap();
    let mut saw_completed = false;
    while let Some(event) = handle.events.recv().await {
        if matches!(event, agent_fw_catalog::IngestionEvent::Completed { .. }) {
            saw_completed = true;
        }
    }
    assert!(
        saw_completed,
        "expected subset database profiling to complete"
    );

    let catalog = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        default_data_scope(),
    )
    .await
    .unwrap();
    let tables = catalog.list_tables().await.unwrap();
    assert!(tables.iter().any(|entry| entry.name == "products"));
    assert!(!tables.iter().any(|entry| entry.name == "orders"));

    let _ = std::fs::remove_file(&target_path);
    let _ = std::fs::remove_file(&catalog_path);
}

#[tokio::test]
async fn profile_then_export_catalog_is_deterministic_and_roundtrips() {
    let target_path = temp_sqlite_path("export-target");
    let catalog_path = temp_sqlite_path("export-catalog");
    seed_target_database(&target_path);

    // Profile the full database into a durable sqlite catalog.
    let command = ProfileDatabaseCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: None,
        workspace_id: None,
        database_id: "acme".to_string(),
        schema_name: None,
        tables: vec![],
        model_id: None,
        sample_size: Some(1),
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));
    let mut handle = profile_database(command, deps).await.unwrap();
    while handle.events.recv().await.is_some() {}

    // Export reads the durable catalog only (no target-database access).
    let export_command = || ExportCatalogCommand {
        data_environment: data_environment_with_sqlite_catalog(&target_path, &catalog_path),
        tenant_id: None,
        workspace_id: None,
    };
    let export = export_catalog(export_command()).await.unwrap();

    assert!(export.summary.entries_written > 0);
    assert!(export
        .entries
        .iter()
        .any(|e| e.kind == agent_fw_catalog::CatalogKind::Table && e.name == "products"));
    assert!(
        export
            .summary
            .counts_by_kind
            .get("table")
            .copied()
            .unwrap_or(0)
            >= 2
    );
    assert!(export
        .entries
        .iter()
        .all(|e| e.kind != agent_fw_catalog::CatalogKind::Enum));

    // Deterministic: a second export serializes byte-identically.
    let export_again = export_catalog(export_command()).await.unwrap();
    let json_a = serde_json::to_vec_pretty(&export.entries).unwrap();
    let json_b = serde_json::to_vec_pretty(&export_again.entries).unwrap();
    assert_eq!(json_a, json_b);

    // Round-trip: exported entries reload as an inline catalog with the same tables.
    let inline_catalog = build_catalog_for_scope(
        CatalogStorageConfig::Inline {
            entries: export.entries.clone(),
        },
        default_data_scope(),
    )
    .await
    .unwrap();
    let mut sqlite_tables: Vec<String> = build_catalog_for_scope(
        CatalogStorageConfig::Sqlite {
            url: sqlite_url(&catalog_path),
            ensure_schema: true,
        },
        default_data_scope(),
    )
    .await
    .unwrap()
    .list_tables()
    .await
    .unwrap()
    .into_iter()
    .map(|e| e.name)
    .collect();
    let mut inline_tables: Vec<String> = inline_catalog
        .list_tables()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    sqlite_tables.sort();
    inline_tables.sort();
    assert_eq!(inline_tables, sqlite_tables);
    assert_eq!(inline_tables.len(), 2);

    let _ = std::fs::remove_file(&target_path);
    let _ = std::fs::remove_file(&catalog_path);
}

#[tokio::test]
async fn export_catalog_redacts_secrets_in_connection_errors() {
    let command = ExportCatalogCommand {
        data_environment: DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: None,
            kv: None,
            catalog: Some(CatalogStorageConfig::Postgres {
                url: Some(
                    "postgresql://catalog_user:s3cr3t-password@127.0.0.1:1/catalog".to_string(),
                ),
                url_env: None,
                ensure_schema: false,
            }),
            catalog_search: None,
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        },
        tenant_id: None,
        workspace_id: None,
    };

    let message = export_catalog(command).await.unwrap_err().to_string();
    assert!(
        !message.contains("s3cr3t-password"),
        "secret leaked in export error: {message}"
    );
    assert!(
        message.contains("catalog_user:***"),
        "expected redacted credentials in export error, got: {message}"
    );
}

#[tokio::test]
async fn profile_table_rejects_blank_database_id_before_opening_environment() {
    let command = ProfileTableCommand {
        data_environment: empty_data_environment(),
        tenant_id: None,
        workspace_id: None,
        database_id: "  ".to_string(),
        schema_name: None,
        table_name: "products".to_string(),
        model_id: None,
        sample_size: None,
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let message = match profile_table(command, deps).await {
        Ok(_) => panic!("blank database_id should fail before profiling starts"),
        Err(error) => error.to_string(),
    };

    assert!(message.contains("catalog profiling database_id must not be blank"));
}

#[tokio::test]
async fn profile_database_rejects_blank_database_id_before_opening_environment() {
    let command = ProfileDatabaseCommand {
        data_environment: empty_data_environment(),
        tenant_id: None,
        workspace_id: None,
        database_id: "\t".to_string(),
        schema_name: None,
        tables: vec![],
        model_id: None,
        sample_size: None,
    };
    let deps = ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));

    let message = match profile_database(command, deps).await {
        Ok(_) => panic!("blank database_id should fail before profiling starts"),
        Err(error) => error.to_string(),
    };

    assert!(message.contains("catalog profiling database_id must not be blank"));
}

fn empty_data_environment() -> DataEnvironmentConfig {
    DataEnvironmentConfig {
        tenant_id: None,
        workspace_id: None,
        kv: None,
        catalog: None,
        catalog_search: None,
        target_database: None,
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    }
}

fn data_environment_with_sqlite_catalog(
    target_path: &Path,
    catalog_path: &Path,
) -> DataEnvironmentConfig {
    DataEnvironmentConfig {
        tenant_id: None,
        workspace_id: None,
        kv: None,
        catalog: Some(CatalogStorageConfig::Sqlite {
            url: sqlite_url(catalog_path),
            ensure_schema: true,
        }),
        catalog_search: None,
        target_database: Some(TargetDatabaseStorageConfig::Sqlite {
            url: sqlite_url(target_path),
        }),
        legacy_target_database_url: None,
        legacy_target_database_schema: None,
    }
}

fn default_data_scope() -> CatalogScope {
    CatalogScope::new(
        TenantId::new_unchecked(flowai_runtime::data::DEFAULT_DATA_TENANT_ID),
        WorkspaceId::default_workspace(),
    )
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite:{}", path.display())
}

fn temp_sqlite_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.db", Uuid::new_v4()))
}

fn seed_target_database(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            price REAL NOT NULL
        );
        INSERT INTO products (id, name, price) VALUES
            (1, 'Tea', 4.5),
            (2, 'Coffee', 7.0);

        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            product_id INTEGER NOT NULL,
            quantity INTEGER NOT NULL,
            FOREIGN KEY(product_id) REFERENCES products(id)
        );
        INSERT INTO orders (id, product_id, quantity) VALUES
            (1, 1, 2),
            (2, 2, 1);
        "#,
    )
    .unwrap();
}
