//! Host-side scope checks and durable fallback views for asynchronous
//! sub-agents. TinyAgents owns live child jobs and their cancellation lifecycle.

mod cancel;
mod registry;
mod roster;
mod task_ledger;

#[cfg(test)]
pub(crate) use cancel::cancel_for_thread_with_hook;
pub(crate) use cancel::{
    cancel_all, cancel_by_session_in_workspace, cancel_by_task_with_workspace_reason,
    cancel_for_thread, stop_for_thread, CancelledSubagent,
};
pub(crate) use registry::admit_spawn;
pub(crate) use roster::active_subagents_context_block;
pub(crate) use task_ledger::reconcile_orphaned_tasks_on_boot;
