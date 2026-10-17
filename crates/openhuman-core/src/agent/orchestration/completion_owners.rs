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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::core::runtime::CoreContext;

/// Bound on noted ids; the oldest is evicted first.
const OWNERS_CAP: usize = 4096;

/// Bound on the profiles one (session) id can name.
const PROFILES_PER_ID: usize = 16;

#[derive(Default)]
struct Owners {
    by_id: HashMap<String, Vec<String>>,
    order: VecDeque<String>,
}

fn owners() -> MutexGuard<'static, Owners> {
    static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
    OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
