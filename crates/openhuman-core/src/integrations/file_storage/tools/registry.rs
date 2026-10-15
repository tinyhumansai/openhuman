//! Builds the full file-storage tool surface for a running config.

use std::path::Path;
use std::sync::Arc;

use crate::config::Config;
use crate::security::SecurityPolicy;
use tinytools::Tool;

use super::delete::StorageDeleteFileTool;
use super::download::StorageDownloadFileTool;
use super::link::StorageGetLinkTool;
use super::list::StorageListFilesTool;
use super::upload::StorageUploadFileTool;
use super::visibility::StorageSetVisibilityTool;

/// Build the file-storage tool surface. Returns empty when no integration
/// client is configured (no backend URL / not signed in), mirroring
/// `build_media_tools`.
pub fn build_file_storage_tools(root_config: &Config, action_dir: &Path) -> Vec<Box<dyn Tool>> {
    let Some(client) = crate::integrations::build_client(root_config) else {
        tracing::debug!("[file_storage] no integration client — file-storage tools skipped");
        return Vec::new();
    };

    let action_dir = action_dir.to_path_buf();
    let security = Arc::new(SecurityPolicy::from_full_config(root_config));
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(StorageUploadFileTool::new_with_security(
            Arc::clone(&client),
            action_dir.clone(),
            Arc::clone(&security),
        )),
        Box::new(StorageDownloadFileTool::new_with_security(
            Arc::clone(&client),
            action_dir,
            Arc::clone(&security),
        )),
        Box::new(StorageListFilesTool::new(Arc::clone(&client))),
        Box::new(StorageGetLinkTool::new_with_security(
            Arc::clone(&client),
            Arc::clone(&security),
        )),
        Box::new(StorageSetVisibilityTool::new_with_security(
            Arc::clone(&client),
            Arc::clone(&security),
        )),
        Box::new(StorageDeleteFileTool::new_with_security(
            Arc::clone(&client),
            Arc::clone(&security),
        )),
    ];
    tracing::debug!(
        "[file_storage] registered {} file-storage tools",
        tools.len()
    );
    tools
}
