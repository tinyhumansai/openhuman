//! Tool: read_diff — structured git diff output for the Critic archetype.

use super::traits::{PermissionLevel, Tool, ToolResult};
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

        let base_str;
        if let Some(b) = base {
            base_str = b.to_string();
            git_args.push(&base_str);
        }

        if let Some(pf) = path_filter {
            git_args.push("--");
            // Safe: path_filter is passed as a git pathspec.
            git_args.push(pf);
        }

        let output = tokio::process::Command::new("git")
            .args(&git_args)
            .current_dir(&self.workspace_dir)
            .output()
            .await?;

        if output.status.success() {
            let diff = String::from_utf8_lossy(&output.stdout);
            if diff.trim().is_empty() {
                Ok(ToolResult {
                    success: true,
                    output: "No changes found.".into(),
                    error: None,
                })
            } else {
                Ok(ToolResult {
                    success: true,
                    output: diff.to_string(),
                    error: None,
                })
            }
        } else {
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
            })
        }
    }
}
