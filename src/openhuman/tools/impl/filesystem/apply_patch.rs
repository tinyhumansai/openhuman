//! `apply_patch` — atomic multi-edit across one or more files.
//!
//! Coding-harness baseline tool (issue #1205). Takes an array of
//! `{path, old_string, new_string}` edits and applies them atomically:
//! every edit is validated up front (path, exact-match, uniqueness)
//! before any file is written. If any edit fails validation, no files
//! are touched.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_EDITS: usize = 50;

pub struct ApplyPatchTool {
    security: Arc<SecurityPolicy>,
}

impl ApplyPatchTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a batch of exact-string edits across one or more files atomically. \
         All edits are validated before any are written; validation failure rolls \
         back the whole batch. Each edit is `{path, old_string, new_string, replace_all?}`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "description": "Ordered list of edits.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let edits = args
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing 'edits' array"))?;
        if edits.is_empty() {
            return Ok(ToolResult::error("`edits` array is empty"));
        }
        if edits.len() > MAX_EDITS {
            return Ok(ToolResult::error(format!(
                "Too many edits: {} (max {MAX_EDITS})",
                edits.len()
            )));
        }

        if !self.security.can_act() {
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // Parse + group edits by file.
        let mut parsed: Vec<ParsedEdit> = Vec::with_capacity(edits.len());
        for (i, raw) in edits.iter().enumerate() {
            let path = raw
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("edit[{i}]: missing `path`"))?;
            let old_string = raw
                .get("old_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("edit[{i}]: missing `old_string`"))?;
            let new_string = raw
                .get("new_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("edit[{i}]: missing `new_string`"))?;
            let replace_all = raw
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if old_string.is_empty() {
                return Ok(ToolResult::error(format!(
                    "edit[{i}]: `old_string` must not be empty"
                )));
            }
            if !self.security.is_path_allowed(path) {
                return Ok(ToolResult::error(format!(
                    "edit[{i}]: path not allowed: {path}"
                )));
            }
            parsed.push(ParsedEdit {
                index: i,
                path: path.to_string(),
                old_string: old_string.to_string(),
                new_string: new_string.to_string(),
                replace_all,
            });
        }

        // Resolve paths + load file contents (once per file). Apply edits in
        // memory; if any edit fails, return without writing.
        let mut buffers: HashMap<String, FileBuffer> = HashMap::new();
        for edit in &parsed {
            if !buffers.contains_key(&edit.path) {
                let full = self.security.workspace_dir.join(&edit.path);

                // Symlink check must happen on the *unresolved* path —
                // canonicalize resolves symlinks, so a check after that
                // point would never see the link.
                if let Ok(meta) = tokio::fs::symlink_metadata(&full).await {
                    if meta.file_type().is_symlink() {
                        return Ok(ToolResult::error(format!(
                            "edit[{}]: refusing to edit through symlink",
                            edit.index
                        )));
                    }
                }

                let resolved = match tokio::fs::canonicalize(&full).await {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "edit[{}]: failed to resolve {}: {e}",
                            edit.index, edit.path
                        )))
                    }
                };
                if !self.security.is_resolved_path_allowed(&resolved) {
                    return Ok(ToolResult::error(format!(
                        "edit[{}]: resolved path escapes workspace",
                        edit.index
                    )));
                }
                if let Ok(meta) = tokio::fs::metadata(&resolved).await {
                    if meta.len() > MAX_FILE_BYTES {
                        return Ok(ToolResult::error(format!(
                            "edit[{}]: file too large ({} bytes)",
                            edit.index,
                            meta.len()
                        )));
                    }
                }
                let contents = match tokio::fs::read_to_string(&resolved).await {
                    Ok(c) => c,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "edit[{}]: failed to read {}: {e}",
                            edit.index, edit.path
                        )))
                    }
                };
                buffers.insert(
                    edit.path.clone(),
                    FileBuffer {
                        resolved,
                        original: contents.clone(),
                        contents,
                        edit_count: 0,
                    },
                );
            }

            let buf = buffers.get_mut(&edit.path).unwrap();
            let count = buf.contents.matches(&edit.old_string).count();
            if count == 0 {
                return Ok(ToolResult::error(format!(
                    "edit[{}]: `old_string` not found in {}",
                    edit.index, edit.path
                )));
            }
            if count > 1 && !edit.replace_all {
                return Ok(ToolResult::error(format!(
                    "edit[{}]: `old_string` matches {count} times in {}; pass `replace_all`",
                    edit.index, edit.path
                )));
            }
            buf.contents = if edit.replace_all {
                buf.contents.replace(&edit.old_string, &edit.new_string)
            } else {
                buf.contents.replacen(&edit.old_string, &edit.new_string, 1)
            };
            buf.edit_count += count;
        }

        // Best-effort atomic write across files. We cannot get true
        // multi-file atomicity without filesystem-level transactions,
        // but if the i-th write fails we attempt to restore originals
        // for the i-1 already-written files from the in-memory snapshot.
        let mut summary: Vec<String> = Vec::new();
        let mut written: Vec<&FileBuffer> = Vec::new();
        for (path, buf) in &buffers {
            if let Err(e) = tokio::fs::write(&buf.resolved, &buf.contents).await {
                let restore_errors = restore_originals(&written).await;
                let suffix = if restore_errors.is_empty() {
                    "; previously-written files restored from snapshot".to_string()
                } else {
                    format!("; restore failed for: {}", restore_errors.join(", "))
                };
                return Ok(ToolResult::error(format!(
                    "Failed to write {path}: {e}{suffix}"
                )));
            }
            written.push(buf);
            summary.push(format!("{path}: {} replacement(s)", buf.edit_count));
        }
        summary.sort();
        Ok(ToolResult::success(format!(
            "Applied {} edit(s) across {} file(s)\n{}",
            parsed.len(),
            buffers.len(),
            summary.join("\n")
        )))
    }
}

async fn restore_originals(written: &[&FileBuffer]) -> Vec<String> {
    let mut errors = Vec::new();
    for buf in written {
        if let Err(e) = tokio::fs::write(&buf.resolved, &buf.original).await {
            errors.push(format!("{}: {e}", buf.resolved.display()));
        }
    }
    errors
}

struct ParsedEdit {
    index: usize,
    path: String,
    old_string: String,
    new_string: String,
    replace_all: bool,
}

struct FileBuffer {
    resolved: PathBuf,
    /// Snapshot of the file's contents as we first read them.
    /// Used to restore on a partial-write failure.
    original: String,
    contents: String,
    edit_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn apply_patch_name() {
        let tool = ApplyPatchTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "apply_patch");
    }

    #[tokio::test]
    async fn apply_patch_applies_multiple_edits() {
        let dir = std::env::temp_dir().join("openhuman_test_patch_multi");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "alpha\nbravo")
            .await
            .unwrap();
        tokio::fs::write(dir.join("b.txt"), "one two")
            .await
            .unwrap();

        let tool = ApplyPatchTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({
                "edits": [
                    { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                    { "path": "b.txt", "old_string": "two", "new_string": "TWO" }
                ]
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let a = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
        let b = tokio::fs::read_to_string(dir.join("b.txt")).await.unwrap();
        assert_eq!(a, "ALPHA\nbravo");
        assert_eq!(b, "one TWO");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn apply_patch_atomic_on_validation_failure() {
        let dir = std::env::temp_dir().join("openhuman_test_patch_atomic");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "alpha").await.unwrap();
        tokio::fs::write(dir.join("b.txt"), "bravo").await.unwrap();

        let tool = ApplyPatchTool::new(test_security(dir.clone()));
        // Second edit will fail (no match) — first must NOT be applied.
        let result = tool
            .execute(json!({
                "edits": [
                    { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                    { "path": "b.txt", "old_string": "missing", "new_string": "x" }
                ]
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        let a = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
        assert_eq!(a, "alpha", "atomic: first edit must not be persisted");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn apply_patch_chained_edits_same_file() {
        let dir = std::env::temp_dir().join("openhuman_test_patch_chain");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "one two three")
            .await
            .unwrap();

        let tool = ApplyPatchTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({
                "edits": [
                    { "path": "a.txt", "old_string": "one", "new_string": "ONE" },
                    { "path": "a.txt", "old_string": "two", "new_string": "TWO" }
                ]
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let updated = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
        assert_eq!(updated, "ONE TWO three");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn apply_patch_rejects_empty_edits() {
        let dir = std::env::temp_dir().join("openhuman_test_patch_empty");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = ApplyPatchTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"edits": []})).await.unwrap();
        assert!(result.is_error);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn apply_patch_rejects_traversal() {
        let tool = ApplyPatchTool::new(test_security(std::env::temp_dir()));
        let result = tool
            .execute(json!({
                "edits": [
                    { "path": "../etc/passwd", "old_string": "x", "new_string": "y" }
                ]
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not allowed"));
    }
}
