//! Tool: run_tests — run test suites for the Critic archetype.

use super::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Runs test suites (cargo test, vitest) and returns pass/fail with output.
pub struct RunTestsTool {
    workspace_dir: PathBuf,
}

impl RunTestsTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
    }

    fn description(&self) -> &str {
        "Run the project test suite. Supports 'cargo_test' for Rust and 'vitest' for \
         TypeScript/JavaScript. Returns pass/fail results with output."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "runner": {
                    "type": "string",
                    "enum": ["cargo_test", "vitest", "auto"],
                    "description": "Which test runner to use. 'auto' detects from project files.",
                    "default": "auto"
                },
                "filter": {
                    "type": "string",
                    "description": "Filter to run specific tests (e.g. test name or module)."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120).",
                    "default": 120
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let runner = args
            .get("runner")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let filter = args.get("filter").and_then(|v| v.as_str());
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);

        let runner = if runner == "auto" {
            if self.workspace_dir.join("Cargo.toml").exists() {
                "cargo_test"
            } else if self.workspace_dir.join("package.json").exists() {
                "vitest"
            } else {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Could not detect project type for testing.".into()),
                });
            }
        } else {
            runner
        };

        let mut cmd = match runner {
            "cargo_test" => {
                let mut c = tokio::process::Command::new("cargo");
                c.arg("test");
                if let Some(f) = filter {
                    c.arg(f);
                }
                c
            }
            "vitest" => {
                let mut c = tokio::process::Command::new("npx");
                c.args(["vitest", "run"]);
                if let Some(f) = filter {
                    c.arg(f);
                }
                c
            }
            other => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Unknown test runner: {other}")),
                });
            }
        };

        cmd.current_dir(&self.workspace_dir);

        let output =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
                .await
                .map_err(|_| anyhow::anyhow!("test execution timed out after {timeout_secs}s"))??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined = format!("{stdout}\n{stderr}");
        // Truncate if very long.
        let truncated = if combined.len() > 8000 {
            format!(
                "{}...\n[truncated, {} total chars]",
                &combined[..8000],
                combined.len()
            )
        } else {
            combined
        };

        Ok(ToolResult {
            success: output.status.success(),
            output: truncated,
            error: if output.status.success() {
                None
            } else {
                Some(format!("Tests exited with code {:?}", output.status.code()))
            },
        })
    }
}
