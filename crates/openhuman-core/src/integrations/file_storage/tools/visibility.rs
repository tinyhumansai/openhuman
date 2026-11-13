//! `storage_set_visibility` — flip a stored file between public and private.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::{file_path, readonly_autonomy_block, validate_file_id, validate_visibility};
use crate::integrations::IntegrationClient;
use crate::security::SecurityPolicy;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use crate::integrations::file_storage::types::FileMeta;

pub struct StorageSetVisibilityTool {
    client: Arc<IntegrationClient>,
    security: Arc<SecurityPolicy>,
}

impl StorageSetVisibilityTool {
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
impl Tool for StorageSetVisibilityTool {
    fn name(&self) -> &str {
        "storage_set_visibility"
    }

    fn description(&self) -> &str {
        "Change a stored file's visibility. Public files get a stable public URL anyone \
         can fetch (egress billed to you); private files are only reachable via \
         authenticated download or presigned links. Visibility changes are free."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "The stored file's id" },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "New visibility" }
            },
            "required": ["file_id", "visibility"]
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

impl StorageSetVisibilityTool {
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
        let visibility = match args.get("visibility").and_then(|v| v.as_str()) {
            Some(v) => match validate_visibility(v) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(e)),
            },
            None => return Ok(ToolResult::error("visibility is required")),
        };
        tracing::debug!("[file_storage] setting visibility={visibility} for file_id={file_id}");
        match self
            .client
            .patch::<FileMeta>(
                approval,
                &file_path(&file_id, ""),
                &json!({ "visibility": visibility }),
            )
            .await
        {
            Ok(meta) => {
                let mut md = format!("File {} is now {}.", meta.file_id, meta.visibility);
                if let Some(url) = &meta.public_url {
                    md.push_str(&format!("\nPublic URL: {url}"));
                }
                Ok(ToolResult::success_with_markdown(meta.to_json(), md))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to change file visibility: {e}"
            ))),
        }
    }
}
