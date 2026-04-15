//! Tool: run_linter — run linting tools for the Critic archetype.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Runs linters (cargo clippy, eslint) and returns structured findings.
pub struct RunLinterTool {
    workspace_dir: PathBuf,
}

impl RunLinterTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for RunLinterTool {
    fn name(&self) -> &str {
        "run_linter"
    }

    fn description(&self) -> &str {
        "Run linting tools on the codebase. Supports 'clippy' for Rust and 'eslint' for \
         TypeScript/JavaScript. Returns warnings and errors."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "linter": {
                    "type": "string",
                    "enum": ["clippy", "eslint", "auto"],
                    "description": "Which linter to run. 'auto' detects from project files.",
                    "default": "auto"
                },
                "path": {
                    "type": "string",
                    "description": "Limit linting to a specific path."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let linter = args
            .get("linter")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let linter = if linter == "auto" {
            if self.workspace_dir.join("Cargo.toml").exists() {
                "clippy"
            } else if self.workspace_dir.join("package.json").exists() {
                "eslint"
            } else {
                return Ok(ToolResult::error(
                    "Could not detect project type for linting.",
                ));
            }
        } else {
            linter
        };

        let output = match linter {
            "clippy" => {
                tokio::process::Command::new("cargo")
                    .args([
                        "clippy",
                        "--message-format=short",
                        "--",
                        "-W",
                        "clippy::all",
                    ])
                    .current_dir(&self.workspace_dir)
                    .output()
                    .await?
            }
            "eslint" => {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                if path.starts_with('/') || path.contains("..") {
                    return Ok(ToolResult::error(
                        "path must be a relative path within the workspace \
                             (no absolute paths or '..')",
                    ));
                }
                tokio::process::Command::new("npx")
                    .args(["eslint", "--format", "compact", path])
                    .current_dir(&self.workspace_dir)
                    .output()
                    .await?
            }
            other => {
                return Ok(ToolResult::error(format!("Unknown linter: {other}")));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined = if stdout.is_empty() {
            stderr.to_string()
        } else {
            format!("{stdout}\n{stderr}")
        };

        if output.status.success() {
            Ok(ToolResult::success(combined))
        } else {
            Ok(ToolResult::error(format!(
                "Linter exited with code {:?}\n{}",
                output.status.code(),
                combined
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_tool(dir: &TempDir) -> RunLinterTool {
        RunLinterTool::new(dir.path().to_path_buf())
    }

    #[test]
    fn name_is_correct() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(make_tool(&tmp).name(), "run_linter");
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
    fn permission_level_is_execute() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(make_tool(&tmp).permission_level(), PermissionLevel::Execute);
    }

    #[tokio::test]
    async fn auto_returns_error_when_no_project_files() {
        let tmp = TempDir::new().unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"linter": "auto"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Could not detect project type"));
    }

    #[tokio::test]
    async fn unknown_linter_returns_error() {
        let tmp = TempDir::new().unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"linter": "rubocop"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Unknown linter"));
    }

    #[tokio::test]
    async fn eslint_rejects_absolute_path() {
        let tmp = TempDir::new().unwrap();
        // Create a package.json so linter resolves to eslint
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"linter": "eslint", "path": "/etc/passwd"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("relative path"));
    }

    #[tokio::test]
    async fn eslint_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();
        let result = make_tool(&tmp)
            .execute(json!({"linter": "eslint", "path": "../secret"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("relative path"));
    }
}
