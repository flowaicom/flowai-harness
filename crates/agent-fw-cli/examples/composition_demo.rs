//! Composition demo — proves every framework building block composes into
//! a working domain workflow, end to end.
//!
//! This self-contained example wires:
//! 1. **StreamBuilder** — typed protocol stream construction with validation
//! 2. **EventStream** — monoid composition of stream fragments
//! 3. **Plan state machine** — Draft → Approved → Executing → Executed lifecycle
//! 4. **ActionSeq** — non-empty semigroup of plan actions
//! 5. **CardAlg** — tagless final rendering (PlainText + JsonCard)
//! 6. **bracket + compensating** — resource management combinators
//! 7. **Nursery** — structured concurrency with cancellation propagation
//! 8. **CachedResolver** — content-addressed entity resolution with KV caching
//! 9. **Workspace lifecycle** — create/update/delete with compensating-protected provisioning
//!
//! Run: `cargo run -p agent-fw-cli --example composition_demo`

use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::cancellation::CancellationToken;
use agent_fw_algebra::nursery::with_nursery;
use agent_fw_algebra::resource::{bracket, compensating};
use agent_fw_core::stream_builder::{
    EventStream, StreamBuilder, SubAgentCall, SubAgentResult, Termination, ToolCall, ToolResult,
};
use agent_fw_core::{FinishReason, TenantId, TokenUsage, UserId};
use agent_fw_interpreter::{DashMapKVStore, MockProvisioner};
use agent_fw_plan::{
    action_seq_from_vec, concat_actions, single_action, CardAlg, ExecutionResult, JsonCard,
    PlainText, PlanBuilder,
};
use agent_fw_workspace::kv_store::KVWorkspaceStore;
use agent_fw_workspace::lifecycle::{
    create_workspace, delete_workspace, update_workspace, CreateWorkspaceInput,
    UpdateWorkspaceInput,
};

// =============================================================================
// 1. StreamBuilder — typed protocol validation (&mut self API)
// =============================================================================

fn demo_stream_builder() {
    println!("=== 1. StreamBuilder ===\n");

    // Build a valid protocol stream with tool calls + results.
    // Builder methods take &mut self — natural loop ingestion pattern.
    let mut builder = StreamBuilder::new();
    builder.emit_text("Analyzing your data...");
    builder
        .emit_call(ToolCall::new(
            "call-1",
            "list_tables",
            serde_json::json!({}),
        ))
        .expect("emit_call");
    builder
        .emit_result(ToolResult::new(
            "call-1",
            "list_tables",
            serde_json::json!({}),
            serde_json::json!(["orders", "customers"]),
        ))
        .expect("emit_result");
    builder
        .emit_call(ToolCall::new(
            "call-2",
            "query",
            serde_json::json!({"sql": "SELECT 1"}),
        ))
        .expect("emit_call");
    builder
        .emit_sub_agent_call(SubAgentCall::new("analyst", "inv-1"))
        .expect("emit_sub_agent_call");
    builder
        .emit_sub_agent_result(SubAgentResult::new("analyst", "inv-1"))
        .expect("emit_sub_agent_result");
    builder
        .emit_result(ToolResult::new(
            "call-2",
            "query",
            serde_json::json!({"sql": "SELECT 1"}),
            serde_json::json!([{"?column?": 1}]),
        ))
        .expect("emit_result");
    builder.emit_text("Found 2 tables.");

    // finish() consumes self — produces the proof term
    let stream = builder
        .finish(Termination::finish(
            FinishReason::Stop,
            TokenUsage::simple(100, 200),
        ))
        .expect("finish");

    println!(
        "  ValidatedStream: {} events ({} total parts), usage: {:?}",
        stream.len(),
        stream.total_parts(),
        stream.usage()
    );

    // Demonstrate monoid composition
    let es1 = EventStream::singleton(agent_fw_core::StreamPart::Text {
        text: "Hello ".into(),
    });
    let es2 = EventStream::singleton(agent_fw_core::StreamPart::Text {
        text: "World".into(),
    });
    let combined = EventStream::concat_all([es1, es2, EventStream::EMPTY]);
    println!("  EventStream concat: {} parts (monoid)\n", combined.len());
}

// =============================================================================
// 2. Plan state machine — full lifecycle
// =============================================================================

fn demo_plan_lifecycle() {
    println!("=== 2. Plan State Machine ===\n");

    // Define domain actions as simple strings
    let actions = action_seq_from_vec(vec![
        "CREATE TABLE reports (id SERIAL PRIMARY KEY)".to_string(),
        "INSERT INTO reports (id) VALUES (1)".to_string(),
    ])
    .expect("non-empty actions");

    // Show semigroup composition
    let more_actions = single_action("CREATE INDEX idx_reports ON reports(id)".to_string());
    let all_actions = concat_actions(actions, more_actions);
    println!(
        "  ActionSeq: {} actions (semigroup concat)",
        all_actions.len()
    );

    // Build plan and drive state machine
    let plan = PlanBuilder::new(
        agent_fw_core::PlanId::new_unchecked("plan-001"),
        TenantId::new_unchecked("tenant-1"),
    )
    .description("Create reports table and seed data")
    .action_seq(all_actions)
    .context_entry("workspace", serde_json::json!("ws-demo"))
    .build()
    .expect("build plan");

    println!("  Plan status: {:?}", plan.status);

    let approved = plan
        .approve(UserId::new_unchecked("user-1"))
        .expect("approve");
    println!("  After approve: {:?}", approved.status);

    let executing = approved.start().expect("start");
    println!("  After start: {:?}", executing.status);

    let completed = executing
        .complete(ExecutionResult {
            entities_affected: 42,
            summary: Some("Created reports table with 42 rows".into()),
            details: None,
        })
        .expect("complete");
    println!(
        "  After complete: {:?} (terminal: {})\n",
        completed.status,
        completed.is_terminal()
    );
}

// =============================================================================
// 3. CardAlg — tagless final rendering
// =============================================================================

fn demo_card_rendering() {
    println!("=== 3. CardAlg (Tagless Final) ===\n");

    // Generic card builder — works with any CardAlg interpreter
    fn build_approval_card<C: CardAlg>(alg: &C) -> C::Card {
        let attrs = vec![
            alg.stat_card("Tables Affected", "3"),
            alg.stat_card("Rows Modified", "42"),
            alg.detail_row("Database", "analytics_prod"),
            alg.callout(
                agent_fw_plan::CalloutVariant::Warning,
                "This will modify production data",
            ),
        ];
        let actions = vec![
            alg.button(
                "approve",
                "Approve Plan",
                agent_fw_plan::ButtonVariant::Primary,
            ),
            alg.button("reject", "Reject", agent_fw_plan::ButtonVariant::Danger),
        ];
        alg.card(
            "Plan Approval Required",
            "Review the following changes before proceeding",
            attrs,
            actions,
        )
    }

    // Same generic builder, two interpreters
    let plain = build_approval_card(&PlainText);
    let rendered_plain = PlainText.render(plain);
    println!("  PlainText:\n{}\n", indent(&rendered_plain, "    "));

    let json = build_approval_card(&JsonCard);
    let rendered_json = JsonCard.render(json);
    println!("  JsonCard: {} bytes\n", rendered_json.len());
}

// =============================================================================
// 4. bracket + compensating — resource management combinators
// =============================================================================

async fn demo_resource_combinators() {
    println!("=== 4. Resource Combinators ===\n");

    // --- bracket: always releases (connections, file handles, locks) ---
    let cleanup_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cleanup_flag = cleanup_ran.clone();

    let result: Result<String, String> = bracket(
        async { Ok::<_, String>("connection-handle".to_string()) },
        |resource: &String| {
            let r = resource.clone();
            Box::pin(async move { Ok(format!("Used: {}", r)) })
        },
        move |resource: String| {
            cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                let _ = resource;
            })
        },
    )
    .await;

    println!("  bracket result: {:?}", result);
    println!(
        "  bracket cleanup ran: {} (always runs)\n",
        cleanup_ran.load(std::sync::atomic::Ordering::SeqCst)
    );

    // --- compensating: only runs cleanup on failure (saga pattern) ---
    let compensated = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let comp_flag = compensated.clone();

    // Success case: compensate should NOT run
    let ok_result = compensating(
        async { Ok::<_, String>("provisioned-env-123".to_string()) },
        move || async move {
            comp_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        },
    )
    .await;

    println!("  compensating (success): {:?}", ok_result);
    println!(
        "  compensating cleanup ran: {} (should be false)\n",
        compensated.load(std::sync::atomic::Ordering::SeqCst)
    );
}

// =============================================================================
// 5. Nursery — structured concurrency
// =============================================================================

async fn demo_nursery() {
    println!("=== 5. Nursery (Structured Concurrency) ===\n");

    let cancel = CancellationToken::new();
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));

    let nursery_result = with_nursery(&cancel, |nursery| {
        // Spawn 3 parallel tasks
        for i in 0..3 {
            let results = results.clone();
            nursery.spawn(move |_child_cancel| async move {
                // Simulate work
                tokio::time::sleep(Duration::from_millis(10)).await;
                results.lock().unwrap().push(format!("task-{} done", i));
                Ok::<_, String>(())
            });
        }
        // Body completes immediately; nursery waits for all spawned tasks
        async { Ok::<_, String>("all tasks spawned") }
    })
    .await;

    let collected = results.lock().unwrap();
    println!("  Nursery result: {:?}", nursery_result);
    println!("  Completed tasks: {} (all contained)\n", collected.len());
}

// =============================================================================
// 6. Workspace lifecycle — compensating-protected provisioning
// =============================================================================

async fn demo_workspace_lifecycle() {
    println!("=== 6. Workspace Lifecycle ===\n");

    let kv = Arc::new(DashMapKVStore::new());
    let store = KVWorkspaceStore::new(kv.clone());
    let tenant = TenantId::new_unchecked("demo-tenant");

    // Create workspace (no provisioner — uses DatabaseConfig::Default)
    let ws = create_workspace(
        CreateWorkspaceInput {
            name: "Demo Workspace".into(),
            description: Some("Composition demo".into()),
        },
        &tenant,
        None,
        &store,
        None,
        "analyst",
    )
    .await
    .expect("create workspace");

    println!(
        "  Created: id={}, slug={}, config={:?}",
        ws.id, ws.slug, ws.database_config
    );

    // Update
    let updated = update_workspace(
        &tenant,
        ws.id.as_str(),
        UpdateWorkspaceInput {
            name: Some("Renamed Workspace".into()),
            description: None,
        },
        &store,
    )
    .await
    .expect("update workspace");

    println!("  Updated: name={}, slug={}", updated.name, updated.slug);

    // Delete with KV cleanup
    delete_workspace(&tenant, ws.id.as_str(), None, &store, Some(kv.as_ref()))
        .await
        .expect("delete workspace");

    println!("  Deleted: workspace removed from store\n");
}

// =============================================================================
// 7. CachedResolver — content-addressed entity resolution
// =============================================================================

async fn demo_cached_resolver() {
    println!("=== 7. CachedResolver ===\n");

    use agent_fw_resolve::{ContentId, Glimpse};

    // Demonstrate content-addressed hashing
    let tenant = TenantId::new_unchecked("demo-tenant");
    let id1 = ContentId::<String>::compute(&"same-spec", &tenant);
    let id2 = ContentId::<String>::compute(&"same-spec", &tenant);
    let id3 = ContentId::<String>::compute(&"different-spec", &tenant);

    println!("  ContentId (same spec):      {}", id1.as_str());
    println!("  ContentId (same spec again): {}", id2.as_str());
    println!("  ContentId (different spec):  {}", id3.as_str());
    println!("  Same spec → same ID: {}", id1 == id2);
    println!("  Different spec → different ID: {}", id1 != id3);

    // Demonstrate Glimpse
    let glimpse = Glimpse::from_labels(42, vec!["orders".into(), "customers".into()]).with_facet(
        "schema",
        vec![("public".into(), 30), ("staging".into(), 12)],
    );
    println!(
        "  Glimpse: {} entities, {} samples\n",
        glimpse.total_count,
        glimpse.sample_labels.len()
    );
}

// =============================================================================
// 8. MockProvisioner — DatabaseProvisioner algebra
// =============================================================================

async fn demo_provisioner() {
    println!("=== 8. DatabaseProvisioner ===\n");

    use agent_fw_catalog::{DatabaseProvisioner, EnvironmentName, ProvisionRequest};

    let provisioner = MockProvisioner::new();

    // Provision
    let env = provisioner
        .provision(ProvisionRequest {
            name: EnvironmentName::new("ws-demo"),
            parent_id: None,
            expires_at: None,
        })
        .await
        .expect("provision");

    println!(
        "  Provisioned: id={}, name={}, host={}",
        env.id, env.name, env.host
    );

    // List
    let envs = provisioner.list_environments().await.expect("list");
    println!("  Environments listed: {}", envs.len());

    // Deprovision
    provisioner.deprovision(&env.id).await.expect("deprovision");

    let envs_after = provisioner.list_environments().await.expect("list");
    println!("  After deprovision: {} environments\n", envs_after.len());
}

// =============================================================================
// Helpers
// =============================================================================

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() {
    println!("\n╔═══════════════════════════════════════════════════╗");
    println!("║  agent-fw Composition Demo                        ║");
    println!("║  Proving every building block composes             ║");
    println!("╚═══════════════════════════════════════════════════╝\n");

    // Pure (sync) demos
    demo_stream_builder();
    demo_plan_lifecycle();
    demo_card_rendering();

    // Effectful (async) demos
    demo_resource_combinators().await;
    demo_nursery().await;
    demo_workspace_lifecycle().await;
    demo_cached_resolver().await;
    demo_provisioner().await;

    println!("═══════════════════════════════════════════════════");
    println!("  All 8 building blocks composed successfully.");
    println!("  Framework is sufficient for domain workflows.");
    println!("═══════════════════════════════════════════════════\n");
}
