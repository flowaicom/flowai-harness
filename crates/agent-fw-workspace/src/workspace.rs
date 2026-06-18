//! Workspace domain types and slug generation.
//!
//! A workspace is the top-level multi-tenancy entity. It scopes:
//! - Database connections (target, catalog, embeddings)
//! - Knowledge base (documents, extracted items)
//! - Test cases and eval configurations
//! - Chat threads and history
//! - Model provider preferences
//!
//! # Invariants
//!
//! - `slug` is URL-safe: `^[a-z0-9][a-z0-9-]*$`
//! - `slug` is unique per tenant
//! - The "default" workspace maps to env-configured databases

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use agent_fw_algebra::resolve_sqlite_url_with_root;
use agent_fw_core::WorkspaceId;

// =============================================================================
// DatabaseConfig — How a workspace's databases are provisioned
// =============================================================================

/// How a workspace's databases are provisioned.
///
/// # Variants
///
/// - `Default` — use the environment-configured databases (backward compat).
/// - `Managed` — auto-provisioned branch (e.g. NeonDB). The `branch_id` and
///   `branch_name` are opaque identifiers from the provisioner.
/// - `External` — manually configured connection URLs.
/// - `Sqlite` — workspace-local embedded databases rooted at `directory`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DatabaseConfig {
    /// Use the default env-configured databases.
    Default,

    /// Managed database branch — auto-provisioned by a provisioner.
    #[serde(rename_all = "camelCase")]
    Managed {
        branch_id: String,
        branch_name: String,
        host: String,
        target_url: String,
        catalog_url: String,
        embeddings_url: String,
    },

    /// External databases — manually configured URLs.
    #[serde(rename_all = "camelCase")]
    External {
        target_url: Option<String>,
        catalog_url: Option<String>,
        embeddings_url: Option<String>,
    },

    /// Workspace-local SQLite runtime rooted at `directory`.
    Sqlite { directory: PathBuf },
}

impl DatabaseConfig {
    /// Resolve runtime database URLs against an optional project root.
    pub fn resolved_urls(&self, project_root: Option<&Path>) -> WorkspaceDatabaseUrls {
        resolve_workspace_database_urls(self, project_root)
    }
}

/// Fully resolved database URLs for a workspace runtime.
///
/// `None` means "fall back to the application-global runtime" for that tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDatabaseUrls {
    pub target_url: Option<String>,
    pub catalog_url: Option<String>,
    pub embeddings_url: Option<String>,
}

impl WorkspaceDatabaseUrls {
    pub const fn new(
        target_url: Option<String>,
        catalog_url: Option<String>,
        embeddings_url: Option<String>,
    ) -> Self {
        Self {
            target_url,
            catalog_url,
            embeddings_url,
        }
    }
}

fn normalize_workspace_database_url(url: &str, project_root: Option<&Path>) -> String {
    resolve_sqlite_url_with_root(url, project_root)
}

/// Resolve a workspace-local SQLite directory against an optional project root.
pub fn resolve_workspace_sqlite_directory(
    directory: &Path,
    project_root: Option<&Path>,
) -> PathBuf {
    if directory.is_absolute() {
        directory.to_path_buf()
    } else if let Some(root) = project_root {
        root.join(directory)
    } else {
        directory.to_path_buf()
    }
}

/// Canonical runtime/database identity for a workspace target database.
pub fn workspace_target_database_id(workspace_id: &WorkspaceId) -> String {
    format!("workspace:{}:target", workspace_id.as_str())
}

/// Recover an explicit thread `source_id` from a runtime/database identity.
///
/// Workspace-local/default target runtimes are normalized to `None`, while
/// source-mode database identities pass through unchanged.
pub fn explicit_thread_source_id_for_database(
    workspace_id: &WorkspaceId,
    database_id: &str,
) -> Option<String> {
    (database_id != workspace_target_database_id(workspace_id)).then(|| database_id.to_string())
}

/// Resolve runtime database URLs for a workspace database configuration.
///
/// SQLite URLs are normalized against `project_root`, while non-SQLite URLs are
/// passed through unchanged.
pub fn resolve_workspace_database_urls(
    database_config: &DatabaseConfig,
    project_root: Option<&Path>,
) -> WorkspaceDatabaseUrls {
    match database_config {
        DatabaseConfig::Default => WorkspaceDatabaseUrls::new(None, None, None),
        DatabaseConfig::Managed {
            target_url,
            catalog_url,
            embeddings_url,
            ..
        } => WorkspaceDatabaseUrls::new(
            Some(normalize_workspace_database_url(target_url, project_root)),
            Some(normalize_workspace_database_url(catalog_url, project_root)),
            Some(normalize_workspace_database_url(
                embeddings_url,
                project_root,
            )),
        ),
        DatabaseConfig::External {
            target_url,
            catalog_url,
            embeddings_url,
        } => WorkspaceDatabaseUrls::new(
            target_url
                .as_deref()
                .map(|url| normalize_workspace_database_url(url, project_root)),
            catalog_url
                .as_deref()
                .map(|url| normalize_workspace_database_url(url, project_root)),
            embeddings_url
                .as_deref()
                .map(|url| normalize_workspace_database_url(url, project_root)),
        ),
        DatabaseConfig::Sqlite { directory } => {
            let directory = resolve_workspace_sqlite_directory(directory, project_root);
            WorkspaceDatabaseUrls::new(
                Some(format!("sqlite:{}", directory.join("target.db").display())),
                Some(format!("sqlite:{}", directory.join("catalog.db").display())),
                Some(format!("sqlite:{}", directory.join("lancedb").display())),
            )
        }
    }
}

// =============================================================================
// ModelConfig — Per-workspace model preferences
// =============================================================================

/// Model provider + model selection for a workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceModelConfig {
    /// LLM provider identifier (e.g. "anthropic", "bedrock").
    pub provider: Option<String>,
    /// Primary model ID (e.g. "claude-opus-4-6").
    pub model_id: Option<String>,
    /// Model ID for enrichment/extraction tasks.
    pub enrichment_model_id: Option<String>,
}

// =============================================================================
// Workspace — The complete entity
// =============================================================================

/// A customer workspace — the complete context for one tenant's dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub database_config: DatabaseConfig,
    pub model_config: WorkspaceModelConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Create the implicit "default" workspace from env config.
    pub fn default_workspace() -> Self {
        Self {
            id: WorkspaceId::default_workspace(),
            name: "Default".to_string(),
            slug: "default".to_string(),
            description: Some(
                "Default workspace using environment-configured databases".to_string(),
            ),
            database_config: DatabaseConfig::Default,
            model_config: WorkspaceModelConfig::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Canonical runtime/database identity for this workspace target.
    pub fn target_database_id(&self) -> String {
        workspace_target_database_id(&self.id)
    }

    /// Resolve workspace runtime URLs against an optional project root.
    pub fn resolved_database_urls(&self, project_root: Option<&Path>) -> WorkspaceDatabaseUrls {
        self.database_config.resolved_urls(project_root)
    }
}

// =============================================================================
// Slug generation
// =============================================================================

/// Generate a URL-safe slug from a name.
///
/// # Examples
///
/// ```
/// use agent_fw_workspace::slugify;
/// assert_eq!(slugify("Danone Ice Tea"), "danone-ice-tea");
/// assert_eq!(slugify("Coca-Cola (2024)"), "coca-cola-2024");
/// assert_eq!(slugify(""), "unnamed");
/// ```
///
/// # Laws
///
/// - **Totality**: Never panics, always returns a non-empty string.
/// - **Idempotency**: `slugify(slugify(x)) == slugify(x)`.
/// - **URL-safety**: Result matches `^[a-z0-9][a-z0-9-]*$` (or "unnamed" for empty input).
pub fn slugify(name: &str) -> String {
    let result: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if result.is_empty() {
        "unnamed".to_string()
    } else {
        result
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // slugify tests
    // =========================================================================

    #[test]
    fn slugify_normal() {
        assert_eq!(slugify("Danone Ice Tea"), "danone-ice-tea");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Coca-Cola (2024)"), "coca-cola-2024");
    }

    #[test]
    fn slugify_idempotent() {
        let s = slugify("Test Project 123");
        assert_eq!(slugify(&s), s);
    }

    #[test]
    fn slugify_empty_returns_unnamed() {
        assert_eq!(slugify(""), "unnamed");
        assert_eq!(slugify("   "), "unnamed");
        assert_eq!(slugify("---"), "unnamed");
    }

    #[test]
    fn slugify_already_slugged() {
        assert_eq!(slugify("already-a-slug"), "already-a-slug");
    }

    #[test]
    fn slugify_unicode() {
        assert_eq!(slugify("café crème"), "caf-cr-me");
    }

    // =========================================================================
    // DatabaseConfig serde tests
    // =========================================================================

    #[test]
    fn database_config_default_roundtrip() {
        let cfg = DatabaseConfig::Default;
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"type\":\"default\""));
        let parsed: DatabaseConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DatabaseConfig::Default));
    }

    #[test]
    fn database_config_managed_roundtrip() {
        let cfg = DatabaseConfig::Managed {
            branch_id: "br-123".to_string(),
            branch_name: "ws-demo".to_string(),
            host: "ep-cool-abc.example.com".to_string(),
            target_url: "postgresql://target".to_string(),
            catalog_url: "postgresql://catalog".to_string(),
            embeddings_url: "postgresql://embeddings".to_string(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"type\":\"managed\""));
        let parsed: DatabaseConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DatabaseConfig::Managed { .. }));
    }

    #[test]
    fn workspace_model_config_rejects_unknown_fields() {
        let err = serde_json::from_value::<WorkspaceModelConfig>(serde_json::json!({
            "provider": "anthropic",
            "unexpected": true
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn database_config_external_roundtrip() {
        let cfg = DatabaseConfig::External {
            target_url: Some("postgresql://target".to_string()),
            catalog_url: None,
            embeddings_url: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"type\":\"external\""));
        let parsed: DatabaseConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DatabaseConfig::External { .. }));
    }

    #[test]
    fn database_config_sqlite_roundtrip() {
        let cfg = DatabaseConfig::Sqlite {
            directory: PathBuf::from(".agent-fw/workspaces/demo"),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"type\":\"sqlite\""));
        let parsed: DatabaseConfig = serde_json::from_str(&json).unwrap();
        match parsed {
            DatabaseConfig::Sqlite { directory } => {
                assert_eq!(directory, PathBuf::from(".agent-fw/workspaces/demo"));
            }
            other => panic!("expected sqlite config, got {other:?}"),
        }
    }

    // =========================================================================
    // Workspace tests
    // =========================================================================

    #[test]
    fn default_workspace_has_correct_defaults() {
        let ws = Workspace::default_workspace();
        assert!(ws.id.is_default());
        assert_eq!(ws.slug, "default");
        assert!(matches!(ws.database_config, DatabaseConfig::Default));
    }

    #[test]
    fn workspace_serde_roundtrip() {
        let ws = Workspace::default_workspace();
        let json = serde_json::to_string(&ws).unwrap();
        let parsed: Workspace = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, ws.id);
        assert_eq!(parsed.slug, ws.slug);
    }

    #[test]
    fn resolve_workspace_database_urls_normalizes_external_sqlite_urls() {
        let urls = resolve_workspace_database_urls(
            &DatabaseConfig::External {
                target_url: Some("sqlite:target.db".to_string()),
                catalog_url: Some("sqlite:catalog.db".to_string()),
                embeddings_url: Some("sqlite:lancedb".to_string()),
            },
            Some(Path::new("/tmp/project")),
        );

        assert_eq!(
            urls,
            WorkspaceDatabaseUrls::new(
                Some("sqlite:/tmp/project/target.db".to_string()),
                Some("sqlite:/tmp/project/catalog.db".to_string()),
                Some("sqlite:/tmp/project/lancedb".to_string()),
            )
        );
    }

    #[test]
    fn resolve_workspace_database_urls_synthesizes_sqlite_workspace_paths() {
        let urls = resolve_workspace_database_urls(
            &DatabaseConfig::Sqlite {
                directory: PathBuf::from(".agent-fw/workspaces/demo"),
            },
            Some(Path::new("/tmp/project")),
        );

        assert_eq!(
            urls,
            WorkspaceDatabaseUrls::new(
                Some("sqlite:/tmp/project/.agent-fw/workspaces/demo/target.db".to_string()),
                Some("sqlite:/tmp/project/.agent-fw/workspaces/demo/catalog.db".to_string()),
                Some("sqlite:/tmp/project/.agent-fw/workspaces/demo/lancedb".to_string()),
            )
        );
    }

    #[test]
    fn explicit_thread_source_id_for_database_omits_workspace_target_identity() {
        let workspace_id = WorkspaceId::new("workspace-alpha").expect("workspace id");
        let workspace_database_id = workspace_target_database_id(&workspace_id);

        assert_eq!(
            explicit_thread_source_id_for_database(&workspace_id, &workspace_database_id),
            None
        );
        assert_eq!(
            explicit_thread_source_id_for_database(&workspace_id, "source-alpha"),
            Some("source-alpha".to_string())
        );
    }

    // =========================================================================
    // Hegel — algebraic laws
    // =========================================================================

    use hegel::generators;

    /// slugify totality: never panics for any input.
    #[hegel::test]
    fn slugify_totality(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let result = slugify(&s);
        assert!(!result.is_empty());
    }

    /// slugify idempotency: f(f(x)) == f(x).
    #[hegel::test]
    fn slugify_idempotent_prop(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let once = slugify(&s);
        let twice = slugify(&once);
        assert_eq!(once, twice);
    }

    /// slugify URL-safety: result matches ^[a-z0-9][a-z0-9-]*$.
    #[hegel::test]
    fn slugify_url_safe(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let result = slugify(&s);
        let first_ok = result.as_bytes()[0].is_ascii_alphanumeric();
        let rest_ok = result
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        assert!(
            first_ok && rest_ok,
            "slugify({:?}) = {:?} doesn't match URL-safe pattern",
            s,
            result
        );
    }
}
