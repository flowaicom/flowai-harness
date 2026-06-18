use std::collections::HashSet;

use agent_fw_core::{StreamPart, ToolAgentState, ToolInvocationState};
use serde::{Deserialize, Serialize};

use super::TrajectoryScorerConfig;

pub const TRAJECTORY_EVENTS_EXTRA_KEY: &str = "trajectoryEvents";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryEventKind {
    Tool,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryEvent {
    pub kind: TrajectoryEventKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
    #[serde(default)]
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentFrame {
    name: String,
    invocation_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct TrajectoryEventCapture {
    stack: Vec<AgentFrame>,
    events: Vec<TrajectoryEvent>,
    seen_tool_call_ids: HashSet<String>,
    seen_agent_call_ids: HashSet<String>,
}

impl TrajectoryEventCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn step(&mut self, part: &StreamPart) {
        match part {
            StreamPart::ToolInvocation(data) if matches!(data.state, ToolInvocationState::Call) => {
                let scoped_id = self.scoped_tool_call_id(&data.id);
                if !self.seen_tool_call_ids.insert(scoped_id) {
                    return;
                }
                self.events.push(TrajectoryEvent {
                    kind: TrajectoryEventKind::Tool,
                    name: data.name.clone(),
                    agent: self.stack.last().map(|frame| frame.name.clone()),
                    invocation_id: Some(data.id.clone()),
                    depth: self.stack.len() as u32,
                });
            }
            StreamPart::ToolAgent(data) if matches!(data.state, ToolAgentState::Call) => {
                if !self.seen_agent_call_ids.insert(data.invocation_id.clone()) {
                    return;
                }
                let depth = self.stack.len() as u32 + 1;
                self.events.push(TrajectoryEvent {
                    kind: TrajectoryEventKind::Agent,
                    name: data.agent_name.clone(),
                    agent: self.stack.last().map(|frame| frame.name.clone()),
                    invocation_id: Some(data.invocation_id.clone()),
                    depth,
                });
                self.stack.push(AgentFrame {
                    name: data.agent_name.clone(),
                    invocation_id: data.invocation_id.clone(),
                });
            }
            StreamPart::ToolAgent(data) if matches!(data.state, ToolAgentState::Result) => {
                if let Some(position) = self
                    .stack
                    .iter()
                    .rposition(|frame| frame.invocation_id == data.invocation_id)
                {
                    self.stack.truncate(position);
                }
            }
            _ => {}
        }
    }

    pub fn into_events(self) -> Vec<TrajectoryEvent> {
        self.events
    }

    fn scoped_tool_call_id(&self, tool_call_id: &str) -> String {
        if self.stack.is_empty() {
            tool_call_id.to_string()
        } else {
            let scope = self
                .stack
                .iter()
                .map(|frame| frame.invocation_id.as_str())
                .collect::<Vec<_>>()
                .join("/");
            format!("{scope}::{tool_call_id}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryProjection {
    pub observed_trajectory: Vec<String>,
    pub scored_trajectory: Vec<String>,
    pub source: &'static str,
    pub config: TrajectoryScorerConfig,
}

pub fn trajectory_events_extra(events: &[TrajectoryEvent]) -> Option<serde_json::Value> {
    if events.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        TRAJECTORY_EVENTS_EXTRA_KEY: events,
    }))
}

pub fn project_trajectory(
    actual_trajectory: &[String],
    extra: Option<&serde_json::Value>,
    config: &TrajectoryScorerConfig,
) -> TrajectoryProjection {
    let ignore_tools: HashSet<&str> = config.ignore_tools.iter().map(String::as_str).collect();
    let observed_trajectory = actual_trajectory.to_vec();

    let events = extra
        .and_then(|extra| extra.get(TRAJECTORY_EVENTS_EXTRA_KEY))
        .and_then(|value| serde_json::from_value::<Vec<TrajectoryEvent>>(value.clone()).ok());

    let (scored_trajectory, source) = match events {
        Some(events) => {
            let scored = events
                .iter()
                .filter_map(|event| match event.kind {
                    TrajectoryEventKind::Tool => {
                        let is_sub_agent_tool = event.depth > 1;
                        if is_sub_agent_tool && !config.include_sub_agents {
                            return None;
                        }
                        if ignore_tools.contains(event.name.as_str()) {
                            return None;
                        }
                        Some(event.name.clone())
                    }
                    TrajectoryEventKind::Agent => None,
                })
                .collect();
            (scored, TRAJECTORY_EVENTS_EXTRA_KEY)
        }
        None => {
            let scored = actual_trajectory
                .iter()
                .filter(|tool| !ignore_tools.contains(tool.as_str()))
                .cloned()
                .collect();
            (scored, "actualTrajectory")
        }
    };

    TrajectoryProjection {
        observed_trajectory,
        scored_trajectory,
        source,
        config: config.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::StreamPart;

    #[test]
    fn projection_can_include_sub_agent_tools() {
        let mut capture = TrajectoryEventCapture::new();
        for part in [
            StreamPart::sub_agent_call("coordinator", "agent-root"),
            StreamPart::tool_call("tool-1", "call_agent", serde_json::json!({})),
            StreamPart::sub_agent_call("planner", "agent-planner"),
            StreamPart::tool_call("tool-2", "storePlan", serde_json::json!({})),
            StreamPart::sub_agent_result("planner", "agent-planner"),
            StreamPart::sub_agent_result("coordinator", "agent-root"),
        ] {
            capture.step(&part);
        }
        let events = capture.into_events();
        let extra = trajectory_events_extra(&events);

        let projection = project_trajectory(
            &["call_agent".to_string(), "storePlan".to_string()],
            extra.as_ref(),
            &TrajectoryScorerConfig {
                include_sub_agents: true,
                ignore_tools: vec!["call_agent".to_string()],
            },
        );

        assert_eq!(projection.scored_trajectory, vec!["storePlan".to_string()]);
    }

    #[test]
    fn capture_scopes_duplicate_tool_ids_by_agent_invocation() {
        let mut capture = TrajectoryEventCapture::new();
        for part in [
            StreamPart::sub_agent_call("coordinator", "agent-root"),
            StreamPart::tool_call("scripted-tool-1", "call_agent", serde_json::json!({})),
            StreamPart::sub_agent_call("planner", "agent-planner"),
            StreamPart::tool_call("scripted-tool-1", "storePlan", serde_json::json!({})),
            StreamPart::sub_agent_result("planner", "agent-planner"),
            StreamPart::tool_result(
                "scripted-tool-1",
                "call_agent",
                serde_json::json!({}),
                serde_json::json!({}),
            ),
            StreamPart::sub_agent_result("coordinator", "agent-root"),
        ] {
            capture.step(&part);
        }

        let tool_names = capture
            .into_events()
            .into_iter()
            .filter(|event| matches!(event.kind, TrajectoryEventKind::Tool))
            .map(|event| event.name)
            .collect::<Vec<_>>();

        assert_eq!(tool_names, vec!["call_agent", "storePlan"]);
    }

    #[test]
    fn projection_defaults_to_root_tool_calls_only() {
        let mut capture = TrajectoryEventCapture::new();
        for part in [
            StreamPart::sub_agent_call("coordinator", "agent-root"),
            StreamPart::tool_call("tool-1", "call_agent", serde_json::json!({})),
            StreamPart::sub_agent_call("planner", "agent-planner"),
            StreamPart::tool_call("tool-2", "storePlan", serde_json::json!({})),
        ] {
            capture.step(&part);
        }
        let events = capture.into_events();
        let extra = trajectory_events_extra(&events);

        let projection = project_trajectory(
            &["call_agent".to_string(), "storePlan".to_string()],
            extra.as_ref(),
            &TrajectoryScorerConfig::default(),
        );

        assert_eq!(projection.scored_trajectory, vec!["call_agent".to_string()]);
    }
}
