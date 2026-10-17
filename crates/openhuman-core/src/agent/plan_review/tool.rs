//! `request_plan_review` — the agent tool that parks the current interactive
//! turn on a plan the user must review before execution.
//!
//! The orchestrator calls this AFTER laying out a thread-scoped plan and BEFORE
//! executing it. On an interactive (`WebChat`) turn the call blocks on
//! [`PlanReviewGate`] until the user decides; the tool result then tells the
//! agent to proceed / stop / revise. On any non-interactive origin (cron,
//! background jobs, CLI, channels) there is no human to ask, so the tool
//! auto-approves immediately — background automation is never blocked.

use async_trait::async_trait;
use serde_json::json;

use crate::agent::turn_origin::AgentTurnOrigin;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext, ToolTimeout};

use super::gate;
use super::types::PlanReviewResolution;

pub struct RequestPlanReviewTool;

impl RequestPlanReviewTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RequestPlanReviewTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RequestPlanReviewTool {
    fn name(&self) -> &str {
        "request_plan_review"
    }

    fn description(&self) -> &str {
        "Pause the turn so the user can approve a thread-scoped plan before you execute it. Blocks until they decide, then returns `approved`, `rejected`, or `revise` with their feedback. Non-interactive turns (cron / CLI) auto-approve."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "One-line description of the plan being reviewed."
                },
                "steps": {
                    "type": "array",
                    "description": "Ordered plan steps shown to the user for review.",
                    "items": { "type": "string" }
                }
            },
            "required": ["summary"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // The gate IS the user consent surface — don't double-gate it through
        // the ApprovalGate as well.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
        // This tool BLOCKS while the user reviews the plan — the global tool
        // timeout (default ~120s) would otherwise drop the parked future before
        // the gate's own 10-minute TTL, so approving the visible card could not
        // resume the turn. The gate is the real deadline (fail-closed reject on
        // TTL), so the harness must not impose its own.
        ToolTimeout::Unbounded
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

impl RequestPlanReviewTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let tool_call_id = crate::tools::host_extensions::tool_call_id(context);
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let steps: Vec<String> = args
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Only interactive (WebChat) turns have a human to review the plan.
        // Anything else auto-approves so background automation isn't wedged.
        let origin = crate::agent::tinyagents::host::run_context::turn_origin_for_tool(context)
            .unwrap_or(AgentTurnOrigin::Unknown);
        if !matches!(origin, AgentTurnOrigin::WebChat { .. }) {
            tracing::debug!(
                origin = ?origin,
                "[tool][request_plan_review] non-interactive turn — auto-approving"
            );
            return Ok(ToolResult::success(
                "approved: non-interactive turn (no review surface) — proceed with the plan."
                    .to_string(),
            ));
        }

        // Route the surface back to the originating chat thread/client (set by
        // the web channel for the turn, the same context the ApprovalGate uses).
        let chat_ctx = crate::agent::tinyagents::host::run_context::approval_for_tool(context).chat;
        let thread_id = chat_ctx.as_ref().map(|c| c.thread_id.clone());
        let client_id = chat_ctx.as_ref().map(|c| c.client_id.clone());

        // Inert outside Plan mode (issue: plan-mode approvals). The chat
        // orchestrator carries this tool on its belt at all times so it is
        // reachable the instant a thread enters Plan mode (`agent.toml`), but
        // a research/lookup turn in ordinary Build mode must never park
        // behind a review card — that is the whole reason the tool was kept
        // off the orchestrator's belt before Plan mode existed. Deny the
        // model's own attempt to call it outside Plan mode with a plain
        // instruction rather than parking, so a mis-fire degrades to a no-op
        // instead of freezing the turn on an approval nobody asked for.
        let mode = thread_id
            .as_deref()
            .map(|thread_id| {
                crate::agent::tinyagents::run_mode::get_mode(
                    &crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context),
                    thread_id,
                )
            })
            .unwrap_or_default();
        if mode != tinyagents_harness::middleware::RunMode::Plan {
            tracing::debug!(
                thread_id = ?thread_id,
                "[tool][request_plan_review] thread is not in plan mode — not parking"
            );
            return Ok(ToolResult::success(
                "not applicable: this thread is not in plan mode, so there is no plan to \
                 review. Do not call `request_plan_review` again this turn."
                    .to_string(),
            ));
        }

        tracing::info!(
            thread_id = ?thread_id,
            steps = steps.len(),
            "[tool][request_plan_review] parking interactive turn for plan review"
        );

        let resolution = gate::global()
            .request_review(thread_id, client_id, summary, steps, tool_call_id)
            .await;

        let result = match resolution {
            PlanReviewResolution::Approve => ToolResult::success(
                "approved: the user approved the plan — proceed and execute it now.".to_string(),
            ),
            PlanReviewResolution::Reject => ToolResult::success(
                "rejected: the user rejected the plan — do NOT execute it. Briefly ask what \
                 they would like to do instead."
                    .to_string(),
            ),
            PlanReviewResolution::Revise { feedback } => ToolResult::success(format!(
                "revise: the user requested changes before executing. Their feedback:\n{feedback}\n\
                 Revise the plan accordingly, then call `request_plan_review` again before \
                 executing."
            )),
        };
        Ok(result)
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
