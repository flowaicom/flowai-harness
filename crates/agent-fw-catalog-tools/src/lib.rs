//! Replacement catalog tool surface for agent data discovery.
//!
//! # Architecture
//!
//! The public surface is the seven-tool contract implemented in [`surface`]:
//!
//! - `search_catalog`
//! - `get_catalog_entities`
//! - `list_schema_fields`
//! - `get_catalog_relations`
//! - `get_relation_paths_between`
//! - `sample_table_data`
//! - `execute_query`
//!
//! # Usage
//!
//! Tools are exposed as [`ToolHandler`](agent_fw_agent::ToolHandler) values
//! composed through the framework's canonical dispatcher.
//!
//! ```ignore
//! use agent_fw_agent::ComposedDispatcher;
//! use agent_fw_catalog_tools::surface_handlers;
//!
//! let dispatcher = ComposedDispatcher::new(env)
//!     .with_handlers(surface_handlers())
//!     .try_build()?;
//! ```

pub mod surface;
pub mod test_builder;
pub mod tier1_discovery;
pub mod tool_metadata;

// Re-export the read-only query execution path consumed by the surface.
pub use tier1_discovery::{execute_query, ExecuteQueryInput, ExecuteQueryOutput};

// Test Builder
pub use test_builder::{
    AddTrajectoryStepHandler, ComposeTrajectoryHandler, GetComposedTrajectoryHandler,
    GetGroundTruthHandler, ListEvalToolsHandler, MergeTraceSegmentHandler,
    RemoveTrajectoryStepHandler, ReorderTrajectoryStepHandler, SessionError,
    SetStructuredGroundTruthHandler, SetTrajectoryModeHandler, TestBuilderToolKit,
    TestCaseBuilderSession, TrajectoryAuthoringToolKit,
};

pub use surface::handlers::surface_handlers;

/// Errors from catalog tool execution.
#[derive(Debug, thiserror::Error)]
pub enum CatalogToolError {
    /// Catalog backend error.
    #[error("Catalog error: {0}")]
    Catalog(#[from] agent_fw_catalog::CatalogError),

    /// Database error (from TargetDatabase).
    #[error("Database error: {0}")]
    Database(#[from] agent_fw_algebra::DbError),

    /// Input validation failed.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Requested item not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Invalid ID format.
    #[error("Invalid ID: {0}")]
    InvalidId(String),
}

/// Validate a table name for safe use in queries.
///
/// Accepts alphanumeric characters, underscores, and dots (for schema.table).
/// Rejects SQL keywords, comments, and special characters.
pub fn is_valid_table_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return false;
    }
    if name.contains("--") || name.contains("/*") {
        return false;
    }
    name.split('.').all(|segment| {
        let lower = segment.to_lowercase();
        !matches!(
            lower.as_str(),
            "drop" | "delete" | "truncate" | "insert" | "update" | "alter" | "create"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::is_valid_table_name;

    #[test]
    fn valid_table_name_rejects_only_exact_keyword_segments() {
        assert!(is_valid_table_name("public.product_updates"));
        assert!(is_valid_table_name("analytics.created_orders"));
        assert!(!is_valid_table_name("public.drop"));
        assert!(!is_valid_table_name("delete"));
    }
}
