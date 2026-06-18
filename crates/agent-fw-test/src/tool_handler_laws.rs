//! ToolHandler and ComposedDispatcher algebraic law test harnesses.
//!
//! # Verified Laws
//!
//! ## ToolHandler
//!
//! - **L2 (Deterministic Schema)**: `definition()` called twice yields
//!   structurally equal `ToolDefinition` values.
//! - **L3 (Error Totality)**: `handle()` never panics; all failures are
//!   `ToolCallResult { is_error: true, .. }`.
//!
//! ## ComposedDispatcher
//!
//! - **L1 (Dispatch Fidelity)**: `dispatch(name, id, input)` routes to the
//!   handler registered under `name`.
//! - **L2 (Unknown Tool Error)**: `dispatch` for unregistered name returns
//!   `ToolCallResult::error(..)`.
//! - **L3 (Monoidal Identity)**: `empty.merge(d) == d`, `d.merge(empty) == d`.
//! - **L4 (Monoidal Associativity)**: `(a.merge(b)).merge(c)` routes the same
//!   as `a.merge(b.merge(c))`.
//! - **L5 (Definition Completeness)**: `tool_definitions()` matches registered
//!   handlers.
//! - **L6 (Disjoint try_merge)**: `try_merge` returns `Ok` if the two
//!   dispatchers have disjoint name sets; otherwise it returns every
//!   duplicate as a `ToolCollision`.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn tool_handler_satisfies_all_laws() {
//!     agent_fw_test::tool_handler_laws::test_all().await;
//! }
//! ```

use agent_fw_agent::{
    traced, ComposedDispatcher, ToolCallResult, ToolDefinition, ToolDispatcher, ToolHandler,
    TracedHandler,
};
use agent_fw_algebra::event_sink::EventSink;
use agent_fw_algebra::{CancellationToken, KVStore, SubAgentInvoker};
use agent_fw_core::stream_part::ToolInvocationState;
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::StreamPart;
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ─── Test Environment Construction ──────────────────────────────────────

struct LawKV;
#[async_trait]
impl KVStore for LawKV {
    async fn put_json(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
        _: Option<std::time::Duration>,
    ) -> Result<(), agent_fw_algebra::KVError> {
        Ok(())
    }
    async fn get_json(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<serde_json::Value>, agent_fw_algebra::KVError> {
        Ok(None)
    }
    async fn delete(&self, _: &str, _: &str) -> Result<bool, agent_fw_algebra::KVError> {
        Ok(false)
    }
    async fn exists(&self, _: &str, _: &str) -> Result<bool, agent_fw_algebra::KVError> {
        Ok(false)
    }
    async fn list_keys(&self, _: &str, _: &str) -> Result<Vec<String>, agent_fw_algebra::KVError> {
        Ok(vec![])
    }
    async fn get_many_json(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, agent_fw_algebra::KVError>
    {
        Ok(Default::default())
    }
}

struct LawSink {
    open: AtomicBool,
}
impl LawSink {
    fn new() -> Self {
        Self {
            open: AtomicBool::new(true),
        }
    }
}
impl EventSink for LawSink {
    fn emit(&self, _: agent_fw_core::StreamPart) -> bool {
        self.is_open()
    }
    fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
    }
    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}

struct LawSubAgent;
#[async_trait]
impl SubAgentInvoker for LawSubAgent {
    async fn invoke(
        &self,
        _: agent_fw_algebra::sub_agent::SubAgentRequest,
    ) -> Result<agent_fw_algebra::sub_agent::SubAgentResult, agent_fw_algebra::SubAgentError> {
        Err(agent_fw_algebra::SubAgentError::NotFound("law-mock".into()))
    }
    fn has_agent(&self, _: &str) -> bool {
        false
    }
    fn available_agents(&self) -> Vec<String> {
        vec![]
    }
}

fn law_env() -> ToolEnvironment {
    let kv: Arc<dyn KVStore> = Arc::new(LawKV);
    let sink: Arc<dyn EventSink> = Arc::new(LawSink::new());
    let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(LawSubAgent);
    let cancel = CancellationToken::new();
    let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("law-test"));
    ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel)
}

// ─── Test ToolHandler Implementations ───────────────────────────────────

/// A handler that echoes input, tagged with its name.
struct TagHandler {
    tag: String,
}

impl TagHandler {
    fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
        }
    }
}

#[async_trait]
impl ToolHandler for TagHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.tag.clone(),
            description: format!("Tag handler: {}", self.tag),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            }),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        _env: &ToolEnvironment,
    ) -> ToolCallResult {
        ToolCallResult::success(
            tool_use_id,
            serde_json::json!({
                "tag": self.tag,
                "input": input,
            }),
        )
    }
}

/// A handler that always returns an error.
struct FailHandler {
    name: String,
}

impl FailHandler {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

#[async_trait]
impl ToolHandler for FailHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: "Always fails".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        _input: serde_json::Value,
        _env: &ToolEnvironment,
    ) -> ToolCallResult {
        ToolCallResult::error(tool_use_id, "intentional failure")
    }
}

// ─── Public Law Test Functions ──────────────────────────────────────────

/// Run all ToolHandler, ComposedDispatcher, and TracedHandler laws.
pub async fn test_all() {
    // ToolHandler laws
    law_deterministic_schema();
    law_error_totality_success().await;
    law_error_totality_failure().await;

    // ComposedDispatcher laws
    law_dispatch_fidelity().await;
    law_unknown_tool_error().await;
    law_monoidal_identity_left();
    law_monoidal_identity_right();
    law_monoidal_associativity().await;
    law_definition_completeness();
    law_try_merge_disjoint_ok();
    law_try_merge_intersection_err();

    // TracedHandler laws
    law_traced_transparency();
    law_traced_semantic_preservation().await;
    law_traced_event_ordering().await;
    law_traced_convenience();
}

/// L2 (Deterministic Schema): `definition()` called twice yields same result.
pub fn law_deterministic_schema() {
    let handler = TagHandler::new("deterministic");
    let d1 = handler.definition();
    let d2 = handler.definition();
    assert_eq!(
        d1.name, d2.name,
        "L2: definition name must be deterministic"
    );
    assert_eq!(
        d1.description, d2.description,
        "L2: definition description must be deterministic"
    );
    assert_eq!(
        d1.input_schema, d2.input_schema,
        "L2: definition input_schema must be deterministic"
    );
}

/// L3 (Error Totality): successful handler returns `is_error: false`.
pub async fn law_error_totality_success() {
    let handler = TagHandler::new("ok");
    let env = law_env();
    let result = handler.handle("id", serde_json::json!({}), &env).await;
    assert!(
        !result.is_error,
        "L3: TagHandler should return success, not error"
    );
}

/// L3 (Error Totality): failing handler returns `is_error: true` (not panic).
pub async fn law_error_totality_failure() {
    let handler = FailHandler::new("fail");
    let env = law_env();
    let result = handler.handle("id", serde_json::json!({}), &env).await;
    assert!(
        result.is_error,
        "L3: FailHandler should return error, not success"
    );
}

/// L1 (Dispatch Fidelity): dispatch routes to the correct handler.
pub async fn law_dispatch_fidelity() {
    let d = ComposedDispatcher::new(law_env()).with_handlers(vec![
        Arc::new(TagHandler::new("alpha")) as Arc<dyn ToolHandler>,
        Arc::new(TagHandler::new("beta")),
    ]);

    let result = d
        .dispatch("alpha", "id-1", serde_json::json!({"value": "a"}))
        .await;
    assert!(
        !result.is_error,
        "L1: dispatch to registered handler should succeed"
    );
    assert_eq!(
        result.content["tag"], "alpha",
        "L1: dispatch must route to the handler named 'alpha'"
    );

    let result = d
        .dispatch("beta", "id-2", serde_json::json!({"value": "b"}))
        .await;
    assert_eq!(
        result.content["tag"], "beta",
        "L1: dispatch must route to the handler named 'beta'"
    );
}

/// L2 (Unknown Tool Error): dispatch for unregistered name returns error.
pub async fn law_unknown_tool_error() {
    let d = ComposedDispatcher::new(law_env()).with_handler(Arc::new(TagHandler::new("known")));

    let result = d.dispatch("unknown", "id-3", serde_json::json!({})).await;
    assert!(
        result.is_error,
        "L2: dispatch to unknown tool must return error"
    );
    assert_eq!(
        result.tool_use_id, "id-3",
        "L2: error must preserve tool_use_id"
    );
}

/// L3 (Monoidal Identity, left): `empty.merge(d)` has same handlers as `d`.
pub fn law_monoidal_identity_left() {
    let empty = ComposedDispatcher::new(law_env());
    let d = ComposedDispatcher::new(law_env()).with_handlers(vec![
        Arc::new(TagHandler::new("a")) as Arc<dyn ToolHandler>,
        Arc::new(TagHandler::new("b")),
    ]);

    let merged = empty.merge(d);
    assert_eq!(
        merged.len(),
        2,
        "L3 left: empty.merge(d) should have d's handlers"
    );
    assert!(
        merged.has_handler("a") && merged.has_handler("b"),
        "L3 left: all handlers from d must be present"
    );
}

/// L3 (Monoidal Identity, right): `d.merge(empty)` has same handlers as `d`.
pub fn law_monoidal_identity_right() {
    let d = ComposedDispatcher::new(law_env()).with_handlers(vec![
        Arc::new(TagHandler::new("a")) as Arc<dyn ToolHandler>,
        Arc::new(TagHandler::new("b")),
    ]);
    let empty = ComposedDispatcher::new(law_env());

    let merged = d.merge(empty);
    assert_eq!(
        merged.len(),
        2,
        "L3 right: d.merge(empty) should have d's handlers"
    );
    assert!(
        merged.has_handler("a") && merged.has_handler("b"),
        "L3 right: all handlers from d must be present"
    );
}

/// L4 (Monoidal Associativity): `(a.merge(b)).merge(c)` routes the same
/// as `a.merge(b.merge(c))`.
pub async fn law_monoidal_associativity() {
    // Build three dispatchers with distinct tools
    let make_a = || {
        ComposedDispatcher::new(law_env())
            .with_handler(Arc::new(TagHandler::new("tool_a")) as Arc<dyn ToolHandler>)
    };
    let make_b = || {
        ComposedDispatcher::new(law_env())
            .with_handler(Arc::new(TagHandler::new("tool_b")) as Arc<dyn ToolHandler>)
    };
    let make_c = || {
        ComposedDispatcher::new(law_env())
            .with_handler(Arc::new(TagHandler::new("tool_c")) as Arc<dyn ToolHandler>)
    };

    // (a.merge(b)).merge(c)
    let left = make_a().merge(make_b()).merge(make_c());
    // a.merge(b.merge(c))
    let right = make_a().merge(make_b().merge(make_c()));

    // Both should have all three tools
    assert_eq!(
        left.len(),
        3,
        "L4: (a.merge(b)).merge(c) should have 3 handlers"
    );
    assert_eq!(
        right.len(),
        3,
        "L4: a.merge(b.merge(c)) should have 3 handlers"
    );

    // Both should route identically for all tool names
    for name in &["tool_a", "tool_b", "tool_c"] {
        let lr = left.dispatch(name, "id", serde_json::json!({})).await;
        let rr = right.dispatch(name, "id", serde_json::json!({})).await;
        assert_eq!(
            lr.content["tag"], rr.content["tag"],
            "L4: associativity violated for tool '{name}'"
        );
    }
}

/// L6 (Disjoint `try_merge`): when two dispatchers have no shared tool names,
/// `try_merge` returns the union of their handlers without raising a
/// collision.
pub fn law_try_merge_disjoint_ok() {
    let a = ComposedDispatcher::new(law_env())
        .with_handler(Arc::new(TagHandler::new("alpha")) as Arc<dyn ToolHandler>);
    let b = ComposedDispatcher::new(law_env())
        .with_handler(Arc::new(TagHandler::new("beta")) as Arc<dyn ToolHandler>);

    let merged = a.try_merge(b).expect("L6: disjoint try_merge must succeed");
    assert_eq!(
        merged.len(),
        2,
        "L6: merged dispatcher must have both tools"
    );
    assert!(merged.has_handler("alpha"));
    assert!(merged.has_handler("beta"));
}

/// L6 (Intersecting `try_merge`): when two dispatchers share at least one tool
/// name, `try_merge` reports every duplicate (and only those) as a
/// `ToolCollision`.
pub fn law_try_merge_intersection_err() {
    let a = ComposedDispatcher::new(law_env()).with_handlers(vec![
        Arc::new(TagHandler::new("shared_one")) as Arc<dyn ToolHandler>,
        Arc::new(TagHandler::new("unique_a")),
    ]);
    let b = ComposedDispatcher::new(law_env()).with_handlers(vec![
        Arc::new(TagHandler::new("shared_one")) as Arc<dyn ToolHandler>,
        Arc::new(TagHandler::new("shared_two")),
        Arc::new(TagHandler::new("unique_b")),
    ]);

    // Salt `b` with one extra overlap so the law exercises >1 duplicate.
    let b = b.with_handler(Arc::new(TagHandler::new("unique_a")));

    let collisions = a
        .try_merge(b)
        .err()
        .expect("L6: overlapping try_merge must fail");

    let mut names: Vec<_> = collisions.iter().map(|c| c.tool_name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["shared_one", "unique_a"],
        "L6: collision set must equal the intersection of name sets"
    );
}

// ─── Recording EventSink for TracedHandler tests ─────────────────────

use agent_fw_algebra::testing::RecordingEventSink;

fn recording_env() -> (ToolEnvironment, Arc<RecordingEventSink>) {
    let kv: Arc<dyn KVStore> = Arc::new(LawKV);
    let sink = Arc::new(RecordingEventSink::new());
    let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(LawSubAgent);
    let cancel = CancellationToken::new();
    let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("law-test"));
    let env = ToolEnvironment::new(
        kv,
        sink.clone() as Arc<dyn EventSink>,
        sub_agents,
        tenant,
        cancel,
    );
    (env, sink)
}

// ─── TracedHandler Laws ─────────────────────────────────────────────

/// TracedHandler L1 (Transparency): `definition()` is identical to inner handler.
pub fn law_traced_transparency() {
    let inner = TagHandler::new("traced_test");
    let handler = TracedHandler::new(TagHandler::new("traced_test"));
    assert_eq!(
        inner.definition().name,
        handler.definition().name,
        "Traced L1: definition name must match inner"
    );
    assert_eq!(
        inner.definition().description,
        handler.definition().description,
        "Traced L1: definition description must match inner"
    );
    assert_eq!(
        inner.definition().input_schema,
        handler.definition().input_schema,
        "Traced L1: definition schema must match inner"
    );
}

/// TracedHandler L2 (Semantic preservation): result identical to unwrapped.
pub async fn law_traced_semantic_preservation() {
    let env = law_env();
    let input = serde_json::json!({"value": "test"});

    let inner_result = TagHandler::new("echo")
        .handle("id-1", input.clone(), &env)
        .await;
    let traced_result = traced(TagHandler::new("echo"))
        .handle("id-2", input, &env)
        .await;

    assert_eq!(
        inner_result.is_error, traced_result.is_error,
        "Traced L2: is_error must match"
    );
    assert_eq!(
        inner_result.content, traced_result.content,
        "Traced L2: content must match"
    );
}

/// TracedHandler L3 (Event ordering): tool_call precedes tool_result.
pub async fn law_traced_event_ordering() {
    let (env, sink) = recording_env();
    let handler = traced(TagHandler::new("ordered"));
    let input = serde_json::json!({"value": "test"});

    handler.handle("trace-1", input.clone(), &env).await;

    let events = sink.events();
    assert_eq!(events.len(), 2, "Traced L3: exactly 2 events emitted");

    assert!(
        matches!(&events[0], StreamPart::ToolInvocation(data)
            if matches!(data.state, ToolInvocationState::Call)),
        "Traced L3: first event must be tool_call"
    );

    assert!(
        matches!(&events[1], StreamPart::ToolInvocation(data)
            if matches!(data.state, ToolInvocationState::Result { .. })),
        "Traced L3: second event must be tool_result"
    );

    // Verify tool_result carries original args
    if let StreamPart::ToolInvocation(data) = &events[1] {
        assert_eq!(
            data.args, input,
            "Traced L3: tool_result must carry original input args"
        );
    }
}

/// TracedHandler convenience function works.
pub fn law_traced_convenience() {
    let handler = traced(TagHandler::new("convenience"));
    assert_eq!(handler.definition().name, "convenience");
}

/// L5 (Definition Completeness): `tool_definitions()` matches handler set.
pub fn law_definition_completeness() {
    let handlers: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(TagHandler::new("x")),
        Arc::new(TagHandler::new("y")),
        Arc::new(TagHandler::new("z")),
    ];

    let d = ComposedDispatcher::new(law_env()).with_handlers(handlers.clone());

    let defs = d.tool_definitions();
    assert_eq!(
        defs.len(),
        handlers.len(),
        "L5: definition count must match handler count"
    );

    let def_names: std::collections::HashSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for h in &handlers {
        let name = h.definition().name;
        assert!(
            def_names.contains(name.as_str()),
            "L5: handler '{name}' missing from tool_definitions()"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tool_handler_and_composed_dispatcher_satisfy_all_laws() {
        test_all().await;
    }
}
