//! Tool: read_workspace_state — read-only workspace overview for Orchestrator/Planner.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Returns a summary of the workspace: git status, file tree, recent commits.
pub struct WorkspaceStateTool {
    workspace_dir: PathBuf,
}

impl WorkspaceStateTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for WorkspaceStateTool {
    fn name(&self) -> &str {
        "read_workspace_state"
    }

    fn description(&self) -> &str {
        "Get a read-only overview of the workspace: git status (modified/untracked files), \
         recent commits, and top-level directory structure. Useful for understanding the \
         current project state before planning tasks."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "include_tree": {
                    "type": "boolean",
                    "description": "Include top-level directory tree (default: true).",
                    "default": true
                },
                "recent_commits": {
                    "type": "integer",
                    "description": "Number of recent commits to show (default: 5).",
                    "default": 5
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let include_tree = args
            .get("include_tree")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let recent_commits = args
            .get("recent_commits")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        tracing::debug!(
            "[workspace_state] dir={}, include_tree={include_tree}, recent_commits={recent_commits}",
            self.workspace_dir.display()
        );

        let mut output = String::new();
        let dir = &self.workspace_dir;

        // Git status
        output.push_str("## Git Status\n");
        match run_git(dir, &["status", "--porcelain"]).await {
            Ok(status) if status.trim().is_empty() => {
                output.push_str("Clean working tree.\n");
            }
            Ok(status) => {
                output.push_str(&status);
            }
            Err(e) => {
                output.push_str(&format!("(not a git repo or error: {e})\n"));
            }
        }

        // Recent commits
        output.push_str(&format!("\n## Recent Commits (last {recent_commits})\n"));
        let log_arg = format!("-{recent_commits}");
        match run_git(dir, &["log", &log_arg, "--oneline", "--no-decorate"]).await {
            Ok(log) => output.push_str(&log),
            Err(e) => output.push_str(&format!("(error: {e})\n")),
        }

        // Directory tree (top-level only)
        if include_tree {
            output.push_str("\n## Directory Tree (top-level)\n");
            match tokio::fs::read_dir(dir).await {
                Ok(mut entries) => {
                    let mut names = Vec::new();
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.starts_with('.') {
                            let suffix = if entry
                                .file_type()
                                .await
                                .map(|ft| ft.is_dir())
                                .unwrap_or(false)
                            {
                                "/"
                            } else {
                                ""
                            };
                            names.push(format!("{name}{suffix}"));
                        }
                    }
                    names.sort();
                    for name in &names {
                        output.push_str(&format!("  {name}\n"));
                    }
                }
                Err(e) => output.push_str(&format!("(error reading dir: {e})\n")),
            }
        }

        tracing::debug!("[workspace_state] output length={}", output.len());
        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_tool(dir: &TempDir) -> WorkspaceStateTool {
        WorkspaceStateTool::new(dir.path().to_path_buf())
    }

    #[test]
    fn name_is_correct() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(make_tool(&tmp).name(), "read_workspace_state");
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
    async fn output_contains_git_status_section() {
        let tmp = TempDir::new().unwrap();
        let result = make_tool(&tmp).execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("Git Status"));
    }

    #[tokio::test]
    async fn include_tree_false_omits_directory_tree() {
        let tmp = TempDir::new().unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"include_tree": false}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(!result.output().contains("Directory Tree"));
    }

    #[tokio::test]
    async fn lists_non_hidden_files_in_tree() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("readme.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join(".hidden"), "skip").unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"include_tree": true, "recent_commits": 0}))
            .await
            .unwrap();
        assert!(!result.is_error);
        let out = result.output();
        assert!(out.contains("readme.txt"));
        assert!(!out.contains(".hidden"));
    }
}

async fn run_git(dir: &std::path::Path, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}
