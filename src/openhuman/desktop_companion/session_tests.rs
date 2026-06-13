//! Tests for companion session lifecycle and state machine.

use super::*;
use crate::openhuman::desktop_companion::types::*;

/// Serialize tests that mutate the process-global session state. Shared with
/// `pipeline_tests` via `lock_test_state()` (defined in `session`) so a
/// reset/transition here can't race a transition in the pipeline tests.
fn with_clean_session<F: FnOnce()>(f: F) {
    let _lock = lock_test_state();
    reset_for_test();
    f();
    reset_for_test();
}

fn start_default_session() -> StartCompanionSessionResult {
    start_session(&StartCompanionSessionParams {
        consent: true,
        ttl_secs: Some(3600),
    })
    .expect("session should start")
}

// ── Session creation ──────────────────────────────────────────────────

#[test]
fn start_session_requires_consent() {
    with_clean_session(|| {
        let result = start_session(&StartCompanionSessionParams {
            consent: false,
            ttl_secs: None,
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("consent"));
    });
}

#[test]
fn start_session_succeeds_with_consent() {
    with_clean_session(|| {
        let result = start_default_session();
        assert!(!result.session_id.is_empty());
        assert_eq!(result.state, CompanionState::Idle);
        assert!(result.expires_at_ms.is_some());
    });
}

#[test]
fn start_session_rejects_duplicate() {
    with_clean_session(|| {
        let _first = start_default_session();
        let second = start_session(&StartCompanionSessionParams {
            consent: true,
            ttl_secs: None,
        });
        assert!(second.is_err());
        assert!(second.unwrap_err().contains("already active"));
    });
}

#[test]
fn start_session_zero_ttl_means_no_expiry() {
    with_clean_session(|| {
        let result = start_session(&StartCompanionSessionParams {
            consent: true,
            ttl_secs: Some(0),
        })
        .unwrap();
        assert!(result.expires_at_ms.is_none());
    });
}

// ── Session stop ──────────────────────────────────────────────────────

#[test]
fn stop_session_succeeds() {
    with_clean_session(|| {
        let _s = start_default_session();
        let result = stop_session(&StopCompanionSessionParams {
            reason: Some("test".into()),
        })
        .unwrap();
        assert!(result.stopped);
        assert_eq!(result.reason.as_deref(), Some("test"));
    });
}

#[test]
fn stop_session_with_no_active_session() {
    with_clean_session(|| {
        let result = stop_session(&StopCompanionSessionParams { reason: None }).unwrap();
        assert!(!result.stopped);
        assert!(result.reason.unwrap().contains("no active"));
    });
}

#[test]
fn stop_allows_new_session() {
    with_clean_session(|| {
        let _s = start_default_session();
        let _ = stop_session(&StopCompanionSessionParams { reason: None });
        let second = start_default_session();
        assert!(!second.session_id.is_empty());
    });
}

// ── Session status ────────────────────────────────────────────────────

#[test]
fn status_inactive_by_default() {
    with_clean_session(|| {
        let status = session_status();
        assert!(!status.active);
        assert_eq!(status.state, CompanionState::Idle);
        assert!(status.session_id.is_none());
    });
}

#[test]
fn status_reflects_active_session() {
    with_clean_session(|| {
        let s = start_default_session();
        let status = session_status();
        assert!(status.active);
        assert_eq!(status.session_id.as_deref(), Some(s.session_id.as_str()));
        assert!(status.remaining_ms.is_some());
    });
}

// ── State transitions ─────────────────────────────────────────────────

#[test]
fn transition_idle_to_listening() {
    with_clean_session(|| {
        let _s = start_default_session();
        let prev = transition_state(CompanionState::Listening, None).unwrap();
        assert_eq!(prev, CompanionState::Idle);
        assert_eq!(session_status().state, CompanionState::Listening);
    });
}

#[test]
fn transition_listening_to_thinking() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        let prev = transition_state(CompanionState::Thinking, None).unwrap();
        assert_eq!(prev, CompanionState::Listening);
    });
}

#[test]
fn transition_thinking_to_speaking() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        transition_state(CompanionState::Thinking, None).unwrap();
        let prev = transition_state(CompanionState::Speaking, None).unwrap();
        assert_eq!(prev, CompanionState::Thinking);
    });
}

#[test]
fn transition_speaking_to_pointing() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        transition_state(CompanionState::Thinking, None).unwrap();
        transition_state(CompanionState::Speaking, None).unwrap();
        let prev = transition_state(CompanionState::Pointing, None).unwrap();
        assert_eq!(prev, CompanionState::Speaking);
    });
}

#[test]
fn transition_pointing_to_idle() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        transition_state(CompanionState::Thinking, None).unwrap();
        transition_state(CompanionState::Pointing, None).unwrap();
        let prev = transition_state(CompanionState::Idle, None).unwrap();
        assert_eq!(prev, CompanionState::Pointing);
    });
}

#[test]
fn transition_full_happy_path() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        transition_state(CompanionState::Thinking, None).unwrap();
        transition_state(CompanionState::Speaking, None).unwrap();
        transition_state(CompanionState::Pointing, None).unwrap();
        transition_state(CompanionState::Idle, None).unwrap();
        assert_eq!(session_status().state, CompanionState::Idle);
    });
}

#[test]
fn transition_any_to_error() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        let prev = transition_state(CompanionState::Error, Some("mic failure".into())).unwrap();
        assert_eq!(prev, CompanionState::Listening);

        let status = session_status();
        assert_eq!(status.state, CompanionState::Error);
        assert_eq!(status.last_error.as_deref(), Some("mic failure"));
    });
}

#[test]
fn transition_error_to_idle() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Error, Some("oops".into())).unwrap();
        let prev = transition_state(CompanionState::Idle, None).unwrap();
        assert_eq!(prev, CompanionState::Error);
    });
}

#[test]
fn transition_speaking_to_listening_interrupt() {
    with_clean_session(|| {
        let _s = start_default_session();
        transition_state(CompanionState::Listening, None).unwrap();
        transition_state(CompanionState::Thinking, None).unwrap();
        transition_state(CompanionState::Speaking, None).unwrap();
        let prev = transition_state(CompanionState::Listening, None).unwrap();
        assert_eq!(prev, CompanionState::Speaking);
    });
}

#[test]
fn transition_invalid_idle_to_speaking() {
    with_clean_session(|| {
        let _s = start_default_session();
        let result = transition_state(CompanionState::Speaking, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid"));
    });
}

#[test]
fn transition_invalid_idle_to_thinking() {
    with_clean_session(|| {
        let _s = start_default_session();
        let result = transition_state(CompanionState::Thinking, None);
        assert!(result.is_err());
    });
}

#[test]
fn transition_requires_active_session() {
    with_clean_session(|| {
        let result = transition_state(CompanionState::Listening, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no active"));
    });
}

// ── Conversation history ──────────────────────────────────────────────

#[test]
fn push_conversation_turn_succeeds() {
    with_clean_session(|| {
        let _s = start_default_session();
        push_conversation_turn(ConversationTurn {
            role: "user".into(),
            content: "hello".into(),
            timestamp_ms: 1000,
        })
        .unwrap();
        let history = conversation_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "hello");
    });
}

#[test]
fn conversation_history_capped() {
    with_clean_session(|| {
        let _s = start_default_session();
        for i in 0..60 {
            push_conversation_turn(ConversationTurn {
                role: "user".into(),
                content: format!("turn {i}"),
                timestamp_ms: i as i64,
            })
            .unwrap();
        }
        let history = conversation_history();
        assert_eq!(history.len(), MAX_CONVERSATION_TURNS);
        // Oldest turns should have been drained.
        assert_eq!(history[0].content, "turn 10");
    });
}

#[test]
fn conversation_history_empty_without_session() {
    with_clean_session(|| {
        assert!(conversation_history().is_empty());
    });
}

#[test]
fn push_turn_fails_without_session() {
    with_clean_session(|| {
        let result = push_conversation_turn(ConversationTurn {
            role: "user".into(),
            content: "hello".into(),
            timestamp_ms: 1000,
        });
        assert!(result.is_err());
    });
}

// ── Auto-expire and TTL edge cases ───────────────────────────────────

#[test]
fn start_session_auto_expires_stale_session() {
    with_clean_session(|| {
        // Start a session with a 1-second TTL.
        let first = start_session(&StartCompanionSessionParams {
            consent: true,
            ttl_secs: Some(1),
        })
        .unwrap();
        assert!(first.expires_at_ms.is_some());

        // Sleep past the TTL so the session becomes stale.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // Starting a new session should succeed — the stale one is auto-expired.
        let second = start_session(&StartCompanionSessionParams {
            consent: true,
            ttl_secs: Some(3600),
        })
        .unwrap();
        assert_ne!(first.session_id, second.session_id);
        assert_eq!(second.state, CompanionState::Idle);
    });
}

#[test]
fn start_session_ttl_overflow_guard() {
    with_clean_session(|| {
        // u64::MAX would overflow i64 when multiplied by 1000 — the guard caps it.
        let result = start_session(&StartCompanionSessionParams {
            consent: true,
            ttl_secs: Some(u64::MAX),
        })
        .unwrap();
        // Session created without panic.
        assert!(!result.session_id.is_empty());
        // expires_at_ms should be set (non-zero TTL) and positive (not overflowed).
        let expires = result
            .expires_at_ms
            .expect("should have expiry with non-zero TTL");
        assert!(
            expires > 0,
            "expires_at_ms should be positive, got {expires}"
        );
    });
}

// ── is_active helper ──────────────────────────────────────────────────

#[test]
fn companion_state_is_active() {
    assert!(!CompanionState::Idle.is_active());
    assert!(CompanionState::Listening.is_active());
    assert!(CompanionState::Thinking.is_active());
    assert!(CompanionState::Speaking.is_active());
    assert!(CompanionState::Pointing.is_active());
    assert!(!CompanionState::Error.is_active());
}
