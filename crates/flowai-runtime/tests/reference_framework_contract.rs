use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::KVStore;
use agent_fw_core::TenantId;
use agent_fw_interpreter::DashMapKVStore;
use agent_fw_reference::{KvReferenceRegistry, ReferenceError, ReferenceRegistry};
use serde_json::json;

#[tokio::test]
async fn kv_reference_registry_round_trips_value_and_cached_glimpse() {
    let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
    let registry = KvReferenceRegistry::new(kv);
    let tenant = TenantId::new_unchecked("tenant-a");
    let value = json!({"productIds": ["sku-1", "sku-2"]});
    let glimpse = json!({"productCount": 2, "preview": ["sku-1", "sku-2"]});

    let first = registry
        .create("ProductSet", value.clone(), glimpse.clone(), &tenant, None)
        .await
        .expect("create reference");
    let second = registry
        .create("ProductSet", value.clone(), glimpse.clone(), &tenant, None)
        .await
        .expect("create same reference");

    assert_eq!(first.kind, "ProductSet");
    assert_eq!(first.id, second.id);

    let stored = registry
        .resolve(&first, &tenant)
        .await
        .expect("resolve reference");
    assert_eq!(stored.value, value);
    assert_eq!(stored.glimpse, glimpse);

    let cached_glimpse = registry
        .glimpse(&first, &tenant)
        .await
        .expect("resolve glimpse");
    assert_eq!(cached_glimpse, glimpse);
}

#[tokio::test]
async fn kv_reference_registry_enforces_tenant_isolation_and_ttl() {
    let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
    let registry = KvReferenceRegistry::new(kv);
    let tenant_a = TenantId::new_unchecked("tenant-a");
    let tenant_b = TenantId::new_unchecked("tenant-b");

    let artifact = registry
        .create(
            "Scope",
            json!({"region": "north"}),
            json!({"label": "north"}),
            &tenant_a,
            Some(Duration::from_millis(25)),
        )
        .await
        .expect("create reference");

    let wrong_tenant = registry.resolve(&artifact, &tenant_b).await.unwrap_err();
    assert!(matches!(wrong_tenant, ReferenceError::NotFound { .. }));

    tokio::time::sleep(Duration::from_millis(60)).await;

    let expired = registry.resolve(&artifact, &tenant_a).await.unwrap_err();
    assert!(matches!(expired, ReferenceError::NotFound { .. }));
}
