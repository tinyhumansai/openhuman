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

use tinyagents::{ReplCallKind, ReplCancelFlag, ReplPolicy, ReplResult, ReplSession};

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
    /// The [`ReplPolicy`] the session was actually built with. `ReplSession`
    /// only exposes builder-style `with_policy(mut self) -> Self` (no
    /// re-policy after construction — vendor `session/mod.rs`), so a reused
    /// session cannot adopt a newly-resolved policy; this is the durable
    /// record of the one that is actually live, read back on every reuse
    /// (E-M2) instead of trusting whatever policy the caller happened to
    /// resolve for *this* call.
    policy: ReplPolicy,
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

/// A handle to a resolved session: the shared session and its cancel flag,
/// the policy actually live on the session, and whether it was newly created
/// this call.
pub(super) struct SlotHandle {
    pub(super) session: Arc<Mutex<ReplSession>>,
    pub(super) cancel: ReplCancelFlag,
    /// The session's *live* policy — on a fresh session this is exactly what
    /// was requested; on a reused session it is the one the session was
    /// originally built with, which may differ from what this call resolved
    /// (E-M2). Callers must compute any per-call bound (outer backstop,
    /// `limits_remaining`) from this field, never from a freshly-resolved
    /// policy.
    pub(super) policy: ReplPolicy,
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
    /// reclaimed. The `build` closure receives `policy` and must produce a
    /// session that already has its cancel flag installed (read back via
    /// `cancel_flag()`).
    ///
    /// `policy` is only ever used to build a **fresh** session. A reused
    /// session keeps the policy it was actually built with (`ReplSession`
    /// exposes no re-policy operation — see [`SlotHandle::policy`]); if the
    /// caller's newly-resolved `policy` differs from the live one, this logs
    /// a warning and returns the session's own policy rather than silently
    /// applying the mismatch to bounds the session was never built to honor
    /// (E-M2).
    pub(super) fn get_or_create(
        &self,
        key: &str,
        policy: ReplPolicy,
        build: impl FnOnce(ReplPolicy) -> ReplSession,
    ) -> SlotHandle {
        let mut map = self.lock();
        Self::evict(&mut map, key);
        let now = Instant::now();
        if let Some(slot) = map.get_mut(key) {
            slot.last_access = now;
            if slot.policy != policy {
                tracing::warn!(
                    session_key = key,
                    "[rhai_workflows] reused session's requested policy differs from its live \
                     policy; keeping the session's original policy (bindings preserved) rather \
                     than the newly-requested one"
                );
            }
            tracing::debug!(
                session_key = key,
                "[rhai_workflows] reusing existing session"
            );
            return SlotHandle {
                session: slot.session.clone(),
                cancel: slot.cancel.clone(),
                policy: slot.policy.clone(),
                fresh: false,
            };
        }
        let session = build(policy.clone());
        let cancel = session.cancel_flag();
        let session = Arc::new(Mutex::new(session));
        map.insert(
            key.to_string(),
            SessionSlot {
                session: session.clone(),
                cancel: cancel.clone(),
                policy: policy.clone(),
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
            policy,
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
    ///
    /// A slot whose cell is still in flight (its session `Mutex` is currently
    /// held by the `spawn_blocking` task running that cell — see `ops.rs`) is
    /// never a candidate for either sweep: evicting it would drop the session
    /// out from under the running cell, so `finish_cell` would return `None`
    /// (phantom-fresh `limits_remaining`) and, worse, a same-key call arriving
    /// while the cell is still running would build a *new* session against the
    /// same key that the evicted cell's completion would then be scored
    /// against (E-m6). Skipping busy slots means the cap/TTL can be exceeded
    /// while every live slot is genuinely in flight — a bounded, logged
    /// overshoot is preferable to corrupting an in-flight run.
    fn evict(map: &mut HashMap<String, SessionSlot>, incoming: &str) {
        let now = Instant::now();
        map.retain(|k, slot| {
            if slot.session.try_lock().is_err() {
                // In flight — never evict, even past the idle TTL.
                return true;
            }
            let keep = now.duration_since(slot.last_access) < IDLE_TTL;
            if !keep {
                tracing::debug!(session_key = %k, "[rhai_workflows] evicting idle session (TTL)");
            }
            keep
        });
        if map.contains_key(incoming) || map.len() < MAX_SESSIONS {
            return;
        }
        // Drop the least-recently-used *idle* slot to make room for the
        // incoming one; a busy (in-flight) slot is never a candidate.
        let lru_key = map
            .iter()
            .filter(|(_, slot)| slot.session.try_lock().is_ok())
            .min_by_key(|(_, slot)| slot.last_access)
            .map(|(k, _)| k.clone());
        match lru_key {
            Some(lru_key) => {
                map.remove(&lru_key);
                tracing::debug!(session_key = %lru_key, "[rhai_workflows] evicting LRU session (cap)");
            }
            None => {
                tracing::warn!(
                    live_sessions = map.len(),
                    "[rhai_workflows] session cap reached but every session is in flight — \
                     admitting one over cap rather than evicting a running cell"
                );
            }
        }
    }

    /// Number of live sessions (for tests/diagnostics).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether a session is currently live under `key` (for tests).
    #[cfg(test)]
    pub(super) fn contains(&self, key: &str) -> bool {
        self.lock().contains_key(key)
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

    fn build_session(policy: ReplPolicy) -> ReplSession {
        ReplSession::<()>::new().with_policy(policy)
    }

    #[test]
    fn namespace_persists_across_cells_in_one_session() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "s1");

        let handle = manager.get_or_create(&key, ReplPolicy::default(), build_session);
        assert!(handle.fresh);
        handle
            .session
            .lock()
            .unwrap()
            .eval_cell("let n = 7;")
            .expect("cell 1");

        // Reusing the same key returns the same (non-fresh) session, so the
        // binding is still visible.
        let handle = manager.get_or_create(&key, ReplPolicy::default(), build_session);
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
        let a = manager.get_or_create(
            &RhaiSessionManager::session_key("t", "a"),
            ReplPolicy::default(),
            build_session,
        );
        a.session.lock().unwrap().eval_cell("let x = 1;").unwrap();

        // A different key has its own namespace, so `x` is undefined there.
        let b = manager.get_or_create(
            &RhaiSessionManager::session_key("t", "b"),
            ReplPolicy::default(),
            build_session,
        );
        assert!(b.session.lock().unwrap().eval_cell("x").is_err());
    }

    #[test]
    fn thread_scope_isolates_the_same_session_id() {
        let manager = RhaiSessionManager::new_for_test();
        let k1 = RhaiSessionManager::session_key("thread-1", "shared");
        let k2 = RhaiSessionManager::session_key("thread-2", "shared");
        assert_ne!(k1, k2);
        manager.get_or_create(&k1, ReplPolicy::default(), build_session);
        let h2 = manager.get_or_create(&k2, ReplPolicy::default(), build_session);
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
                ReplPolicy::default(),
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
        manager.get_or_create(&key, ReplPolicy::default(), build_session);
        assert_eq!(manager.len(), 1);
        manager.close(&key);
        assert_eq!(manager.len(), 0);
        // Re-creating after close is a fresh session.
        assert!(
            manager
                .get_or_create(&key, ReplPolicy::default(), build_session)
                .fresh
        );
    }

    #[test]
    fn finish_cell_accumulates_and_bounds_are_reported() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "stats");
        let policy = ReplPolicy::default();
        let handle = manager.get_or_create(&key, policy.clone(), build_session);
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

    /// E-M2: `ReplSession` cannot be re-policied after construction, so a
    /// reused session must keep the policy it was actually built with — not
    /// silently adopt whatever a later caller resolves for that call.
    #[test]
    fn reused_session_keeps_its_own_stored_policy() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "s1");

        let original = ReplPolicy {
            max_tool_calls: 5,
            timeout: Some(Duration::from_secs(300)),
            ..ReplPolicy::default()
        };
        let handle = manager.get_or_create(&key, original.clone(), build_session);
        assert!(handle.fresh);
        assert_eq!(handle.policy, original);

        // A later call on the same key resolves a very different policy
        // (e.g. a cell passing `timeout_secs: 30` and a tighter tool-call
        // budget) — the session must keep serving the original one.
        let different = ReplPolicy {
            max_tool_calls: 999,
            timeout: Some(Duration::from_secs(30)),
            ..ReplPolicy::default()
        };
        let handle2 = manager.get_or_create(&key, different, build_session);
        assert!(!handle2.fresh);
        assert_eq!(
            handle2.policy, original,
            "reused session must report its own live policy, not the newly requested one"
        );
    }

    /// E-m6: an in-flight cell's slot (its session `Mutex` currently held)
    /// must never be chosen as the LRU eviction candidate — evicting it would
    /// drop the session out from under the running cell.
    #[test]
    fn lru_eviction_skips_an_in_flight_cell() {
        let manager = RhaiSessionManager::new_for_test();
        let policy = ReplPolicy::default();

        let mut first_handle = None;
        for i in 0..MAX_SESSIONS {
            let handle = manager.get_or_create(
                &RhaiSessionManager::session_key("t", &format!("s{i}")),
                policy.clone(),
                build_session,
            );
            if i == 0 {
                first_handle = Some(handle);
            }
        }
        let handle = first_handle.expect("first handle recorded");
        let s0_key = RhaiSessionManager::session_key("t", "s0");
        let s1_key = RhaiSessionManager::session_key("t", "s1");

        // `s0` is the least-recently-used slot (created first). Hold its
        // session lock to simulate an in-flight cell.
        let _held = handle.session.lock().unwrap();

        // One more insert pushes past the cap; eviction must skip the busy
        // `s0` slot and remove the next-LRU idle slot (`s1`) instead.
        manager.get_or_create(
            &RhaiSessionManager::session_key("t", "new"),
            policy.clone(),
            build_session,
        );

        assert!(
            manager.contains(&s0_key),
            "in-flight session must not be evicted"
        );
        assert!(
            !manager.contains(&s1_key),
            "the next-LRU idle session should have been evicted instead"
        );
        assert!(manager.contains(&RhaiSessionManager::session_key("t", "new")));
        assert_eq!(
            manager.len(),
            MAX_SESSIONS,
            "cap is still honored by evicting an idle slot instead"
        );
    }

    /// When every live slot is in flight, eviction admits one over cap rather
    /// than corrupting a running cell.
    #[test]
    fn lru_eviction_admits_over_cap_when_every_slot_is_busy() {
        let manager = RhaiSessionManager::new_for_test();
        let policy = ReplPolicy::default();
        // Keep the `Arc<Mutex<_>>`s alive in `sessions` so the guards taken
        // from them below can outlive each loop iteration.
        let mut sessions = Vec::new();
        for i in 0..MAX_SESSIONS {
            let handle = manager.get_or_create(
                &RhaiSessionManager::session_key("t", &format!("s{i}")),
                policy.clone(),
                build_session,
            );
            sessions.push(handle.session);
        }
        let _guards: Vec<_> = sessions.iter().map(|s| s.lock().unwrap()).collect();

        manager.get_or_create(
            &RhaiSessionManager::session_key("t", "new"),
            policy,
            build_session,
        );

        assert_eq!(
            manager.len(),
            MAX_SESSIONS + 1,
            "admits one over cap when nothing idle can be evicted"
        );
    }
}
