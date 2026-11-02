//! Workspace-wide conversation purge.

use super::support::{envelope, run_to_completion, workspace_dir};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::store as conversations;
use crate::threads::{ApiEnvelope, EmptyRequest, PurgeConversationThreadsResponse};
use std::path::PathBuf;

/// Purges all conversation threads and messages.
///
/// Same cancellation contract as `thread_delete`: the purge and its sub-agent
/// / turn-snapshot cleanup are one [`run_to_completion`] unit, so a dropped
/// caller cannot leave every thread wiped while their sub-agents keep running.
pub async fn threads_purge(
    ctx: &DispatchCtx,
    _request: EmptyRequest,
) -> Result<Outcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion(ctx, "threads_purge", threads_purge_inner(ctx.clone(), dir)).await
}

async fn threads_purge_inner(
    ctx: DispatchCtx,
    dir: PathBuf,
) -> Result<Outcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    threads_purge_inner_with_snapshot_hook(ctx, dir, || async {}).await
}

pub(crate) async fn threads_purge_inner_with_snapshot_hook<F, Fut>(
    ctx: DispatchCtx,
    dir: PathBuf,
    after_snapshot: F,
) -> Result<Outcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String>
where
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = ()> + Send,
{
    let admission_workspace = dir.clone();
    let _admission = ctx
        .spawn_blocking(move || {
            crate::threads::store::backend::begin_thread_purge(&admission_workspace)
        })
        .await
        .map_err(|error| format!("failed to close thread admission for purge: {error}"))?;

    // Capture and fence the parents before deleting their rows. The owner
    // index is keyed by thread, and the permanent fence prevents another child
    // from being admitted while the purge settles the jobs already running.
    let threads = conversations::blocking::list_threads(ctx.clone(), dir.clone()).await?;
    after_snapshot().await;
    let thread_ids = threads
        .into_iter()
        .map(|thread| thread.id)
        .collect::<Vec<_>>();
    let mut fenced: Vec<String> = Vec::with_capacity(thread_ids.len());
    for thread_id in &thread_ids {
        if let Err(error) = crate::agent::orchestration::completion_owners::begin_thread_deletion(
            &ctx, &dir, thread_id,
        ) {
            for fenced_thread in &fenced {
                crate::agent::orchestration::completion_owners::release_thread_deletion(
                    &ctx,
                    &dir,
                    fenced_thread,
                );
            }
            return Err(error);
        }
        fenced.push(thread_id.clone());
    }
    // Settle the children while their owner scopes are still known. If the
    // context cannot safely cancel an indexed job, leave the parent threads in
    // place so the caller can retry rather than orphaning that live job after
    // the purge removes its thread row.
    use crate::agent::orchestration::running_subagents;
    let cancelled_threads = match running_subagents::cancel_all(&ctx, &dir, &thread_ids).await {
        Ok(cancelled) => cancelled.len(),
        Err(error) => {
            for thread_id in &fenced {
                crate::agent::orchestration::completion_owners::release_thread_deletion(
                    &ctx, &dir, thread_id,
                );
            }
            return Err(error);
        }
    };
    let stats = match conversations::blocking::purge_threads(ctx.clone(), dir.clone()).await {
        Ok(stats) => stats,
        Err(error) => {
            for thread_id in &fenced {
                crate::agent::orchestration::completion_owners::release_thread_deletion(
                    &ctx, &dir, thread_id,
                );
            }
            return Err(error);
        }
    };
    // No parent thread survives a purge. Cancel and settle each thread's live
    // driver jobs, then durably tombstone each parent before the final queue
    // wipe so a late result cannot be delivered after purge returns.
    use crate::agent::orchestration::background_completions;
    let mut discarded = 0;
    let mut cleanup_failures = Vec::new();
    for thread_id in &thread_ids {
        // Install the in-memory late-result fence before the durable marker.
        // `delete_parent_for_thread` persists the same lifecycle transition.
        discarded += background_completions::discard_for_thread(&ctx, &dir, thread_id);
        match background_completions::delete_parent_for_thread(&ctx, &dir, thread_id) {
            Ok(_) => {}
            Err(error) => cleanup_failures.push(format!(
                "failed to close purged thread {thread_id}: {error}"
            )),
        }
    }
    discarded += background_completions::clear_all(&ctx, &dir);
    if !cleanup_failures.is_empty() {
        return Err(format!(
            "threads purged but child cleanup failed: {}",
            cleanup_failures.join("; ")
        ));
    }
    log::debug!(
        "[threads] threads_purge cancelled_threads={} discarded_completions={}",
        cancelled_threads,
        discarded
    );
    // Threads are gone, so any orphan turn snapshots can never be
    // reattached to a live thread. Wipe them in the same call so
    // `turn_state_list` returns an empty set after a purge. Use the
    // parse-independent `clear_all` so corrupted / half-written
    // snapshot files (which `list()` would warn-and-skip) are also
    // removed — a destructive cleanup must not leave behind anything
    // it failed to deserialize. Failures surface as RPC errors.
    crate::storage::sessions::turn_states_for(&dir)
        .clear_all()
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
