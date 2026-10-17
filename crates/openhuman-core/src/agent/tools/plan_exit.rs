//! `plan_exit` — signal the end of a plan-mode pass.
//!
//! Coding-harness baseline tool (issue #1205). When a plan-mode agent
//! is ready to hand off to an execution-mode agent, it calls
//! `plan_exit { plan }`. The tool returns a structured marker AND flips the
//! calling thread's `RunMode` (`agent::tinyagents::run_mode::set_mode`) back
//! to `Build`, so the very next tool exposure/execution check on that thread
//! sees every tool again — the actual gating lives in
//! `PlanModeMiddleware` (wired per-turn in `turn_middleware.rs`), not here;
//! this tool only flips the live handle the middleware reads.
//!
//! Threadless callers (no `ToolRunContext::thread_id`, e.g. a test double or
//! a run with no thread identity) have no per-thread mode to flip — the
//! marker is still returned so the plan text is not lost.

use async_trait::async_trait;
use serde_json::json;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

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
        self.execute_in_context(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, context).await
    }
}

impl PlanExitTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let plan = args
            .get("plan")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'plan' parameter"))?;
        let trimmed = plan.trim();
        if trimmed.is_empty() {
            return Ok(ToolResult::error("`plan` must not be empty"));
        }
        if let Some(thread_id) = context.and_then(ToolRunContext::thread_id) {
            // Only flip to Build when there is no plan review still parked on
            // this thread. A review resolves (approve/reject/revise) before
            // `request_plan_review` returns control to the agent, so by the
            // time a well-behaved agent calls `plan_exit` after an approval
            // the review is already gone from the gate's parked map — this
            // check is a no-op there. It only matters for a mis-timed or
            // concurrent `plan_exit` call that races a still-pending review:
            // flipping the mode early would unlock every tool for the thread
            // before the user has actually approved anything.
            if crate::agent::plan_review::gate::global()
                .parked_review_for_thread(thread_id)
                .is_some()
            {
                tracing::warn!(
                    thread_id = %thread_id,
                    "[tool][plan_exit] a plan review is still parked on this thread — not flipping to build"
                );
            } else {
                tracing::info!(
                    thread_id = %thread_id,
                    "[tool][plan_exit] flipping thread run mode to build"
                );
                crate::agent::tinyagents::run_mode::set_mode(
                    &crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context),
                    thread_id,
                    tinyagents_harness::middleware::RunMode::Build,
                );
            }
        } else {
            tracing::debug!("[tool][plan_exit] no thread id on this run context — nothing to flip");
        }
        Ok(ToolResult::success(format!(
            "{PLAN_EXIT_MARKER}\n{trimmed}"
        )))
    }
}

#[cfg(test)]
#[path = "plan_exit_tests.rs"]
mod tests;
