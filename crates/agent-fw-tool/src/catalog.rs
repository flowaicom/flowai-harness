use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Functional tier a tool belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolTier {
    Discovery,
    Search,
    Graph,
    Assembly,
    Planning,
    Execution,
    Delegation,
}

impl ToolTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Search => "search",
            Self::Graph => "graph",
            Self::Assembly => "assembly",
            Self::Planning => "planning",
            Self::Execution => "execution",
            Self::Delegation => "delegation",
        }
    }
}

impl std::fmt::Display for ToolTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tool catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tier: ToolTier,
    pub parameters: Value,
}

/// Standard result format from tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub success: bool,
    /// Pre-formatted data for LLM/UI consumption.
    pub data: String,
    pub count: Option<usize>,
    pub ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(data: String, count: Option<usize>, ids: Vec<String>) -> Self {
        Self {
            success: true,
            data,
            count,
            ids,
            error: None,
        }
    }

    pub fn err(error: String) -> Self {
        Self {
            success: false,
            data: String::new(),
            count: None,
            ids: vec![],
            error: Some(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolExecutionError {
    #[error("Tool not found: {0}")]
    NotFound(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
}

/// Catalog/dispatch contract for application-level tool registries.
#[async_trait]
pub trait ToolRegistry<E>: Send + Sync {
    fn catalog(&self) -> &[ToolInfo];
    async fn execute(
        &self,
        tool_id: &str,
        input: Value,
        env: E,
    ) -> Result<ToolResult, ToolExecutionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_tier_serde_roundtrip() {
        for (tier, json) in [
            (ToolTier::Discovery, "\"discovery\""),
            (ToolTier::Search, "\"search\""),
            (ToolTier::Graph, "\"graph\""),
            (ToolTier::Assembly, "\"assembly\""),
            (ToolTier::Planning, "\"planning\""),
            (ToolTier::Execution, "\"execution\""),
            (ToolTier::Delegation, "\"delegation\""),
        ] {
            assert_eq!(serde_json::to_string(&tier).unwrap(), json);
            let parsed: ToolTier = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, tier);
        }
    }

    #[test]
    fn tool_info_serializes() {
        let info = ToolInfo {
            id: "listTables".into(),
            name: "listTables".into(),
            description: "List tables".into(),
            tier: ToolTier::Discovery,
            parameters: serde_json::json!({"type":"object"}),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"id\":\"listTables\""));
        assert!(json.contains("\"tier\":\"discovery\""));
    }

    #[test]
    fn tool_result_ok_constructor() {
        let r = ToolResult::ok("data".into(), Some(5), vec!["id1".into()]);
        assert!(r.success);
        assert_eq!(r.data, "data");
        assert_eq!(r.count, Some(5));
        assert_eq!(r.ids, vec!["id1"]);
        assert!(r.error.is_none());
    }

    #[test]
    fn tool_result_err_constructor() {
        let r = ToolResult::err("bad input".into());
        assert!(!r.success);
        assert_eq!(r.data, "");
        assert_eq!(r.count, None);
        assert!(r.ids.is_empty());
        assert_eq!(r.error.as_deref(), Some("bad input"));
    }
}
