//! Read-only rosters over the sub-agent registry: the durable+live merge used to render the
//! `[active_subagents]` context block.

use tinyagents_orchestration::subagent::{snapshot_for_owner, SubagentSnapshot};

use super::registry::{admit_workspace, owner_key, registry};
use crate::agent::orchestration::fleet_tools::FleetToolSet;

/// Snapshot the sub-agents registered under `parent_session`, with each status
/// read live from its watch channel (ordered by `agent_id` then `task_id`).
pub(crate) fn snapshot_for_parent(
    ctx: &crate::core::runtime::DispatchCtx,
    parent_session: &str,
) -> Vec<SubagentSnapshot> {
    let Ok(owner) = owner_key(ctx, parent_session) else {
        return Vec::new();
    };
    snapshot_for_owner(registry(), &owner).unwrap_or_else(|err| {
        log::warn!(
            "[running_subagents] roster unavailable parent_session={parent_session} err={err:?}"
        );
        Vec::new()
    })
}

/// The follow-up guidance sentence, built from the tools the parent can see.
fn roster_guidance(fleet: &FleetToolSet) -> String {
    let mut parts: Vec<String> = Vec::new();
    if fleet.has("wait_subagent") {
        parts.push("use wait_subagent to collect a `completed` one".into());
    } else {
        parts.push(
            "a `running` or `completed` worker's result is delivered to you automatically on a \
             later turn — do not wait or poll for it"
                .into(),
        );
    }
    if fleet.has("steer_subagent") {
        parts.push("steer_subagent to redirect a `running` one".into());
    }
    if fleet.has("continue_subagent") {
        parts.push(
            "continue_subagent to answer an `awaiting_user` one or to RESUME an `idle` one with \
             a follow-up (it keeps its full prior context — do NOT re-delegate the same task \
             from scratch)"
                .into(),
        );
    }
    if fleet.has("close_subagent") {
        parts.push("close_subagent when done".into());
    }
    if fleet.has("list_subagents") {
        parts.push("list_subagents to re-enumerate".into());
    }
    let mut sentence = parts.join(", ");
    sentence.push('.');
    sentence
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
/// The roster merges two sources:
/// 1. the in-memory registry (live async workers spawned this process), and
/// 2. the durable per-workspace `subagent_sessions` store — workers from
///    EARLIER turns / process lifetimes. Without this second source a
///    cold-booted parent had no idea its previous sub-agents existed and
///    would re-delegate from scratch instead of resuming by
///    `subagent_session_id` (the "fresh context from day 0" bug).
///
/// `fleet` is the parent's fleet-control vocabulary: the guidance sentence
/// only names tools the parent can call (the orchestrator has no
/// `wait_subagent` / `steer_subagent` / `close_subagent` since #5701, and
/// telling it otherwise cost an iteration of confused reasoning per turn).
pub(crate) fn active_subagents_context_block(
    ctx: &crate::core::runtime::DispatchCtx,
    parent_session: &str,
    workspace_dir: &std::path::Path,
    fleet: &FleetToolSet,
) -> Option<String> {
    admit_workspace(ctx, workspace_dir).ok()?;
    let workers = snapshot_for_parent(ctx, parent_session);

    // Durable sessions not already represented by a live registry entry.
    let live_session_ids: std::collections::HashSet<String> = workers
        .iter()
        .filter_map(|w| w.subagent_session_id.clone())
        .collect();
    let store = crate::agent::orchestration::subagent_sessions::SubagentSessionStore {
        workspace_dir: workspace_dir.to_path_buf(),
    };
    let durable: Vec<_> = match crate::agent::orchestration::subagent_sessions::list_for_parent(
        &store,
        parent_session,
        None,
    ) {
        Ok(sessions) => sessions
            .into_iter()
            .filter(|s| {
                use crate::agent::orchestration::subagent_sessions::DurableSubagentStatus;
                s.status != DurableSubagentStatus::Closed
                    && !live_session_ids.contains(&s.subagent_session_id)
            })
            .take(DURABLE_ROSTER_CAP)
            .collect(),
        Err(err) => {
            log::warn!(
                    "[running_subagents] durable roster load failed parent_session={parent_session} error={err}"
                );
            Vec::new()
        }
    };

    if workers.is_empty() && durable.is_empty() {
        return None;
    }
    let mut block = format!(
        "[active_subagents]\n\
         You have {} sub-agent worker(s) for this conversation (live and/or from earlier \
         turns). This is your authoritative roster — trust it over memory. Track each by \
         subagent_session_id. {} Never fabricate a result for a worker still running \
         or one that has failed.\n",
        workers.len() + durable.len(),
        roster_guidance(fleet)
    );
    for w in &workers {
        let session = w.subagent_session_id.as_deref().unwrap_or("(none)");
        block.push_str(&format!(
            "- {} · session={} · task={} · status={}\n",
            w.agent_id, session, w.task_id, w.status
        ));
    }
    for s in &durable {
        use crate::agent::orchestration::subagent_sessions::DurableSubagentStatus;
        let status = match s.status {
            DurableSubagentStatus::Running => "running",
            DurableSubagentStatus::Idle => "idle",
            DurableSubagentStatus::AwaitingUser => "awaiting_user",
            DurableSubagentStatus::Failed => "failed",
            DurableSubagentStatus::Closed => "closed",
        };
        let task = s.current_task_id.as_deref().unwrap_or("(none)");
        block.push_str(&format!(
            "- {} · session={} · task={} · status={} · about: {}\n",
            s.agent_id, s.subagent_session_id, task, status, s.task_title
        ));
    }
    block.push_str("[/active_subagents]\n\n");
    Some(block)
}
