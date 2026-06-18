//! NeonDB Management API interpreter for [`DatabaseProvisioner`].
//!
//! Provisions workspace environments as Neon branches — each branch is a
//! copy-on-write fork of the parent, providing instant isolation.
//!
//! API base: `https://console.neon.tech/api/v2`
//! Auth: Bearer token via `NEON_API_KEY`.
//!
//! See: <https://api-docs.neon.tech/reference/getting-started-with-neon-api>

use agent_fw_catalog::provisioner::{
    DatabaseProvisioner, EnvironmentId, EnvironmentName, EnvironmentSummary, ProvisionRequest,
    ProvisionedConnection, ProvisionedEnvironment, ProvisioningError,
};
use async_trait::async_trait;

/// Redact user-info (credentials) from a connection URI so it is safe to
/// embed in error messages.
///
/// Mirrors the userinfo handling of `redact_url` in `flowai_runtime::storage`,
/// kept local because the interpreter crate cannot depend on the (higher-
/// level) runtime crate. Only the `user:pass@` authority component is
/// redacted; a malformed URI with no `@` is returned unchanged.
fn redact_uri(uri: &str) -> String {
    let Some(scheme_end) = uri.find("://") else {
        return uri.to_string();
    };
    let after_scheme = scheme_end + 3;
    let authority_end = uri[after_scheme..]
        .find(['/', '?', '#'])
        .map(|idx| after_scheme + idx)
        .unwrap_or(uri.len());
    let authority = &uri[after_scheme..authority_end];
    let Some(at) = authority.rfind('@') else {
        return uri.to_string();
    };
    let host_part = &authority[at..];
    let mut out = String::with_capacity(uri.len());
    out.push_str(&uri[..after_scheme]);
    out.push_str("***:***");
    out.push_str(host_part);
    out.push_str(&uri[authority_end..]);
    out
}

/// Extract the host component from a PostgreSQL connection URI.
///
/// Input: `postgresql://user:pass@host.neon.tech/dbname?sslmode=require`
/// Output: `host.neon.tech`
///
/// Returns `ProvisioningError::Api` if the URI is malformed. The URI is
/// redacted before being placed in the error message so embedded credentials
/// are never leaked through `ProvisioningError`.
fn extract_host_from_uri(uri: &str) -> Result<String, ProvisioningError> {
    let after_at = uri
        .split('@')
        .nth(1)
        .ok_or_else(|| ProvisioningError::Api {
            status: 0,
            message: format!(
                "Malformed connection URI: no '@' delimiter in '{}'",
                redact_uri(uri)
            ),
        })?;
    // Host ends at '/' (path) or ':' (port) or '?' (params) — whichever comes first
    let host = after_at
        .split(|c: char| c == '/' || c == ':' || c == '?')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProvisioningError::Api {
            status: 0,
            message: format!(
                "Malformed connection URI: no host component in '{}'",
                redact_uri(uri)
            ),
        })?;
    Ok(host.to_string())
}

/// Default timeout for Neon API requests (30s).
///
/// Branch creation can take 5–15s under normal load. 30s gives ample
/// headroom while preventing indefinite hangs on network issues.
const NEON_API_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// NeonDB provisioner backed by the Neon Management API.
///
/// Provisions workspace environments as Neon branches — each branch is a
/// copy-on-write fork of the parent, providing instant isolation.
///
/// # Construction
///
/// ```ignore
/// let prov = NeonProvisioner::new(api_key, project_id);
/// ```
pub struct NeonProvisioner {
    client: reqwest::Client,
    api_key: String,
    project_id: String,
    base_url: String,
}

impl NeonProvisioner {
    /// Create a new Neon provisioner.
    ///
    /// `api_key` is the Neon API key (from `NEON_API_KEY` env).
    /// `project_id` is the Neon project to manage branches within.
    pub fn new(api_key: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(NEON_API_TIMEOUT)
                .build()
                .expect("failed to build reqwest Client"),
            api_key: api_key.into(),
            project_id: project_id.into(),
            base_url: "https://console.neon.tech/api/v2".to_string(),
        }
    }

    /// Override the base URL (for testing against a mock server).
    #[cfg(test)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}/projects/{}{}", self.base_url, self.project_id, path)
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    /// Map a reqwest error to ProvisioningError.
    fn map_network(e: reqwest::Error) -> ProvisioningError {
        ProvisioningError::Network(e.to_string())
    }
}

#[async_trait]
impl DatabaseProvisioner for NeonProvisioner {
    async fn provision(
        &self,
        req: ProvisionRequest,
    ) -> Result<ProvisionedEnvironment, ProvisioningError> {
        // Neon API request body
        #[derive(serde::Serialize)]
        struct Body {
            branch: BranchBody,
            endpoints: Vec<EndpointSpec>,
        }
        #[derive(serde::Serialize)]
        struct BranchBody {
            name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_id: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            expires_at: Option<String>,
        }
        #[derive(serde::Serialize)]
        struct EndpointSpec {
            #[serde(rename = "type")]
            endpoint_type: String,
        }

        let body = Body {
            branch: BranchBody {
                name: req.name.to_string(),
                parent_id: req.parent_id.map(|id| id.to_string()),
                expires_at: req.expires_at.map(|dt| dt.to_rfc3339()),
            },
            endpoints: vec![EndpointSpec {
                endpoint_type: "read_write".to_string(),
            }],
        };

        let resp = self
            .client
            .post(self.url("/branches"))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(Self::map_network)?;

        let status = resp.status().as_u16();
        if status == 409 {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Conflict(text));
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Api {
                status,
                message: text,
            });
        }

        // Parse response
        #[derive(serde::Deserialize)]
        struct CreateResp {
            branch: BranchRespBody,
            endpoints: Vec<EndpointResp>,
        }
        #[derive(serde::Deserialize)]
        struct BranchRespBody {
            id: String,
            name: String,
            parent_id: Option<String>,
            current_state: String,
            created_at: String,
        }
        #[derive(serde::Deserialize)]
        struct EndpointResp {
            host: String,
        }

        let parsed: CreateResp = resp.json().await.map_err(|e| ProvisioningError::Api {
            status: 0,
            message: format!("Parse error: {e}"),
        })?;

        let host = parsed
            .endpoints
            .first()
            .map(|e| e.host.clone())
            .ok_or_else(|| ProvisioningError::Api {
                status: 0,
                message: "Neon returned no endpoints for branch".to_string(),
            })?;

        Ok(ProvisionedEnvironment {
            id: EnvironmentId::new(parsed.branch.id),
            name: EnvironmentName::new(parsed.branch.name),
            parent_id: parsed.branch.parent_id.map(EnvironmentId::new),
            host,
            current_state: parsed.branch.current_state,
            created_at: parsed.branch.created_at,
        })
    }

    async fn deprovision(&self, env_id: &EnvironmentId) -> Result<(), ProvisioningError> {
        let resp = self
            .client
            .delete(self.url(&format!("/branches/{env_id}")))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(Self::map_network)?;

        if resp.status().as_u16() == 404 {
            return Err(ProvisioningError::NotFound(env_id.to_string()));
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Api {
                status,
                message: text,
            });
        }

        Ok(())
    }

    async fn list_environments(&self) -> Result<Vec<EnvironmentSummary>, ProvisioningError> {
        let resp = self
            .client
            .get(self.url("/branches"))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(Self::map_network)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Api {
                status,
                message: text,
            });
        }

        #[derive(serde::Deserialize)]
        struct ListResp {
            branches: Vec<EnvironmentSummary>,
        }

        let parsed: ListResp = resp.json().await.map_err(|e| ProvisioningError::Api {
            status: 0,
            message: format!("Parse: {e}"),
        })?;

        Ok(parsed.branches)
    }

    async fn get_connection(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        role_name: &str,
    ) -> Result<ProvisionedConnection, ProvisioningError> {
        let base = format!(
            "{}/projects/{}/connection_uri",
            self.base_url, self.project_id
        );
        let url = reqwest::Url::parse_with_params(
            &base,
            &[
                ("branch_id", env_id.as_str()),
                ("database_name", database_name),
                ("role_name", role_name),
            ],
        )
        .map_err(|e| ProvisioningError::Api {
            status: 0,
            message: format!("Invalid URL: {e}"),
        })?;

        let resp = self
            .client
            .get(url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(Self::map_network)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Api {
                status,
                message: text,
            });
        }

        #[derive(serde::Deserialize)]
        struct ConnResp {
            uri: String,
        }

        let parsed: ConnResp = resp.json().await.map_err(|e| ProvisioningError::Api {
            status: 0,
            message: format!("Parse: {e}"),
        })?;

        // Extract host from URI — fail explicitly if malformed
        let host = extract_host_from_uri(&parsed.uri)?;

        Ok(ProvisionedConnection {
            host,
            connection_uri: Some(parsed.uri),
            role_name: role_name.to_string(),
            role_password: String::new(), // password is embedded in the URI
        })
    }

    async fn create_database(
        &self,
        env_id: &EnvironmentId,
        database_name: &str,
        owner_name: &str,
    ) -> Result<(), ProvisioningError> {
        #[derive(serde::Serialize)]
        struct Body {
            database: DbBody,
        }
        #[derive(serde::Serialize)]
        struct DbBody {
            name: String,
            owner_name: String,
        }

        let resp = self
            .client
            .post(self.url(&format!("/branches/{env_id}/databases")))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&Body {
                database: DbBody {
                    name: database_name.to_string(),
                    owner_name: owner_name.to_string(),
                },
            })
            .send()
            .await
            .map_err(Self::map_network)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProvisioningError::Api {
                status,
                message: text,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_standard_uri() {
        let uri =
            "postgresql://user:pass@ep-cool-abc123.us-east-2.aws.neon.tech/target?sslmode=require";
        assert_eq!(
            extract_host_from_uri(uri).unwrap(),
            "ep-cool-abc123.us-east-2.aws.neon.tech"
        );
    }

    #[test]
    fn extract_host_with_port() {
        let uri = "postgresql://user:pass@localhost:5432/mydb";
        assert_eq!(extract_host_from_uri(uri).unwrap(), "localhost");
    }

    #[test]
    fn extract_host_minimal_uri() {
        let uri = "postgresql://user@host/db";
        assert_eq!(extract_host_from_uri(uri).unwrap(), "host");
    }

    #[test]
    fn extract_host_no_at_sign_fails() {
        let uri = "not-a-uri";
        assert!(extract_host_from_uri(uri).is_err());
    }

    #[test]
    fn extract_host_empty_host_fails() {
        let uri = "postgresql://user:pass@/db";
        assert!(extract_host_from_uri(uri).is_err());
    }

    #[test]
    #[ignore = "requires NEON_API_KEY and NEON_PROJECT_ID env vars"]
    fn integration_list_environments() {
        let api_key = std::env::var("NEON_API_KEY").unwrap();
        let project_id = std::env::var("NEON_PROJECT_ID").unwrap();
        let prov = NeonProvisioner::new(api_key, project_id);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let envs = rt.block_on(prov.list_environments()).unwrap();
        // Should return at least the main branch
        assert!(!envs.is_empty());
    }
}
