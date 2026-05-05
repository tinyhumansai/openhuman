//! `edit` — string-replace edit on a single file.
//!
//! Coding-harness baseline tool (issue #1205). Models the
//! Anthropic/Claude-Code `Edit` semantics: exact-match `old_string` →
//! `new_string` substitution. By default, `old_string` MUST match
//! exactly once in the file (so the model can't accidentally edit
//! every match). Set `replace_all` to override.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

pub struct EditFileTool {
    security: Arc<SecurityPolicy>,
}

impl EditFileTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file by exact string replacement. By default `old_string` must \
         match exactly once. Set `replace_all` to true to replace every match."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative file path." },
                "old_string": { "type": "string", "description": "Text to find. Must match exactly." },
                "new_string": { "type": "string", "description": "Replacement text." },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace every occurrence (default false).",
                    "default": false
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'old_string' parameter"))?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'new_string' parameter"))?;
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_string.is_empty() {
            return Ok(ToolResult::error("`old_string` must not be empty"));
        }
        if old_string == new_string {
            return Ok(ToolResult::error(
                "`old_string` and `new_string` are identical — nothing to do",
            ));
        }

        if !self.security.can_act() {
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.is_path_allowed(path) {
            return Ok(ToolResult::error(format!(
                "Path not allowed by security policy: {path}"
            )));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let full = self.security.workspace_dir.join(path);

        // Symlink check must happen on the *unresolved* path —
        // `canonicalize` resolves symlinks, so checking after that point
        // would always see the link's final target.
        if let Ok(meta) = tokio::fs::symlink_metadata(&full).await {
            if meta.file_type().is_symlink() {
                return Ok(ToolResult::error(format!(
                    "Refusing to edit through symlink: {}",
                    full.display()
                )));
            }
        }

        let resolved = match tokio::fs::canonicalize(&full).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Failed to resolve path: {e}"))),
        };
        if !self.security.is_resolved_path_allowed(&resolved) {
            return Ok(ToolResult::error(format!(
                "Resolved path escapes workspace: {}",
                resolved.display()
            )));
        }

        if let Ok(meta) = tokio::fs::metadata(&resolved).await {
            if meta.len() > MAX_FILE_BYTES {
                return Ok(ToolResult::error(format!(
                    "File too large: {} bytes (limit: {MAX_FILE_BYTES} bytes)",
                    meta.len()
                )));
            }
        }

        let contents = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read file: {e}"))),
        };

        let count = contents.matches(old_string).count();
        if count == 0 {
            return Ok(ToolResult::error(format!(
                "`old_string` not found in {path}"
            )));
        }
        if count > 1 && !replace_all {
            return Ok(ToolResult::error(format!(
                "`old_string` matches {count} times in {path}; pass `replace_all: true` or \
                 expand `old_string` so it is unique"
            )));
        }

        let updated = if replace_all {
            contents.replace(old_string, new_string)
        } else {
            contents.replacen(old_string, new_string, 1)
        };

        match tokio::fs::write(&resolved, &updated).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Edited {path}: {count} replacement(s)"
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {e}"))),
        }
    }
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

    fn test_security_readonly(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn edit_name() {
        let tool = EditFileTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "edit");
    }

    #[tokio::test]
    async fn edit_replaces_unique_match() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_unique");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "alpha bravo")
            .await
            .unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "bravo", "new_string": "charlie"}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let updated = tokio::fs::read_to_string(dir.join("f.txt")).await.unwrap();
        assert_eq!(updated, "alpha charlie");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_rejects_ambiguous_match() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_ambig");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "x x x").await.unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "x", "new_string": "y"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("matches 3 times"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_replace_all() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_all");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "x x x").await.unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(
                json!({"path": "f.txt", "old_string": "x", "new_string": "y", "replace_all": true}),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let updated = tokio::fs::read_to_string(dir.join("f.txt")).await.unwrap();
        assert_eq!(updated, "y y y");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_no_match() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_nomatch");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "alpha").await.unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "zulu", "new_string": "x"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not found"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_blocks_readonly_mode() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_ro");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "abc").await.unwrap();

        let tool = EditFileTool::new(test_security_readonly(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "abc", "new_string": "xyz"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_rejects_empty_old_string() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_empty_old");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "abc").await.unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "", "new_string": "x"}))
            .await
            .unwrap();
        assert!(result.is_error);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn edit_rejects_identical_strings() {
        let dir = std::env::temp_dir().join("openhuman_test_edit_same");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("f.txt"), "abc").await.unwrap();

        let tool = EditFileTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "f.txt", "old_string": "abc", "new_string": "abc"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("identical"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
