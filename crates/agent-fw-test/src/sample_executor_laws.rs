//! Law tests for `SampleExecutor`.
//!
//! # Laws
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L1 | Override-wins | `ResolvedModelConfig::resolve` uses explicit values over defaults |
//! | L2 | Timeout-respected | Execution does not exceed the provided timeout (within tolerance) |
//! | L3 | Latency-present | `duration_ms > 0` for any non-trivial execution |

use std::time::Duration;

use agent_fw_eval::{ResolvedModelConfig, SampleExecutor, SampleInput, TimeoutSampleExecutor};

/// L1: Override-wins — explicit values take precedence over defaults.
pub fn test_override_wins() {
    // Explicit values win
    let config = ResolvedModelConfig::resolve(
        Some("custom"),
        Some("custom-model"),
        "default-provider",
        "default-model",
    );
    assert_eq!(config.provider, "custom", "L1: explicit provider must win");
    assert_eq!(config.model, "custom-model", "L1: explicit model must win");

    // Defaults used when no overrides
    let config = ResolvedModelConfig::resolve(None, None, "default-provider", "default-model");
    assert_eq!(
        config.provider, "default-provider",
        "L1: default provider when no override"
    );
    assert_eq!(
        config.model, "default-model",
        "L1: default model when no override"
    );

    // Partial override — provider explicit, model default
    let config =
        ResolvedModelConfig::resolve(Some("custom"), None, "default-provider", "default-model");
    assert_eq!(
        config.provider, "custom",
        "L1: explicit provider wins in partial override"
    );
    assert_eq!(
        config.model, "default-model",
        "L1: default model used when not overridden"
    );

    // Partial override — provider default, model explicit
    let config = ResolvedModelConfig::resolve(
        None,
        Some("custom-model"),
        "default-provider",
        "default-model",
    );
    assert_eq!(
        config.provider, "default-provider",
        "L1: default provider used when not overridden"
    );
    assert_eq!(
        config.model, "custom-model",
        "L1: explicit model wins in partial override"
    );
}

/// L3: Stub executor produces latency > 0 (stub hardcodes 100ms).
pub async fn test_stub_latency_present() {
    use agent_fw_core::TestCaseId;
    use agent_fw_eval::types::{EvalMode, EvalTestCase, TrajectoryMode};
    use agent_fw_eval::StubSampleExecutor;

    let executor = StubSampleExecutor;
    let input = SampleInput {
        test_case: EvalTestCase {
            id: TestCaseId::new_unchecked("tc-law"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["draft_plan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        },
        sample_index: 0,
        eval_mode: EvalMode::Sequential,
        target_agent_id: None,
        run_id: "run-law".into(),
    };
    let config = ResolvedModelConfig {
        provider: "anthropic".into(),
        model: "claude-sonnet".into(),
    };
    let output = executor.execute(input, &config, None).await.unwrap();
    assert!(output.duration_ms > 0, "L3: duration_ms must be > 0");
}

/// L2: Timeout-respected — execution returns `TimedOut` when timeout < delay.
pub async fn test_timeout_respected() {
    use agent_fw_core::TestCaseId;
    use agent_fw_eval::types::{EvalMode, EvalTestCase, TrajectoryMode};
    use agent_fw_eval::SampleExecutionError;

    let executor = TimeoutSampleExecutor {
        delay: Duration::from_millis(500),
    };
    let input = SampleInput {
        test_case: EvalTestCase {
            id: TestCaseId::new_unchecked("tc-timeout"),
            tags: vec![],
            input: "test".into(),
            expected_trajectory: vec!["draft_plan".into()],
            trajectory_mode: TrajectoryMode::Unordered,
            ground_truth: None,
            final_response: None,
            source_thread_id: None,
        },
        sample_index: 0,
        eval_mode: EvalMode::Sequential,
        target_agent_id: None,
        run_id: "run-timeout".into(),
    };
    let config = ResolvedModelConfig {
        provider: "anthropic".into(),
        model: "claude-sonnet".into(),
    };

    let result = executor
        .execute(input, &config, Some(Duration::from_millis(100)))
        .await;

    match result {
        Err(SampleExecutionError::TimedOut { timeout: _ }) => {} // expected
        other => panic!("L2: expected TimedOut error, got {:?}", other),
    }
}

/// Run all laws.
pub async fn test_all() {
    test_override_wins();
    test_stub_latency_present().await;
    test_timeout_respected().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_override_wins() {
        test_override_wins();
    }

    #[tokio::test]
    async fn l2_timeout_respected() {
        test_timeout_respected().await;
    }

    #[tokio::test]
    async fn l3_stub_latency_present() {
        test_stub_latency_present().await;
    }

    #[tokio::test]
    async fn all_laws() {
        test_all().await;
    }
}
