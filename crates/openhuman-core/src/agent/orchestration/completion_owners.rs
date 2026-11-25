//! Which SaaS profile a detached sub-agent's completion belongs to, for the
//! bus subscriber that delivers it.
//!
//! The background-completion tables are keyed per profile
//! (`tenant::profile_key_in`), but the delivery subscriber
//! ([`super::background_delivery`]) runs off-task with no `CoreContext` scope,
//! so in SaaS it cannot read them. [`record_outcome`] runs inside the user's
//! scope: it notes the profile here under the sub-agent's task id (core-minted,
//! unique) and the parent session id, and the subscriber re-enters that
//! profile's context ([`context_for_profile`]) before it looks anything up.
//!
//! A session id is not unique across profiles (a web-chat one is
//! `{client_id, thread_id}`), so it can name several owners; each is drained in
//! its own scope, where it can only reach its own tables. Desktop contexts
//! carry no profile, so nothing is noted there and delivery stays unscoped.
//!
//! [`record_outcome`]: super::background_completions::record_outcome

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::core::runtime::CoreContext;

/// Bound on noted ids; the oldest is evicted first.
const OWNERS_CAP: usize = 4096;

/// Live detached jobs are never evicted: losing their scope would make thread
/// deletion unable to cancel and settle them safely.
const ACTIVE_DRIVER_TASKS_CAP: usize = 4096;

/// Terminal task owners are retained only to preserve late cancel-by-task
/// reporting. Evicting these cannot leave a child running.
const TERMINAL_DRIVER_TASKS_CAP: usize = 4096;

/// Bound on the profiles one (session) id can name.
const PROFILES_PER_ID: usize = 16;

#[derive(Default)]
struct Owners {
    by_id: HashMap<String, Vec<String>>,
    order: VecDeque<String>,
    driver_tasks: HashMap<String, DriverTaskOwner>,
    terminal_driver_tasks: HashMap<String, DriverTaskOwner>,
    terminal_driver_task_order: VecDeque<String>,
    driver_cancel_reasons: HashMap<String, String>,
    deleting_threads: HashMap<DriverThreadScope, ThreadDeletionFence>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct DriverThreadScope {
    profile: Option<String>,
    workspace_dir: PathBuf,
    thread_id: String,
}

#[derive(Default)]
struct ThreadDeletionFence {
    active_attempts: usize,
    committed: bool,
}

#[derive(Clone)]
pub(crate) struct DriverTaskOwner {
    pub(crate) task_id: String,
    pub(crate) profile: Option<String>,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) workspace: tinytools::WorkspaceDescriptor,
    pub(crate) parent_session: String,
    pub(crate) parent_thread_id: Option<String>,
}

fn owners() -> MutexGuard<'static, Owners> {
    static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
    OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn driver_task_key(ctx: &crate::core::runtime::DispatchCtx, task_id: &str) -> String {
    use crate::core::runtime::tenant;
    tenant::profile_key_in(
        &tenant::isolated_unless_tenant(ctx.tenant(), "driver_task_owner"),
        task_id,
    )
}

/// Remember the exact scope needed to resolve and control a live driver task.
/// The job registry owns the lifecycle; this bounded index recovers its
/// parent/session scope from the product-facing task id.
pub(crate) fn note_driver_task(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    workspace_dir: PathBuf,
    workspace: tinytools::WorkspaceDescriptor,
    parent_session: String,
    parent_thread_id: Option<String>,
) -> Result<(), String> {
    let owner = DriverTaskOwner {
        task_id: task_id.to_owned(),
        profile: ctx.tenant().ok().and_then(|tenant| tenant.profile),
        workspace_dir,
        workspace,
        parent_session,
        parent_thread_id,
    };
    let key = driver_task_key(ctx, task_id);
    let mut st = owners();
    remember_active_driver_task(&mut st, key, owner, ACTIVE_DRIVER_TASKS_CAP)
}

fn remember_active_driver_task(
    st: &mut Owners,
    key: String,
    owner: DriverTaskOwner,
    capacity: usize,
) -> Result<(), String> {
    if let Some(thread_id) = owner.parent_thread_id.as_deref() {
        let scope = driver_thread_scope(
            owner.profile.clone(),
            owner.workspace_dir.clone(),
            thread_id.to_owned(),
        );
        if st.deleting_threads.contains_key(&scope) {
            return Err("parent thread deletion is in progress".into());
        }
    }
    if !st.driver_tasks.contains_key(&key) && st.driver_tasks.len() >= capacity {
        return Err(format!(
            "active sub-agent owner capacity ({capacity}) is full"
        ));
    }
    st.terminal_driver_tasks.remove(&key);
    st.terminal_driver_task_order
        .retain(|existing| existing != &key);
    st.driver_tasks.insert(key, owner);
    Ok(())
}

fn driver_thread_scope(
    profile: Option<String>,
    workspace_dir: PathBuf,
    thread_id: String,
) -> DriverThreadScope {
    DriverThreadScope {
        profile,
        workspace_dir,
        thread_id,
    }
}

fn begin_thread_deletion_in(st: &mut Owners, scope: DriverThreadScope) {
    st.deleting_threads
        .entry(scope)
        .or_default()
        .active_attempts += 1;
}

fn commit_thread_deletion_in(st: &mut Owners, scope: DriverThreadScope) {
    let fence = st.deleting_threads.entry(scope).or_default();
    fence.committed = true;
    fence.active_attempts = fence.active_attempts.saturating_sub(1);
}

fn release_thread_deletion_in(st: &mut Owners, scope: &DriverThreadScope) {
    let Some(fence) = st.deleting_threads.get_mut(scope) else {
        return;
    };
    fence.active_attempts = fence.active_attempts.saturating_sub(1);
    if fence.active_attempts == 0 && !fence.committed {
        st.deleting_threads.remove(scope);
    }
}

/// Close owner registration before thread deletion starts its one-time child sweep.
pub(crate) fn begin_thread_deletion(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Result<(), String> {
    let profile = ctx
        .tenant()
        .map_err(|error| format!("cannot resolve profile for thread deletion: {error}"))?
        .profile;
    let mut st = owners();
    begin_thread_deletion_in(
        &mut st,
        driver_thread_scope(profile, workspace_dir.to_path_buf(), thread_id.to_owned()),
    );
    Ok(())
}

/// Mark that the durable deleted-parent marker was written successfully.
pub(crate) fn commit_thread_deletion(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Result<(), String> {
    let profile = ctx
        .tenant()
        .map_err(|error| format!("cannot resolve profile for thread deletion: {error}"))?
        .profile;
    let mut st = owners();
    commit_thread_deletion_in(
        &mut st,
        driver_thread_scope(profile, workspace_dir.to_path_buf(), thread_id.to_owned()),
    );
    Ok(())
}

/// Release a temporary deletion fence when the conversation delete itself failed.
pub(crate) fn release_thread_deletion(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) {
    let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
    let mut st = owners();
    release_thread_deletion_in(
        &mut st,
        &driver_thread_scope(profile, workspace_dir.to_path_buf(), thread_id.to_owned()),
    );
}

/// Move a settled task out of the non-evictable live-owner set while retaining
/// its scope for the bounded late cancel-by-task compatibility path.
pub(crate) fn finish_driver_task(ctx: &crate::core::runtime::DispatchCtx, task_id: &str) {
    let key = driver_task_key(ctx, task_id);
    let mut st = owners();
    terminalize_driver_task(&mut st, &key, TERMINAL_DRIVER_TASKS_CAP);
}

fn terminalize_driver_task(st: &mut Owners, key: &str, capacity: usize) {
    let Some(owner) = st.driver_tasks.remove(key) else {
        return;
    };
    st.terminal_driver_tasks.insert(key.to_owned(), owner);
    st.terminal_driver_task_order.push_back(key.to_owned());
    while st.terminal_driver_task_order.len() > capacity {
        if let Some(oldest) = st.terminal_driver_task_order.pop_front() {
            st.terminal_driver_tasks.remove(&oldest);
        }
    }
}

/// Return the exact parent scope cached when the current process registered
/// this task. A missing entry means the process has no live owner metadata.
pub(crate) fn driver_task_owner(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
) -> Option<DriverTaskOwner> {
    let st = owners();
    let key = driver_task_key(ctx, task_id);
    st.driver_tasks
        .get(&key)
        .or_else(|| st.terminal_driver_tasks.get(&key))
        .cloned()
}

pub(crate) fn note_driver_cancel_reason(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    reason: &str,
) {
    owners()
        .driver_cancel_reasons
        .insert(driver_task_key(ctx, task_id), reason.to_owned());
}

pub(crate) fn take_driver_cancel_reason(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
) -> Option<String> {
    owners()
        .driver_cancel_reasons
        .remove(&driver_task_key(ctx, task_id))
}

pub(crate) fn clear_driver_cancel_reason(ctx: &crate::core::runtime::DispatchCtx, task_id: &str) {
    owners()
        .driver_cancel_reasons
        .remove(&driver_task_key(ctx, task_id));
}

#[cfg(test)]
pub(crate) fn clear_driver_task_owner_for_test(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
) {
    let key = driver_task_key(ctx, task_id);
    let mut st = owners();
    st.driver_tasks.remove(&key);
    st.terminal_driver_tasks.remove(&key);
    st.terminal_driver_task_order
        .retain(|existing| existing != &key);
}

/// Return registered task scopes for one exact workspace/thread pair.
pub(crate) fn driver_tasks_for_thread(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Vec<(String, DriverTaskOwner)> {
    let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
    owners()
        .driver_tasks
        .values()
        .filter(|owner| {
            owner.profile == profile
                && owner.workspace_dir == workspace_dir
                && owner.parent_thread_id.as_deref() == Some(thread_id)
        })
        .map(|owner| (owner.task_id.clone(), owner.clone()))
        .collect()
}

#[cfg(test)]
#[path = "completion_owners_tests.rs"]
mod tests;

pub(crate) fn driver_tasks_for_parent(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    parent_session: &str,
) -> Vec<(String, DriverTaskOwner)> {
    let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
    owners()
        .driver_tasks
        .values()
        .filter(|owner| {
            owner.profile == profile
                && owner.workspace_dir == workspace_dir
                && owner.parent_session == parent_session
        })
        .map(|owner| (owner.task_id.clone(), owner.clone()))
        .collect()
}

/// Note the calling task's profile as an owner of each id. A no-op outside a
/// profile (the desktop, embedded agents).
pub(crate) fn note(ctx: &crate::core::runtime::DispatchCtx, ids: &[&str]) {
    let Ok(Some(profile)) = ctx.tenant().map(|t| t.profile) else {
        return;
    };
    note_profile(ids, &profile);
}

/// Note `profile` as an owner of each id.
pub(crate) fn note_profile(ids: &[&str], profile: &str) {
    let mut st = owners();
    for id in ids {
        let fresh = !st.by_id.contains_key(*id);
        let profiles = st.by_id.entry((*id).to_string()).or_default();
        if !profiles.iter().any(|p| p == profile) {
            if profiles.len() >= PROFILES_PER_ID {
                profiles.remove(0);
            }
            profiles.push(profile.to_string());
        }
        if fresh {
            st.order.push_back((*id).to_string());
        }
    }
    while st.order.len() > OWNERS_CAP {
        if let Some(oldest) = st.order.pop_front() {
            st.by_id.remove(&oldest);
        }
    }
}

/// The profiles noted as owners of `id`, oldest first.
pub(crate) fn profiles_of(id: &str) -> Vec<String> {
    owners().by_id.get(id).cloned().unwrap_or_default()
}

/// The context to act for `profile` under, off-task: the SaaS agent host's
/// agent for it, reopened if it was evicted. `None` outside SaaS or for a
/// profile the host does not know.
pub(crate) async fn context_for_profile(profile: &str) -> Option<Arc<CoreContext>> {
    let id = crate::profiles::ProfileId::parse(profile).ok()?;
    match crate::profiles::host::host()?.open(&id).await {
        Ok(state) => Some(Arc::clone(state.context())),
        Err(error) => {
            log::warn!("[completion_owners] cannot open the owning agent: {error}");
            None
        }
    }
}
