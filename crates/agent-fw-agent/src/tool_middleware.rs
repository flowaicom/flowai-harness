//! Tool handler middleware combinators: Timeout, Retry.
//!
//! Composable wrappers that add cross-cutting behavior to any [`ToolHandler`].
//! Each combinator preserves the inner handler's definition (transparency law)
//! and composes naturally:
//!
//! ```rust,ignore
//! use agent_fw_agent::{traced, with_tool_timeout, with_tool_retry};
//! use std::time::Duration;
//!
//! let handler = traced(
//!     with_tool_timeout(
//!         with_tool_retry(my_handler, 3, Duration::from_millis(100)),
//!         Duration::from_secs(30),
//!     )
//! );
//! ```
//!
//! # Laws
//!
//! ## TimeoutHandler
//!
//! - **L1 (Transparency)**: `definition()` delegates to inner handler.
//! - **L2 (Timeout→Error)**: If inner handler exceeds deadline,
//!   returns `ToolCallResult::error` with a timeout message.
//! - **L3 (Success passthrough)**: If inner handler completes within
//!   deadline, result is identical to unwrapped handler.
//!
//! ## RetryHandler
//!
//! - **L1 (Transparency)**: `definition()` delegates to inner handler.
//! - **L2 (No retry on success)**: If inner handler succeeds, result is
//!   returned immediately without retry.
//! - **L3 (Retry on error)**: If inner handler returns `is_error: true`,
//!   the handler is retried up to `max_retries` times.
//! - **L4 (Exhaustion)**: After all retries exhausted, the last error
//!   result is returned.

use async_trait::async_trait;
use std::time::Duration;

use agent_fw_tool::ToolEnvironment;

use crate::{ToolCallResult, ToolDefinition, ToolHandler};

// ─── TimeoutHandler ──────────────────────────────────────────────────

/// Wraps a [`ToolHandler`] with a tokio timeout.
///
/// If the inner handler does not complete within the deadline, an error
/// `ToolCallResult` is returned instead.
pub struct TimeoutHandler<H> {
    inner: H,
    deadline: Duration,
}

impl<H> TimeoutHandler<H> {
    /// Wrap a handler with a timeout.
    pub fn new(inner: H, deadline: Duration) -> Self {
        Self { inner, deadline }
    }
}

/// Wrap a handler with a timeout.
pub fn with_tool_timeout<H: ToolHandler>(handler: H, deadline: Duration) -> TimeoutHandler<H> {
    TimeoutHandler::new(handler, deadline)
}

#[async_trait]
impl<H: ToolHandler> ToolHandler for TimeoutHandler<H> {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        match tokio::time::timeout(self.deadline, self.inner.handle(tool_use_id, input, env)).await
        {
            Ok(result) => result,
            Err(_) => ToolCallResult::error(
                tool_use_id,
                format!(
                    "Tool '{}' timed out after {:?}",
                    self.inner.definition().name,
                    self.deadline
                ),
            ),
        }
    }
}

// ─── RetryHandler ────────────────────────────────────────────────────

/// Wraps a [`ToolHandler`] with retry-on-error logic.
///
/// On `is_error` results, the handler is retried up to `max_retries` times
/// with a fixed delay between attempts. Successful results are returned
/// immediately without retry.
pub struct RetryHandler<H> {
    inner: H,
    max_retries: u32,
    delay: Duration,
}

impl<H> RetryHandler<H> {
    /// Wrap a handler with retry.
    pub fn new(inner: H, max_retries: u32, delay: Duration) -> Self {
        Self {
            inner,
            max_retries,
            delay,
        }
    }
}

/// Wrap a handler with retry-on-error.
pub fn with_tool_retry<H: ToolHandler>(
    handler: H,
    max_retries: u32,
    delay: Duration,
) -> RetryHandler<H> {
    RetryHandler::new(handler, max_retries, delay)
}

#[async_trait]
impl<H: ToolHandler> ToolHandler for RetryHandler<H> {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: serde_json::Value,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        // ToolHandler::handle takes owned `Value` by design (handlers may
        // destructure it), so each attempt must clone `input`. For max_retries=N,
        // that's N+1 clones total. Reducing this requires a trait change to accept
        // `&Value` or `Arc<Value>`, tracked separately.
        let mut last_result = self.inner.handle(tool_use_id, input.clone(), env).await;

        for _ in 0..self.max_retries {
            if !last_result.is_error {
                return last_result;
            }
            // P2-A: Cooperative cancellation — check before sleeping.
            if env.is_cancelled() {
                return last_result;
            }
            tokio::time::sleep(self.delay).await;
            // Check again after waking — cancellation may have fired during sleep.
            if env.is_cancelled() {
                return last_result;
            }
            last_result = self.inner.handle(tool_use_id, input.clone(), env).await;
        }

        last_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::event_sink::EventSink;
    use agent_fw_algebra::testing::{NullEventSink, NullKVStore, NullSubAgentInvoker};
    use agent_fw_algebra::{CancellationToken, KVStore, SubAgentInvoker};
    use agent_fw_core::tenant::TenantContext;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn test_env() -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test"));
        ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel)
    }

    // ── Test handlers ──────────────────────────────────────────────────

    struct EchoHandler;
    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".into(),
                description: "Echo input".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, input)
        }
    }

    struct SlowHandler {
        delay: Duration,
    }
    #[async_trait]
    impl ToolHandler for SlowHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "slow".into(),
                description: "Slow handler".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            tokio::time::sleep(self.delay).await;
            ToolCallResult::success(tool_use_id, input)
        }
    }

    struct CountingFailHandler {
        call_count: AtomicU32,
        succeed_after: u32,
    }

    impl CountingFailHandler {
        fn new(succeed_after: u32) -> Self {
            Self {
                call_count: AtomicU32::new(0),
                succeed_after,
            }
        }
    }

    #[async_trait]
    impl ToolHandler for CountingFailHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "counting_fail".into(),
                description: "Fails N times then succeeds".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn handle(
            &self,
            tool_use_id: &str,
            input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count >= self.succeed_after {
                ToolCallResult::success(tool_use_id, input)
            } else {
                ToolCallResult::error(tool_use_id, format!("fail attempt {}", count))
            }
        }
    }

    struct AlwaysFailHandler;
    #[async_trait]
    impl ToolHandler for AlwaysFailHandler {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "always_fail".into(),
                description: "Always fails".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn handle(
            &self,
            tool_use_id: &str,
            _input: serde_json::Value,
            _env: &ToolEnvironment,
        ) -> ToolCallResult {
            ToolCallResult::error(tool_use_id, "always fails")
        }
    }

    // ── TimeoutHandler tests ───────────────────────────────────────────

    #[test]
    fn timeout_transparency() {
        let handler = with_tool_timeout(EchoHandler, Duration::from_secs(10));
        let def = handler.definition();
        assert_eq!(def.name, "echo");
        assert_eq!(def.description, "Echo input");
    }

    #[tokio::test]
    async fn timeout_success_passthrough() {
        let handler = with_tool_timeout(EchoHandler, Duration::from_secs(10));
        let env = test_env();
        let result = handler
            .handle("id-1", serde_json::json!({"x": 1}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["x"], 1);
    }

    #[tokio::test]
    async fn timeout_returns_error_on_deadline() {
        let handler = with_tool_timeout(
            SlowHandler {
                delay: Duration::from_secs(5),
            },
            Duration::from_millis(10),
        );
        let env = test_env();
        let result = handler.handle("id-2", serde_json::json!({}), &env).await;
        assert!(result.is_error);
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("timed out"));
    }

    // ── RetryHandler tests ─────────────────────────────────────────────

    #[test]
    fn retry_transparency() {
        let handler = with_tool_retry(EchoHandler, 3, Duration::from_millis(1));
        let def = handler.definition();
        assert_eq!(def.name, "echo");
    }

    #[tokio::test]
    async fn retry_no_retry_on_success() {
        let handler = with_tool_retry(EchoHandler, 3, Duration::from_millis(1));
        let env = test_env();
        let result = handler
            .handle("id-1", serde_json::json!({"ok": true}), &env)
            .await;
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn retry_succeeds_after_failures() {
        let handler = with_tool_retry(
            CountingFailHandler::new(2), // fails twice then succeeds
            3,
            Duration::from_millis(1),
        );
        let env = test_env();
        let result = handler
            .handle("id-2", serde_json::json!({"val": 42}), &env)
            .await;
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn retry_exhaustion_returns_last_error() {
        let handler = with_tool_retry(AlwaysFailHandler, 2, Duration::from_millis(1));
        let env = test_env();
        let result = handler.handle("id-3", serde_json::json!({}), &env).await;
        assert!(result.is_error);
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("always fails"));
    }

    // ── RetryHandler cancellation tests (P3-B) ──────────────────────────

    fn cancelled_env() -> ToolEnvironment {
        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test"));
        ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel)
    }

    /// A pre-cancelled token should prevent any retries — only the
    /// initial call executes, then the check-before-sleep returns.
    #[tokio::test]
    async fn retry_handler_respects_pre_cancellation() {
        let counting = Arc::new(CountingFailHandler::new(u32::MAX));
        // Wrap in a newtype that shares the counter.
        struct SharedCounter(Arc<CountingFailHandler>);
        #[async_trait]
        impl ToolHandler for SharedCounter {
            fn definition(&self) -> ToolDefinition {
                self.0.definition()
            }
            async fn handle(
                &self,
                id: &str,
                input: serde_json::Value,
                env: &ToolEnvironment,
            ) -> ToolCallResult {
                self.0.handle(id, input, env).await
            }
        }
        let handler = with_tool_retry(SharedCounter(counting.clone()), 3, Duration::from_millis(1));
        let env = cancelled_env();

        let result = handler
            .handle("id-cancel", serde_json::json!({}), &env)
            .await;
        assert!(result.is_error);
        // Pre-cancelled: first call executes, then cancel check prevents retries.
        assert_eq!(
            counting.call_count.load(Ordering::SeqCst),
            1,
            "Should execute exactly once with pre-cancelled token"
        );
    }

    /// After cancellation, the retry loop exits at the next check point
    /// (before or after the sleep). With a short delay the handler should
    /// complete promptly and not exhaust all retries.
    #[tokio::test]
    async fn retry_handler_cancels_during_retries() {
        let handler = with_tool_retry(
            AlwaysFailHandler,
            100, // many retries — would take a while without cancellation
            Duration::from_millis(1),
        );

        let kv: Arc<dyn KVStore> = Arc::new(NullKVStore);
        let sink: Arc<dyn EventSink> = Arc::new(NullEventSink);
        let sub_agents: Arc<dyn SubAgentInvoker> = Arc::new(NullSubAgentInvoker);
        let cancel = CancellationToken::new();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new_unchecked("test"));
        let env = ToolEnvironment::new(kv, sink, sub_agents, tenant, cancel.clone());

        // Cancel after a short delay
        let cancel2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            cancel2.cancel();
        });

        let start = tokio::time::Instant::now();
        let result = handler
            .handle("id-cancel2", serde_json::json!({}), &env)
            .await;
        let elapsed = start.elapsed();

        assert!(result.is_error);
        // With 100 retries × 1ms delay, exhaustion would take ~100ms.
        // Cancellation after 5ms should short-circuit well before that.
        assert!(
            elapsed < Duration::from_millis(50),
            "Should short-circuit: {elapsed:?}"
        );
    }

    // ── Composition tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn timeout_and_retry_compose() {
        let handler = with_tool_timeout(
            with_tool_retry(EchoHandler, 2, Duration::from_millis(1)),
            Duration::from_secs(10),
        );
        let env = test_env();
        let result = handler
            .handle("id-4", serde_json::json!({"composed": true}), &env)
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content["composed"], true);
    }

    #[tokio::test]
    async fn traced_timeout_retry_compose() {
        let handler = crate::traced(with_tool_timeout(
            with_tool_retry(EchoHandler, 2, Duration::from_millis(1)),
            Duration::from_secs(10),
        ));
        let env = test_env();
        let def = handler.definition();
        assert_eq!(def.name, "echo");

        let result = handler
            .handle("id-5", serde_json::json!({"full_stack": true}), &env)
            .await;
        assert!(!result.is_error);
    }
}
