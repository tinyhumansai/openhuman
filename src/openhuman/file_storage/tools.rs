//! Agent-facing file-storage tools backed by the OpenHuman backend's
//! `file_storage` provider (S3 under the hood).
//!
//! **Endpoints** (see the file-storage API contract):
//!   - `POST   /agent-integrations/file-storage/files` (multipart upload)
//!   - `GET    /agent-integrations/file-storage/files` (list)
//!   - `GET    /agent-integrations/file-storage/files/{id}/download` (302 → presigned S3)
//!   - `POST   /agent-integrations/file-storage/files/{id}/link` (presigned link)
//!   - `PATCH  /agent-integrations/file-storage/files/{id}` (visibility)
//!   - `DELETE /agent-integrations/file-storage/files/{id}`
//!
//! Billing: uploads are charged upfront for the whole TTL at S3 rates plus a
//! margin; downloads and link generation are charged as egress. Quota is
//! 1 GiB per user; TTL is 7 days on the free plan / up to 1 year on paid
//! plans. Public files get a stable public URL.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult,
};
use tinyagents::harness::tool::ToolExecutionContext;

use super::types::{DeleteResponse, FileMeta, LinkResponse, ListFilesResponse, UploadResponse};

const FILES_PATH: &str = "/agent-integrations/file-storage/files";

/// Subdirectory (under `action_dir`) where downloaded files are stored.
/// Mirrors `media_generation`'s `generated-media/` root — the action dir is
/// the agent's canonical read/write root, so this stays read-only-container
/// compatible.
const DOWNLOADS_DIR: &str = "storage-downloads";

// ── Shared helpers ──────────────────────────────────────────────────

/// Resolve `raw` (absolute or relative to `action_dir`) to a canonical path
/// and reject anything that escapes the action dir (the agent's workspace).
/// The file must exist — canonicalization also resolves symlinks, so a
/// symlink pointing outside the workspace is rejected too.
fn resolve_upload_path(action_dir: &Path, raw: &str) -> Result<PathBuf, String> {
    let candidate = {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            action_dir.join(p)
        }
    };
    let root = action_dir.canonicalize().map_err(|e| {
        format!(
            "workspace dir {} is not accessible: {e}",
            action_dir.display()
        )
    })?;
    let resolved = candidate.canonicalize().map_err(|e| {
        format!(
            "path {} does not exist or is not readable: {e}",
            candidate.display()
        )
    })?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "path {} escapes the agent workspace ({}) — only files inside the workspace can be uploaded",
            raw,
            root.display()
        ));
    }
    if !resolved.is_file() {
        return Err(format!("path {} is not a regular file", resolved.display()));
    }
    Ok(resolved)
}

/// Validate a caller-supplied file id before interpolating it into a URL
/// path segment.
fn validate_file_id(args: &Value) -> Result<String, String> {
    let id = args
        .get("file_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default();
    if id.is_empty() {
        return Err("file_id is required".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("file_id '{id}' contains invalid characters"));
    }
    Ok(id.to_string())
}

/// Parse + validate a `visibility` arg value.
fn validate_visibility(raw: &str) -> Result<String, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        v @ ("public" | "private") => Ok(v.to_string()),
        other => Err(format!(
            "visibility must be 'public' or 'private' (got '{other}')"
        )),
    }
}

/// Strip path separators / traversal from a caller- or server-supplied
/// filename so it always lands directly inside the downloads dir.
fn sanitize_filename(name: &str) -> Option<String> {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches('.');
    if base.is_empty() {
        return None;
    }
    let safe: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim().to_string();
    if safe.is_empty() {
        None
    } else {
        Some(safe)
    }
}

/// Pick a file extension from a content type (mirrors
/// `media_generation::download::extension_for`'s content-type branch, plus
/// common document types).
fn extension_for_content_type(content_type: Option<&str>) -> &'static str {
    let Some(ct) = content_type else { return "bin" };
    let ct = ct.to_ascii_lowercase();
    for (needle, ext) in [
        ("png", "png"),
        ("webp", "webp"),
        ("jpeg", "jpg"),
        ("jpg", "jpg"),
        ("gif", "gif"),
        ("mp4", "mp4"),
        ("webm", "webm"),
        ("pdf", "pdf"),
        ("zip", "zip"),
        ("json", "json"),
        ("csv", "csv"),
        ("html", "html"),
        ("text/plain", "txt"),
    ] {
        if ct.contains(needle) {
            return ext;
        }
    }
    "bin"
}

/// Guess a mime type from a filename extension for the multipart upload
/// part. Best-effort — the backend stores whatever we send and S3 doesn't
/// care; unknown extensions fall back to `application/octet-stream`.
fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("txt" | "md" | "log") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Resolve the effective action dir for a call, preferring the TinyAgents
/// workspace from the execution context (mirrors `media_generation`).
fn action_dir_for_context(
    default_action_dir: &Path,
    context: Option<&ToolExecutionContext>,
    tool_name: &str,
) -> PathBuf {
    if let Some(workspace) = context.and_then(|ctx| ctx.workspace.as_ref()) {
        tracing::debug!(
            tool = tool_name,
            workspace_root = %workspace.root.display(),
            policy_id = %workspace.policy_id,
            "[file_storage] using ToolExecutionContext workspace root"
        );
        return workspace.root.clone();
    }
    default_action_dir.to_path_buf()
}

fn file_path(file_id: &str, suffix: &str) -> String {
    format!("{FILES_PATH}/{file_id}{suffix}")
}

fn readonly_autonomy_block(security: &SecurityPolicy) -> Option<ToolResult> {
    if security.can_act() {
        None
    } else {
        Some(ToolResult::error(
            "[policy-blocked] Action blocked: autonomy is read-only",
        ))
    }
}

// ── StorageUploadFileTool ───────────────────────────────────────────

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

    async fn run(&self, args: Value, action_dir: &Path) -> anyhow::Result<ToolResult> {
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
            Some(v) => match validate_visibility(v) {
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
            .upload_multipart::<UploadResponse>(FILES_PATH, form)
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
        self.run(args, &self.action_dir).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let action_dir = action_dir_for_context(&self.action_dir, context, self.name());
        self.run(args, &action_dir).await
    }
}

// ── StorageDownloadFileTool ─────────────────────────────────────────

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

    async fn run(&self, args: Value, action_dir: &Path) -> anyhow::Result<ToolResult> {
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
            .get_bytes(&file_path(&file_id, "/download"))
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
        self.run(args, &self.action_dir).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let action_dir = action_dir_for_context(&self.action_dir, context, self.name());
        self.run(args, &action_dir).await
    }
}

// ── StorageListFilesTool ────────────────────────────────────────────

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
        tracing::debug!("[file_storage] listing files");
        match self.client.get::<ListFilesResponse>(FILES_PATH).await {
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

// ── StorageGetLinkTool ──────────────────────────────────────────────

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

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;
