
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
            .post::<LinkResponse>(&file_path(&file_id, "/link"), &body)
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

// ── StorageSetVisibilityTool ────────────────────────────────────────

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

// ── StorageDeleteFileTool ───────────────────────────────────────────

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
            .delete::<DeleteResponse>(&file_path(&file_id, ""))
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

// ── Builder ─────────────────────────────────────────────────────────

/// Build the file-storage tool surface. Returns empty when no integration
/// client is configured (no backend URL / not signed in), mirroring
/// `build_media_tools`.
pub fn build_file_storage_tools(root_config: &Config, action_dir: &Path) -> Vec<Box<dyn Tool>> {
    let Some(client) = crate::openhuman::integrations::build_client(root_config) else {
        tracing::debug!("[file_storage] no integration client — file-storage tools skipped");
        return Vec::new();
    };

    let action_dir = action_dir.to_path_buf();
    let security = Arc::new(SecurityPolicy::from_config(
        &root_config.autonomy,
        &root_config.workspace_dir,
        &root_config.action_dir,
    ));
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
