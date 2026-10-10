//! In-flight user turns, tracked per profile so background delivery defers
//! while one runs. See [`TurnBusy`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use super::background_completions;
use crate::core::runtime::tenant::{self, Tenant};

/// Sessions with a user turn in flight — delivery defers while busy. Keyed by
/// [`tenant::profile_key`]: two SaaS profiles can share a session id (desktop: bare id).
///
/// Each key holds the ids of the live [`TurnBusy`] guards on it, so a replacement
/// turn that starts before the one it interrupted has unwound stays busy when
/// the older guard drops.
pub(super) fn busy() -> &'static Mutex<HashMap<String, HashSet<u64>>> {
    static BUSY: OnceLock<Mutex<HashMap<String, HashSet<u64>>>> = OnceLock::new();
    BUSY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The calling task's profile, as the tenant its busy/slot keys carry.
pub(super) fn caller() -> Tenant {
    tenant::current_tenant_or_isolated("background_delivery").profile_only()
}

/// The session id of a busy key, when the key belongs to `me`.
pub(super) fn session_of<'k>(key: &'k str, me: &Tenant) -> Option<&'k str> {
    tenant::split_key(key)
        .filter(|(owner, _)| owner == me)
        .map(|(_, session)| session)
}

/// A user turn in flight on a session for the calling profile, busy until dropped.
/// The turn loop takes it in scope (the bus subscriber runs off-task and cannot
/// tell whose session it is); the key is fixed, so a cancelled turn clears it.
pub(crate) struct TurnBusy {
    pub(super) key: String,
    id: u64,
}

impl TurnBusy {
    pub(crate) fn start(session_id: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let key = tenant::profile_key(session_id);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        busy()
            .lock()
            .expect("background_delivery busy poisoned")
            .entry(key.clone())
            .or_default()
            .insert(id);
        Self { key, id }
    }

    /// A turn in flight on `thread_id` itself, for a turn whose session id does
    /// not map to its thread: a relayed channel turn runs under the channel
    /// pipeline's own session id, which [`background_completions`] never
    /// learns, so marking that session would leave the thread looking idle.
    pub(crate) fn start_on_thread(thread_id: &str) -> Self {
        Self::start(&format!("{THREAD_MARK}{thread_id}"))
    }
}

/// Prefix of a busy session id that names its thread directly
/// ([`TurnBusy::start_on_thread`]). Not a character a session id carries.
const THREAD_MARK: char = '\u{2}';

/// The thread a busy session id is a turn on.
fn thread_of(session: &str) -> Option<String> {
    match session.strip_prefix(THREAD_MARK) {
        Some(thread) => Some(thread.to_string()),
        None => background_completions::thread_for_session(session),
    }
}

impl Drop for TurnBusy {
    fn drop(&mut self) {
        if let Ok(mut busy) = busy().lock() {
            if let Some(guards) = busy.get_mut(&self.key) {
                guards.remove(&self.id);
                if guards.is_empty() {
                    busy.remove(&self.key);
                }
            }
        }
    }
}

/// Is any in-flight turn of the calling profile running on `thread_id`?
pub(crate) fn is_busy(thread_id: &str) -> bool {
    let me = caller();
    busy()
        .lock()
        .expect("background_delivery busy poisoned")
        .keys()
        .filter_map(|key| session_of(key, &me))
        .any(|session| thread_of(session).as_deref() == Some(thread_id))
}

/// Forget every in-flight turn of the calling profile on `thread_id`. A turn
/// that is cancelled cooperatively (Stop) can end without `AgentTurnCompleted`
/// or `AgentError`, and its session would otherwise stay "busy" and defer this
/// thread's deliveries until restart. Another profile's turn on a thread with
/// the same id is left alone. Returns how many sessions were cleared.
pub(crate) fn clear_busy_for_thread(thread_id: &str) -> usize {
    let me = caller();
    let mut busy = busy().lock().expect("background_delivery busy poisoned");
    let before = busy.len();
    busy.retain(|key, _| {
        session_of(key, &me).is_none_or(|session| thread_of(session).as_deref() != Some(thread_id))
    });
    let cleared = before - busy.len();
    if cleared > 0 {
        log::debug!(
            "[background_delivery] cleared {cleared} stale busy session(s) thread_id={thread_id}"
        );
    }
    cleared
}
