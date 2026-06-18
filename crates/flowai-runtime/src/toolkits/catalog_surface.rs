//! `catalog` toolkit — replacement seven-tool catalog surface.

use std::sync::Arc;

use agent_fw_agent::ToolHandler;
use agent_fw_catalog_tools::surface::handlers::surface_handlers;

use super::{filter_by_config, ToolkitConfig, ToolkitError};

pub(super) fn handlers(
    toolkit_id: &str,
    cfg: &ToolkitConfig,
) -> Result<Vec<Arc<dyn ToolHandler>>, ToolkitError> {
    filter_by_config(toolkit_id, surface_handlers(), cfg)
}
