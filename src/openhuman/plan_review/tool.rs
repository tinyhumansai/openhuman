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
use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
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
        "Pause an interactive turn so the user can review a thread-scoped plan \
         BEFORE you execute it (the Codex/Claude plan-mode contract). Call this \
         once you have laid out a multi-step plan (e.g. via the `todo` tool) and \
         BEFORE doing any of the work. The call BLOCKS until the user decides; \
         the result tells you what to do next: \
         `approved` → proceed and execute the plan now; \
         `rejected` → do NOT execute, ask the user what they want instead; \
         `revise` → the result carries the user's feedback — revise the plan \
         and call `request_plan_review` again before executing. \
         On non-interactive turns (cron / subconscious / CLI) this returns \
         `approved` immediately so automation is never blocked. \
         Pass `summary` (one line) and `steps` (the ordered plan items)."
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
mod tests {
    use super::*;
    use crate::openhuman::agent::turn_origin::with_origin;

    #[tokio::test]
    async fn non_interactive_origin_auto_approves() {
        let tool = RequestPlanReviewTool::new();
        let out = with_origin(
            AgentTurnOrigin::Cli,
            tool.execute(json!({ "summary": "do x", "steps": ["a", "b"] })),
        )
        .await
        .unwrap();
        assert!(!out.is_error);
        assert!(out.output().starts_with("approved"));
    }

    #[tokio::test]
    async fn interactive_turn_parks_until_resolved() {
        let tool = RequestPlanReviewTool::new();
        let fut = with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: "t-int".into(),
                client_id: "c-int".into(),
            },
            tool.execute(json!({ "summary": "plan", "steps": ["one"] })),
        );
        // An interactive turn must BLOCK on the gate rather than return
        // immediately — a short timeout elapses with no result (the parked
        // future is then dropped, and the gate cleans up).
        let res = tokio::time::timeout(std::time::Duration::from_millis(60), fut).await;
        assert!(
            res.is_err(),
            "interactive turn should park, not resolve immediately"
        );
    }
}
