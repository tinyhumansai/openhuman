//! `storage_get_link` — presigned short-lived download link for a stored file.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::{file_path, readonly_autonomy_block, validate_file_id};
use crate::integrations::IntegrationClient;
use openhuman_embed::__host::security::SecurityPolicy;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use crate::integrations::file_storage::types::LinkResponse;

pub struct StorageGetLinkTool {
    client: Arc<IntegrationClient>,
    security: Arc<SecurityPolicy>,
}

impl StorageGetLinkTool {
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
impl Tool for StorageGetLinkTool {
    fn name(&self) -> &str {
        "storage_get_link"
    }

    fn description(&self) -> &str {
        "Generate a short-lived presigned download link for a stored file (works for \
         private files; 60s to 7 days, default 1 hour). Link generation is billed as \
         egress at S3 rates plus margin. For a stable permanent URL, set the file's \
         visibility to public instead."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "The stored file's id" },
                "expires_in_seconds": { "type": "integer", "minimum": 60, "maximum": 604800, "description": "Link lifetime in seconds (default 3600)" }
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
        self.run(args, &Default::default()).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval =
            openhuman_embed::__host::agent::tinyagents::host::run_context::approval_for_tool(
                context,
            );
        self.run(args, &approval).await
    }
}

impl StorageGetLinkTool {
    /// Runs the tool for a turn whose approval context is `approval`.
    pub(crate) async fn run(
        &self,
        args: Value,
        approval: &openhuman_embed::__host::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = readonly_autonomy_block(&self.security) {
            return Ok(blocked);
        }

        let file_id = match validate_file_id(&args) {
            Ok(id) => id,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let mut body = json!({});
        if let Some(secs) = args.get("expires_in_seconds").and_then(|v| v.as_u64()) {
            body["expiresInSeconds"] = json!(secs.clamp(60, 604_800));
        }
        tracing::debug!("[file_storage] generating link for file_id={file_id}");
        match self
            .client
            .post::<LinkResponse>(approval, &file_path(&file_id, "/link"), &body)
            .await
        {
            Ok(resp) => Ok(ToolResult::success_with_markdown(
                json!({
                    "file_id": file_id,
                    "url": resp.url,
                    "expires_at": resp.expires_at,
                    "cost_usd": resp.cost_usd,
                }),
                format!(
                    "Presigned link for {} (expires {}): {}\nCost: ${:.4}",
                    file_id,
                    resp.expires_at.as_deref().unwrap_or("unknown"),
                    resp.url,
                    resp.cost_usd
                ),
            )),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to generate download link: {e}"
            ))),
        }
    }
}
