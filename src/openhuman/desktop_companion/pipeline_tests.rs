//! Tests for the companion interaction pipeline.
//!
//! These tests exercise the pipeline's orchestration logic — state
//! transitions, cancellation, conversation history, and POINT-tag
//! integration. Real STT/LLM/TTS calls are not made; the pipeline
//! falls back to stubs in a test environment (no backend token).

use super::*;
use crate::openhuman::desktop_companion::pointing::ScreenGeometry;
use crate::openhuman::desktop_companion::session;
use crate::openhuman::desktop_companion::types::*;

/// Serialize tests that touch the process-global session state. Shared with
/// `session_tests` via `session::lock_test_state()` so transitions in one test
/// module can't race a reset/transition in the other.
fn lock_and_reset() -> std::sync::MutexGuard<'static, ()> {
    let guard = session::lock_test_state();
    session::reset_for_test();
    session::start_session(&StartCompanionSessionParams {
        consent: true,
        ttl_secs: Some(3600),
    })
    .expect("session should start");
    guard
}

fn single_screen() -> Vec<ScreenGeometry> {
    vec![ScreenGeometry {
        index: 0,
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    }]
}

// ── Helper tests ─────────────────────────────────────────────────────

#[test]
fn tail_history_returns_last_n() {
    let turns: Vec<ConversationTurn> = (0..10)
        .map(|i| ConversationTurn {
            role: "user".into(),
            content: format!("turn {i}"),
            timestamp_ms: i,
        })
        .collect();
    let tail = tail_history(&turns, 3);
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0].content, "turn 7");
    assert_eq!(tail[2].content, "turn 9");
}

#[test]
fn tail_history_handles_small_history() {
    let turns = vec![ConversationTurn {
        role: "user".into(),
        content: "only".into(),
        timestamp_ms: 0,
    }];
    let tail = tail_history(&turns, 10);
    assert_eq!(tail.len(), 1);
}

#[test]
fn tail_history_empty() {
    let turns: Vec<ConversationTurn> = Vec::new();
    let tail = tail_history(&turns, 5);
    assert!(tail.is_empty());
}

#[test]
fn cancelled_result_has_correct_fields() {
    let r = cancelled_result("hello");
    assert_eq!(r.transcript, "hello");
    assert!(r.response_text.is_empty());
    assert!(r.targets.is_empty());
    assert!(!r.tts_synthesized);
    assert!(r.handoff_events.is_empty());
    assert!(r.cancelled);
}

#[test]
fn extract_chat_completion_text_valid() {
    let raw = json!({
        "choices": [{ "message": { "content": "  Hello!  " } }]
    });
    assert_eq!(
        extract_chat_completion_text(&raw),
        Some("Hello!".to_string())
    );
}

#[test]
fn extract_chat_completion_text_empty_choices() {
    assert_eq!(
        extract_chat_completion_text(&json!({ "choices": [] })),
        None
    );
}

#[test]
fn extract_chat_completion_text_malformed() {
    assert_eq!(extract_chat_completion_text(&json!({})), None);
    assert_eq!(extract_chat_completion_text(&json!(42)), None);
}

// ── Text turn tests ──────────────────────────────────────────────────

#[tokio::test]
async fn text_turn_rejects_empty_input() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    let result = run_text_turn("", &single_screen(), cancel).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
    session::reset_for_test();
}

#[tokio::test]
async fn text_turn_rejects_whitespace_only() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    let result = run_text_turn("   \n  ", &single_screen(), cancel).await;
    assert!(result.is_err());
    session::reset_for_test();
}

#[tokio::test]
async fn text_turn_cancellation_returns_cancelled() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    cancel.cancel();
    // Transition to Listening first so Thinking is a valid transition.
    session::transition_state(CompanionState::Listening, None).unwrap();
    let result = run_text_turn("hello", &single_screen(), cancel).await;
    let turn = result.unwrap();
    assert!(turn.cancelled);
    assert!(turn.response_text.is_empty());
    session::reset_for_test();
}

// ── Audio turn tests ─────────────────────────────────────────────────

#[tokio::test]
async fn audio_turn_rejects_empty_samples() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    let result = run_audio_turn(&[], 16_000, &single_screen(), cancel).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no audio"));
    session::reset_for_test();
}

// ── Screen context ───────────────────────────────────────────────────

#[tokio::test]
async fn gather_screen_context_returns_option() {
    let ctx = gather_screen_context().await;
    // Just verify it doesn't panic — value depends on platform.
    let _ = ctx;
}

// ── System prompt ────────────────────────────────────────────────────

#[test]
fn companion_system_prompt_mentions_point_tags() {
    assert!(COMPANION_SYSTEM_PROMPT.contains("[POINT:"));
    assert!(COMPANION_SYSTEM_PROMPT.contains("screenN"));
}

#[test]
fn companion_system_prompt_discourages_markdown() {
    assert!(COMPANION_SYSTEM_PROMPT.contains("markdown"));
}
