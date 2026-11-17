//! Shared path/id/mime helpers used by every file-storage tool.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tinytools::ToolRunContext;

use openhuman_embed::__host::security::SecurityPolicy;
use tinytools::ToolResult;

pub(crate) const FILES_PATH: &str = "/agent-integrations/file-storage/files";

/// Subdirectory (under `action_dir`) where downloaded files are stored.
/// Mirrors `media_generation`'s `generated-media/` root — the action dir is
/// the agent's canonical read/write root, so this stays read-only-container
/// compatible.
pub(super) const DOWNLOADS_DIR: &str = "storage-downloads";

/// Resolve `raw` (absolute or relative to `action_dir`) to a canonical path
/// and reject anything that escapes the action dir (the agent's workspace).
/// The file must exist — canonicalization also resolves symlinks, so a
/// symlink pointing outside the workspace is rejected too.
pub(super) fn resolve_upload_path(action_dir: &Path, raw: &str) -> Result<PathBuf, String> {
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
pub(super) fn validate_file_id(args: &Value) -> Result<String, String> {
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
pub(super) fn validate_visibility(raw: &str) -> Result<String, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        v @ ("public" | "private") => Ok(v.to_string()),
        other => Err(format!(
            "visibility must be 'public' or 'private' (got '{other}')"
        )),
    }
}

/// Strip path separators / traversal from a caller- or server-supplied
/// filename so it always lands directly inside the downloads dir.
pub(super) fn sanitize_filename(name: &str) -> Option<String> {
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
pub(super) fn extension_for_content_type(content_type: Option<&str>) -> &'static str {
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
pub(crate) fn mime_for_path(path: &Path) -> &'static str {
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
pub(super) fn action_dir_for_context(
    default_action_dir: &Path,
    context: Option<&dyn ToolRunContext>,
    tool_name: &str,
) -> PathBuf {
    if let Some(workspace) = context.and_then(|ctx| ctx.workspace()) {
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

pub(crate) fn file_path(file_id: &str, suffix: &str) -> String {
    format!("{FILES_PATH}/{file_id}{suffix}")
}

pub(super) fn readonly_autonomy_block(security: &SecurityPolicy) -> Option<ToolResult> {
    if security.can_act() {
        None
    } else {
        Some(ToolResult::error(
            "[policy-blocked] Action blocked: autonomy is read-only",
        ))
    }
}
