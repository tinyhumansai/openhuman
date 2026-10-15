//! Host context for ordinary goals and durable autonomous work.
use crate::agent::goals::runtime::GoalBudgetStopHook;
use std::path::Path;
pub async fn for_turn(
    workspace: &Path,
    thread: Option<&str>,
    suppress: bool,
) -> (String, Option<GoalBudgetStopHook>) {
    if suppress {
        return (String::new(), None);
    }
    let mut context = String::new();
    let mut budget_hook = None;
    let autonomous = if let Some(thread) = thread {
        crate::agent::autonomous::store::get(workspace, thread)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    let active_goal = if suppress {
        None
    } else {
        let loaded = crate::agent::goals::runtime::load_for_thread(workspace, thread).await;
        match loaded {
            Some(goal)
                if matches!(goal.status, crate::agent::goals::ThreadGoalStatus::Paused)
                    && crate::agent::turn_origin::current_is_user_authored()
                    && autonomous.is_none() =>
            {
                crate::agent::goals::runtime::resume_for_thread(workspace, thread)
                    .await
                    .unwrap_or(Some(goal))
            }
            other => other,
        }
    };
    if let Some(work) = &autonomous {
        context.push_str(&format!("[autonomous task] Phase: {:?}. Objective: {}. Checkpoint: {}. Use autonomous_work status/checkpoint/review; goal_complete cannot complete this task. If stopped, resume only when the user asks. Never treat a code block or a worker claim as a created, verified artifact.\n",work.phase,work.objective,work.checkpoint));
    }
    if let Some(goal) = &active_goal {
        if autonomous
            .as_ref()
            .is_none_or(|w| w.phase == crate::agent::autonomous::types::Phase::Complete)
        {
            if let Some(block) = tinyagents_graph::goals::active_goal_context_block(goal) {
                context.push_str(&block);
            }
        }
        if let Some(hook) =
            crate::agent::goals::runtime::GoalBudgetStopHook::for_goal(workspace, goal)
        {
            budget_hook = Some(hook);
        }
    }
    (context, budget_hook)
}
#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
