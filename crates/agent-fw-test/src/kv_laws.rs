//! KVStore algebraic law test harnesses.
//!
//! These harnesses verify that a `KVStore` implementation satisfies
//! all documented algebraic laws. Use `test_all` for deterministic
//! tests and the `proptest` module for property-based testing.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_store_satisfies_kv_laws() {
//!     let store = MyKVStore::new();
//!     agent_fw_test::kv_laws::test_all(&store).await;
//! }
//! ```

use agent_fw_algebra::{KVStore, KVStoreExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Test value used in KV law harnesses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestValue {
    pub name: String,
    pub count: u32,
}

/// Run all deterministic KV laws against the given store.
///
/// This exercises Laws L1–L10 with fixed inputs. For randomized
/// coverage, use the `proptest` strategies in this module.
pub async fn test_all(store: &dyn KVStore) {
    law_get_after_put(store).await;
    law_put_overwrites(store).await;
    law_delete_removes(store).await;
    law_get_missing(store).await;
    law_delete_idempotent(store).await;
    law_exists_consistency(store).await;
    law_permanence(store).await;
    law_tenant_isolation(store).await;
    law_get_many_consistency(store).await;
    law_ttl_expiry(store).await;
}

/// L1: Get-After-Put
/// put(k, v); get(k) == Some(v)
pub async fn law_get_after_put(store: &dyn KVStore) {
    let v = TestValue {
        name: "l1".into(),
        count: 1,
    };
    store.put("t_l1", "k1", &v, None).await.unwrap();
    let retrieved: Option<TestValue> = store.get("t_l1", "k1").await.unwrap();
    assert_eq!(
        retrieved,
        Some(v),
        "L1: get after put must return stored value"
    );
}

/// L2: Put-Overwrites
/// put(k, v1); put(k, v2); get(k) == Some(v2)
pub async fn law_put_overwrites(store: &dyn KVStore) {
    let v1 = TestValue {
        name: "l2a".into(),
        count: 1,
    };
    let v2 = TestValue {
        name: "l2b".into(),
        count: 2,
    };
    store.put("t_l2", "k1", &v1, None).await.unwrap();
    store.put("t_l2", "k1", &v2, None).await.unwrap();
    let retrieved: Option<TestValue> = store.get("t_l2", "k1").await.unwrap();
    assert_eq!(retrieved, Some(v2), "L2: second put must overwrite first");
}

/// L3: Delete-Removes
/// put(k, v); delete(k); get(k) == None
pub async fn law_delete_removes(store: &dyn KVStore) {
    let v = TestValue {
        name: "l3".into(),
        count: 3,
    };
    store.put("t_l3", "k1", &v, None).await.unwrap();
    let deleted = store.delete("t_l3", "k1").await.unwrap();
    assert!(deleted, "L3: delete of existing key returns true");
    let retrieved: Option<TestValue> = store.get("t_l3", "k1").await.unwrap();
    assert_eq!(retrieved, None, "L3: get after delete must return None");
}

/// L4: Get-Missing
/// get(k) on empty store == None
pub async fn law_get_missing(store: &dyn KVStore) {
    let retrieved: Option<TestValue> = store
        .get("t_l4_nonexistent", "k_nonexistent")
        .await
        .unwrap();
    assert_eq!(retrieved, None, "L4: get on missing key must return None");
}

/// L5: Delete-Idempotent
/// delete(k) on absent key succeeds, returns false
pub async fn law_delete_idempotent(store: &dyn KVStore) {
    let v = TestValue {
        name: "l5".into(),
        count: 5,
    };
    store.put("t_l5", "k1", &v, None).await.unwrap();
    let first = store.delete("t_l5", "k1").await.unwrap();
    let second = store.delete("t_l5", "k1").await.unwrap();
    assert!(first, "L5: first delete returns true");
    assert!(!second, "L5: second delete returns false");
}

/// L6: Exists-Get-Consistency
/// exists(k) ⟺ get(k).is_some()
pub async fn law_exists_consistency(store: &dyn KVStore) {
    let v = TestValue {
        name: "l6".into(),
        count: 6,
    };
    // Before put
    let exists_before = store.exists("t_l6", "k1").await.unwrap();
    let get_before: Option<TestValue> = store.get("t_l6", "k1").await.unwrap();
    assert_eq!(
        exists_before,
        get_before.is_some(),
        "L6: exists must match get.is_some() (before put)"
    );

    // After put
    store.put("t_l6", "k1", &v, None).await.unwrap();
    let exists_after = store.exists("t_l6", "k1").await.unwrap();
    let get_after: Option<TestValue> = store.get("t_l6", "k1").await.unwrap();
    assert_eq!(
        exists_after,
        get_after.is_some(),
        "L6: exists must match get.is_some() (after put)"
    );
}

/// L8: Permanence
/// put(k, v, None); /* wait briefly */; get(k) == Some(v)
///
/// Entries stored with no TTL (`None`) never expire. Dual of L7 (TTL-Expiry).
pub async fn law_permanence(store: &dyn KVStore) {
    let v = TestValue {
        name: "l8_permanent".into(),
        count: 8,
    };
    store.put("t_l8", "perm-key", &v, None).await.unwrap();

    // Wait briefly to ensure the value persists (no spurious expiry)
    tokio::time::sleep(Duration::from_millis(500)).await;

    let retrieved: Option<TestValue> = store.get("t_l8", "perm-key").await.unwrap();
    assert_eq!(
        retrieved,
        Some(v),
        "L8 Permanence: value stored with no TTL must persist indefinitely"
    );
}

/// L9: Tenant-Isolation
/// tenant1.put(k, v); tenant2.get(k) == None
pub async fn law_tenant_isolation(store: &dyn KVStore) {
    let v = TestValue {
        name: "l9".into(),
        count: 9,
    };
    store.put("t_l9_a", "shared_key", &v, None).await.unwrap();
    let other_tenant: Option<TestValue> = store.get("t_l9_b", "shared_key").await.unwrap();
    assert_eq!(
        other_tenant, None,
        "L9: different tenant must not see other tenant's data"
    );
}

/// L10: GetMany-Consistency
/// get_many([k1, k2]) ≡ individual get(k1) + get(k2)
pub async fn law_get_many_consistency(store: &dyn KVStore) {
    let v1 = TestValue {
        name: "l10a".into(),
        count: 10,
    };
    let v2 = TestValue {
        name: "l10b".into(),
        count: 20,
    };
    store.put("t_l10", "k1", &v1, None).await.unwrap();
    store.put("t_l10", "k2", &v2, None).await.unwrap();

    // Individual gets
    let i1: Option<serde_json::Value> = store.get_json("t_l10", "k1").await.unwrap();
    let i2: Option<serde_json::Value> = store.get_json("t_l10", "k2").await.unwrap();
    let i3: Option<serde_json::Value> = store.get_json("t_l10", "k_missing").await.unwrap();

    // Batch get
    let keys = vec!["k1".to_string(), "k2".to_string(), "k_missing".to_string()];
    let batch: HashMap<String, serde_json::Value> =
        store.get_many_json("t_l10", &keys).await.unwrap();

    assert_eq!(
        batch.get("k1").cloned(),
        i1,
        "L10: batch k1 must match individual"
    );
    assert_eq!(
        batch.get("k2").cloned(),
        i2,
        "L10: batch k2 must match individual"
    );
    assert_eq!(
        batch.get("k_missing").cloned(),
        i3,
        "L10: batch missing must match individual"
    );
}

/// L7: TTL-Expiry
/// put(k, v, Some(d)); sleep past d; get(k) == None
///
/// This test verifies that entries with a TTL are automatically evicted
/// after the TTL duration elapses. Implementations that do not support
/// TTL expiry should skip this test.
pub async fn law_ttl_expiry(store: &dyn KVStore) {
    let v = TestValue {
        name: "l7_ttl".into(),
        count: 7,
    };
    store
        .put("t_l7", "ttl-key", &v, Some(Duration::from_millis(50)))
        .await
        .unwrap();

    // Value should be present immediately after put
    let before: Option<TestValue> = store.get("t_l7", "ttl-key").await.unwrap();
    assert!(
        before.is_some(),
        "L7 TTL: value must be present before TTL expires"
    );

    // Wait past the TTL
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Value should have expired
    let after: Option<TestValue> = store.get("t_l7", "ttl-key").await.unwrap();
    assert_eq!(after, None, "L7 TTL: value must be None after TTL expires");
}

/// Proptest strategies for KV law property-based testing.
///
/// # Usage
///
/// ```ignore
/// use agent_fw_test::kv_laws::proptest_strategies::*;
/// use proptest::prelude::*;
///
/// proptest! {
///     #[test]
///     fn my_store_get_after_put(
///         tenant in arb_tenant(),
///         key in arb_key(),
///         value in arb_test_value(),
///         ttl in arb_ttl()
///     ) {
///         tokio_test::block_on(async {
///             let store = MyStore::new();
///             store.put(&tenant, &key, &value, ttl).await.unwrap();
///             let got: Option<TestValue> = store.get(&tenant, &key).await.unwrap();
///             prop_assert_eq!(got, Some(value));
///             Ok(())
///         })?;
///     }
/// }
/// ```
pub mod proptest_strategies {
    use super::TestValue;
    use proptest::prelude::*;
    use std::time::Duration;

    /// Generate valid key strings (alphanumeric, 1-20 chars).
    pub fn arb_key() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9]{1,20}".prop_map(|s| s.to_string())
    }

    /// Generate valid tenant strings (alphanumeric, 1-10 chars).
    pub fn arb_tenant() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9]{1,10}".prop_map(|s| s.to_string())
    }

    /// Generate test values.
    pub fn arb_test_value() -> impl Strategy<Value = TestValue> {
        ("[a-zA-Z]{1,10}", 0..1000u32).prop_map(|(name, count)| TestValue { name, count })
    }

    /// Generate TTL: None (permanent) or Some(1s..24h).
    /// Long enough to never expire during test execution.
    pub fn arb_ttl() -> impl Strategy<Value = Option<Duration>> {
        prop_oneof![
            Just(None),
            (1u64..=86400).prop_map(|s| Some(Duration::from_secs(s))),
        ]
    }
}
