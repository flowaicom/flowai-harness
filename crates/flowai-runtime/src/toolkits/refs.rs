//! `references` toolkit — typed reference lookups.
//!
//! Exposes two tools backed by the runtime-owned
//! [`ReferenceRegistry`](crate::references::ReferenceRegistry):
//!
//! - `resolveRef` returns the full materialised payload for an artifact
//!   reference.
//! - `glimpseRef` returns just the host-precomputed glimpse (cheaper
//!   to inspect than the full body).
//!
//! Both handlers hold an `Arc` clone of the registry — the registry is a
//! runtime-owned singleton, not an env extension, so we inject it at
//! handler construction. Tenant scoping comes from
//! [`ToolEnvironment::resource_id`] at dispatch time.

use std::sync::Arc;

use agent_fw_agent::{ToolCallResult, ToolDefinition, ToolHandler};
use agent_fw_tool::{ToolEnvironment, ToolSchema};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::references::{ReferenceError, ReferenceRegistry};
use crate::ArtifactRef;

use super::{filter_by_config, ToolkitConfig, ToolkitError};

/// Tool input shape mirroring [`ArtifactRef`].
#[derive(Debug, Clone, Deserialize, agent_fw_tool_macro::ToolSchema)]
#[serde(rename_all = "camelCase")]
struct RefInput {
    /// Artifact kind — must match a registered [`crate::ReferenceSpec`] name.
    #[schema(description = "Reference kind, matching a declared ReferenceSpec name")]
    kind: String,
    /// Content-addressed artifact identifier returned by `create`.
    #[schema(description = "Content-addressed artifact id")]
    id: String,
}

impl From<RefInput> for ArtifactRef {
    fn from(input: RefInput) -> Self {
        Self {
            kind: input.kind,
            id: input.id,
        }
    }
}

fn parse_input(tool_use_id: &str, input: JsonValue) -> Result<ArtifactRef, ToolCallResult> {
    serde_json::from_value::<RefInput>(input)
        .map(ArtifactRef::from)
        .map_err(|e| ToolCallResult::error(tool_use_id, format!("Invalid input: {e}")))
}

fn reference_error_to_result(tool_use_id: &str, err: ReferenceError) -> ToolCallResult {
    // `NotFound` is the expected "miss" path — surface as an error result
    // so the LLM can react, but with a stable, parseable shape.
    ToolCallResult::error(tool_use_id, err.to_string())
}

// ─── resolveRef ─────────────────────────────────────────────────────

/// Handler for `resolveRef`. Returns the full stored value.
pub(crate) struct ResolveRefHandler {
    registry: Arc<ReferenceRegistry>,
}

impl ResolveRefHandler {
    pub(crate) fn new(registry: Arc<ReferenceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for ResolveRefHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "resolveRef".to_string(),
            description: "Resolve a typed artifact reference to its full stored payload."
                .to_string(),
            input_schema: RefInput::json_schema(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let artifact = match parse_input(tool_use_id, input) {
            Ok(a) => a,
            Err(err) => return err,
        };
        match self.registry.resolve(&artifact, env.resource_id()).await {
            Ok(body) => ToolCallResult::success(
                tool_use_id,
                json!({
                    "kind": body.kind,
                    "value": body.value,
                    "glimpse": body.glimpse,
                }),
            ),
            Err(e) => reference_error_to_result(tool_use_id, e),
        }
    }
}

// ─── glimpseRef ─────────────────────────────────────────────────────

/// Handler for `glimpseRef`. Returns only the host-precomputed glimpse.
pub(crate) struct GlimpseRefHandler {
    registry: Arc<ReferenceRegistry>,
}

impl GlimpseRefHandler {
    pub(crate) fn new(registry: Arc<ReferenceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for GlimpseRefHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "glimpseRef".to_string(),
            description: "Resolve only the cached glimpse for a typed artifact reference."
                .to_string(),
            input_schema: RefInput::json_schema(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let artifact = match parse_input(tool_use_id, input) {
            Ok(a) => a,
            Err(err) => return err,
        };
        match self.registry.glimpse(&artifact, env.resource_id()).await {
            Ok(glimpse) => ToolCallResult::success(
                tool_use_id,
                json!({ "kind": artifact.kind, "glimpse": glimpse }),
            ),
            Err(e) => reference_error_to_result(tool_use_id, e),
        }
    }
}

// ─── Toolkit entry point ────────────────────────────────────────────

pub(super) fn handlers(
    toolkit_id: &str,
    cfg: &ToolkitConfig,
    registry: Arc<ReferenceRegistry>,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    let handlers: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(ResolveRefHandler::new(registry.clone())),
        Arc::new(GlimpseRefHandler::new(registry)),
    ];
    filter_by_config(toolkit_id, handlers, cfg)
}
