//! Integration tests: run law harnesses against framework implementations.

use agent_fw_algebra::EventSink;
use agent_fw_core::StreamPart;
use agent_fw_interpreter::{ChannelEventSink, DashMapKVStore, MockVectorStore};
use std::sync::Arc;
use tokio_stream::StreamExt;

#[tokio::test]
async fn dashmap_kv_satisfies_all_laws() {
    let store = DashMapKVStore::new();
    agent_fw_test::kv_laws::test_all(&store).await;
}

#[tokio::test]
async fn channel_sink_order_preservation() {
    let (sink, mut rx) = ChannelEventSink::new(100);

    // Emit events in known order
    sink.emit(StreamPart::text("first"));
    sink.emit(StreamPart::text("second"));
    sink.emit(StreamPart::text("third"));
    sink.close();
    drop(sink); // Drop sender to unblock receiver

    // Drain from the ReceiverStream
    let mut events = Vec::new();
    while let Some(event) = rx.next().await {
        events.push(event);
    }

    let texts: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            StreamPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    assert!(texts.len() >= 3, "must receive at least 3 text events");
    assert_eq!(texts[0], "first");
    assert_eq!(texts[1], "second");
    assert_eq!(texts[2], "third");
}

#[tokio::test]
async fn channel_sink_closure_and_idempotent_close() {
    let (sink, _rx) = ChannelEventSink::new(100);
    agent_fw_test::event_sink_laws::law_totality(&sink);

    // Use a fresh sink for closure test (totality doesn't close)
    let (sink2, _rx2) = ChannelEventSink::new(100);
    agent_fw_test::event_sink_laws::law_closure_semantics(&sink2);

    // And another fresh one for the combined test
    let (sink3, _rx3) = ChannelEventSink::new(100);
    agent_fw_test::event_sink_laws::law_closure_semantics(&sink3);
    agent_fw_test::event_sink_laws::law_idempotent_close(&sink3);
}

#[tokio::test]
async fn cancellation_satisfies_all_laws() {
    agent_fw_test::cancellation_laws::test_all().await;
}

#[test]
fn plan_satisfies_all_transition_laws() {
    agent_fw_test::plan_laws::test_all();
}

#[test]
fn metric_point_satisfies_all_monoid_laws() {
    agent_fw_test::metric_point_laws::test_all();
}

#[test]
fn trajectory_scorer_satisfies_all_eval_scorer_laws() {
    let scorer = agent_fw_eval::TrajectoryScorer::default();
    agent_fw_test::eval_scorer_laws::test_all(&scorer);
}

#[test]
fn composite_scorer_satisfies_all_eval_scorer_laws() {
    let scorer = agent_fw_eval::CompositeScorer::trajectory_only();
    agent_fw_test::eval_scorer_laws::test_all(&scorer);
}

#[tokio::test]
async fn kv_workspace_store_satisfies_all_laws() {
    let kv = Arc::new(DashMapKVStore::new());
    let store = agent_fw_workspace::KVWorkspaceStore::new(kv);
    agent_fw_test::workspace_store_laws::test_all(&store).await;
}

#[tokio::test]
async fn mock_vector_store_satisfies_all_laws() {
    let store = MockVectorStore::new();
    agent_fw_test::vector_store_laws::test_all(&store).await;
}

#[tokio::test]
async fn mock_embedding_service_satisfies_all_laws() {
    let service = agent_fw_test::embedding_service_laws::MockEmbeddingService::new(64);
    agent_fw_test::embedding_service_laws::test_all(&service).await;
}

#[test]
fn non_empty_satisfies_all_laws() {
    agent_fw_test::non_empty_laws::test_all();
}

#[tokio::test]
async fn fallback_satisfies_all_laws() {
    agent_fw_test::fallback_laws::test_all().await;
}

#[test]
fn relocation_satisfies_all_laws() {
    agent_fw_test::relocation_laws::test_all();
}

#[test]
fn table_role_satisfies_all_laws() {
    agent_fw_test::table_role_laws::test_all();
}

#[test]
fn semantic_model_satisfies_all_laws() {
    agent_fw_test::semantic_laws::test_all();
}

#[tokio::test]
async fn pipeline_ctx_satisfies_all_laws() {
    let make_kv = || Arc::new(DashMapKVStore::new()) as Arc<dyn agent_fw_algebra::KVStore>;
    agent_fw_test::pipeline_ctx_laws::test_all(make_kv).await;
}
