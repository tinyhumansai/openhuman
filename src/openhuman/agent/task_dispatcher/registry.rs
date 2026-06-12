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
    true
}
