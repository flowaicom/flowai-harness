use serde::{Deserialize, Serialize};

/// Generic connection probe result for provider or endpoint verification.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProbeResult {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub models: Option<Vec<String>>,
}

impl ConnectionProbeResult {
    pub fn connected(latency_ms: u64, models: Option<Vec<String>>) -> Self {
        Self {
            connected: true,
            latency_ms: Some(latency_ms),
            error: None,
            models,
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            connected: false,
            latency_ms: None,
            error: Some(error.into()),
            models: None,
        }
    }

    pub fn failed_with_latency(latency_ms: u64, error: impl Into<String>) -> Self {
        Self {
            connected: false,
            latency_ms: Some(latency_ms),
            error: Some(error.into()),
            models: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConnectionProbeResult;

    #[test]
    fn connected_result_sets_success_fields() {
        let result = ConnectionProbeResult::connected(12, Some(vec!["model-a".to_string()]));
        assert!(result.connected);
        assert_eq!(result.latency_ms, Some(12));
        assert_eq!(result.models.as_deref(), Some(&["model-a".to_string()][..]));
        assert!(result.error.is_none());
    }

    #[test]
    fn failed_result_sets_error_fields() {
        let result = ConnectionProbeResult::failed("boom");
        assert!(!result.connected);
        assert_eq!(result.error.as_deref(), Some("boom"));
        assert!(result.latency_ms.is_none());
        assert!(result.models.is_none());
    }
}
