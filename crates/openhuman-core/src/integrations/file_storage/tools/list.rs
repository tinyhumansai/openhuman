//! `storage_list_files` — list stored files and quota usage.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::helpers::FILES_PATH;
use crate::integrations::IntegrationClient;
use tinytools::{Tool, ToolCategory, ToolResult};

use crate::integrations::file_storage::types::{FileMeta, ListFilesResponse};

pub struct StorageListFilesTool {
    client: Arc<IntegrationClient>,
}

impl StorageListFilesTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StorageListFilesTool {
    fn name(&self) -> &str {
        "storage_list_files"
    }

    fn description(&self) -> &str {
        "List your files in managed cloud file storage with sizes, visibility, expiry, \
         and current storage usage against the 1 GiB quota. Listing is free."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        self.run(_args, &Default::default()).await
    }

    async fn execute_with_context(
        &self,
        _args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(_args, &approval).await
    }
}

impl StorageListFilesTool {
    /// Runs the tool for a turn whose approval context is `approval`.
    pub(crate) async fn run(
        &self,
        _args: Value,
        approval: &crate::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        tracing::debug!("[file_storage] listing files");
        match self
            .client
            .get::<ListFilesResponse>(approval, FILES_PATH)
            .await
        {
            Ok(resp) => {
                let mut lines = vec![format!(
                    "{} file(s), using {} of {} bytes:",
                    resp.files.len(),
                    resp.usage.used_bytes,
                    resp.usage.limit_bytes
                )];
                for f in &resp.files {
                    lines.push(format!(
                        "- {} — {} ({} bytes, {}, expires {}){}",
                        f.file_id,
                        f.filename,
                        f.size,
                        f.visibility,
                        f.expires_at.as_deref().unwrap_or("unknown"),
                        f.public_url
                            .as_deref()
                            .map(|u| format!(" — {u}"))
                            .unwrap_or_default(),
                    ));
                }
                let payload = json!({
                    "files": resp.files.iter().map(FileMeta::to_json).collect::<Vec<_>>(),
                    "next_cursor": resp.next_cursor,
                    "usage": {
                        "used_bytes": resp.usage.used_bytes,
                        "limit_bytes": resp.usage.limit_bytes,
                    },
                });
                Ok(ToolResult::success_with_markdown(payload, lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to list files: {e}"))),
        }
    }
}
