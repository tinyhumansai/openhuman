use super::{
    preferences,
    store,
    types::{Phase, Work},
};
use crate::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult, ToolRunContext};

pub struct AutonomousWorkTool {
    pub workspace: PathBuf,
}
fn text(args: &Value, name: &str) -> anyhow::Result<String> {
    let value = args[name].as_str().unwrap_or("").trim();
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 16000,
        "{name} must contain 1–16000 bytes"
    );
    Ok(value.into())
}
fn strings(args: &Value, name: &str) -> anyhow::Result<Vec<String>> {
    let values = args[name]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{name} must be an array"))?;
    anyhow::ensure!(
        !values.is_empty() && values.len() <= 50,
        "{name} requires 1–50 entries"
    );
    values
        .iter()
        .map(|v| text(&json!({"value": v}), "value"))
        .collect()
}
#[async_trait]
impl Tool for AutonomousWorkTool {
    fn name(&self) -> &str {
        "autonomous_work"
    }
    fn description(&self) -> &str {
        "Durable long-running coding task: start with the user's objective and acceptance criteria, checkpoint progress, request a separate review, or report a blocker. Runs continue across turns and app restarts until verified, stopped, blocked or budget-limited. Finish is accepted only during the host's review pass and after real verification tools succeed. Use status to inspect; resume only on a user's instruction. Status supports task_id or offset to page large plans. Use plan_task (task_id, title, acceptance_criteria, optional parent_id and depends_on) to decompose projects up to ten levels. task_start begins a ready package; task_review requests its separate verification; task_reopen invalidates affected components after a change. Global review requires every package verified. No overall time limit; token_budget only if requested."
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn policy(&self) -> tinytools::ToolPolicy {
        tinytools::ToolPolicy {
            classified: true,
            ..Default::default()
        }
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","required":["action"],"properties":{
            "action":{"type":"string","enum":["start","status","checkpoint","review","finish","block","resume","plan_task","task_start","task_review","task_reopen"]},
            "objective":{"type":"string"},"acceptance_criteria":{"type":"array","items":{"type":"string"}},
            "checkpoint":{"type":"string"},"evidence":{"type":"array","items":{"type":"string"}},
            "offset":{"type":"integer","minimum":0},"task_id":{"type":"string"},"parent_id":{"type":"string"},"title":{"type":"string"},
            "depends_on":{"type":"array","items":{"type":"string"}},
            "reason":{"type":"string"},"token_budget":{"type":"integer","minimum":1}
        }})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        _: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let report_args = args.clone();
        match self.apply(args, context).await {
            Ok(work) => Ok(ToolResult::success(serde_json::to_string(
                &json!({"work":super::plan::report(&work, &report_args),"note":"Only phase complete is finished. A requested finish is pending host verification."}),
            )?)),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}
impl AutonomousWorkTool {
    async fn apply(
        &self,
        args: Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<Work> {
        let thread = context
            .and_then(ToolRunContext::thread_id)
            .ok_or_else(|| anyhow::anyhow!("A chat thread is required"))?;
        let action = text(&args, "action")?;
        if action == "start" {
            anyhow::ensure!(
                matches!(
                    turn_origin::current(),
                    Some(AgentTurnOrigin::WebChat { .. })
                ),
                "Only a user-initiated turn can start autonomous work"
            );
            let objective = text(&args, "objective")?;
            let criteria = strings(&args, "acceptance_criteria")?;
            if let Some(old) = store::get(&self.workspace, thread).await? {
                anyhow::ensure!(
                    !old.runnable(),
                    "An unfinished task already exists; use checkpoint or review"
                );
            }
            let budget = args
                .get("token_budget")
                .map(|v| {
                    v.as_u64()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow::anyhow!("token_budget must be positive"))
                })
                .transpose()?;
            let model = args.get("model").and_then(|v| v.as_str()).map(ToString::to_string);
            let reasoning = args.get("reasoning").and_then(|v| v.as_str()).map(ToString::to_string);
            let temperature = args.get("temperature").and_then(|v| v.as_f64());
            if model.is_some() || reasoning.is_some() || temperature.is_some() {
                let prefs = preferences::Preferences { model, reasoning, temperature };
                let _ = preferences::save(&self.workspace, thread, &prefs).await;
            }
            return store::start(&self.workspace, thread, objective, criteria, budget).await;
        }
        let old = match store::get(&self.workspace, thread).await? {
            Some(existing) => existing,
            None => {
                if action == "plan_task" || action == "checkpoint" {
                    anyhow::ensure!(
                        matches!(
                            turn_origin::current(),
                            Some(AgentTurnOrigin::WebChat { .. })
                        ),
                        "Only a user-initiated turn can start autonomous work"
                    );
                    let objective = text(&args, "objective")
                        .or_else(|_| text(&args, "title"))
                        .unwrap_or_else(|_| "Autonomous project execution".to_string());
                    let criteria = strings(&args, "acceptance_criteria")
                        .unwrap_or_else(|_| vec!["Implement and verify all required project deliverables".to_string()]);
                    let budget = args
                        .get("token_budget")
                        .and_then(|v| v.as_u64())
                        .filter(|n| *n > 0);
                    let model = args.get("model").and_then(|v| v.as_str()).map(ToString::to_string);
                    let reasoning = args.get("reasoning").and_then(|v| v.as_str()).map(ToString::to_string);
                    let temperature = args.get("temperature").and_then(|v| v.as_f64());
                    if model.is_some() || reasoning.is_some() || temperature.is_some() {
                        let prefs = preferences::Preferences { model, reasoning, temperature };
                        let _ = preferences::save(&self.workspace, thread, &prefs).await;
                    }
                    store::start(&self.workspace, thread, objective, criteria, budget).await?
                } else {
                    anyhow::bail!("No autonomous task on this thread");
                }
            }
        };
        if action == "status" {
            return Ok(old);
        }
        if action == "resume" {
            anyhow::ensure!(
                matches!(
                    turn_origin::current(),
                    Some(AgentTurnOrigin::WebChat { .. })
                ),
                "Resume requires a user-initiated turn"
            );
            anyhow::ensure!(
                old.phase != Phase::Complete,
                "Completed work cannot be resumed; start a new task"
            );
            let current_goal = crate::agent::goals::store::get(&self.workspace, thread)
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                current_goal.is_some_and(|g| g.goal_id == old.goal_id),
                "Goal was replaced; start a new autonomous task"
            );
            let goal = crate::agent::goals::store::resume(&self.workspace, thread)
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                goal.status.is_active(),
                "Goal budget exhausted; the user must increase it before resuming"
            );
        }
        let review_origin = matches!(turn_origin::current(), Some(AgentTurnOrigin::TrustedAutomation { job_id, source:TrustedAutomationSource::AutonomousWork }) if old.lease.as_deref() == Some(job_id.as_str()));
        store::update(&self.workspace, thread, &old.id, |w| {
            if !turn_origin::current_is_user_authored() {
                anyhow::ensure!(
                    w.lease == old.lease && review_origin,
                    "Stale or unrelated background turn"
                );
            }
            anyhow::ensure!(
                w.runnable() || action == "resume",
                "Task is stopped; a user must resume it"
            );
            match action.as_str() {
                "plan_task" => {
                    let task_id = text(&args, "task_id")
                        .or_else(|_| text(&args, "id"))
                        .unwrap_or_else(|_| format!("task-{}", w.plan.len() + 1));
                    let title = text(&args, "title")
                        .or_else(|_| text(&args, "objective"))
                        .unwrap_or_else(|_| "Planned component".to_string());
                    let criteria = strings(&args, "acceptance_criteria")
                        .unwrap_or_else(|_| vec!["Implement and test component".to_string()]);
                    let depends_on = match args.get("depends_on") {
                        None => Vec::new(),
                        Some(Value::Array(items)) if items.is_empty() => Vec::new(),
                        Some(_) => strings(&args, "depends_on").unwrap_or_default(),
                    };
                    super::plan::add(
                        w,
                        super::plan::Task {
                            id: task_id,
                            parent: args
                                .get("parent_id")
                                .map(|_| text(&args, "parent_id"))
                                .transpose()?,
                            title,
                            acceptance_criteria: criteria,
                            depends_on,
                            state: super::plan::TaskState::Pending,
                            evidence: vec![],
                            revision: 0,
                        },
                    )?;
                }
                "task_start" => super::plan::begin(w, &text(&args, "task_id")?)?,
                "task_review" => {
                    anyhow::ensure!(
                        !w.checkpoint.is_empty(),
                        "Save implementation and test evidence in a checkpoint before review"
                    );
                    super::plan::request_review(w, &text(&args, "task_id")?)?;
                }
                "task_reopen" => {
                    let reason = text(&args, "reason")?;
                    super::plan::invalidate(w, &text(&args, "task_id")?)?;
                    w.last_error = Some(format!(
                        "Component changed; affected work requires verification again: {reason}"
                    ));
                }
                "checkpoint" => {
                    w.review_task = None;
                    w.checkpoint = text(&args, "checkpoint")?;
                    w.phase = Phase::Working;
                    w.finish_requested = false;
                    w.evidence.clear();
                }
                "review" => {
                    anyhow::ensure!(
                        super::plan::all_verified(w),
                        "Verify each planned work package with task_review before project review"
                    );
                    w.review_task = None;
                    anyhow::ensure!(
                        !w.checkpoint.is_empty(),
                        "Save an implementation and test checkpoint first"
                    );
                    w.phase = Phase::Reviewing;
                    w.finish_requested = false;
                }
                "finish" => {
                    anyhow::ensure!(
                        review_origin && w.phase == Phase::Reviewing,
                        "Completion requires the host's separate review pass"
                    );
                    let evidence = strings(&args, "evidence")?;
                    anyhow::ensure!(
                        evidence.len() == super::plan::criteria(w)?.len(),
                        "Provide one verification evidence entry for every acceptance criterion"
                    );
                    w.evidence = evidence;
                    w.finish_requested = true;
                }
                "block" => {
                    w.last_error = Some(text(&args, "reason")?);
                    w.phase = Phase::Blocked;
                    w.finish_requested = false;
                }
                "resume" => {
                    anyhow::ensure!(
                        w.phase != Phase::Complete,
                        "Completed work cannot be resumed; start a new task"
                    );
                    super::service::allow(thread);
                    w.lease = None;
                    w.phase = Phase::Working;
                    w.failures = 0;
                    w.stagnant_turns = 0;
                    w.next_run_ms = 0;
                    w.last_error = None;
                    w.finish_requested = false;
                }
                _ => anyhow::bail!("Unknown action"),
            }
            Ok(())
        })
        .await
    }
}
#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

/// Register the host's related completion controls together.
pub fn register_with_goals(tools: &mut Vec<Box<dyn Tool>>, workspace: &std::path::Path) {
    tools.extend(crate::agent::goals::goal_tools(workspace));
    tools.push(Box::new(AutonomousWorkTool {
        workspace: workspace.to_path_buf(),
    }));
}
