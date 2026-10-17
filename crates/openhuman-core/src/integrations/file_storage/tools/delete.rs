//! `storage_delete_file` — permanently delete a stored file.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::{file_path, readonly_autonomy_block, validate_file_id};
use crate::integrations::IntegrationClient;
use crate::security::SecurityPolicy;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use crate::integrations::file_storage::types::DeleteResponse;

pub struct StorageDeleteFileTool {
    client: Arc<IntegrationClient>,
    security: Arc<SecurityPolicy>,
}

impl StorageDeleteFileTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self::new_with_security(client, Arc::new(SecurityPolicy::default()))
    }

    pub fn new_with_security(
        client: Arc<IntegrationClient>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self { client, security }
    }
}

#[async_trait]
impl Tool for StorageDeleteFileTool {
    fn name(&self) -> &str {
        "storage_delete_file"
    }

    fn description(&self) -> &str {
        "Permanently delete a file from managed cloud file storage, freeing quota. \
         Deletion is free and cannot be undone."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "The stored file's id" }
            },
            "required": ["file_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.run(args, &Default::default()).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(args, &approval).await
    }
}

impl StorageDeleteFileTool {
    /// Runs the tool for a turn whose approval context is `approval`.
    pub(crate) async fn run(
        &self,
        args: Value,
        approval: &crate::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = readonly_autonomy_block(&self.security) {
            return Ok(blocked);
        }

        let file_id = match validate_file_id(&args) {
            Ok(id) => id,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        tracing::debug!("[file_storage] deleting file_id={file_id}");
        match self
            .client
            .delete::<DeleteResponse>(approval, &file_path(&file_id, ""))
            .await
        {
            Ok(resp) if resp.deleted => Ok(ToolResult::success_with_markdown(
                json!({ "file_id": file_id, "deleted": true }),
                format!("Deleted file {file_id}."),
            )),
            Ok(_) => Ok(ToolResult::error(format!(
                "Backend did not confirm deletion of file {file_id}"
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to delete file: {e}"))),
        }
    }
}
