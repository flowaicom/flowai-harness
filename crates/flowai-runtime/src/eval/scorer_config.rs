use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TrajectoryScorerConfig {
    pub include_sub_agents: bool,
    pub ignore_tools: Vec<String>,
}

impl Default for TrajectoryScorerConfig {
    fn default() -> Self {
        Self {
            include_sub_agents: false,
            ignore_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalResponseScorerConfig {
    pub include_judge_trace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessScorerConfig {
    pub trajectory: TrajectoryScorerConfig,
    pub final_response: FinalResponseScorerConfig,
}

impl HarnessScorerConfig {
    pub fn from_value(value: Option<&serde_json::Value>) -> Result<Self, serde_json::Error> {
        match value {
            Some(value) => serde_json::from_value(value.clone()),
            None => Ok(Self::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_scorer_config_defaults_final_response_trace_off() {
        let config = HarnessScorerConfig::from_value(None).expect("config");

        assert!(!config.final_response.include_judge_trace);
        assert!(!config.trajectory.include_sub_agents);
        assert!(config.trajectory.ignore_tools.is_empty());
    }

    #[test]
    fn harness_scorer_config_reads_final_response_judge_trace() {
        let config = HarnessScorerConfig::from_value(Some(&serde_json::json!({
            "finalResponse": {
                "includeJudgeTrace": true
            },
            "trajectory": {
                "includeSubAgents": true,
                "ignoreTools": ["call_agent"]
            }
        })))
        .expect("config");

        assert!(config.final_response.include_judge_trace);
        assert!(config.trajectory.include_sub_agents);
        assert_eq!(config.trajectory.ignore_tools, vec!["call_agent"]);
    }
}
