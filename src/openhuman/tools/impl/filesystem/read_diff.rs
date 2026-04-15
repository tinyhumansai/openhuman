//! Tool: read_diff — structured git diff output for the Critic archetype.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Returns `git diff` output in a structured format.
pub struct ReadDiffTool {
    workspace_dir: PathBuf,
}

impl ReadDiffTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for ReadDiffTool {
    fn name(&self) -> &str {
        "read_diff"
    }

    fn description(&self) -> &str {
        "Get the git diff of current changes. Can diff staged, unstaged, or against a \
         specific base branch/commit. Returns file paths and hunks."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "base": {
                    "type": "string",
                    "description": "Base ref to diff against (e.g. 'main', 'HEAD~3'). Default: unstaged changes."
                },
                "staged": {
                    "type": "boolean",
                    "description": "Show staged changes only (--cached). Default: false."
                },
                "path_filter": {
                    "type": "string",
                    "description": "Limit diff to a specific path or glob."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let base = args.get("base").and_then(|v| v.as_str());
        let staged = args
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let path_filter = args.get("path_filter").and_then(|v| v.as_str());

        let mut git_args = vec!["diff", "--stat", "-p"];

        if staged {
            git_args.push("--cached");
        }

        let base_str = base.map(|b| b.to_string());
        if let Some(ref bs) = base_str {
            git_args.push(bs);
        }

        if let Some(pf) = path_filter {
            git_args.push("--");
            git_args.push(pf);
        }

        tracing::debug!(
            workspace = %self.workspace_dir.display(),
            ?git_args,
            "[read_diff] running git diff"
        );

        let output = tokio::process::Command::new("git")
            .args(&git_args)
            .current_dir(&self.workspace_dir)
            .output()
            .await?;

        if output.status.success() {
            let diff = String::from_utf8_lossy(&output.stdout);
            tracing::debug!("[read_diff] success, diff length={}", diff.len());
            if diff.trim().is_empty() {
                Ok(ToolResult::success("No changes found."))
            } else {
                Ok(ToolResult::success(diff.to_string()))
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("[read_diff] failed: {stderr}");
            Ok(ToolResult::error(stderr.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_tool(dir: &TempDir) -> ReadDiffTool {
        ReadDiffTool::new(dir.path().to_path_buf())
    }

    #[test]
    fn name_is_correct() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(make_tool(&tmp).name(), "read_diff");
    }

    #[test]
    fn description_is_non_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(!make_tool(&tmp).description().is_empty());
    }

    #[test]
    fn schema_is_object_type() {
        let tmp = TempDir::new().unwrap();
        let schema = make_tool(&tmp).parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn permission_level_is_read_only() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            make_tool(&tmp).permission_level(),
            PermissionLevel::ReadOnly
        );
    }

    #[tokio::test]
    async fn execute_returns_error_for_non_git_dir() {
        let tmp = TempDir::new().unwrap();
        let result = make_tool(&tmp).execute(json!({})).await.unwrap();
        // Non-git dir: git will fail, tool returns error
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn execute_no_changes_in_clean_git_repo() {
        let tmp = TempDir::new().unwrap();
        // Init a git repo and make an initial commit so there's nothing to diff
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(tmp.path())
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "t@t.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "t@t.com")
            .output();
        let result = make_tool(&tmp).execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("No changes found."));
    }
}
