//! Algebraic law harnesses for workspace store sub-traits.
//!
//! # Laws Tested
//!
//! ## Per Sub-Trait
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L1 | Roundtrip | `insert(x); get(x.id)` ≡ `Some(x)` |
//! | L2 | Delete-Get | `delete(id); get(id)` ≡ `None` |
//! | L3 | Upsert Idempotence | `upsert(x); upsert(x)` ≡ `upsert(x)` |
//! | L4 | List Consistency | after `insert(x)`, `list()` contains `x` |
//!
//! ## Cross-Cutting
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L5 | Tenant Isolation | tenant A cannot see tenant B's data |
//! | L6 | Cascade Delete | `delete_thread(id)` removes thread AND messages |
//! | L7 | Message Pagination | `get_messages(tid, limit, 0).len()` ≤ `limit` |
//!
//! # Usage
//!
//! ```ignore
//! use agent_fw_workspace::WorkspaceStore;
//!
//! #[tokio::test]
//! async fn my_store_satisfies_laws() {
//!     let store = MyWorkspaceStore::new();
//!     agent_fw_test::workspace_store_laws::test_all(&store).await;
//! }
//! ```

use agent_fw_core::{TenantId, TestCaseId};
use agent_fw_eval::test_case::AuthoredTestCase;
use agent_fw_eval::types::{EvalConfig, EvalRun};
use agent_fw_workspace::data_source::DataSource;
use agent_fw_workspace::store::WorkspaceStore;
use agent_fw_workspace::thread::{Message, Thread};
use agent_fw_workspace::workspace::Workspace;

fn tenant() -> TenantId {
    TenantId::new_unchecked("law-test-tenant")
}

fn other_tenant() -> TenantId {
    TenantId::new_unchecked("law-test-other")
}

fn make_data_source(id: &str) -> DataSource {
    DataSource {
        id: id.to_string(),
        name: format!("DS {}", id),
        database_type: agent_fw_workspace::data_source::DatabaseType::PostgreSQL,
        host: "localhost".to_string(),
        port: 5432,
        database_name: "testdb".to_string(),
        schema_name: "public".to_string(),
        encrypted_credentials: None,
        is_active: true,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

// =============================================================================
// ThreadStore Laws
// =============================================================================

/// L1: Roundtrip — upsert then get returns the same entity.
pub async fn law_thread_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let thread = Thread::with_id("law-rt-1", Some("Roundtrip".to_string()));
    store.upsert_thread(&t, &thread).await.unwrap();
    let got = store.get_thread(&t, "law-rt-1").await.unwrap();
    assert!(
        got.is_some(),
        "L1 Thread Roundtrip: get after upsert must return Some"
    );
    assert_eq!(got.unwrap().id, "law-rt-1");
}

/// L2: Delete-Get — delete then get returns None.
pub async fn law_thread_delete_get(store: &dyn WorkspaceStore) {
    let t = tenant();
    let thread = Thread::with_id("law-dg-1", None);
    store.upsert_thread(&t, &thread).await.unwrap();
    store.delete_thread(&t, "law-dg-1").await.unwrap();
    let got = store.get_thread(&t, "law-dg-1").await.unwrap();
    assert!(
        got.is_none(),
        "L2 Thread Delete-Get: get after delete must return None"
    );
}

/// L3: Upsert Idempotence — upserting twice produces same result.
pub async fn law_thread_upsert_idempotent(store: &dyn WorkspaceStore) {
    let t = tenant();
    let thread = Thread::with_id("law-ui-1", Some("Idempotent".to_string()));
    store.upsert_thread(&t, &thread).await.unwrap();
    store.upsert_thread(&t, &thread).await.unwrap();
    let list = store.list_threads(&t).await.unwrap();
    let count = list.iter().filter(|th| th.id == "law-ui-1").count();
    assert_eq!(
        count, 1,
        "L3 Thread Upsert Idempotence: entity must appear exactly once in list"
    );
}

/// L4: List Consistency — inserted entity appears in list.
pub async fn law_thread_list_consistency(store: &dyn WorkspaceStore) {
    let t = tenant();
    let thread = Thread::with_id("law-lc-1", Some("Listed".to_string()));
    store.upsert_thread(&t, &thread).await.unwrap();
    let list = store.list_threads(&t).await.unwrap();
    assert!(
        list.iter().any(|th| th.id == "law-lc-1"),
        "L4 Thread List Consistency: inserted thread must appear in list"
    );
}

// =============================================================================
// MessageStore Laws
// =============================================================================

/// L1: Insert then get_all contains the message.
pub async fn law_message_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let msg = Message::new("user", "law test message");
    store
        .insert_message(&t, &msg, "law-msg-thread")
        .await
        .unwrap();
    let all = store.get_all_messages(&t, "law-msg-thread").await.unwrap();
    assert!(
        all.iter().any(|m| m.id == msg.id),
        "L1 Message Roundtrip: inserted message must appear in get_all"
    );
}

/// L7: Pagination limit — get_messages respects limit parameter.
pub async fn law_message_pagination(store: &dyn WorkspaceStore) {
    let t = tenant();
    for i in 0..5 {
        let msg = Message::new("user", format!("paginated-{}", i));
        store
            .insert_message(&t, &msg, "law-pg-thread")
            .await
            .unwrap();
    }
    let page = store.get_messages(&t, "law-pg-thread", 2, 0).await.unwrap();
    assert!(
        page.len() <= 2,
        "L7 Message Pagination: get_messages(limit=2) returned {} items",
        page.len()
    );
}

// =============================================================================
// TestCaseStore Laws
// =============================================================================

/// L1: Roundtrip — insert then get.
pub async fn law_test_case_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let tc = AuthoredTestCase::new(
        TestCaseId::new_unchecked("law-tc-rt-1"),
        "law test input".to_string(),
    );
    store.insert_test_case(&t, &tc).await.unwrap();
    let got = store.get_test_case(&t, "law-tc-rt-1").await.unwrap();
    assert!(
        got.is_some(),
        "L1 TestCase Roundtrip: get after insert must return Some"
    );
    assert_eq!(got.unwrap().input, "law test input");
}

/// L2: Delete-Get.
pub async fn law_test_case_delete_get(store: &dyn WorkspaceStore) {
    let t = tenant();
    let tc = AuthoredTestCase::new(
        TestCaseId::new_unchecked("law-tc-dg-1"),
        "delete me".to_string(),
    );
    store.insert_test_case(&t, &tc).await.unwrap();
    store.delete_test_case(&t, "law-tc-dg-1").await.unwrap();
    let got = store.get_test_case(&t, "law-tc-dg-1").await.unwrap();
    assert!(
        got.is_none(),
        "L2 TestCase Delete-Get: get after delete must return None"
    );
}

/// L4: List Consistency.
pub async fn law_test_case_list_consistency(store: &dyn WorkspaceStore) {
    let t = tenant();
    let tc = AuthoredTestCase::new(
        TestCaseId::new_unchecked("law-tc-lc-1"),
        "list me".to_string(),
    );
    store.insert_test_case(&t, &tc).await.unwrap();
    let list = store.list_test_cases(&t).await.unwrap();
    assert!(
        list.iter().any(|c| c.id.as_str() == "law-tc-lc-1"),
        "L4 TestCase List Consistency: inserted test case must appear in list"
    );
}

// =============================================================================
// EvalStore Laws
// =============================================================================

/// L1: Roundtrip — insert_run then get_run.
pub async fn law_eval_run_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let run = EvalRun::new(EvalConfig::default());
    let id = run.id.as_str().to_string();
    store.insert_eval_run(&t, &run).await.unwrap();
    let got = store.get_eval_run(&t, &id).await.unwrap();
    assert!(
        got.is_some(),
        "L1 EvalRun Roundtrip: get_run after insert must return Some"
    );
    assert_eq!(got.unwrap().id, run.id);
}

/// L2: Delete-Get.
pub async fn law_eval_run_delete_get(store: &dyn WorkspaceStore) {
    let t = tenant();
    let run = EvalRun::new(EvalConfig::default());
    let id = run.id.as_str().to_string();
    store.insert_eval_run(&t, &run).await.unwrap();
    store.delete_eval_run(&t, &id).await.unwrap();
    let got = store.get_eval_run(&t, &id).await.unwrap();
    assert!(
        got.is_none(),
        "L2 EvalRun Delete-Get: get_run after delete must return None"
    );
}

/// L4: List Consistency.
pub async fn law_eval_run_list_consistency(store: &dyn WorkspaceStore) {
    let t = tenant();
    let run = EvalRun::new(EvalConfig::default());
    let id = run.id.clone();
    store.insert_eval_run(&t, &run).await.unwrap();
    let list = store.list_eval_runs(&t).await.unwrap();
    assert!(
        list.iter().any(|r| r.id == id),
        "L4 EvalRun List Consistency: inserted eval run must appear in list"
    );
}

// =============================================================================
// DataSourceStore Laws
// =============================================================================

/// L1: Roundtrip — upsert then get.
pub async fn law_data_source_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ds = make_data_source("law-ds-rt-1");
    store.upsert_data_source(&t, &ds).await.unwrap();
    let got = store.get_data_source(&t, "law-ds-rt-1").await.unwrap();
    assert!(
        got.is_some(),
        "L1 DataSource Roundtrip: get after upsert must return Some"
    );
    assert_eq!(got.unwrap().id, "law-ds-rt-1");
}

/// L2: Delete-Get.
pub async fn law_data_source_delete_get(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ds = make_data_source("law-ds-dg-1");
    store.upsert_data_source(&t, &ds).await.unwrap();
    store.delete_data_source(&t, "law-ds-dg-1").await.unwrap();
    let got = store.get_data_source(&t, "law-ds-dg-1").await.unwrap();
    assert!(
        got.is_none(),
        "L2 DataSource Delete-Get: get after delete must return None"
    );
}

/// L3: Upsert Idempotence.
pub async fn law_data_source_upsert_idempotent(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ds = make_data_source("law-ds-ui-1");
    store.upsert_data_source(&t, &ds).await.unwrap();
    store.upsert_data_source(&t, &ds).await.unwrap();
    let list = store.list_data_sources(&t).await.unwrap();
    let count = list.iter().filter(|d| d.id == "law-ds-ui-1").count();
    assert_eq!(
        count, 1,
        "L3 DataSource Upsert Idempotence: must appear exactly once"
    );
}

/// L4: List Consistency.
pub async fn law_data_source_list_consistency(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ds = make_data_source("law-ds-lc-1");
    store.upsert_data_source(&t, &ds).await.unwrap();
    let list = store.list_data_sources(&t).await.unwrap();
    assert!(
        list.iter().any(|d| d.id == "law-ds-lc-1"),
        "L4 DataSource List Consistency: inserted data source must appear in list"
    );
}

// =============================================================================
// WorkspaceEntityStore Laws
// =============================================================================

/// L1: Roundtrip — create then get.
pub async fn law_workspace_entity_roundtrip(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ws = Workspace::default_workspace();
    store.create_workspace(&t, &ws).await.unwrap();
    let got = store.get_workspace(&t, "default").await.unwrap();
    assert!(
        got.is_some(),
        "L1 Workspace Roundtrip: get after create must return Some"
    );
    assert_eq!(got.unwrap().slug, "default");
}

/// L2: Delete-Get.
pub async fn law_workspace_entity_delete_get(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ws = Workspace::default_workspace();
    store.create_workspace(&t, &ws).await.unwrap();
    store.delete_workspace(&t, "default").await.unwrap();
    let got = store.get_workspace(&t, "default").await.unwrap();
    assert!(
        got.is_none(),
        "L2 Workspace Delete-Get: get after delete must return None"
    );
}

/// L4: List Consistency.
pub async fn law_workspace_entity_list_consistency(store: &dyn WorkspaceStore) {
    let t = tenant();
    let ws = Workspace::default_workspace();
    store.create_workspace(&t, &ws).await.unwrap();
    let list = store.list_workspaces(&t).await.unwrap();
    assert!(
        list.iter().any(|w| w.id.is_default()),
        "L4 Workspace List Consistency: created workspace must appear in list"
    );
}

// =============================================================================
// Cross-Cutting Laws
// =============================================================================

/// L5: Tenant Isolation — data inserted under tenant A is invisible to tenant B.
pub async fn law_tenant_isolation(store: &dyn WorkspaceStore) {
    let t1 = tenant();
    let t2 = other_tenant();

    let thread = Thread::with_id("law-iso-1", Some("Isolated".to_string()));
    store.upsert_thread(&t1, &thread).await.unwrap();

    let got = store.get_thread(&t2, "law-iso-1").await.unwrap();
    assert!(
        got.is_none(),
        "L5 Tenant Isolation: tenant B must not see tenant A's thread"
    );
}

/// L6: Cascade Delete — deleting a thread also deletes its messages.
pub async fn law_cascade_delete(store: &dyn WorkspaceStore) {
    let t = tenant();
    let thread = Thread::with_id("law-cascade-1", None);
    let msg = Message::new("user", "will be deleted");

    store.upsert_thread(&t, &thread).await.unwrap();
    store
        .insert_message(&t, &msg, "law-cascade-1")
        .await
        .unwrap();
    store.delete_thread(&t, "law-cascade-1").await.unwrap();

    let messages = store.get_all_messages(&t, "law-cascade-1").await.unwrap();
    assert!(
        messages.is_empty(),
        "L6 Cascade Delete: messages must be deleted when thread is deleted"
    );
}

// =============================================================================
// test_all — Run all laws
// =============================================================================

/// Run all workspace store laws against the given implementation.
///
/// Tests L1-L7 across all 6 sub-traits + cross-cutting laws.
pub async fn test_all(store: &dyn WorkspaceStore) {
    // ThreadStore
    law_thread_roundtrip(store).await;
    law_thread_delete_get(store).await;
    law_thread_upsert_idempotent(store).await;
    law_thread_list_consistency(store).await;

    // MessageStore
    law_message_roundtrip(store).await;
    law_message_pagination(store).await;

    // TestCaseStore
    law_test_case_roundtrip(store).await;
    law_test_case_delete_get(store).await;
    law_test_case_list_consistency(store).await;

    // EvalStore
    law_eval_run_roundtrip(store).await;
    law_eval_run_delete_get(store).await;
    law_eval_run_list_consistency(store).await;

    // DataSourceStore
    law_data_source_roundtrip(store).await;
    law_data_source_delete_get(store).await;
    law_data_source_upsert_idempotent(store).await;
    law_data_source_list_consistency(store).await;

    // WorkspaceEntityStore
    law_workspace_entity_roundtrip(store).await;
    law_workspace_entity_delete_get(store).await;
    law_workspace_entity_list_consistency(store).await;

    // Cross-cutting
    law_tenant_isolation(store).await;
    law_cascade_delete(store).await;
}
