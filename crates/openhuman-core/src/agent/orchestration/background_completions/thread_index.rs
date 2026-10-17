//! The process-local index from chat thread and session ids to the workspace
//! and thread their completions belong to, keyed per profile (the SaaS tenant of
//! the dispatch context the caller acts under).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tinyagents_tasks::CompletionRouter;

use super::{router_for_workspace, state, SESSION_THREADS_CAP};
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

/// Remember which workspace holds `thread_id`'s completions.
pub(super) fn note_thread_workspace(ctx: &DispatchCtx, thread_id: &str, workspace_dir: &Path) {
    state()
        .thread_workspaces
        .insert(key(ctx, thread_id), workspace_dir.to_path_buf());
}

/// The router that holds `thread_id`'s completions, if any child of that thread
/// was spawned or recorded in this process.
pub(crate) fn router_for_thread(
    ctx: &DispatchCtx,
    thread_id: &str,
) -> Option<Arc<CompletionRouter>> {
    let workspace = state()
        .thread_workspaces
        .get(&key(ctx, thread_id))
        .cloned()?;
    Some(router_for_workspace(&workspace))
}

/// The workspace holding `thread_id`'s completions, if known to this process.
pub(crate) fn workspace_for_thread(ctx: &DispatchCtx, thread_id: &str) -> Option<PathBuf> {
    state().thread_workspaces.get(&key(ctx, thread_id)).cloned()
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
