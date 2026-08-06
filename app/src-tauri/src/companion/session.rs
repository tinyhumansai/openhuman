//! Companion session lifecycle and state machine (shell-side).
//!
//! A companion session represents a single period of desktop companion
//! activity. It owns the state machine (idle → listening → thinking →
//! speaking → idle), TTL enforcement, and conversation history.
//!
//! Only one session may be active at a time. Sessions are created with
//! explicit user consent and can be stopped manually or via TTL expiry.
//!
//! Migrated from the former core `desktop_companion::session`. The only
//! behavioural change: state transitions emit the `companion://state_changed`
//! Tauri event (`super::events`) instead of the old Socket.IO broadcast bus,
//! and the core `event_bus` session-lifecycle publishes are dropped (the
//! frontend now derives session-active purely from state transitions).

use log::{debug, info, warn};
use parking_lot::Mutex;

use super::events;
use super::types::*;

const LOG_PREFIX: &str = "[companion]";

/// Maximum number of conversation turns retained per session.
const MAX_CONVERSATION_TURNS: usize = 50;

/// Process-global singleton for the active companion session.
/// The `Mutex` serializes all session operations — no separate lock needed.
static ACTIVE_SESSION: Mutex<Option<CompanionSessionInner>> = Mutex::new(None);

/// Internal mutable session state (not serialized directly).
struct CompanionSessionInner {
    id: String,
    state: CompanionState,
    started_at_ms: i64,
    expires_at_ms: Option<i64>,
    #[allow(dead_code)]
    ttl_secs: u64,
    conversation: Vec<ConversationTurn>,
    last_error: Option<String>,
}

/// Start a new companion session.
///
/// Returns an error if consent is not granted or a session is already active.
pub fn start_session(
    params: &StartCompanionSessionParams,
) -> Result<StartCompanionSessionResult, String> {
    if !params.consent {
        warn!("{LOG_PREFIX} start_session denied — consent=false");
        return Err("user consent is required to start a companion session".into());
    }

    let mut guard = ACTIVE_SESSION.lock();
    if let Some(ref inner) = *guard {
        // Auto-expire stale sessions so callers don't have to poll status() first.
        let now_ms = chrono::Utc::now().timestamp_millis();
        let expired = inner
            .expires_at_ms
            .map(|exp| now_ms >= exp)
            .unwrap_or(false);
        if expired {
            let stale_id = inner.id.clone();
            info!("{LOG_PREFIX} auto-expiring stale session id={stale_id} during start_session");
            let _ = guard.take();
        } else {
            return Err("a companion session is already active — stop it first".into());
        }
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let ttl_secs = params
        .ttl_secs
        .unwrap_or(CompanionConfig::default().ttl_secs);
    // Guard against overflow: cap so that now_ms + ttl_ms never exceeds i64::MAX.
    let max_ttl_ms = (i64::MAX - now_ms) as u64;
    let ttl_ms = ttl_secs.saturating_mul(1000).min(max_ttl_ms);
    let expires_at_ms = if ttl_secs > 0 {
        Some(now_ms + ttl_ms as i64)
    } else {
        None
    };
    let session_id = uuid::Uuid::new_v4().to_string();

    info!(
        "{LOG_PREFIX} starting session id={} ttl_secs={} expires_at_ms={:?}",
        session_id, ttl_secs, expires_at_ms
    );

    let inner = CompanionSessionInner {
        id: session_id.clone(),
        state: CompanionState::Idle,
        started_at_ms: now_ms,
        expires_at_ms,
        ttl_secs,
        conversation: Vec::new(),
        last_error: None,
    };
    *guard = Some(inner);
    drop(guard);

    Ok(StartCompanionSessionResult {
        session_id,
        state: CompanionState::Idle,
        expires_at_ms,
    })
}

/// Stop the active companion session.
pub fn stop_session(
    params: &StopCompanionSessionParams,
) -> Result<StopCompanionSessionResult, String> {
    let mut guard = ACTIVE_SESSION.lock();
    match guard.take() {
        Some(inner) => {
            let reason = params
                .reason
                .clone()
                .unwrap_or_else(|| "user_requested".into());
            let session_id = inner.id.clone();
            let turn_count = inner.conversation.len();
            info!(
                "{LOG_PREFIX} stopping session id={session_id} reason={reason} turns={turn_count}",
            );
            let previous = inner.state;
            drop(guard);
            if previous != CompanionState::Idle {
                events::emit_state_changed(&session_id, CompanionState::Idle, previous);
            }

            Ok(StopCompanionSessionResult {
                stopped: true,
                reason: Some(reason),
            })
        }
        None => {
            debug!("{LOG_PREFIX} stop_session called with no active session");
            Ok(StopCompanionSessionResult {
                stopped: false,
                reason: Some("no active session".into()),
            })
        }
    }
}

/// Return whether the active session has exceeded its TTL.
pub fn session_is_expired() -> bool {
    let guard = ACTIVE_SESSION.lock();
    guard
        .as_ref()
        .and_then(|inner| inner.expires_at_ms)
        .is_some_and(|expires_at_ms| chrono::Utc::now().timestamp_millis() >= expires_at_ms)
}

/// Atomically remove the active session only when its TTL has elapsed.
pub fn expire_session_if_needed() -> bool {
    let mut guard = ACTIVE_SESSION.lock();
    let Some(inner) = guard.as_ref() else {
        return false;
    };
    let expired = inner
        .expires_at_ms
        .is_some_and(|expires_at_ms| chrono::Utc::now().timestamp_millis() >= expires_at_ms);
    if !expired {
        return false;
    }

    let stale = guard.take().expect("active session checked above");
    let session_id = stale.id.clone();
    let previous = stale.state;
    let turn_count = stale.conversation.len();
    drop(guard);
    info!("{LOG_PREFIX} auto-expiring stale session id={session_id} turns={turn_count}");
    if previous != CompanionState::Idle {
        events::emit_state_changed(&session_id, CompanionState::Idle, previous);
    }
    true
}

/// Get the current session status.
pub fn session_status() -> CompanionSessionStatus {
    let guard = ACTIVE_SESSION.lock();
    match guard.as_ref() {
        Some(inner) => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let remaining_ms = inner.expires_at_ms.map(|exp| (exp - now_ms).max(0));

            CompanionSessionStatus {
                active: true,
                state: inner.state,
                session_id: Some(inner.id.clone()),
                started_at_ms: Some(inner.started_at_ms),
                expires_at_ms: inner.expires_at_ms,
                remaining_ms,
                turn_count: inner.conversation.len(),
                last_error: inner.last_error.clone(),
            }
        }
        None => CompanionSessionStatus {
            active: false,
            state: CompanionState::Idle,
            session_id: None,
            started_at_ms: None,
            expires_at_ms: None,
            remaining_ms: None,
            turn_count: 0,
            last_error: None,
        },
    }
}

/// Transition the companion to a new state.
///
/// Returns the previous state, or an error if no session is active or the
/// transition is invalid. Emits `companion://state_changed` to the frontend on
/// success.
pub fn transition_state(
    new_state: CompanionState,
    message: Option<String>,
) -> Result<CompanionState, String> {
    let mut guard = ACTIVE_SESSION.lock();
    let inner = guard.as_mut().ok_or_else(|| {
        warn!("{LOG_PREFIX} transition_state called with no active session target={new_state}");
        "no active companion session".to_string()
    })?;

    let previous = inner.state;

    // Validate transitions.
    if !is_valid_transition(previous, new_state) {
        warn!(
            "{LOG_PREFIX} rejected state transition: {} -> {} session={}",
            previous, new_state, inner.id
        );
        return Err(format!(
            "invalid companion state transition: {} -> {}",
            previous, new_state
        ));
    }

    debug!(
        "{LOG_PREFIX} state transition: {} -> {} session={}",
        previous, new_state, inner.id
    );

    inner.state = new_state;

    if new_state == CompanionState::Error {
        inner.last_error = message.clone();
    }

    let session_id = inner.id.clone();
    drop(guard);

    events::emit_state_changed(&session_id, new_state, previous);

    Ok(previous)
}

/// Record a failed turn and return the active session to `Idle` so the next
/// activation can start a fresh capture.
pub fn recover_after_turn_error(message: String) {
    let status = session_status();
    if !status.active {
        return;
    }
    if status.state != CompanionState::Error {
        let _ = transition_state(CompanionState::Error, Some(message));
    }
    let _ = transition_state(CompanionState::Idle, None);
}

/// Finish a turn without clobbering a newly-started interrupting capture.
///
/// The state check and update share the session lock, closing the race where an
/// old cancelled task could observe `Thinking`, a new capture could enter
/// `Listening`, and the old task could then reset that fresh capture to `Idle`.
pub fn finish_turn() {
    let mut guard = ACTIVE_SESSION.lock();
    let Some(inner) = guard.as_mut() else {
        return;
    };
    if matches!(
        inner.state,
        CompanionState::Idle | CompanionState::Listening
    ) {
        return;
    }
    let previous = inner.state;
    if !is_valid_transition(previous, CompanionState::Idle) {
        return;
    }
    inner.state = CompanionState::Idle;
    let session_id = inner.id.clone();
    drop(guard);
    events::emit_state_changed(&session_id, CompanionState::Idle, previous);
}

/// Return a pre-STT/empty-transcript turn from `Listening` to `Idle`.
///
/// The caller must ensure no newer microphone capture owns `Listening`.
pub fn finish_listening_turn() {
    let mut guard = ACTIVE_SESSION.lock();
    let Some(inner) = guard.as_mut() else {
        return;
    };
    if inner.state != CompanionState::Listening {
        return;
    }
    inner.state = CompanionState::Idle;
    let session_id = inner.id.clone();
    drop(guard);
    events::emit_state_changed(&session_id, CompanionState::Idle, CompanionState::Listening);
}

/// Add a conversation turn to the session history.
pub fn push_conversation_turn(turn: ConversationTurn) -> Result<(), String> {
    let mut guard = ACTIVE_SESSION.lock();
    let inner = guard.as_mut().ok_or("no active companion session")?;

    inner.conversation.push(turn);

    // Cap the history to prevent unbounded growth.
    if inner.conversation.len() > MAX_CONVERSATION_TURNS {
        let drain_count = inner.conversation.len() - MAX_CONVERSATION_TURNS;
        inner.conversation.drain(..drain_count);
    }

    Ok(())
}

/// Get a snapshot of the conversation history for LLM context.
pub fn conversation_history() -> Vec<ConversationTurn> {
    let guard = ACTIVE_SESSION.lock();
    match guard.as_ref() {
        Some(inner) => inner.conversation.clone(),
        None => Vec::new(),
    }
}

/// Check whether a state transition is valid.
///
/// Valid transitions:
/// - Idle → Listening (activation)
/// - Listening → Thinking (transcript received)
/// - Listening → Idle (cancelled / released)
/// - Thinking → Speaking (response ready)
/// - Thinking → Idle (cancelled)
/// - Speaking → Idle (TTS done)
/// - Speaking → Listening (interrupted — new turn)
/// - Error → Idle (reset)
/// - Any → Error (error from any state)
fn is_valid_transition(from: CompanionState, to: CompanionState) -> bool {
    // Any state can transition to Error.
    if to == CompanionState::Error {
        return true;
    }

    matches!(
        (from, to),
        (CompanionState::Idle, CompanionState::Listening)
            | (CompanionState::Listening, CompanionState::Thinking)
            | (CompanionState::Listening, CompanionState::Idle)
            | (CompanionState::Thinking, CompanionState::Speaking)
            | (CompanionState::Thinking, CompanionState::Idle)
            | (CompanionState::Speaking, CompanionState::Idle)
            | (CompanionState::Speaking, CompanionState::Listening)
            | (CompanionState::Error, CompanionState::Idle)
    )
}

/// Reset the global session state. Used only in tests.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    let mut guard = ACTIVE_SESSION.lock();
    *guard = None;
}

/// Process-wide serialization lock for tests that mutate the global
/// `ACTIVE_SESSION`. Both `session_tests` and `pipeline_tests` exercise the
/// same process-global state; without a *shared* lock each module's local
/// mutex only serializes its own tests, letting a reset/transition in one
/// module race a transition in the other (e.g. observing `Idle` mid-turn and
/// rejecting `Idle -> Thinking`). Lock this in every test that touches session
/// state so the whole binary serializes those tests.
#[cfg(test)]
pub(crate) static TEST_STATE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the shared session-state test lock, recovering from poisoning so a
/// panicking test doesn't cascade into spurious failures in the rest.
#[cfg(test)]
pub(crate) fn lock_test_state() -> std::sync::MutexGuard<'static, ()> {
    TEST_STATE_MUTEX.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
