//! OpenHuman's tenant-aware adapter for the harness-owned `plan_exit` tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness::middleware::RunModeHandle;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

/// Resolve the live thread mode only when no plan review is still pending.
fn mode_lookup(context: &dyn ToolRunContext) -> Option<RunModeHandle> {
    let thread_id = context.thread_id()?;
    let dispatch_ctx =
        crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(Some(context));
    let tenant = dispatch_ctx.tenant().ok()?;
    if crate::plan_review::gate::global()
        .parked_review_for_thread(&tenant, thread_id)
        .is_some()
    {
        return None;
    }
    Some(crate::threads::run_mode::handle_for_thread(
        &dispatch_ctx,
        thread_id,
    ))
}

fn owner_tool() -> tinyagents_harness_tools::PlanExitTool {
    tinyagents_harness_tools::PlanExitTool::new(Arc::new(mode_lookup))
}

/// Public unit-shaped compatibility adapter around TinyAgents' tool engine.
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
        tinyagents_harness_tools::PlanExitTool::NAME
    }

    fn description(&self) -> &str {
        tinyagents_harness_tools::PlanExitTool::DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        owner_tool().parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        owner_tool().permission_level()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        owner_tool().execute(args).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        owner_tool()
            .execute_with_context(args, options, context)
            .await
    }
}

#[cfg(test)]
#[path = "plan_exit_tests.rs"]
mod tests;
