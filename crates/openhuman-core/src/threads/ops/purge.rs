//! Workspace-wide conversation purge.

use super::support::{envelope, run_to_completion, workspace_dir};
use crate::core::Outcome;
use crate::threads::store as conversations;
use crate::threads::{ApiEnvelope, EmptyRequest, PurgeConversationThreadsResponse};
use std::path::PathBuf;
use tinyagents_session::turn_state;

/// Purges all conversation threads and messages.
///
/// Same cancellation contract as `thread_delete`: the purge and its sub-agent
/// / turn-snapshot cleanup are one [`run_to_completion`] unit, so a dropped
/// caller cannot leave every thread wiped while their sub-agents keep running.
pub async fn threads_purge(
    _request: EmptyRequest,
) -> Result<Outcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("threads_purge", threads_purge_inner(dir)).await
}

async fn threads_purge_inner(
    dir: PathBuf,
) -> Result<Outcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let stats = conversations::blocking::purge_threads(dir.clone()).await?;
    // No parent thread survives a purge, so cancel every detached sub-agent and
    // wipe every queued result. Same ordering as `thread_delete`: abort the
    // in-flight runs first, then clear the delivery queue. Tombstone each
    // cancelled sub-agent's thread BEFORE the final wipe so a straggler that
    // wins the cooperative-abort race (records after the wipe) is still dropped
    // by `record_completion` rather than delivered into a purged thread.
    use crate::agent::orchestration::{background_completions, running_subagents};
    let cancelled_threads = running_subagents::cancel_all();
    let mut discarded = 0;
    for thread_id in &cancelled_threads {
        discarded += background_completions::discard_for_thread(thread_id);
    }
    discarded += background_completions::clear_all();
    log::debug!(
        "[threads] threads_purge cancelled_threads={} discarded_completions={}",
        cancelled_threads.len(),
        discarded
    );
    // Threads are gone, so any orphan turn snapshots can never be
    // reattached to a live thread. Wipe them in the same call so
    // `turn_state_list` returns an empty set after a purge. Use the
    // parse-independent `clear_all` so corrupted / half-written
    // snapshot files (which `list()` would warn-and-skip) are also
    // removed — a destructive cleanup must not leave behind anything
    // it failed to deserialize. Failures surface as RPC errors.
    turn_state::store::clear_all(dir.clone())
        .map_err(|err| format!("threads purged but turn-snapshot cleanup failed: {err}"))?;
    Ok(envelope(
        PurgeConversationThreadsResponse {
            messages_deleted: stats.message_count,
            agent_threads_deleted: stats.thread_count,
            agent_messages_deleted: stats.message_count,
        },
        None,
        None,
    ))
}
