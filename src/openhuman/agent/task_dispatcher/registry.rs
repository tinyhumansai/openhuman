//! In-flight autonomous run registry.
//!
//! Tracks active runs by session `thread_id` so the web-channel cancel path
//! can abort them even though they are detached tokio tasks rather than
//! web-channel turns.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::types::ActiveRun;

static ACTIVE_RUNS: OnceLock<Mutex<HashMap<String, ActiveRun>>> = OnceLock::new();

pub(super) fn active_runs() -> &'static Mutex<HashMap<String, ActiveRun>> {
    ACTIVE_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn register_active_run(thread_id: String, run: ActiveRun) {
    active_runs()
        .lock()
        .expect("active_runs mutex poisoned")
        .insert(thread_id, run);
}

/// Remove and return the active-run entry for `thread_id`. The naturally
/// completing run and a concurrent [`cancel_session`] race on this — whoever
/// gets `Some` "owns" the terminal board write-back, so it happens exactly once.
pub(super) fn take_active_run(thread_id: &str) -> Option<ActiveRun> {
    active_runs()
        .lock()
        .expect("active_runs mutex poisoned")
        .remove(thread_id)
}

/// Atomically remove the active-run entry for `thread_id`, but only when it
/// matches `request_id` (when `Some`).
///
/// The match check and the removal happen under a single lock acquisition, so
/// there is no window in which the matched run could complete and be replaced
/// by a newer run before removal — the "stale cancel kills a newer turn" race a
/// separate peek-then-`take_active_run` would reopen (#4760). A `None`
/// `request_id` removes whatever run is on the thread (unscoped Stop /
/// teardown).
///
/// Both scoped no-op cases (no active run, or a `run_id` mismatch from a
/// superseded/unrelated request) emit grep-friendly `debug` diagnostics so an
/// intentional no-op cancel is still traceable.
pub(super) fn take_active_run_if(thread_id: &str, request_id: Option<&str>) -> Option<ActiveRun> {
    let mut guard = active_runs().lock().expect("active_runs mutex poisoned");
    if let Some(rid) = request_id {
        match guard.get(thread_id) {
            None => {
                tracing::debug!(
                    thread_id = %thread_id,
                    request_id = %rid,
                    "[task_dispatcher] scoped cancel ignored: no active run on thread"
                );
                return None;
            }
            Some(run) if run.run_id != rid => {
                tracing::debug!(
                    thread_id = %thread_id,
                    request_id = %rid,
                    active_run_id = %run.run_id,
                    "[task_dispatcher] scoped cancel ignored: run_id mismatch (superseded/unrelated request)"
                );
                return None;
            }
            _ => {}
        }
    }
    guard.remove(thread_id)
}

/// Cancel the in-flight autonomous run streaming into session `thread_id`.
///
/// Aborts the detached run task, stops its heartbeat, marks the card `blocked`
/// (user-cancelled) so it doesn't dangle `in_progress`, and emits the terminal
/// chat event (broadcast as `"system"`) so the session UI stops "processing".
/// Returns `true` if a run was found and cancelled. Wired into the web channel's
/// `channel_web_cancel` as the fallback when the thread has no web-channel turn.
pub async fn cancel_session(thread_id: &str) -> bool {
    let Some(run) = take_active_run(thread_id) else {
        return false;
    };
    cancel_taken_run(thread_id, run);
    true
}

/// Drive the cancellation side effects for a run that has **already** been
/// removed from the registry: abort the task, stop its heartbeat, write the
/// card back to a terminal state (the aborted task never reaches its own
/// write-back), and emit the terminal `chat_error` event.
///
/// Split out of [`cancel_session`] so [`cancel_session_scoped`] can cancel the
/// exact run it atomically removed via [`take_active_run_if`], rather than
/// re-acquiring the lock and racing a replacement run (#4760).
fn cancel_taken_run(thread_id: &str, run: ActiveRun) {
    run.abort.abort();
    let _ = run.hb_cancel.send(true);
    // The aborted task never reaches its own write-back — do it here so the
    // card lands in a terminal state instead of a stale `in_progress`.
    super::executor::write_back(
        &run.location,
        &run.card_id,
        &run.run_id,
        Err("Cancelled by user".to_string()),
    );
    crate::openhuman::channels::providers::web::publish_web_channel_event(
        crate::core::socketio::WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: "system".to_string(),
            thread_id: thread_id.to_string(),
            request_id: run.run_id.clone(),
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            ..Default::default()
        },
    );
    tracing::info!(
        thread_id = %thread_id,
        card_id = %run.card_id,
        run_id = %run.run_id,
        "[task_dispatcher] cancelled autonomous run via chat cancel"
    );
}

/// Request-scoped variant of [`cancel_session`].
///
/// When `request_id` is `Some`, the active run is aborted only if its `run_id`
/// matches — a scoped cancel for a superseded or unrelated request is a no-op so
/// it can't tear down a newer autonomous run on the thread (#4760). When
/// `request_id` is `None`, this behaves exactly like [`cancel_session`] (stop
/// whatever run is on the thread — the Stop button / session-teardown path).
/// Returns `true` if a run was found and cancelled.
pub async fn cancel_session_scoped(thread_id: &str, request_id: Option<&str>) -> bool {
    // Atomic match + remove: holding the lock across both closes the TOCTOU
    // window where the matched run could complete and be replaced by a newer run
    // before we remove it, which would cancel the *new* run — the exact #4760
    // bug this path guards against. `take_active_run_if` also logs the scoped
    // no-op cases (no active run / run_id mismatch).
    let Some(run) = take_active_run_if(thread_id, request_id) else {
        return false;
    };
    cancel_taken_run(thread_id, run);
    true
}
