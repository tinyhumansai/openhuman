//! The run-state block for OpenHuman turns.
//!
//! TinyAgents' [`StateBlockMiddleware`] adds one block to every model request
//! of the turn: the thread goal, the `todo` plan and facts derived from the
//! turn's own tool calls. It is request-only (never on the transcript) and
//! emits no events, so the UI keeps showing only the goal chip.
//!
//! It is installed on root orchestrator turns, the scope of the requirements
//! check. On those turns the turn start does not prepend the goal to the user's
//! message (`session_host::runtime_session::prelude`): the block shows the goal
//! once, current on every call, and a paused goal comes with the choice to
//! resume it, replace it or leave it, which the model makes with the goal
//! tools after reading the message.
//!
//! The block also shows the agent's own notes (facts, blockers, current
//! approach), written through the `todo` tool. After a turn's final reply,
//! [`close_finished_plan`] checks whether the plan finished: if so its notes
//! move to the thread's "last finished task" (active memory, shown on the
//! thread's next turns) and come back for the session host to queue into
//! long-term memory (`memory::lifecycle::plan_notes`).

use std::sync::Arc;

use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::store::Store;
use tinyagents_harness_tools::scratchpad::{store as scratchpad_store, ClosedPlan};
use tinyagents_harness_tools::state_block::StateBlockMiddleware;

/// Whether a turn gets the run-state block.
pub(crate) fn applies(is_subagent: bool, agent_definition_id: Option<&str>) -> bool {
    super::verify_before_finish::applies(is_subagent, agent_definition_id)
}

/// Installs the block for a turn in scope that has the session store the goal
/// and `todo` tools write, with the key the `todo` tool falls back to when the
/// turn has no thread (the session id).
///
/// A turn that suppresses the thread goal (`goal_suppressed`, from
/// `TurnOverrides::suppress_active_goal`: a conversational reply, a flow or a
/// cron run) still gets the block, without the goal: the plan, the agent's
/// notes and the turn's facts show, and the goal does not reach that turn.
pub(super) fn install<C: Send + Sync + 'static>(
    harness: &mut AgentHarness<(), C>,
    source: Option<(Arc<dyn Store>, String)>,
    is_subagent: bool,
    agent_definition_id: Option<&str>,
    goal_suppressed: bool,
) {
    match source {
        Some((store, fallback_thread)) if applies(is_subagent, agent_definition_id) => {
            if goal_suppressed {
                tracing::debug!(
                    agent = agent_definition_id.unwrap_or("<none>"),
                    "[state_block] installed without the goal: this turn suppresses it"
                );
            }
            harness.push_middleware(Arc::new(middleware(
                store,
                fallback_thread,
                goal_suppressed,
            )));
        }
        _ => tracing::debug!(
            is_subagent,
            agent = agent_definition_id.unwrap_or("<none>"),
            "[state_block] not installed for this turn"
        ),
    }
}

/// The block for one turn: the session store, the thread-less fallback key,
/// and the goal left out when the turn suppresses it.
fn middleware(
    store: Arc<dyn Store>,
    fallback_thread: String,
    goal_suppressed: bool,
) -> StateBlockMiddleware {
    StateBlockMiddleware::new(store)
        .with_fallback_thread(fallback_thread)
        .hide_goal(goal_suppressed)
}

/// A plan that finished this turn, with what it was for.
pub(crate) struct FinishedPlan {
    /// The plan with its notes.
    pub plan: ClosedPlan,
    /// The thread goal's objective when the plan finished, if it had one.
    pub goal: Option<String>,
}

/// After the final reply: when the thread's plan finished, moves its notes to
/// the thread's last finished task and returns the plan with its notes for
/// long-term memory. `None` without a thread, while the plan is open, without
/// notes, or when the store cannot be read.
pub(crate) async fn close_finished_plan(
    workspace_dir: &std::path::Path,
    thread_id: Option<&str>,
    context: &crate::core::runtime::DispatchCtx,
) -> Option<FinishedPlan> {
    let thread_id = thread_id.map(str::trim).filter(|id| !id.is_empty())?;
    let store = crate::storage::sessions::stores_or_ephemeral(workspace_dir).kv;
    match scratchpad_store::close_finished_plan(&store, thread_id).await {
        Ok(Some(plan)) => {
            tracing::debug!(
                thread_id,
                facts = plan.notes.facts.len(),
                blockers = plan.notes.blockers.len(),
                "[state_block] plan finished: notes moved to the last task"
            );
            let goal =
                crate::agent::goals::load_for_thread(workspace_dir, context, Some(thread_id))
                    .await
                    .map(|goal| goal.objective);
            Some(FinishedPlan { plan, goal })
        }
        Ok(None) => None,
        Err(error) => {
            tracing::debug!(thread_id, %error, "[state_block] close_finished_plan failed");
            None
        }
    }
}

#[cfg(test)]
#[path = "state_block_tests.rs"]
mod tests;
