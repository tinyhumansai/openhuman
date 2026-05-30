//! `plan_exit` — signal the end of a plan-mode pass.
//!
//! Coding-harness baseline tool (issue #1205). When a plan-mode agent
//! is ready to hand off to an execution-mode agent, it calls
//! `plan_exit { plan }`. The tool returns a structured marker that the
//! agent harness can recognize to transition modes; absent a harness
//! that consumes the marker, callers can still read the rendered plan
//! out of the result.
//!
//! This is intentionally a thin primitive — the actual mode switch
//! lives outside the tool. The follow-up `plan` vs `build` mode work
//! (referenced in issue #1205) will wire the harness side.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Stable marker the harness greps for to detect a plan→build hand-off.
pub const PLAN_EXIT_MARKER: &str = "[plan_exit]";

pub struct PlanExitTool;

impl PlanExitTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PlanExitTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PlanExitTool {
    fn name(&self) -> &str {
        "plan_exit"
    }

    fn description(&self) -> &str {
        "Exit plan mode and hand off the plan to execution. Call this once \
         the plan is complete — the wrapped harness will switch to build \
         mode (when wired). The `plan` argument is the user-facing plan \
         text that downstream agents will execute against."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "Markdown-formatted plan text to hand off."
                }
            },
            "required": ["plan"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let plan = args
            .get("plan")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'plan' parameter"))?;
        let trimmed = plan.trim();
        if trimmed.is_empty() {
            return Ok(ToolResult::error("`plan` must not be empty"));
        }
        Ok(ToolResult::success(format!(
            "{PLAN_EXIT_MARKER}\n{trimmed}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plan_exit_emits_marker() {
        let tool = PlanExitTool::new();
        let result = tool
            .execute(json!({ "plan": "1. Read X\n2. Edit Y" }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let output = result.output();
        assert!(output.starts_with(PLAN_EXIT_MARKER));
        assert!(output.contains("Read X"));
    }

    #[tokio::test]
    async fn plan_exit_rejects_empty() {
        let tool = PlanExitTool::new();
        let result = tool.execute(json!({ "plan": "   " })).await.unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn plan_exit_metadata() {
        let tool = PlanExitTool::new();
        assert_eq!(tool.name(), "plan_exit");
        assert_eq!(tool.permission_level(), PermissionLevel::None);
    }
}
