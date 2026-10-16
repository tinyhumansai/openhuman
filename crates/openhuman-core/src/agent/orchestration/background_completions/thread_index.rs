//! The process-local index from chat thread and session ids to the workspace
//! and thread their completions belong to, keyed per profile (the SaaS tenant of
//! the dispatch context the caller acts under) and, for the thread index, per
//! storage scope: the records live in one scope's store, so the index says
//! which (#7687).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tinyagents_tasks::CompletionRouter;

use super::{entry_for_key, scope_now, state, SESSION_THREADS_CAP};
use crate::core::runtime::DispatchCtx;

/// The key of a thread or session id in `HostState`'s maps: per profile of
/// `ctx`'s tenant (see `profile_key_in`).
pub(super) fn key(ctx: &DispatchCtx, id: &str) -> String {
    use crate::core::runtime::tenant;
    tenant::profile_key_in(
        &tenant::isolated_unless_tenant(ctx.tenant(), "profile_key"),
        id,
    )
}

/// Remember which workspace, under which storage scope, holds `thread_id`'s
/// completions.
///
/// A call with no scope records nothing durable, so there is nothing to point
/// the index at.
pub(super) fn note_thread_workspace(ctx: &DispatchCtx, thread_id: &str, workspace_dir: &Path) {
    let Ok(scope) = scope_now() else {
        log::warn!("[background_completions] no storage scope; not indexing thread_id={thread_id}");
        return;
    };
    state()
        .thread_workspaces
        .insert((key(ctx, thread_id), scope), workspace_dir.to_path_buf());
}

/// The router that holds `thread_id`'s completions, if any child of that thread
/// was spawned or recorded in this process **under this call's scope**.
///
/// The scope is part of the lookup, so a thread-scoped Stop or delete reaches
/// the store that holds the records instead of reopening another scope's
/// workspace entry (#7687).
pub(crate) fn router_for_thread(
    ctx: &DispatchCtx,
    thread_id: &str,
) -> Option<Arc<CompletionRouter>> {
    let key = thread_key(ctx, thread_id)?;
    let workspace = state().thread_workspaces.get(&key).cloned()?;
    Some(entry_for_key(&(workspace, key.1)).router.clone())
}

/// The workspace holding `thread_id`'s completions under this call's scope, if
/// known to this process.
pub(crate) fn workspace_for_thread(ctx: &DispatchCtx, thread_id: &str) -> Option<PathBuf> {
    state()
        .thread_workspaces
        .get(&thread_key(ctx, thread_id)?)
        .cloned()
}

/// `thread_id`'s index key for this call: the per-profile thread key and the
/// storage scope. `None` when the call has no scope, which is also when it has
/// no records of its own to find.
fn thread_key(ctx: &DispatchCtx, thread_id: &str) -> Option<super::ThreadKey> {
    Some((key(ctx, thread_id), scope_now().ok()?))
}

/// Remember that `session_id` is a turn on `thread_id`.
pub(crate) fn note_session_thread(ctx: &DispatchCtx, session_id: &str, thread_id: &str) {
    let session_key = key(ctx, session_id);
    let mut st = state();
    if st
        .session_threads
        .insert(session_key.clone(), thread_id.to_string())
        .is_none()
    {
        st.session_order.push_back(session_key);
        while st.session_order.len() > SESSION_THREADS_CAP {
            if let Some(oldest) = st.session_order.pop_front() {
                st.session_threads.remove(&oldest);
            }
        }
    }
}

/// The chat thread a session id belongs to: the cached mapping, else the
/// `thread_id` a web-channel session id carries in its JSON body.
pub(crate) fn thread_for_session(ctx: &DispatchCtx, session_id: &str) -> Option<String> {
    if let Some(thread) = state().session_threads.get(&key(ctx, session_id)) {
        return Some(thread.clone());
    }
    if !session_id.starts_with('{') {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(session_id)
        .ok()?
        .get("thread_id")?
        .as_str()
        .map(str::to_owned)
}
