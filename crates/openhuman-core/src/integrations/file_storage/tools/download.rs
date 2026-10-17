//! `storage_download_file` — download a stored file into the agent workspace.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::{
    action_dir_for_context, extension_for_content_type, file_path, readonly_autonomy_block,
    sanitize_filename, validate_file_id, DOWNLOADS_DIR,
};
use crate::integrations::IntegrationClient;
use crate::security::SecurityPolicy;
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult};

pub struct StorageDownloadFileTool {
    client: Arc<IntegrationClient>,
    action_dir: PathBuf,
    security: Arc<SecurityPolicy>,
}

impl StorageDownloadFileTool {
    pub fn new(client: Arc<IntegrationClient>, action_dir: PathBuf) -> Self {
        Self::new_with_security(client, action_dir, Arc::new(SecurityPolicy::default()))
    }

    pub fn new_with_security(
        client: Arc<IntegrationClient>,
        action_dir: PathBuf,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            client,
            action_dir,
            security,
        }
    }

    async fn run(
        &self,
        args: Value,
        action_dir: &Path,
        approval: &crate::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = readonly_autonomy_block(&self.security) {
            return Ok(blocked);
        }

        let file_id = match validate_file_id(&args) {
            Ok(id) => id,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let requested_name = args
            .get("filename")
            .and_then(|v| v.as_str())
            .and_then(sanitize_filename);

        tracing::debug!("[file_storage] downloading file_id={file_id}");
        let (body, content_type, server_name) = match self
            .client
            .get_bytes(approval, &file_path(&file_id, "/download"))
            .await
        {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::error(format!("File download failed: {e}"))),
        };

        // Naming: explicit arg > server Content-Disposition > file_id + a
        // content-type-derived extension (mirrors persist_media's scheme).
        let filename = requested_name
            .or_else(|| server_name.as_deref().and_then(sanitize_filename))
            .unwrap_or_else(|| {
                format!(
                    "{file_id}.{}",
                    extension_for_content_type(content_type.as_deref())
                )
            });

        let dir = action_dir.join(DOWNLOADS_DIR);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            return Ok(ToolResult::error(format!(
                "failed to create downloads dir {}: {e}",
                dir.display()
            )));
        }
        let path = dir.join(&filename);
        if let Err(e) = tokio::fs::write(&path, &body).await {
            return Ok(ToolResult::error(format!(
                "failed to write {}: {e}",
                path.display()
            )));
        }
        tracing::debug!(
            "[file_storage] saved file_id={} → {} ({} bytes)",
            file_id,
            path.display(),
            body.len()
        );

        let payload = json!({
            "file_id": file_id,
            "path": path.display().to_string(),
            "size": body.len(),
            "content_type": content_type,
        });
        Ok(ToolResult::success_with_markdown(
            payload,
            format!(
                "Downloaded file {} ({} bytes) → {}",
                file_id,
                body.len(),
                path.display()
            ),
        ))
    }
}

#[async_trait]
impl Tool for StorageDownloadFileTool {
    fn name(&self) -> &str {
        "storage_download_file"
    }

    fn description(&self) -> &str {
        "Download a file from managed cloud file storage into the agent workspace and \
         return the saved local path. Egress is billed at S3 rates plus margin."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "The stored file's id (from storage_upload_file / storage_list_files)" },
                "filename": { "type": "string", "description": "Optional local filename to save as (defaults to the stored filename)" }
            },
            "required": ["file_id"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.run(args, &self.action_dir, &Default::default()).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let action_dir = action_dir_for_context(&self.action_dir, context, self.name());
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(args, &action_dir, &approval).await
    }
}
