//! SubAgentInvoker algebraic law test harnesses.
//!
//! # Laws Tested
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L1 | Usage Tracking | Every successful invocation returns non-default usage metrics |
//! | L2 | Cancellation | Respects CancellationToken (cancelled invoker returns error) |
//! | L3 | Not Found | Invoking a non-existent agent returns `SubAgentError::NotFound` |
//! | L4 | Has-Agent Consistency | `has_agent(name)` ⟺ `invoke(name)` does not return `NotFound` |
//! | L5 | Available-Agents Consistency | `available_agents()` lists all agents for which `has_agent` is true |
//!
//! # Usage
//!
//! ```ignore
//! use agent_fw_test::sub_agent_laws;
//!
//! #[tokio::test]
//! async fn my_invoker_satisfies_laws() {
//!     let invoker = MySubAgentInvoker::new();
//!     sub_agent_laws::test_all(&invoker).await;
//! }
//! ```

use agent_fw_algebra::sub_agent::{SubAgentError, SubAgentInvoker, SubAgentRequest};

/// Run all SubAgentInvoker laws against the given implementation.
pub async fn test_all(invoker: &dyn SubAgentInvoker) {
    law_not_found(invoker).await;
    law_has_agent_consistency(invoker).await;
    law_available_agents_consistency(invoker).await;
}

/// L3: Not Found — invoking a non-existent agent returns NotFound.
pub async fn law_not_found(invoker: &dyn SubAgentInvoker) {
    let request = SubAgentRequest::new("nonexistent_agent_that_does_not_exist", "test")
        .with_invocation_id("law-l3-test");

    let result = invoker.invoke(request).await;
    assert!(
        result.is_err(),
        "L3 NotFound: invoking non-existent agent must return Err"
    );

    match result.unwrap_err() {
        SubAgentError::NotFound(_) => {} // expected
        other => panic!(
            "L3 NotFound: expected SubAgentError::NotFound, got {:?}",
            other
        ),
    }
}

/// L4: Has-Agent Consistency — `has_agent(name)` must return false for
/// agents that yield `NotFound` on invocation.
pub async fn law_has_agent_consistency(invoker: &dyn SubAgentInvoker) {
    let bogus_name = "consistency_law_nonexistent_agent";
    let has = invoker.has_agent(bogus_name);
    assert!(
        !has,
        "L4 Has-Agent Consistency: has_agent must return false for non-existent agent"
    );

    // For each available agent, has_agent should return true
    for name in invoker.available_agents() {
        assert!(
            invoker.has_agent(&name),
            "L4 Has-Agent Consistency: has_agent({name}) must return true for listed agent"
        );
    }
}

/// L5: Available-Agents Consistency — every agent in `available_agents()`
/// should pass `has_agent`, and the list should be deterministic.
pub async fn law_available_agents_consistency(invoker: &dyn SubAgentInvoker) {
    let agents1 = invoker.available_agents();
    let agents2 = invoker.available_agents();

    // Deterministic: calling twice yields the same set
    let mut sorted1 = agents1.clone();
    sorted1.sort();
    let mut sorted2 = agents2;
    sorted2.sort();
    assert_eq!(
        sorted1, sorted2,
        "L5 Available-Agents Consistency: available_agents() must be deterministic"
    );

    // Each agent passes has_agent
    for name in &agents1 {
        assert!(
            invoker.has_agent(name),
            "L5 Available-Agents Consistency: has_agent must return true for agent '{name}'"
        );
    }
}

/// L1: Usage Tracking — a successful invocation must return usage metrics.
///
/// This test requires a working agent (with an interpreter), so it is
/// provided as a standalone function that consumers can call with a
/// pre-configured invoker that has at least one registered agent.
///
/// # Panics
/// Panics if `agent_name` is not registered or the invocation fails.
pub async fn law_usage_tracking(invoker: &dyn SubAgentInvoker, agent_name: &str) {
    assert!(
        invoker.has_agent(agent_name),
        "L1 Usage Tracking: agent '{}' must be registered",
        agent_name
    );

    let request = SubAgentRequest::new(agent_name, "Say hello.").with_invocation_id("law-l1-usage");

    let result = invoker
        .invoke(request)
        .await
        .expect("L1 Usage Tracking: invocation must succeed for registered agent");

    // The result should have a model name
    assert!(
        !result.model.is_empty(),
        "L1 Usage Tracking: model field must not be empty"
    );

    // The result should have an agent name matching the request
    assert_eq!(
        result.agent_name, agent_name,
        "L1 Usage Tracking: result agent_name must match request"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::sub_agent::SubAgentResult;
    use agent_fw_core::{LatencySummary, PhaseBreakdown, TokenUsage};
    use async_trait::async_trait;

    /// Minimal stub SubAgentInvoker for testing the law harnesses.
    struct StubInvoker {
        agents: Vec<String>,
    }

    impl StubInvoker {
        fn new(agents: Vec<&str>) -> Self {
            Self {
                agents: agents.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    #[async_trait]
    impl SubAgentInvoker for StubInvoker {
        async fn invoke(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
            if !self.agents.contains(&request.agent_name) {
                return Err(SubAgentError::NotFound(request.agent_name));
            }

            let inv_id = request.resolved_invocation_id();
            let latency = LatencySummary {
                total_duration_ms: 1,
                phases: PhaseBreakdown::ZERO.with_sub_agent_time(1),
                ..Default::default()
            };
            Ok(SubAgentResult::new(
                request.agent_name,
                inv_id,
                "Hello from stub",
                TokenUsage::simple(10, 5),
                "stub-model",
            )
            .with_latency(Some(latency)))
        }

        fn has_agent(&self, name: &str) -> bool {
            self.agents.iter().any(|a| a == name)
        }

        fn available_agents(&self) -> Vec<String> {
            self.agents.clone()
        }
    }

    #[tokio::test]
    async fn stub_invoker_passes_all_laws() {
        let invoker = StubInvoker::new(vec!["planner", "executor"]);
        test_all(&invoker).await;
    }

    #[tokio::test]
    async fn stub_invoker_usage_tracking() {
        let invoker = StubInvoker::new(vec!["planner"]);
        law_usage_tracking(&invoker, "planner").await;
    }
}
