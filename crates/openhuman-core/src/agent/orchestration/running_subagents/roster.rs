//! Read-only rosters over the sub-agent registry: the durable+live merge used to render the
//! `[active_subagents]` context block.

use super::registry::admit_workspace;
use tinyagents_orchestration::subagent::{DurableSubagentSessionSummary, ParentRosterEntry};

#[cfg(test)]
#[path = "roster_tests.rs"]
mod tests;

/// The single supported follow-up path for a durable sub-agent task.
fn roster_guidance() -> &'static str {
    "reuse its task_key with spawn_subagent to continue or answer a clarification; completed results are delivered automatically."
}

/// Apply OpenHuman's prompt wording to the owner-scoped data supplied by
/// TinyAgents. Keeping this formatter here lets the host own product language
/// while the upstream registry remains the source of live job state.
fn format_roster(entries: &[ParentRosterEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut block = format!(
        "[active_subagents]\n\
         You have {} sub-agent worker(s). This is your authoritative roster — trust it over memory. Track each by \
         subagent_session_id. {} Never fabricate a result for a worker still running \
         or one that has failed.\n",
        entries.len(),
        roster_guidance()
    );
    for entry in entries {
        let session = entry.session.as_ref();
        let (session_id, task_id, task_title, status) = if let Some(session) = session {
            (
                session.subagent_session_id.as_str(),
                session
                    .current_task_id
                    .as_deref()
                    .unwrap_or(entry.task_id.as_str()),
                session.task_title.as_str(),
                match session.status {
                    tinyagents_orchestration::subagent::DurableSubagentStatus::Running => "running",
                    tinyagents_orchestration::subagent::DurableSubagentStatus::Idle => "idle",
                    tinyagents_orchestration::subagent::DurableSubagentStatus::AwaitingUser => {
                        "awaiting_user"
                    }
                    tinyagents_orchestration::subagent::DurableSubagentStatus::Failed => "failed",
                    tinyagents_orchestration::subagent::DurableSubagentStatus::Closed => "closed",
                },
            )
        } else if let Some(job) = entry.live_job.as_ref() {
            (
                "(live)",
                entry.task_id.as_str(),
                "(no durable session)",
                match job.status {
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Queued => "queued",
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Running => "running",
                    tinyagents_orchestration::subagent::SubAgentJobStatus::AwaitingInput => {
                        "awaiting_user"
                    }
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Completed => "completed",
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Failed => "failed",
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Incomplete => {
                        "incomplete"
                    }
                    tinyagents_orchestration::subagent::SubAgentJobStatus::Cancelled => "cancelled",
                },
            )
        } else {
            continue;
        };
        block.push_str(&format!(
            "- {} · session={} · task={} · status={} · about: {}\n",
            entry.agent_id, session_id, task_id, status, task_title
        ));
    }
    block.push_str("[/active_subagents]\n\n");
    block
}

fn workspace_scope_for_thread(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
    workspace: Option<&tinytools::WorkspaceDescriptor>,
) -> tinytools::WorkspaceDescriptor {
    workspace.cloned().unwrap_or_else(|| {
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            ctx,
            workspace_dir,
            thread_id,
        )
        .into_iter()
        .next()
        .map(|(_, owner)| owner.workspace)
        .unwrap_or_else(|| tinytools::WorkspaceDescriptor::new(workspace_dir))
    })
}

/// Most-recent durable sessions surfaced in the roster when they are not in
/// the live registry (cold boot / later turn). Bounds prompt growth on
/// threads with a long delegation history.
const DURABLE_ROSTER_CAP: usize = 12;

/// Build the ambient `[active_subagents]` block prepended to a parent's turn
/// context. Returns `None` when the parent owns no sub-agents at all, so the
/// block only appears when it is actionable — turns for agents that never
/// spawn are untouched. Mirrors the thread-goal `[active_goal]` block: it
/// rides the per-turn context (not the cached system-prompt prefix), so it
/// reflects live status every turn.
///
/// The roster reads the durable per-workspace `subagent_sessions` store — workers from
///    EARLIER turns / process lifetimes. Without this second source a
///    cold-booted parent had no idea its previous sub-agents existed and
///    would re-delegate from scratch instead of resuming by
///    `subagent_session_id` (the "fresh context from day 0" bug).
///
pub(crate) async fn active_subagents_context_block(
    ctx: &crate::core::runtime::DispatchCtx,
    parent_session: &str,
    workspace_dir: &std::path::Path,
    workspace: Option<&tinytools::WorkspaceDescriptor>,
) -> Option<String> {
    admit_workspace(ctx, workspace_dir).ok()?;
    let thread_id = crate::agent::orchestration::background_completions::thread_for_session(
        ctx,
        parent_session,
    )?;
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(ctx);
    let sessions = match crate::agent::orchestration::subagent_sessions::open_for_context(
        ctx,
        workspace_dir.to_path_buf(),
        parent_session,
        Some(thread_id.clone()),
        Some((*jobs).clone()),
    ) {
        Ok(store) => store
            .list_for_parent(parent_session, Some(&thread_id))
            .await
            .unwrap_or_else(|error| {
                log::warn!(
                    "[running_subagents] durable roster load failed parent_session={parent_session} error={error}"
                );
                Vec::new()
            }),
        Err(error) => {
            log::warn!(
                "[running_subagents] durable roster open failed parent_session={parent_session} error={error}"
            );
            Vec::new()
        }
    };
    let durable: Vec<DurableSubagentSessionSummary> = sessions
        .into_iter()
        .filter(|session| {
            session.status != tinyagents_orchestration::subagent::DurableSubagentStatus::Closed
        })
        .take(DURABLE_ROSTER_CAP)
        .map(|session| DurableSubagentSessionSummary {
            subagent_session_id: session.subagent_session_id,
            parent_thread_id: session.parent_thread_id,
            worker_thread_id: session.worker_thread_id,
            agent_id: session.agent_id,
            display_name: session.display_name,
            model: session.model,
            sandbox_mode: session.sandbox_mode,
            action_root: session.action_root,
            task_key: session.task_key,
            task_title: session.task_title,
            current_task_id: session.current_task_id,
            status: session.status,
            reusable: session.reusable,
            latest_error: session.latest_error,
            created_at: session.created_at,
            updated_at: session.updated_at,
            last_used_at: session.last_used_at,
        })
        .collect();
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        workspace_dir,
        parent_session,
        Some(&thread_id),
    );
    let scope = tinyagents_orchestration::subagent::ParentJobScope::new(
        workspace_scope_for_thread(ctx, workspace_dir, &thread_id, workspace),
        thread_id,
    );
    let block = tinyagents_orchestration::subagent::format_parent_roster_for_owner(
        &jobs,
        &owner,
        &scope,
        &durable,
        format_roster,
    )
    .ok()?;
    (!block.is_empty()).then_some(block)
}
