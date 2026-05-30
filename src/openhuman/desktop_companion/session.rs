//! Companion session lifecycle and state machine.
//!
//! A companion session represents a single period of desktop companion
//! activity. It owns the state machine (idle → listening → thinking →
//! speaking → pointing → idle), TTL enforcement, and conversation history.
//!
//! Only one session may be active at a time. Sessions are created with
//! explicit user consent and can be stopped manually or via TTL expiry.

use log::{debug, info, warn};
use parking_lot::Mutex;

use super::bus;
use super::types::*;

const LOG_PREFIX: &str = "[desktop_companion]";

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

    // Publish session-started event for Socket.IO bridge / subscribers.
    let _ = crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::CompanionSessionStarted {
            session_id: session_id.clone(),
            ttl_secs,
        },
    );

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
            drop(guard);

            let _ = crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::CompanionSessionEnded {
                    session_id,
                    reason: reason.clone(),
                    turn_count,
                },
            );

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

/// Get the current session status.
pub fn session_status() -> CompanionSessionStatus {
    let mut guard = ACTIVE_SESSION.lock();
    match guard.as_ref() {
        Some(inner) => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let remaining_ms = inner.expires_at_ms.map(|exp| (exp - now_ms).max(0));

            // Auto-expire if TTL exceeded.
            // Clear inline (guard.take) instead of calling stop_session() to
            // avoid a TOCTOU race where another thread starts a new session
            // between drop(guard) and the stop_session() call.
            if let Some(remaining) = remaining_ms {
                if remaining == 0 {
                    let stale = guard.take().expect("checked is_some");
                    let stale_id = stale.id.clone();
                    let turn_count = stale.conversation.len();
                    drop(stale);
                    drop(guard);
                    info!(
                        "{LOG_PREFIX} auto-expiring stale session id={stale_id} turns={turn_count}"
                    );
                    let _ = crate::core::event_bus::publish_global(
                        crate::core::event_bus::DomainEvent::CompanionSessionEnded {
                            session_id: stale_id,
                            reason: "ttl_expired".into(),
                            turn_count,
                        },
                    );
                    return CompanionSessionStatus {
                        active: false,
                        state: CompanionState::Idle,
                        session_id: None,
                        started_at_ms: None,
                        expires_at_ms: None,
                        remaining_ms: None,
                        turn_count: 0,
                        last_error: Some("session expired".into()),
                    };
                }
            }

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
/// transition is invalid.
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

    // Publish the state change event.
    let session_id = inner.id.clone();
    drop(guard);

    bus::publish_state_changed(CompanionStateChangedEvent {
        session_id,
        state: new_state,
        previous_state: previous,
        message,
    });

    Ok(previous)
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
/// - Thinking → Pointing (response has point targets, no TTS)
/// - Thinking → Idle (cancelled)
/// - Speaking → Pointing (TTS done, point targets present)
/// - Speaking → Idle (TTS done, no pointing)
/// - Speaking → Listening (interrupted — new turn)
/// - Pointing → Idle (animation done)
/// - Pointing → Listening (interrupted — new turn)
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
            | (CompanionState::Thinking, CompanionState::Pointing)
            | (CompanionState::Thinking, CompanionState::Idle)
            | (CompanionState::Speaking, CompanionState::Pointing)
            | (CompanionState::Speaking, CompanionState::Idle)
            | (CompanionState::Speaking, CompanionState::Listening)
            | (CompanionState::Pointing, CompanionState::Idle)
            | (CompanionState::Pointing, CompanionState::Listening)
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
