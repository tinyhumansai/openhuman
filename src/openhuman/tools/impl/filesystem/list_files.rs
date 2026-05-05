//! `list` — directory listing.
//!
//! Coding-harness baseline tool (issue #1205): non-recursive directory
//! listing keyed by a workspace-relative path. Distinguishes files,
//! directories, and symlinks. Path sandboxing matches `file_read`.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

const MAX_ENTRIES: usize = 1_000;

pub struct ListFilesTool {
    security: Arc<SecurityPolicy>,
}

impl ListFilesTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list"
    }

    fn description(&self) -> &str {
        "List entries in a workspace directory (non-recursive). Each line is \
         `<kind>\t<name>` where kind is `dir`, `file`, or `link`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path relative to the workspace (default `.`).",
                    "default": "."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

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

        let mut read = match tokio::fs::read_dir(&resolved).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read directory: {e}"))),
        };

        let mut entries: Vec<(String, String)> = Vec::new();
        loop {
            match read.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let kind = match entry.file_type().await {
                        Ok(t) if t.is_symlink() => "link",
                        Ok(t) if t.is_dir() => "dir",
                        Ok(_) => "file",
                        Err(_) => "unknown",
                    };
                    entries.push((kind.to_string(), name));
                    if entries.len() >= MAX_ENTRIES {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to enumerate directory: {e}"
                    )))
                }
            }
        }

        entries.sort_by(|a, b| a.1.cmp(&b.1));

        let mut body = format!("{} entr(ies) in {path}", entries.len());
        for (kind, name) in entries {
            body.push('\n');
            body.push_str(&kind);
            body.push('\t');
            body.push_str(&name);
        }
        Ok(ToolResult::success(body))
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

    #[test]
    fn list_name() {
        let tool = ListFilesTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "list");
    }

    #[tokio::test]
    async fn list_lists_files_and_dirs() {
        let dir = std::env::temp_dir().join("openhuman_test_list");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("sub")).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "x").await.unwrap();

        let tool = ListFilesTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        let output = result.output();
        assert!(output.contains("file\ta.txt"));
        assert!(output.contains("dir\tsub"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn list_blocks_path_traversal() {
        let tool = ListFilesTool::new(test_security(std::env::temp_dir()));
        let result = tool.execute(json!({"path": "../../etc"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not allowed"));
    }

    #[tokio::test]
    async fn list_missing_dir() {
        let dir = std::env::temp_dir().join("openhuman_test_list_missing");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let tool = ListFilesTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"path": "nope"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Failed to resolve"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
