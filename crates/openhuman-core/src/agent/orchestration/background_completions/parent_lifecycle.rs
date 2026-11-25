use super::*;

/// Drop every queued completion for `thread_id` and everything that finishes
/// for it later. Called when the thread is deleted: the router's deleted-parent
/// marker is durable, and the thread is also remembered as deleted in memory, so
/// a straggler that wins the cooperative-abort race, a child that registers after
/// the delete, or a result that finishes after a restart is dropped rather than
/// delivered into a thread that no longer exists. Takes the caller's workspace so
/// the marker is written even when no child of the thread was seen by this
/// process. Returns the number of queued completions removed.
pub(crate) fn discard_for_thread(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    thread_id: &str,
) -> usize {
    let seen = {
        let thread_key = key(ctx, thread_id);
        let mut st = state();
        st.deleted_threads.insert(thread_key.clone());
        st.stopped_threads.remove(&thread_key);
        st.thread_workspaces.contains_key(&thread_key)
    };
    let entry = entry_for_ctx(ctx, workspace_dir);
    // An ordinary thread that never admitted background work needs only the
    // in-memory fence. Durable lifecycle markers and settled completion history
    // both remain in the store after compaction, so a previously used thread
    // still gets a permanent delete marker after a restart.
    let has_history = match entry.store.try_list(Some(thread_id)) {
        Ok(records) => !records.is_empty(),
        Err(error) => {
            log::warn!(
                "[background_completions] could not inspect thread history before delete \
                 thread_id={thread_id} error={error}"
            );
            true
        }
    };
    let has_legacy_delete_marker = is_marked_deleted(&entry, thread_id);
    if !seen && !has_history && !has_legacy_delete_marker {
        return 0;
    }
    let removed = cancel_deleted_parent(&entry, thread_id);
    log::debug!(
        "[background_completions] discard_for_thread thread_id={thread_id} removed={removed}"
    );
    removed
}

/// Durably close a deleted parent and withdraw queued results, even when its
/// child sweep reported an unresolved task. The conversation row is already
/// gone, so late results must remain rejected after a process restart.
pub(crate) fn delete_parent_for_thread(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<usize, String> {
    let thread_key = key(ctx, thread_id);
    {
        let mut st = state();
        st.deleted_threads.insert(thread_key.clone());
        st.stopped_threads.remove(&thread_key);
        st.thread_workspaces
            .insert(thread_key, workspace_dir.to_path_buf());
    }
    let entry = entry_for_ctx(ctx, workspace_dir);
    let removed = entry
        .router
        .delete_parent(thread_id)
        .map_err(|error| format!("failed to persist deleted-parent marker: {error}"))?;
    crate::agent::orchestration::completion_owners::commit_thread_deletion(
        ctx,
        workspace_dir,
        thread_id,
    )?;
    log::debug!(
        "[background_completions] delete_parent_for_thread thread_id={thread_id} removed={removed}"
    );
    Ok(removed)
}

/// Task id used by the legacy durable "this thread was deleted" marker.
/// Current deletions use the router's `ParentDeleted` lifecycle state; this
/// marker remains readable for rows written by older host versions.
fn deleted_marker_id(thread_id: &str) -> String {
    format!("\u{1}host-thread-deleted:{thread_id}")
}

pub(super) fn is_marked_deleted(entry: &Entry, thread_id: &str) -> bool {
    entry.store.get(&deleted_marker_id(thread_id)).is_some()
}

/// Permanently delete `thread_id` in the router. Failures are logged; the
/// in-memory deleted set still gates this process.
fn cancel_deleted_parent(entry: &Entry, thread_id: &str) -> usize {
    entry
        .router
        .delete_parent(thread_id)
        .unwrap_or_else(|error| {
            log::error!(
                "[background_completions] delete_parent failed thread_id={thread_id} error={error}"
            );
            0
        })
}

/// Drop every queued completion for `thread_id` and gate late results from the
/// stopped generation.
///
/// The Stop-button counterpart of [`discard_for_thread`]: the user halted the
/// thread's work, so results that finished but were not yet delivered must not
/// start a fresh delivery turn behind their back. The thread stays alive; the
/// gate lifts at [`resume_stopped_thread`] (the next accepted user turn), while
/// the stopped generation's task ids stay tombstoned
/// ([`finish_stop_for_thread`]). Returns the number of queued completions removed.
pub(crate) fn discard_pending_for_thread(ctx: &DispatchCtx, thread_id: &str) -> usize {
    state().stopped_threads.insert(key(ctx, thread_id));
    let Some(router) = router_for_thread(ctx, thread_id) else {
        return 0;
    };
    let removed = router.cancel_parent(thread_id).unwrap_or_else(|error| {
        log::error!(
            "[background_completions] cancel_parent failed thread_id={thread_id} error={error}"
        );
        0
    });
    log::debug!(
        "[background_completions] discard_pending_for_thread thread_id={thread_id} removed={removed}"
    );
    removed
}

/// Complete a Stop after its registered children were aborted: tombstone their
/// task ids so a straggler from the stopped generation cannot record once a
/// later user turn reopens the thread.
pub(crate) fn finish_stop_for_thread(ctx: &DispatchCtx, thread_id: &str, task_ids: &[String]) {
    let Some(router) = router_for_thread(ctx, thread_id) else {
        return;
    };
    for task_id in task_ids {
        if let Err(error) = router.tombstone(task_id) {
            log::warn!("[background_completions] tombstone failed task_id={task_id} error={error}");
        }
    }
}

/// Reopen a thread's completion gate for a newly accepted user turn.
///
/// The Stop gate deliberately outlives registry cancellation, because a detached
/// child may be between driver admission and durable owner-row persistence
/// when Stop is pressed. New task ids remain distinct from the stopped generation.
/// Only a thread this process stopped or cancelled pays for the durable resume
/// marker, so an ordinary chat message writes nothing.
pub(crate) fn resume_stopped_thread(ctx: &DispatchCtx, thread_id: &str) {
    let was_stopped = state().stopped_threads.remove(&key(ctx, thread_id));
    if !was_stopped {
        return;
    }
    if let Some(router) = router_for_thread(ctx, thread_id) {
        router.resume_parent(thread_id);
        log::debug!("[background_completions] resumed thread_id={thread_id}");
    }
}

/// Record a child that registers while its parent thread is stopped or deleted.
///
/// Registration happens after the detached task is spawned. If Stop (or delete)
/// races that narrow interval the registry sweep cannot see the child;
/// tombstoning its task id here keeps it rejected even after a later user turn
/// reopens a stopped thread. Also notes which workspace holds the thread's
/// completions, so a later thread-scoped Stop can find the router. The first
/// child this process spawns on a (not deleted) thread lifts any
/// cancelled-parent marker an earlier process left behind (a Stop before a
/// restart): a live spawn proves the user re-engaged the thread. Returns whether
/// the child must be aborted.
pub(crate) fn mark_stopped_task_if_thread_stopped(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    thread_id: &str,
    task_id: &str,
) -> bool {
    let entry = entry_for_ctx(ctx, workspace_dir);
    let thread_key = key(ctx, thread_id);
    let (first_sight, mut stopped) = {
        let mut st = state();
        let first_sight = st
            .thread_workspaces
            .insert(thread_key.clone(), workspace_dir.to_path_buf())
            .is_none();
        let stopped =
            st.stopped_threads.contains(&thread_key) || st.deleted_threads.contains(&thread_key);
        (first_sight, stopped)
    };
    // Existing host delete markers predate the router's distinct permanent
    // deletion state. Preserve them, then consult the owner's lifecycle marker
    // for threads deleted by current versions.
    if first_sight && !stopped {
        if is_marked_deleted(&entry, thread_id) {
            state().deleted_threads.insert(thread_key.clone());
            stopped = true;
        } else {
            // A Stop is explicitly lifted by a new accepted child, but a
            // deleted parent cannot be resumed. Re-admit after attempting the
            // resume so deletion and storage errors fail closed.
            entry.router.resume_parent(thread_id);
            match entry.router.admit_child_generation(thread_id) {
                Ok(Some(_)) => {}
                admission => {
                    // Neither a closed lifecycle nor a transient store error
                    // proves permanent deletion: resume may have failed while
                    // the subsequent admission successfully read the old Stop
                    // marker. Refuse this child and let the next one retry.
                    if let Err(error) = admission {
                        log::warn!(
                            "[background_completions] parent admission failed thread_id={thread_id} error={error}"
                        );
                    }
                    if let Err(tombstone_error) = entry.router.tombstone(task_id) {
                        log::warn!(
                            "[background_completions] tombstone failed task_id={task_id} error={tombstone_error}"
                        );
                    }
                    let mut st = state();
                    if st
                        .thread_workspaces
                        .get(&thread_key)
                        .is_some_and(|path| path == workspace_dir)
                    {
                        st.thread_workspaces.remove(&thread_key);
                    }
                    return true;
                }
            }
        }
    }
    if stopped {
        if let Err(error) = entry.router.tombstone(task_id) {
            log::warn!("[background_completions] tombstone failed task_id={task_id} error={error}");
        }
        return true;
    }
    false
}

/// Withdraw every queued completion of `workspace_dir`. Called on a full thread
/// purge of that workspace; each thread with undelivered results is cancelled
/// durably (and remembered as deleted), so stragglers are still dropped. Other
/// workspaces are untouched. Returns the number of completions removed.
pub(crate) fn clear_all(ctx: &DispatchCtx, workspace_dir: &Path) -> usize {
    let entry = entry_for_ctx(ctx, workspace_dir);
    let parents: HashSet<String> = entry
        .store
        .list(None)
        .into_iter()
        .filter(|r| r.state == CompletionState::Pending && !r.parent_key.is_empty())
        .map(|r| r.parent_key)
        .collect();
    let mut removed = 0;
    for parent in parents {
        state().deleted_threads.insert(key(ctx, &parent));
        removed += cancel_deleted_parent(&entry, &parent);
    }
    log::debug!("[background_completions] clear_all removed={removed}");
    removed
}
