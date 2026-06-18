use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatibleProbeResult {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub models: Option<Vec<RemoteModel>>,
}

impl OpenAiCompatibleProbeResult {
    pub fn connected(latency_ms: u64, models: Vec<RemoteModel>) -> Self {
        Self {
            connected: true,
            latency_ms: Some(latency_ms),
            error: None,
            models: Some(models),
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

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    object: Option<String>,
}

pub fn parse_models_response(body: &str) -> Result<Vec<RemoteModel>, serde_json::Error> {
    let parsed: ModelsResponse = serde_json::from_str(body)?;
    Ok(parsed
        .data
        .into_iter()
        .map(|m| RemoteModel {
            id: m.id,
            object: m.object,
        })
        .collect())
}

pub fn validate_remote_url(raw: &str) -> Result<(), String> {
    let lower = raw.to_ascii_lowercase();

    let after_scheme = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return Err("URL must use http or https scheme".into());
    };

    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err("URL has no host".into());
    }

    let host_port = if let Some(idx) = authority.rfind('@') {
        &authority[idx + 1..]
    } else {
        authority
    };

    let host = if host_port.starts_with('[') {
        match host_port.find(']') {
            Some(end) => &host_port[1..end],
            None => return Err("Malformed IPv6 address in URL".into()),
        }
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };

    if host.is_empty() {
        return Err("URL has an empty host".into());
    }

    if host == "localhost"
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".localhost")
    {
        return Err(format!(
            "Requests to private/internal hostname '{host}' are not allowed"
        ));
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                octets[0] == 127
                    || octets[0] == 10
                    || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                    || (octets[0] == 192 && octets[1] == 168)
                    || (octets[0] == 169 && octets[1] == 254)
                    || octets[0] == 0
            }
            std::net::IpAddr::V6(v6) => {
                let segments = v6.segments();
                v6.is_loopback()
                    || (segments[0] & 0xfe00) == 0xfc00
                    || (segments[0] & 0xffc0) == 0xfe80
                    || matches!(v6.to_ipv4_mapped(), Some(v4) if {
                        let octets = v4.octets();
                        octets[0] == 127
                            || octets[0] == 10
                            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                            || (octets[0] == 192 && octets[1] == 168)
                            || (octets[0] == 169 && octets[1] == 254)
                            || octets[0] == 0
                    })
            }
        };
        if is_private {
            return Err(format!(
                "Requests to private/internal IP address '{host}' are not allowed"
            ));
        }
    }

    Ok(())
}

pub async fn verify_openai_compatible_endpoint(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> OpenAiCompatibleProbeResult {
    if let Err(message) = validate_remote_url(base_url) {
        return OpenAiCompatibleProbeResult::failed(message);
    }

    let base_url = base_url.trim_end_matches('/');
    let candidate_urls = [
        format!("{base_url}/v1/models"),
        format!("{base_url}/models"),
    ];

    for (idx, url) in candidate_urls.iter().enumerate() {
        let start = std::time::Instant::now();
        let mut request = client.get(url);
        if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
            request = request.header("Authorization", format!("Bearer {api_key}"));
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                if idx + 1 < candidate_urls.len() {
                    continue;
                }
                return OpenAiCompatibleProbeResult::failed(error.to_string());
            }
        };

        if !response.status().is_success() {
            if response.status() == reqwest::StatusCode::NOT_FOUND && idx + 1 < candidate_urls.len()
            {
                continue;
            }
            return OpenAiCompatibleProbeResult::failed_with_latency(
                start.elapsed().as_millis() as u64,
                format!("HTTP {}", response.status()),
            );
        }

        let body = match response.text().await {
            Ok(body) => body,
            Err(error) => {
                return OpenAiCompatibleProbeResult::failed_with_latency(
                    start.elapsed().as_millis() as u64,
                    format!("Failed to read response: {error}"),
                );
            }
        };

        match parse_models_response(&body) {
            Ok(models) => {
                return OpenAiCompatibleProbeResult::connected(
                    start.elapsed().as_millis() as u64,
                    models,
                );
            }
            Err(error) => {
                if idx + 1 < candidate_urls.len() {
                    continue;
                }
                return OpenAiCompatibleProbeResult::failed_with_latency(
                    start.elapsed().as_millis() as u64,
                    format!("Failed to parse models response: {error}"),
                );
            }
        }
    }

    OpenAiCompatibleProbeResult::failed("Connection failed: no compatible models endpoint found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_response_extracts_remote_models() {
        let models = parse_models_response(
            r#"{"data":[{"id":"model-a","object":"model"},{"id":"model-b"}]}"#,
        )
        .expect("models");
        assert_eq!(
            models,
            vec![
                RemoteModel {
                    id: "model-a".into(),
                    object: Some("model".into()),
                },
                RemoteModel {
                    id: "model-b".into(),
                    object: None,
                },
            ]
        );
    }

    #[test]
    fn validate_remote_url_rejects_private_hostnames() {
        let error = validate_remote_url("http://localhost:8080").expect_err("should reject");
        assert!(error.contains("private/internal hostname"));
    }
}
