//! Builds the full file-storage tool surface for a running config.

use std::path::Path;
use std::sync::Arc;

use openhuman_embed::__host::config::Config;
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
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(StorageUploadFileTool::new(
            Arc::clone(&client),
            action_dir.clone(),
        )),
        Box::new(StorageDownloadFileTool::new(
            Arc::clone(&client),
            action_dir,
        )),
        Box::new(StorageListFilesTool::new(Arc::clone(&client))),
        Box::new(StorageGetLinkTool::new(Arc::clone(&client))),
        Box::new(StorageSetVisibilityTool::new(Arc::clone(&client))),
        Box::new(StorageDeleteFileTool::new(Arc::clone(&client))),
    ];
    tracing::debug!(
        "[file_storage] registered {} file-storage tools",
        tools.len()
    );
    tools
}
