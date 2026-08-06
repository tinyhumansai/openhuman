//! Tests for the companion interaction pipeline.
//!
//! These tests exercise the pipeline's orchestration logic — state
//! transitions, cancellation, and conversation history. Real STT/LLM/TTS
//! calls are not made; the pipeline's
//! network calls fail fast in a test environment (no embedded core).

use super::*;
use crate::companion::session;

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
    assert!(!r.tts_synthesized);
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

#[test]
fn stt_dispatch_params_encode_wav_for_provider_dispatch() {
    assert_eq!(
        stt_dispatch_params(b"wav"),
        json!({
            "audio_base64": "d2F2",
            "mime_type": "audio/wav",
            "file_name": "companion.wav",
        })
    );
}

#[test]
fn tts_dispatch_params_defer_provider_selection_to_core() {
    assert_eq!(tts_dispatch_params("hello"), json!({ "text": "hello" }));
}

#[test]
fn companion_chat_endpoint_joins_only_the_backend_path() {
    assert_eq!(
        companion_chat_endpoint("https://api.tinyhumans.ai/"),
        "https://api.tinyhumans.ai/openai/v1/chat/completions"
    );
}

// ── Text turn tests ──────────────────────────────────────────────────

#[tokio::test]
async fn text_turn_rejects_empty_input() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    let result = run_text_turn("", cancel).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
    session::reset_for_test();
}

#[tokio::test]
async fn text_turn_rejects_whitespace_only() {
    let _guard = lock_and_reset();
    let cancel = CancellationToken::new();
    let result = run_text_turn("   \n  ", cancel).await;
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
    let result = run_text_turn("hello", cancel).await;
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
    let result = run_audio_turn(&[], 16_000, cancel).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no audio"));
    let status = session::session_status();
    assert_eq!(status.state, CompanionState::Idle);
    assert_eq!(status.last_error.as_deref(), Some("no audio samples"));
    session::reset_for_test();
}

// ── System prompt ────────────────────────────────────────────────────

#[test]
fn companion_system_prompt_does_not_request_screen_context_or_point_tags() {
    assert!(!COMPANION_SYSTEM_PROMPT.contains("screen context"));
    assert!(!COMPANION_SYSTEM_PROMPT.contains("[POINT:"));
}

#[test]
fn companion_system_prompt_discourages_markdown() {
    assert!(COMPANION_SYSTEM_PROMPT.contains("markdown"));
}
