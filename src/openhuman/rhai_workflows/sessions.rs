//! Session manager: a process-global, bounded map of persistent `.ragsh`
//! sessions.
//!
//! Sessions are keyed `<thread_id>:<session_id>` so parallel chats never share
//! a namespace. Each session is `Send` but `eval_cell` takes `&mut self`, so a
//! session runs **one cell at a time**, serialized by a per-session
//! [`std::sync::Mutex`]; a second concurrent call on a busy session sees a
//! `try_lock` failure and returns a typed "busy" error rather than queueing
//! (see [`super::ops`]). The map is bounded fail-closed: an idle-TTL sweep plus
//! an LRU cap keep the number of live namespaces finite.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use tinyagents::{ReplCallKind, ReplCancelFlag, ReplResult, ReplSession};

/// Maximum number of live sessions before the least-recently-used one is
/// evicted on the next access.
pub(super) const MAX_SESSIONS: usize = 16;

/// Idle time after which a session is evicted on the next access.
pub(super) const IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// A live session and its bookkeeping.
struct SessionSlot {
    /// The session, behind a `Mutex` so only one cell runs at a time; a busy
    /// `try_lock` maps to a typed "session busy" error.
    session: Arc<Mutex<ReplSession>>,
    /// The session's cancel flag (a clone of the one installed on the session),
    /// so a run-cancellation watcher can abort an in-flight cell.
    cancel: ReplCancelFlag,
    /// Last time the session was accessed, for idle-TTL and LRU eviction.
    last_access: Instant,
    /// Cells evaluated so far (for `cells_used`).
    cells: usize,
    /// Cumulative capability-call counts (for `limits_remaining`, since the
    /// crate does not expose a session's internal counters).
    model_calls: usize,
    tool_calls: usize,
    agent_calls: usize,
}

/// A handle to a resolved session: the shared session and its cancel flag, plus
/// whether it was newly created this call.
pub(super) struct SlotHandle {
    pub(super) session: Arc<Mutex<ReplSession>>,
    pub(super) cancel: ReplCancelFlag,
    pub(super) fresh: bool,
}

/// A snapshot of a session's cumulative usage after a cell, used to compute
/// `limits_remaining`.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct CellStats {
    pub(super) cells: usize,
    pub(super) model_calls: usize,
    pub(super) tool_calls: usize,
    pub(super) agent_calls: usize,
}

/// The process-global session manager.
pub(super) struct RhaiSessionManager {
    inner: Mutex<HashMap<String, SessionSlot>>,
}

static MANAGER: OnceLock<RhaiSessionManager> = OnceLock::new();

impl RhaiSessionManager {
    /// Returns the process-global manager, initialising it on first use.
    pub(super) fn global() -> &'static RhaiSessionManager {
        MANAGER.get_or_init(|| RhaiSessionManager {
            inner: Mutex::new(HashMap::new()),
        })
    }

    /// Composes the map key from the parent thread scope and the session id.
    pub(super) fn session_key(thread_scope: &str, session_id: &str) -> String {
        format!("{thread_scope}:{session_id}")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionSlot>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resolves the session for `key`, building a fresh one with `build` if
    /// absent. Runs an eviction sweep first so idle/over-cap sessions are
    /// reclaimed. The `build` closure must produce a session that already has
    /// its cancel flag installed (read back via `cancel_flag()`).
    pub(super) fn get_or_create(
        &self,
        key: &str,
        build: impl FnOnce() -> ReplSession,
    ) -> SlotHandle {
        let mut map = self.lock();
        Self::evict(&mut map, key);
        let now = Instant::now();
        if let Some(slot) = map.get_mut(key) {
            slot.last_access = now;
            tracing::debug!(
                session_key = key,
                "[rhai_workflows] reusing existing session"
            );
            return SlotHandle {
                session: slot.session.clone(),
                cancel: slot.cancel.clone(),
                fresh: false,
            };
        }
        let session = build();
        let cancel = session.cancel_flag();
        let session = Arc::new(Mutex::new(session));
        map.insert(
            key.to_string(),
            SessionSlot {
                session: session.clone(),
                cancel: cancel.clone(),
                last_access: now,
                cells: 0,
                model_calls: 0,
                tool_calls: 0,
                agent_calls: 0,
            },
        );
        tracing::debug!(
            session_key = key,
            live_sessions = map.len(),
            "[rhai_workflows] created new session"
        );
        SlotHandle {
            session,
            cancel,
            fresh: true,
        }
    }

    /// Records a completed cell against `key`: bumps the cell count, accumulates
    /// capability-call counts from `result`, and returns the cumulative
    /// snapshot. Returns `None` if the slot was evicted mid-cell.
    pub(super) fn finish_cell(&self, key: &str, result: &ReplResult) -> Option<CellStats> {
        let mut map = self.lock();
        let slot = map.get_mut(key)?;
        slot.cells += 1;
        slot.last_access = Instant::now();
        for call in &result.calls {
            match call.kind {
                ReplCallKind::Model => slot.model_calls += 1,
                ReplCallKind::Tool => slot.tool_calls += 1,
                ReplCallKind::Agent => slot.agent_calls += 1,
                ReplCallKind::Graph | ReplCallKind::Emit => {}
            }
        }
        Some(CellStats {
            cells: slot.cells,
            model_calls: slot.model_calls,
            tool_calls: slot.tool_calls,
            agent_calls: slot.agent_calls,
        })
    }

    /// Drops the session for `key` (explicit close, or on a poisoned/errored
    /// session that must never be reused).
    pub(super) fn close(&self, key: &str) {
        if self.lock().remove(key).is_some() {
            tracing::debug!(session_key = key, "[rhai_workflows] closed session");
        }
    }

    /// Evicts idle (past [`IDLE_TTL`]) sessions, then — if still at or above the
    /// [`MAX_SESSIONS`] cap and `incoming` is not already present — the
    /// least-recently-used session, so inserting `incoming` stays within cap.
    fn evict(map: &mut HashMap<String, SessionSlot>, incoming: &str) {
        let now = Instant::now();
        map.retain(|k, slot| {
            let keep = now.duration_since(slot.last_access) < IDLE_TTL;
            if !keep {
                tracing::debug!(session_key = %k, "[rhai_workflows] evicting idle session (TTL)");
            }
            keep
        });
        if map.contains_key(incoming) || map.len() < MAX_SESSIONS {
            return;
        }
        // Drop the least-recently-used slot to make room for the incoming one.
        if let Some(lru_key) = map
            .iter()
            .min_by_key(|(_, slot)| slot.last_access)
            .map(|(k, _)| k.clone())
        {
            map.remove(&lru_key);
            tracing::debug!(session_key = %lru_key, "[rhai_workflows] evicting LRU session (cap)");
        }
    }

    /// Number of live sessions (for tests/diagnostics).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lock().len()
    }

    /// A standalone (non-global) manager instance, so tests are isolated from
    /// the process-global singleton and from each other.
    #[cfg(test)]
    pub(super) fn new_for_test() -> Self {
        RhaiSessionManager {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyagents::{ReplPolicy, ReplSession, ReplValue};

    fn build_session() -> ReplSession {
        ReplSession::<()>::new()
    }

    #[test]
    fn namespace_persists_across_cells_in_one_session() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "s1");

        let handle = manager.get_or_create(&key, build_session);
        assert!(handle.fresh);
        handle
            .session
            .lock()
            .unwrap()
            .eval_cell("let n = 7;")
            .expect("cell 1");

        // Reusing the same key returns the same (non-fresh) session, so the
        // binding is still visible.
        let handle = manager.get_or_create(&key, build_session);
        assert!(!handle.fresh);
        let result = handle
            .session
            .lock()
            .unwrap()
            .eval_cell("n + 1")
            .expect("cell 2");
        assert_eq!(result.value, Some(ReplValue::Int(8)));
    }

    #[test]
    fn distinct_session_keys_are_isolated() {
        let manager = RhaiSessionManager::new_for_test();
        let a = manager.get_or_create(&RhaiSessionManager::session_key("t", "a"), build_session);
        a.session.lock().unwrap().eval_cell("let x = 1;").unwrap();

        // A different key has its own namespace, so `x` is undefined there.
        let b = manager.get_or_create(&RhaiSessionManager::session_key("t", "b"), build_session);
        assert!(b.session.lock().unwrap().eval_cell("x").is_err());
    }

    #[test]
    fn thread_scope_isolates_the_same_session_id() {
        let manager = RhaiSessionManager::new_for_test();
        let k1 = RhaiSessionManager::session_key("thread-1", "shared");
        let k2 = RhaiSessionManager::session_key("thread-2", "shared");
        assert_ne!(k1, k2);
        manager.get_or_create(&k1, build_session);
        let h2 = manager.get_or_create(&k2, build_session);
        assert!(
            h2.fresh,
            "same session_id under a different thread is a fresh namespace"
        );
    }

    #[test]
    fn lru_cap_bounds_the_number_of_live_sessions() {
        let manager = RhaiSessionManager::new_for_test();
        for i in 0..(MAX_SESSIONS + 5) {
            manager.get_or_create(
                &RhaiSessionManager::session_key("t", &format!("s{i}")),
                build_session,
            );
        }
        assert!(
            manager.len() <= MAX_SESSIONS,
            "live sessions {} exceeded the cap {MAX_SESSIONS}",
            manager.len()
        );
    }

    #[test]
    fn close_drops_the_session() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "closeme");
        manager.get_or_create(&key, build_session);
        assert_eq!(manager.len(), 1);
        manager.close(&key);
        assert_eq!(manager.len(), 0);
        // Re-creating after close is a fresh session.
        assert!(manager.get_or_create(&key, build_session).fresh);
    }

    #[test]
    fn finish_cell_accumulates_and_bounds_are_reported() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "stats");
        let policy = ReplPolicy::default();
        let handle = manager.get_or_create(&key, || {
            ReplSession::<()>::new().with_policy(policy.clone())
        });
        let result = handle
            .session
            .lock()
            .unwrap()
            .eval_cell("emit(\"hi\"); 1")
            .expect("cell");
        let stats = manager.finish_cell(&key, &result).expect("stats");
        assert_eq!(stats.cells, 1);
        // `emit` is not a model/tool/agent call, so those stay zero.
        assert_eq!(stats.tool_calls, 0);
        assert_eq!(stats.model_calls, 0);
    }
}
