use super::*;
use crate::openhuman::meet_agent::session::registry;

#[tokio::test]
async fn run_turn_skips_short_buffers() {
    registry().start("brain-skip", 16_000).unwrap();
    registry()
        .with_session("brain-skip", |s| {
            s.push_inbound_pcm(&vec![0; 800]); // 50ms — under floor
        })
        .unwrap();
    assert_eq!(run_turn("brain-skip").await.unwrap(), false);
    let _ = registry().stop("brain-skip");
}

#[tokio::test]
async fn run_turn_falls_back_to_stub_without_backend() {
    // No backend session in test env → STT/LLM/TTS all fail and
    // each stage falls back to its stub. The turn still produces
    // a Heard event, a Spoke event, and synthesized PCM, so the
    // smoke-test contract holds.
    registry().start("brain-fallback", 16_000).unwrap();
    registry()
        .with_session("brain-fallback", |s| {
            s.push_inbound_pcm(&vec![1000; 16_000]); // 1s
        })
        .unwrap();
    assert_eq!(run_turn("brain-fallback").await.unwrap(), true);
    registry()
        .with_session("brain-fallback", |s| {
            let kinds: Vec<_> = s.events().iter().map(|e| format!("{:?}", e.kind)).collect();
            assert!(kinds.contains(&"Heard".to_string()));
            assert!(kinds.contains(&"Spoke".to_string()));
            assert_eq!(s.turn_count, 1);
            assert!(s.spoken_seconds() > 0.0);
        })
        .unwrap();
    let _ = registry().stop("brain-fallback");
}

#[test]
fn extract_chat_completion_text_pulls_first_choice() {
    let raw = json!({
        "choices": [
            { "message": { "content": "  hello world  " } }
        ]
    });
    assert_eq!(
        extract_chat_completion_text(&raw),
        Some("hello world".to_string())
    );
}

#[test]
fn extract_chat_completion_text_returns_none_on_malformed() {
    assert_eq!(extract_chat_completion_text(&json!({})), None);
    assert_eq!(
        extract_chat_completion_text(&json!({ "choices": [] })),
        None
    );
}

#[test]
fn recent_dialog_history_maps_event_kinds_to_chat_roles() {
    let now = 0;
    let events = vec![
        SessionEvent {
            kind: SessionEventKind::Heard,
            text: "Alice: how's the build going".into(),
            timestamp_ms: now,
        },
        SessionEvent {
            kind: SessionEventKind::Note,
            text: "wake word".into(),
            timestamp_ms: now,
        },
        SessionEvent {
            kind: SessionEventKind::Spoke,
            text: "Build is green.".into(),
            timestamp_ms: now,
        },
        SessionEvent {
            kind: SessionEventKind::Heard,
            text: "Bob: ship it".into(),
            timestamp_ms: now,
        },
    ];
    let history = recent_dialog_history(&events, 10);
    assert_eq!(history.len(), 3, "Note events are dropped");
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[2].role, "user");
    assert_eq!(history[2].content, "Bob: ship it");
}

#[test]
fn recent_dialog_history_caps_at_window_keeping_most_recent() {
    let events: Vec<SessionEvent> = (0..30)
        .map(|i| SessionEvent {
            kind: SessionEventKind::Heard,
            text: format!("line {i}"),
            timestamp_ms: 0,
        })
        .collect();
    let history = recent_dialog_history(&events, 5);
    assert_eq!(history.len(), 5);
    assert_eq!(history[0].content, "line 25");
    assert_eq!(history[4].content, "line 29");
}

#[test]
fn strip_for_speech_removes_markdown_punctuation_and_fences() {
    let raw = "**Got it.** Adding `that` to your follow-ups.";
    assert_eq!(
        strip_for_speech(raw),
        "Got it. Adding that to your follow-ups."
    );
    let fenced = "Sure:\n```\ncode\n```\nDone.";
    assert_eq!(strip_for_speech(fenced), "Sure: Done.");
    let bullets = "- one\n- two";
    assert_eq!(strip_for_speech(bullets), "one two");
}

#[test]
fn strip_for_speech_preserves_empty_when_input_empty() {
    assert_eq!(strip_for_speech(""), "");
    assert_eq!(strip_for_speech("   \n  "), "");
}

#[test]
fn soft_deny_message_names_both_owner_and_asker() {
    let line = soft_deny_message("Bob", "Alice");
    assert!(line.contains("Bob"), "must address the asker: {line}");
    assert!(line.contains("Alice"), "must name the owner: {line}");
    assert!(
        line.to_lowercase().contains("allow"),
        "must hint the magic word: {line}"
    );
}

#[test]
fn soft_deny_message_handles_missing_names_gracefully() {
    // No asker, no owner — should still be a polite English sentence,
    // not a templated stub with empty placeholders.
    let line = soft_deny_message("", "");
    assert!(!line.is_empty());
    assert!(
        !line.contains("{"),
        "must not leak format placeholders: {line}"
    );
}

#[test]
fn looks_like_grant_intent_accepts_canonical_phrases() {
    // Whole-prompt approvals.
    for phrase in ["allow", "yes", "ok", "okay", "go ahead", "permit"] {
        assert!(
            looks_like_grant_intent(phrase),
            "must accept bare approval phrase: {phrase}"
        );
    }
    // Common longer forms.
    for phrase in [
        "allow them",
        "allow Bob to ask",
        "let them in",
        "let them ask",
        "let her ask",
        "go ahead and answer them",
        "yes go ahead",
        "permit Bob",
        "you can tell Bob",
    ] {
        assert!(looks_like_grant_intent(phrase), "should accept: {phrase}");
    }
}

#[test]
fn classify_unauthorized_intent_treats_bare_wake_as_greeting() {
    // Empty tail after the wake phrase — the non-owner just
    // said "hey openhuman" with nothing else. Friendly hi-back
    // is the right call, not a refusal.
    assert_eq!(
        classify_unauthorized_intent("hey openhuman"),
        UnauthorizedIntent::Greeting
    );
    assert_eq!(
        classify_unauthorized_intent("Hi openhuman."),
        UnauthorizedIntent::Greeting
    );
}

#[test]
fn classify_unauthorized_intent_treats_filler_as_greeting() {
    // Common pleasantries that contain greeting words only.
    for text in [
        "hello openhuman there",
        "hi openhuman everyone",
        "hey openhuman hi",
        "hey openhuman good morning",
    ] {
        assert_eq!(
            classify_unauthorized_intent(text),
            UnauthorizedIntent::Greeting,
            "should be greeting: {text}"
        );
    }
}

#[test]
fn classify_unauthorized_intent_flags_task_asks() {
    // Substantive task asks — refuse + tell owner how to grant.
    for text in [
        "hey openhuman read my slack",
        "hi openhuman what's on alice's calendar",
        "openhuman send the report",
        "hello openhuman remember the launch",
    ] {
        assert_eq!(
            classify_unauthorized_intent(text),
            UnauthorizedIntent::TaskAsk,
            "should be task: {text}"
        );
    }
}

#[test]
fn looks_like_grant_intent_rejects_unrelated_prompts() {
    // Words that happen to contain "allow" / "yes" mid-prompt
    // shouldn't hijack a normal question — the matcher only
    // honors prompts that BEGIN with a permit verb.
    for phrase in [
        "what's on my calendar today",
        "did i allow that meeting earlier",
        "yesterday's notes please",
        "remind me to ok the budget",
        "permittivity of free space",
    ] {
        assert!(
            !looks_like_grant_intent(phrase),
            "must not match unrelated prompt: {phrase}"
        );
    }
}
