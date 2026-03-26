use super::traits::{Tool, ToolResult};
use crate::openhuman::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Write file contents with path sandboxing
pub struct FileWriteTool {
    security: Arc<SecurityPolicy>,
}

impl FileWriteTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write contents to a file in the workspace"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the file within the workspace"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }

        if self.security.is_rate_limited() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: too many actions in the last hour".into()),
            });
        }

        // Security check: validate path is within workspace
        if !self.security.is_path_allowed(path) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Path not allowed by security policy: {path}")),
            });
        }

        let full_path = self.security.workspace_dir.join(path);

        let Some(parent) = full_path.parent() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Invalid path: missing parent directory".into()),
            });
        };

        // Ensure parent directory exists
        tokio::fs::create_dir_all(parent).await?;

        // Resolve parent AFTER creation to block symlink escapes.
        let resolved_parent = match tokio::fs::canonicalize(parent).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to resolve file path: {e}")),
                });
            }
        };

        if !self.security.is_resolved_path_allowed(&resolved_parent) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Resolved path escapes workspace: {}",
                    resolved_parent.display()
                )),
            });
        }

        let Some(file_name) = full_path.file_name() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Invalid path: missing file name".into()),
            });
        };

        let resolved_target = resolved_parent.join(file_name);

        // If the target already exists and is a symlink, refuse to follow it
        if let Ok(meta) = tokio::fs::symlink_metadata(&resolved_target).await {
            if meta.file_type().is_symlink() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Refusing to write through symlink: {}",
                        resolved_target.display()
                    )),
                });
            }
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: action budget exhausted".into()),
            });
        }

        match tokio::fs::write(&resolved_target, content).await {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Written {} bytes to {path}", content.len()),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to write file: {e}")),
            }),
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

    fn test_security_with(
        workspace: std::path::PathBuf,
        autonomy: AutonomyLevel,
        max_actions_per_hour: u32,
    ) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            workspace_dir: workspace,
            max_actions_per_hour,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn file_write_name() {
        let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "file_write");
    }

    #[test]
    fn file_write_schema_has_path_and_content() {
        let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("content")));
    }

    #[tokio::test]
    async fn file_write_creates_file() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "out.txt", "content": "written!"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("8 bytes"));

        let content = tokio::fs::read_to_string(dir.join("out.txt"))
            .await
            .unwrap();
        assert_eq!(content, "written!");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_nested");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "a/b/c/deep.txt", "content": "deep"}))
            .await
            .unwrap();
        assert!(result.success);

        let content = tokio::fs::read_to_string(dir.join("a/b/c/deep.txt"))
            .await
            .unwrap();
        assert_eq!(content, "deep");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_overwrites_existing() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_overwrite");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("exist.txt"), "old")
            .await
            .unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "exist.txt", "content": "new"}))
            .await
            .unwrap();
        assert!(result.success);

        let content = tokio::fs::read_to_string(dir.join("exist.txt"))
            .await
            .unwrap();
        assert_eq!(content, "new");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_blocks_path_traversal() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_traversal");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "../../etc/evil", "content": "bad"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not allowed"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_blocks_absolute_path() {
        let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
        let result = tool
            .execute(json!({"path": "/etc/evil", "content": "bad"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not allowed"));
    }

    #[tokio::test]
    async fn file_write_missing_path_param() {
        let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
        let result = tool.execute(json!({"content": "data"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_write_missing_content_param() {
        let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
        let result = tool.execute(json!({"path": "file.txt"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_write_empty_content() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_empty");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "empty.txt", "content": ""}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("0 bytes"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_write_blocks_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join("openhuman_test_file_write_symlink_escape");
        let workspace = root.join("workspace");
        let outside = root.join("outside");

        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        symlink(&outside, workspace.join("escape_dir")).unwrap();

        let tool = FileWriteTool::new(test_security(workspace.clone()));
        let result = tool
            .execute(json!({"path": "escape_dir/hijack.txt", "content": "bad"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("escapes workspace"));
        assert!(!outside.join("hijack.txt").exists());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn file_write_blocks_readonly_mode() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_readonly");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security_with(dir.clone(), AutonomyLevel::ReadOnly, 20));
        let result = tool
            .execute(json!({"path": "out.txt", "content": "should-block"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("read-only"));
        assert!(!dir.join("out.txt").exists());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_blocks_when_rate_limited() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_rate_limited");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security_with(
            dir.clone(),
            AutonomyLevel::Supervised,
            0,
        ));
        let result = tool
            .execute(json!({"path": "out.txt", "content": "should-block"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Rate limit exceeded"));
        assert!(!dir.join("out.txt").exists());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // ── §5.1 TOCTOU / symlink file write protection tests ────

    #[cfg(unix)]
    #[tokio::test]
    async fn file_write_blocks_symlink_target_file() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join("openhuman_test_file_write_symlink_target");
        let workspace = root.join("workspace");
        let outside = root.join("outside");

        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        // Create a file outside and symlink to it inside workspace
        tokio::fs::write(outside.join("target.txt"), "original")
            .await
            .unwrap();
        symlink(outside.join("target.txt"), workspace.join("linked.txt")).unwrap();

        let tool = FileWriteTool::new(test_security(workspace.clone()));
        let result = tool
            .execute(json!({"path": "linked.txt", "content": "overwritten"}))
            .await
            .unwrap();

        assert!(!result.success, "writing through symlink must be blocked");
        assert!(
            result.error.as_deref().unwrap_or("").contains("symlink"),
            "error should mention symlink"
        );

        // Verify original file was not modified
        let content = tokio::fs::read_to_string(outside.join("target.txt"))
            .await
            .unwrap();
        assert_eq!(content, "original", "original file must not be modified");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn file_write_blocks_null_byte_in_path() {
        let dir = std::env::temp_dir().join("openhuman_test_file_write_null");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "file\u{0000}.txt", "content": "bad"}))
            .await
            .unwrap();
        assert!(!result.success, "paths with null bytes must be blocked");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
