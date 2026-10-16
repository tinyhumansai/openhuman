//! `storage_upload_file` — upload a workspace file to managed cloud storage.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::{mime_for_path, readonly_autonomy_block, resolve_upload_path};
use crate::integrations::IntegrationClient;
use openhuman_embed::__host::security::SecurityPolicy;
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult};

use super::helpers::{action_dir_for_context, FILES_PATH};
use crate::integrations::file_storage::types::UploadResponse;

pub struct StorageUploadFileTool {
    client: Arc<IntegrationClient>,
    action_dir: PathBuf,
    security: Arc<SecurityPolicy>,
}

impl StorageUploadFileTool {
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
        approval: &openhuman_embed::__host::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = readonly_autonomy_block(&self.security) {
            return Ok(blocked);
        }

        let raw_path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => return Ok(ToolResult::error("path is required")),
        };
        let resolved = match resolve_upload_path(action_dir, raw_path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(e)),
        };

        let visibility = match args.get("visibility").and_then(|v| v.as_str()) {
            Some(v) => match super::helpers::validate_visibility(v) {
                Ok(v) => Some(v),
                Err(e) => return Ok(ToolResult::error(e)),
            },
            None => None,
        };
        let ttl_days = match args.get("ttl_days") {
            Some(v) => match v.as_u64() {
                Some(d) if d >= 1 => Some(d),
                _ => return Ok(ToolResult::error("ttl_days must be a positive integer")),
            },
            None => None,
        };

        let bytes = match tokio::fs::read(&resolved).await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "failed to read {}: {e}",
                    resolved.display()
                )))
            }
        };
        let filename = resolved
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let mime = mime_for_path(&resolved);

        tracing::debug!(
            "[file_storage] uploading {} ({} bytes, mime={}, visibility={:?}, ttl_days={:?})",
            resolved.display(),
            bytes.len(),
            mime,
            visibility,
            ttl_days
        );

        let part = match reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.clone())
            .mime_str(mime)
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid mime '{mime}': {e}"))),
        };
        let mut form = reqwest::multipart::Form::new().part("file", part);
        if let Some(v) = &visibility {
            form = form.text("visibility", v.clone());
        }
        if let Some(d) = ttl_days {
            form = form.text("ttlDays", d.to_string());
        }

        match self
            .client
            .upload_multipart::<UploadResponse>(approval, FILES_PATH, form)
            .await
        {
            Ok(resp) => {
                let mut lines = vec![format!(
                    "Uploaded {} ({} bytes) as file_id {} — visibility {}, expires {}.",
                    resp.filename,
                    resp.size,
                    resp.file_id,
                    resp.visibility,
                    resp.expires_at.as_deref().unwrap_or("unknown"),
                )];
                if let Some(url) = &resp.public_url {
                    lines.push(format!("Public URL: {url}"));
                }
                lines.push(format!("Cost: ${:.4}", resp.cost_usd));
                let payload = json!({
                    "file_id": resp.file_id,
                    "filename": resp.filename,
                    "size": resp.size,
                    "content_type": resp.content_type,
                    "visibility": resp.visibility,
                    "expires_at": resp.expires_at,
                    "public_url": resp.public_url,
                    "cost_usd": resp.cost_usd,
                });
                Ok(ToolResult::success_with_markdown(payload, lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("File upload failed: {e}"))),
        }
    }
}

#[async_trait]
impl Tool for StorageUploadFileTool {
    fn name(&self) -> &str {
        "storage_upload_file"
    }

    fn description(&self) -> &str {
        "Upload a file from the agent workspace to managed cloud file storage and get a \
         file_id (and, for public files, a stable public URL). The path must be inside the \
         agent workspace. Files are billed at S3 rates plus margin (storage for the whole \
         TTL charged upfront on upload). Quota: 1 GiB per user. TTL: 7 days free / up to \
         1 year on paid plans."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to upload — absolute or relative to the agent workspace; must resolve inside the workspace" },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "Default private. Public files get a stable public URL anyone can fetch (egress billed to you)." },
                "ttl_days": { "type": "integer", "minimum": 1, "description": "File lifetime in days (clamped to plan max: 7 free / 365 paid). Default: plan max." }
            },
            "required": ["path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
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
        let approval =
            openhuman_embed::__host::agent::tinyagents::host::run_context::approval_for_tool(
                context,
            );
        self.run(args, &action_dir, &approval).await
    }
}
