//! Tool: run_linter — run linting tools for the Critic archetype.

use super::traits::{PermissionLevel, Tool, ToolResult};
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
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Could not detect project type for linting.".into()),
                });
            }
        } else {
            linter
        };

        let output = match linter {
            "clippy" => {
                tokio::process::Command::new("cargo")
                    .args(["clippy", "--message-format=short", "--", "-W", "clippy::all"])
                    .current_dir(&self.workspace_dir)
                    .output()
                    .await?
            }
            "eslint" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                tokio::process::Command::new("npx")
                    .args(["eslint", "--format", "compact", path])
                    .current_dir(&self.workspace_dir)
                    .output()
                    .await?
            }
            other => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Unknown linter: {other}")),
                });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined = if stdout.is_empty() {
            stderr.to_string()
        } else {
            format!("{stdout}\n{stderr}")
        };

        Ok(ToolResult {
            success: output.status.success(),
            output: combined,
            error: if output.status.success() {
                None
            } else {
                Some(format!("Linter exited with code {:?}", output.status.code()))
            },
        })
    }
}
