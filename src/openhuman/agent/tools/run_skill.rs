//! Tool: `run_skill` — let the orchestrator kick off another bundled
//! `skill_run` as a fresh autonomous background job.
//!
//! Use case: skill chaining. `github-issue-crusher` opens a draft PR at
//! step 9, then at step 10 it calls `run_skill` with `skill_id =
//! "pr-review-shepherd"` and `pr = <number>` so the shepherd takes over
//! the Phase-6 (CI + review) loop. The issue-crusher returns immediately
//! with the shepherd's `run_id` + log path; the two runs are independent
//! background tokio tasks with their own logs and their own autonomous
//! iter caps, so the issue-crusher exits cleanly while the shepherd keeps
//! driving the PR to mergeable.
//!
//! Implementation simply delegates to
//! `crate::openhuman::skills::schemas::spawn_skill_run_background` — the
//! same helper `openhuman.skills_run` JSON-RPC uses. Errors before the
//! spawn (unknown skill, missing required inputs) come back to the
//! orchestrator as a normal `ToolResult::error` so the model can correct
//! and retry. After the spawn succeeds the tool returns a small JSON
//! object with `run_id`, `skill_id`, and `log` for the orchestrator to
//! surface in its final response.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::skills::schemas::spawn_skill_run_background;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Tool name surfaced to the LLM's function-calling schema.
pub const RUN_SKILL_TOOL_NAME: &str = "run_skill";

/// `run_skill` agent tool — orchestrator-callable spawn of another bundled
/// skill_run.
pub struct RunSkillTool;

impl Default for RunSkillTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RunSkillTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for RunSkillTool {
    fn name(&self) -> &str {
        RUN_SKILL_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Spawn another bundled skill as a fresh autonomous background run. \
         Fire-and-forget: returns immediately with the new run's `run_id` and \
         streaming `log` path; the spawned run continues independently to DONE \
         / DEGENERATE / FAILED. \
         Use this to chain skills together — for example, after \
         `github-issue-crusher` opens a draft PR, call `run_skill` with \
         `skill_id: \"pr-review-shepherd\"` and the new PR number so the \
         shepherd takes over the CI + review loop while the crusher exits \
         cleanly. Arguments mirror the `openhuman.skills_run` JSON-RPC: \
         `skill_id` (string, required) names a skill from `skills_list`; \
         `inputs` (object, required) is the same input map that skill would \
         take via the RPC. Errors (unknown skill, missing required inputs) \
         come back synchronously so you can fix and retry without spawning \
         anything."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "Id of the bundled skill to spawn (must \
                                    appear in `skills_list`)."
                },
                "inputs": {
                    "type": "object",
                    "description": "Input object passed to the spawned skill, \
                                    same shape as the `inputs` field of \
                                    `openhuman.skills_run`. Required keys are \
                                    declared by the target skill's [[inputs]] \
                                    block."
                }
            },
            "required": ["skill_id", "inputs"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Spawning another autonomous skill_run carries the same blast radius
        // as the parent skill_run that's calling it (background tokio task,
        // no approval gate). The parent is already inside an autonomous
        // context, so promoting `run_skill` past the gate would be
        // double-counting — keep it at None (no extra prompt) and let the
        // target skill's SKILL.md govern what its run is allowed to do.
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let skill_id = match args.get("skill_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "run_skill: missing required argument `skill_id` (non-empty string)",
                ));
            }
        };
        let inputs = args.get("inputs").cloned();

        tracing::debug!(skill_id = %skill_id, "[run_skill] dispatching spawn_skill_run_background");
        match spawn_skill_run_background(skill_id.clone(), inputs).await {
            Ok(started) => {
                tracing::debug!(
                    skill_id = %started.skill_id,
                    run_id = %started.run_id,
                    "[run_skill] spawn succeeded"
                );
                Ok(ToolResult::success(
                    serde_json::json!({
                        "run_id": started.run_id,
                        "status": "started",
                        "skill_id": started.skill_id,
                        "log": started.log_path.display().to_string(),
                    })
                    .to_string(),
                ))
            }
            Err(e) => {
                tracing::debug!(skill_id = %skill_id, error = %e, "[run_skill] spawn failed");
                Ok(ToolResult::error(format!("run_skill: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_schema_basics() {
        let t = RunSkillTool::new();
        assert_eq!(t.name(), "run_skill");
        let schema = t.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required array");
        assert!(required.iter().any(|v| v.as_str() == Some("skill_id")));
        assert!(required.iter().any(|v| v.as_str() == Some("inputs")));
    }

    #[tokio::test]
    async fn missing_skill_id_returns_tool_error_not_panic() {
        let t = RunSkillTool::new();
        let res = t
            .execute(serde_json::json!({"inputs": {}}))
            .await
            .expect("Ok(ToolResult)");
        assert!(res.is_error, "expected ToolResult::error");
        assert!(res.output().contains("skill_id"));
    }
}
