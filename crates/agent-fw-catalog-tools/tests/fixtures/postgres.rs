use agent_fw_catalog::{CatalogScope, CatalogWriter};
use agent_fw_interpreter::{PostgresCatalog, ScopedPostgresCatalog};

use super::business_catalog;
use super::tenant_catalog;

#[allow(dead_code)]
pub async fn catalog_from_env() -> Option<ScopedPostgresCatalog> {
    catalog_from_env_with_entries(business_catalog::entries()).await
}

#[allow(dead_code)]
pub async fn catalog_from_env_with_entries(
    entries: Vec<agent_fw_catalog::CatalogEntry>,
) -> Option<ScopedPostgresCatalog> {
    let url = match std::env::var("AGENT_FW_TEST_POSTGRES_CATALOG_URL") {
        Ok(url) => url,
        Err(_) => return None,
    };

    let catalog = PostgresCatalog::connect(&url).await.unwrap();
    catalog.ensure_schema().await.unwrap();

    // This URL must point at a disposable test database. The fixture wipes the
    // catalog tables before every test so SQLite and Postgres observe identical data.
    sqlx::query("TRUNCATE TABLE catalog_relations, catalog_entries")
        .execute(catalog.pool())
        .await
        .unwrap();

    let catalog = catalog.with_scope(CatalogScope::legacy_unscoped());
    catalog.save_in_transaction(entries).await.unwrap();

    Some(catalog)
}

#[allow(dead_code)]
pub async fn tenant_catalog_from_env() -> Option<ScopedPostgresCatalog> {
    let url = match std::env::var("AGENT_FW_TEST_POSTGRES_CATALOG_URL") {
        Ok(url) => url,
        Err(_) => return None,
    };

    let catalog = PostgresCatalog::connect(&url).await.unwrap();
    catalog.ensure_schema().await.unwrap();

    // This URL must point at a disposable test database. The fixture wipes the
    // catalog tables before every test so tenant isolation failures are visible.
    sqlx::query("TRUNCATE TABLE catalog_relations, catalog_entries")
        .execute(catalog.pool())
        .await
        .unwrap();

    catalog
        .with_scope(tenant_catalog::scope_a())
        .save_in_transaction(tenant_catalog::entries_a())
        .await
        .unwrap();
    catalog
        .with_scope(tenant_catalog::scope_b())
        .save_in_transaction(tenant_catalog::entries_b())
        .await
        .unwrap();

    Some(catalog.with_scope(tenant_catalog::scope_a()))
}
