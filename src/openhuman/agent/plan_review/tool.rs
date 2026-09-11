//! `request_plan_review` — the agent tool that parks the current interactive
//! turn on a plan the user must review before execution.
//!
//! The orchestrator calls this AFTER laying out a thread-scoped plan and BEFORE
//! executing it. On an interactive (`WebChat`) turn the call blocks on
//! [`PlanReviewGate`] until the user decides; the tool result then tells the
//! agent to proceed / stop / revise. On any non-interactive origin (cron,
//! subconscious, CLI, channels) there is no human to ask, so the tool
//! auto-approves immediately — background automation is never blocked.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin};
use crate::openhuman::security::approval::APPROVAL_CHAT_CONTEXT;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};

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
        "Pause the turn so the user can approve a thread-scoped plan before you execute it. Blocks until they decide, then returns `approved`, `rejected`, or `revise` with their feedback. Non-interactive turns (cron / subconscious / CLI) auto-approve."
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
        let origin = turn_origin::current().unwrap_or(AgentTurnOrigin::Unknown);
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
        // the web channel around the turn, same task-local the ApprovalGate uses).
        let chat_ctx = APPROVAL_CHAT_CONTEXT.try_with(|c| c.clone()).ok();
        let thread_id = chat_ctx.as_ref().map(|c| c.thread_id.clone());
        let client_id = chat_ctx.as_ref().map(|c| c.client_id.clone());

        tracing::info!(
            thread_id = ?thread_id,
            steps = steps.len(),
            "[tool][request_plan_review] parking interactive turn for plan review"
        );

        let resolution = gate::global()
            .request_review(thread_id, client_id, summary, steps)
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
