//! VectorStore algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1 (Upsert Idempotency): Upserting the same id overwrites without duplicates
//! - L2 (Search Monotonicity): Results ordered by descending similarity
//! - L3 (Delete Prefix): Exactly removes entries with specified prefix
//! - L4 (Get After Upsert): `upsert(id, ..); get_by_id(id)` returns `Some`
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_vector_store_satisfies_laws() {
//!     let store = MyVectorStore::new();
//!     agent_fw_test::vector_store_laws::test_all(&store).await;
//! }
//! ```

use agent_fw_algebra::vector_store::VectorStore;

/// Run all VectorStore laws.
pub async fn test_all(store: &dyn VectorStore) {
    law_get_after_upsert(store).await;
    law_upsert_idempotency(store).await;
    law_delete_prefix(store).await;
    law_search_monotonicity(store).await;
    law_search_threshold_filtering(store).await;
    law_count_consistency(store).await;
}

/// L4: Get After Upsert — upsert then get_by_id returns Some with matching content.
pub async fn law_get_after_upsert(store: &dyn VectorStore) {
    store
        .upsert_embedding(
            "law-l4-item",
            "test content",
            "doc",
            serde_json::json!({"key": "value"}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .expect("L4: upsert should succeed");

    let hit = store
        .get_by_id("law-l4-item")
        .await
        .expect("L4: get_by_id should succeed");

    assert!(hit.is_some(), "L4: get_by_id after upsert must return Some");
    let hit = hit.unwrap();
    assert_eq!(hit.id, "law-l4-item", "L4: id must match");
    assert_eq!(hit.content, "test content", "L4: content must match");

    // Cleanup
    store.delete_by_prefix("law-l4-").await.unwrap();
}

/// L1: Upsert Idempotency — upserting same id overwrites without duplicates.
pub async fn law_upsert_idempotency(store: &dyn VectorStore) {
    // Upsert v1
    store
        .upsert_embedding(
            "law-l1-item",
            "version 1",
            "doc",
            serde_json::json!({}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .expect("L1: first upsert should succeed");

    // Upsert v2 (same id, different content)
    store
        .upsert_embedding(
            "law-l1-item",
            "version 2",
            "doc",
            serde_json::json!({}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .expect("L1: second upsert should succeed");

    // Get by id should return v2
    let hit = store
        .get_by_id("law-l1-item")
        .await
        .expect("L1: get_by_id should succeed")
        .expect("L1: should find item");
    assert_eq!(
        hit.content, "version 2",
        "L1: upsert must overwrite content"
    );

    // Search should return exactly 1 hit (not 2)
    let hits = store
        .search_similar(&[1.0, 0.0, 0.0], 10, 0.0)
        .await
        .expect("L1: search should succeed");
    let matching: Vec<_> = hits.iter().filter(|h| h.id == "law-l1-item").collect();
    assert_eq!(
        matching.len(),
        1,
        "L1: upsert idempotency — should have exactly 1 entry, not {}",
        matching.len()
    );

    // Cleanup
    store.delete_by_prefix("law-l1-").await.unwrap();
}

/// L3: Delete Prefix — exactly removes entries with specified prefix.
pub async fn law_delete_prefix(store: &dyn VectorStore) {
    // Insert 3 items: 2 with "law-l3-foo-" prefix, 1 with "law-l3-bar-"
    store
        .upsert_embedding(
            "law-l3-foo-1",
            "f1",
            "doc",
            serde_json::json!({}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .unwrap();
    store
        .upsert_embedding(
            "law-l3-foo-2",
            "f2",
            "doc",
            serde_json::json!({}),
            &[0.0, 1.0, 0.0],
        )
        .await
        .unwrap();
    store
        .upsert_embedding(
            "law-l3-bar-1",
            "b1",
            "doc",
            serde_json::json!({}),
            &[0.0, 0.0, 1.0],
        )
        .await
        .unwrap();

    // Delete "law-l3-foo-" prefix
    let deleted = store
        .delete_by_prefix("law-l3-foo-")
        .await
        .expect("L3: delete_by_prefix should succeed");
    assert_eq!(deleted, 2, "L3: should delete exactly 2 items");

    // bar-1 should still exist
    let bar = store
        .get_by_id("law-l3-bar-1")
        .await
        .expect("L3: get_by_id should succeed");
    assert!(bar.is_some(), "L3: non-prefixed item must survive delete");

    // foo-1 should be gone
    let foo = store
        .get_by_id("law-l3-foo-1")
        .await
        .expect("L3: get_by_id should succeed");
    assert!(foo.is_none(), "L3: prefixed item must be deleted");

    // Cleanup
    store.delete_by_prefix("law-l3-").await.unwrap();
}

/// L5: Search threshold filtering — min_score parameter filters low-similarity results.
pub async fn law_search_threshold_filtering(store: &dyn VectorStore) {
    // Insert one close and one far item
    store
        .upsert_embedding(
            "law-l5-close",
            "close",
            "doc",
            serde_json::json!({}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .unwrap();
    store
        .upsert_embedding(
            "law-l5-far",
            "far",
            "doc",
            serde_json::json!({}),
            &[0.0, 0.0, 1.0],
        )
        .await
        .unwrap();

    // Search with high threshold — should filter out the far item
    let hits = store
        .search_similar(&[1.0, 0.0, 0.0], 10, 0.9)
        .await
        .expect("L5: search should succeed");

    for hit in &hits {
        assert!(
            hit.score >= 0.9,
            "L5: all returned hits must meet min_score threshold, got {}",
            hit.score
        );
    }

    // Cleanup
    store.delete_by_prefix("law-l5-").await.unwrap();
}

/// L6: Count consistency — after N upserts with distinct ids, search returns at most N.
pub async fn law_count_consistency(store: &dyn VectorStore) {
    let n = 5;
    for i in 0..n {
        store
            .upsert_embedding(
                &format!("law-l6-item-{i}"),
                &format!("content {i}"),
                "doc",
                serde_json::json!({}),
                &[1.0, 0.0, 0.0],
            )
            .await
            .expect("L6: upsert should succeed");
    }

    let hits = store
        .search_similar(&[1.0, 0.0, 0.0], 100, 0.0)
        .await
        .expect("L6: search should succeed");

    let matching: Vec<_> = hits
        .iter()
        .filter(|h| h.id.starts_with("law-l6-item-"))
        .collect();
    assert_eq!(
        matching.len(),
        n,
        "L6: should find exactly {n} items, got {}",
        matching.len()
    );

    // Cleanup
    store.delete_by_prefix("law-l6-").await.unwrap();
}

/// L2: Search Monotonicity — results ordered by descending similarity.
pub async fn law_search_monotonicity(store: &dyn VectorStore) {
    // Insert two items with known embeddings
    store
        .upsert_embedding(
            "law-l2-close",
            "close to query",
            "doc",
            serde_json::json!({}),
            &[1.0, 0.0, 0.0],
        )
        .await
        .unwrap();
    store
        .upsert_embedding(
            "law-l2-far",
            "far from query",
            "doc",
            serde_json::json!({}),
            &[0.0, 0.0, 1.0],
        )
        .await
        .unwrap();

    // Search with query [1,0,0] — "close" should rank higher
    let hits = store
        .search_similar(&[1.0, 0.0, 0.0], 2, 0.0)
        .await
        .expect("L2: search should succeed");

    assert!(
        hits.len() >= 2,
        "L2: should return at least 2 hits, got {}",
        hits.len()
    );

    // Verify monotonic ordering (descending score)
    for pair in hits.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "L2: search results must be ordered by descending score: {} >= {}",
            pair[0].score,
            pair[1].score
        );
    }

    // The closest item should be first
    assert_eq!(
        hits[0].id, "law-l2-close",
        "L2: closest item must rank first"
    );

    // Cleanup
    store.delete_by_prefix("law-l2-").await.unwrap();
}
