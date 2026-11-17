use super::support::envelope;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::store as conversations;
use crate::threads::{
    ApiEnvelope, DeleteConversationThreadRequest, DeleteConversationThreadResponse,
};
use std::path::PathBuf;

pub(super) async fn thread_delete_inner(
    ctx: DispatchCtx,
    config: crate::config::Config,
    request: DeleteConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let dir: PathBuf = config.workspace_dir.clone();
    crate::agent::orchestration::completion_owners::begin_thread_deletion(
        &ctx,
        &dir,
        &request.thread_id,
    )?;
    let deleted = match conversations::blocking::delete_thread(
        ctx.clone(),
        dir.clone(),
        request.thread_id.clone(),
        request.deleted_at.clone(),
    )
    .await
    {
        Ok(deleted) => deleted,
        Err(error) => {
            crate::agent::orchestration::completion_owners::release_thread_deletion(
                &ctx,
                &dir,
                &request.thread_id,
            );
            return Err(error);
        }
    };
    // Invalidate the in-process web-channel session BEFORE the
    // turn-state cleanup. The snapshot deletion is fallible and
    // returns early on error; if invalidation ran after, an active
    // session for the now-deleted thread could linger and try to
    // append to a thread index row that no longer exists.
    crate::web_chat::invalidate_thread_sessions(&ctx, &request.thread_id).await;
    // Cancel any detached sub-agents this thread spawned BEFORE clearing their
    // queued results: abort the in-flight ones first so a child can't record a
    // completion in the gap between the two calls, then discard anything already
    // queued for delivery. Both target a thread that's being deleted, so there's
    // nowhere left to deliver to — abort + cleanup is the whole behavior.
    let (cancelled_legacy, cancelled_driver) = if ctx.resolved().is_none() {
        if !crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &ctx,
            &dir,
            &request.thread_id,
        )
        .is_empty()
        {
            crate::agent::orchestration::background_completions::delete_parent_for_thread(
                &ctx,
                &dir,
                &request.thread_id,
            )?;
            return Err("cannot resolve core context to settle thread-owned driver jobs".into());
        }
        (0, 0)
    } else {
        let cancelled_legacy =
            match crate::agent::orchestration::running_subagents::cancel_for_thread(
                &ctx,
                &request.thread_id,
            )
            .await
            {
                Ok(cancelled) => cancelled,
                Err(error) => {
                    crate::agent::orchestration::background_completions::delete_parent_for_thread(
                        &ctx,
                        &dir,
                        &request.thread_id,
                    )
                    .map_err(|close_error| format!("{error}; {close_error}"))?;
                    return Err(error);
                }
            };
        let jobs = crate::core::runtime::slot_for::<
            tinyagents_orchestration::subagent::SubAgentJobRegistry,
        >(&ctx);
        let cancelled_driver =
            crate::agent::orchestration::subagent_sessions::cancel_driver_jobs_for_thread(
                &crate::agent::orchestration::subagent_sessions::SubagentSessionStore::new(
                    dir.clone(),
                ),
                &dir,
                &jobs,
                &request.thread_id,
            )
            .len();
        (cancelled_legacy, cancelled_driver)
    };
    let cancelled = cancelled_legacy + cancelled_driver;
    let discarded = crate::agent::orchestration::background_completions::delete_parent_for_thread(
        &ctx,
        &dir,
        &request.thread_id,
    )?;
    log::debug!(
        "[threads] thread_delete thread_id={} cancelled_subagents={} discarded_completions={}",
        request.thread_id,
        cancelled,
        discarded
    );
    // The thread's conversation memory goes too, for good (by `memory_ids`).
    // Record its deletion obligation before the fallible snapshot cleanup can
    // return early. Memory off or a failed forget queues it for the next
    // sign-in rather than failing the delete: the thread itself is already gone.
    let forgotten = crate::memory::deletion::forget_thread(&config, &request.thread_id).await;
    log::debug!(
        "[threads] thread_delete thread_id={} memory_items_forgotten={}",
        request.thread_id,
        forgotten
    );
    // Drop any persisted in-flight turn snapshot for this thread —
    // otherwise `threads_turn_state_list` keeps surfacing it (as
    // `Interrupted` on next restart) for a thread that no longer
    // exists. Failure here is surfaced as an RPC error so callers
    // can't observe a thread "deleted" while its snapshot (which
    // mirrors conversation-derived state) remains on disk; the
    // thread row itself is already gone at this point so the caller
    // sees a partial failure they can act on instead of silent drift.
    crate::storage::sessions::turn_states_for(&dir)
        .delete(&request.thread_id)
        .map_err(|err| {
            format!(
                "thread {} deleted but turn-snapshot cleanup failed: {err}",
                request.thread_id
            )
        })?;
    Ok(envelope(
        DeleteConversationThreadResponse { deleted },
        None,
        None,
    ))
}
