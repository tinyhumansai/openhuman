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
                return Ok(ToolResult::error(
                    "Could not detect project type for testing.",
                ));
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
                return Ok(ToolResult::error(format!("Unknown test runner: {other}")));
            }
        };

        tracing::debug!("[run_tests] runner={runner}, filter={filter:?}, timeout={timeout_secs}s");

        cmd.current_dir(&self.workspace_dir);
        cmd.kill_on_drop(true);

        let output =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
                .await
            {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolResult::error(format!(
                        "failed to spawn test runner: {e}"
                    )));
                }
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "test execution timed out after {timeout_secs}s"
                    )));
                }
            };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");

        // Truncate on a safe UTF-8 char boundary.
        let truncated = if combined.len() > 8000 {
            let safe_end = combined
                .char_indices()
                .take_while(|(i, _)| *i <= 8000)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!(
                "{}...\n[truncated, {} total chars]",
                &combined[..safe_end],
                combined.len()
            )
        } else {
            combined
        };

        tracing::debug!(
            "[run_tests] exit_code={:?}, output_len={}",
            output.status.code(),
            truncated.len()
        );

        if output.status.success() {
            Ok(ToolResult::success(truncated))
        } else {
            Ok(ToolResult::error(format!(
                "Tests exited with code {:?}\n\n{}",
                output.status.code(),
                truncated
            )))
        }
    }
}
