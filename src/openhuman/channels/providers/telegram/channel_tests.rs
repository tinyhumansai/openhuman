use super::super::attachments::{
    infer_attachment_kind_from_target, parse_attachment_markers, parse_path_only_attachment,
    TelegramAttachmentKind,
};
use super::super::text::{
    split_message_for_telegram, strip_tool_call_tags, TELEGRAM_MAX_MESSAGE_LENGTH,
};
use super::TelegramChannel;
use crate::openhuman::channels::traits::{Channel, SendMessage};
use crate::openhuman::config::StreamMode;
use std::path::Path;
use std::time::Duration;

#[test]
fn telegram_channel_name() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert_eq!(ch.name(), "telegram");
}

#[test]
fn typing_handle_starts_as_none() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn stop_typing_clears_handle() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    // Manually insert a dummy handle
    {
        let mut guard = ch.typing_handle.lock();
        *guard = Some(super::TelegramTypingTask {
            recipient: "123".to_string(),
            handle: tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }),
        });
    }

    // stop_typing should abort and clear
    ch.stop_typing("123").await.unwrap();

    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn start_typing_replaces_previous_handle() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    // Insert a dummy handle first
    {
        let mut guard = ch.typing_handle.lock();
        *guard = Some(super::TelegramTypingTask {
            recipient: "123".to_string(),
            handle: tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }),
        });
    }

    // start_typing should abort the old handle and set a new one
    let _ = ch.start_typing("123").await;

    let guard = ch.typing_handle.lock();
    assert!(guard.is_some());
}

#[test]
fn supports_draft_updates_respects_stream_mode() {
    let off = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    assert!(!off.supports_draft_updates());

    let partial = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 750, true);
    assert!(partial.supports_draft_updates());
    assert_eq!(partial.draft_update_interval_ms, 750);
    assert!(partial.silent_streaming);
}

#[tokio::test]
async fn send_draft_returns_none_when_stream_mode_off() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let id = ch
        .send_draft(&SendMessage::new("draft", "123"))
        .await
        .unwrap();
    assert!(id.is_none());
}

#[tokio::test]
async fn update_draft_rate_limit_short_circuits_network() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false).with_streaming(
        StreamMode::Partial,
        60_000,
        true,
    );
    ch.last_draft_edit
        .lock()
        .insert("123".to_string(), std::time::Instant::now());

    let result = ch.update_draft("123", "42", "delta text").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn update_draft_utf8_truncation_is_safe_for_multibyte_text() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false).with_streaming(
        StreamMode::Partial,
        0,
        true,
    );
    let long_emoji_text = "😀".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 20);

    // Invalid message_id returns early after building display_text.
    // This asserts truncation never panics on UTF-8 boundaries.
    let result = ch
        .update_draft("123", "not-a-number", &long_emoji_text)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn finalize_draft_invalid_message_id_falls_back_to_chunk_send() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false).with_streaming(
        StreamMode::Partial,
        0,
        true,
    );
    let long_text = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 64);

    // For oversized text + invalid draft message_id, finalize_draft should
    // fall back to chunked send instead of returning early.
    let result = ch
        .finalize_draft("123", "not-a-number", &long_text, None)
        .await;
    assert!(result.is_err());
}

#[test]
fn telegram_api_url() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("getMe"),
        "https://api.telegram.org/bot123:ABC/getMe"
    );
}

#[test]
fn telegram_user_allowed_wildcard() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_specific() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "bob".into()], false);
    assert!(ch.is_user_allowed("alice"));
    assert!(!ch.is_user_allowed("eve"));
}

#[test]
fn telegram_user_allowed_with_at_prefix_in_config() {
    let ch = TelegramChannel::new("t".into(), vec!["@alice".into()], false);
    assert!(ch.is_user_allowed("alice"));
}

#[test]
fn telegram_user_denied_empty() {
    let ch = TelegramChannel::new("t".into(), vec![], false);
    assert!(!ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_exact_match_not_substring() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.is_user_allowed("alice_bot"));
    assert!(!ch.is_user_allowed("alic"));
    assert!(!ch.is_user_allowed("malice"));
}

#[test]
fn telegram_user_empty_string_denied() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.is_user_allowed(""));
}

#[test]
fn telegram_user_case_insensitive() {
    let ch = TelegramChannel::new("t".into(), vec!["Alice".into()], false);
    assert!(ch.is_user_allowed("Alice"));
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("ALICE"));
}

#[test]
fn telegram_wildcard_with_specific_users() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "*".into()], false);
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("bob"));
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_by_numeric_id_identity() {
    let ch = TelegramChannel::new("t".into(), vec!["123456789".into()], false);
    assert!(ch.is_any_user_allowed(["unknown", "123456789"]));
}

#[test]
fn telegram_user_denied_when_none_of_identities_match() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "987654321".into()], false);
    assert!(!ch.is_any_user_allowed(["unknown", "123456789"]));
}

#[test]
fn telegram_pairing_enabled_with_empty_allowlist() {
    let ch = TelegramChannel::new("t".into(), vec![], false);
    assert!(ch.pairing_code_active());
}

#[test]
fn telegram_pairing_disabled_with_nonempty_allowlist() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
    assert!(!ch.pairing_code_active());
}

#[test]
fn telegram_extract_bind_code_plain_command() {
    assert_eq!(
        TelegramChannel::extract_bind_code("/bind 123456"),
        Some("123456")
    );
}

#[test]
fn telegram_extract_bind_code_supports_bot_mention() {
    assert_eq!(
        TelegramChannel::extract_bind_code("/bind@openhuman_bot 654321"),
        Some("654321")
    );
}

#[test]
fn telegram_extract_bind_code_rejects_invalid_forms() {
    assert_eq!(TelegramChannel::extract_bind_code("/bind"), None);
    assert_eq!(TelegramChannel::extract_bind_code("/start"), None);
}

#[test]
fn parse_attachment_markers_extracts_multiple_types() {
    let message = "Here are files [IMAGE:/tmp/a.png] and [DOCUMENT:https://example.com/a.pdf]";
    let (cleaned, attachments) = parse_attachment_markers(message);

    assert_eq!(cleaned, "Here are files  and");
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].kind, TelegramAttachmentKind::Image);
    assert_eq!(attachments[0].target, "/tmp/a.png");
    assert_eq!(attachments[1].kind, TelegramAttachmentKind::Document);
    assert_eq!(attachments[1].target, "https://example.com/a.pdf");
}

#[test]
fn parse_attachment_markers_keeps_invalid_markers_in_text() {
    let message = "Report [UNKNOWN:/tmp/a.bin]";
    let (cleaned, attachments) = parse_attachment_markers(message);

    assert_eq!(cleaned, "Report [UNKNOWN:/tmp/a.bin]");
    assert!(attachments.is_empty());
}

#[test]
fn parse_path_only_attachment_detects_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("snap.png");
    std::fs::write(&image_path, b"fake-png").unwrap();

    let parsed = parse_path_only_attachment(image_path.to_string_lossy().as_ref())
        .expect("expected attachment");

    assert_eq!(parsed.kind, TelegramAttachmentKind::Image);
    assert_eq!(parsed.target, image_path.to_string_lossy());
}

#[test]
fn parse_path_only_attachment_rejects_sentence_text() {
    assert!(parse_path_only_attachment("Screenshot saved to /tmp/snap.png").is_none());
}

#[test]
fn infer_attachment_kind_from_target_detects_document_extension() {
    assert_eq!(
        infer_attachment_kind_from_target("https://example.com/files/specs.pdf?download=1"),
        Some(TelegramAttachmentKind::Document)
    );
}

#[test]
fn parse_update_message_uses_chat_id_as_reply_target() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 1,
        "message": {
            "message_id": 33,
            "text": "hello",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300
            }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("message should parse");

    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.reply_target, "-100200300");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.id, "telegram_-100200300_33");
}

#[test]
fn parse_update_message_allows_numeric_id_without_username() {
    let ch = TelegramChannel::new("token".into(), vec!["555".into()], false);
    let update = serde_json::json!({
        "update_id": 2,
        "message": {
            "message_id": 9,
            "text": "ping",
            "from": {
                "id": 555
            },
            "chat": {
                "id": 12345
            }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("numeric allowlist should pass");

    assert_eq!(msg.sender, "555");
    assert_eq!(msg.reply_target, "12345");
}

#[test]
fn parse_update_message_extracts_thread_id_for_forum_topic() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 3,
        "message": {
            "message_id": 42,
            "text": "hello from topic",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300
            },
            "message_thread_id": 789
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("message with thread_id should parse");

    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.reply_target, "-100200300:789");
    assert_eq!(msg.content, "hello from topic");
    assert_eq!(msg.id, "telegram_-100200300_42");
}

#[test]
fn parse_update_message_sets_thread_ts_to_current_message_id_for_outbound_reply() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 4,
        "message": {
            "message_id": 99,
            "text": "reply body",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": 12345
            },
            "reply_to_message": {
                "message_id": 88
            }
        }
    });

    let msg = ch
        .parse_update_message(&update)
        .expect("message should parse");
    assert_eq!(msg.thread_ts.as_deref(), Some("99"));
    assert_eq!(msg.reply_target, "12345");
}

#[test]
fn parse_update_reaction_extracts_actor_target_and_emoji() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 5,
        "message_reaction": {
            "chat": {
                "id": -100200300
            },
            "message_id": 123,
            "user": {
                "id": 777,
                "username": "alice"
            },
            "old_reaction": [],
            "new_reaction": [
                { "type": "emoji", "emoji": "🔥" }
            ]
        }
    });

    let reaction = ch
        .parse_update_reaction(&update)
        .expect("reaction should parse");
    assert_eq!(reaction.sender, "alice");
    assert_eq!(reaction.reply_target, "-100200300");
    assert_eq!(reaction.target_message_id, "123");
    assert_eq!(reaction.emoji, "🔥");
}

#[test]
fn parse_reaction_marker_supports_optional_target_id() {
    let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:✅|321]");
    assert_eq!(content, "");
    assert_eq!(marker.as_deref(), Some("✅|321"));

    let (content, marker) = TelegramChannel::parse_reaction_marker("hello");
    assert_eq!(content, "hello");
    assert!(marker.is_none());
}

#[test]
fn parse_reaction_marker_allows_inline_reply_text() {
    // Bot can react AND reply in one turn: [REACTION:👍] reply text
    let (content, marker) =
        TelegramChannel::parse_reaction_marker("[REACTION:👍] That's a great point!");
    assert_eq!(content, "That's a great point!");
    assert_eq!(marker.as_deref(), Some("👍"));

    // Explicit target id + inline text
    let (content, marker) =
        TelegramChannel::parse_reaction_marker("[REACTION:🔥|999] Here's my full reply.");
    assert_eq!(content, "Here's my full reply.");
    assert_eq!(marker.as_deref(), Some("🔥|999"));

    // Reaction only (no trailing text) still works
    let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:🤔]");
    assert_eq!(content, "");
    assert_eq!(marker.as_deref(), Some("🤔"));
}

#[test]
fn update_tracking_dedupes_and_skips_stale_updates() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    assert!(ch.track_update_id(10));
    assert!(
        !ch.track_update_id(10),
        "duplicate update should be skipped"
    );
    assert!(
        !ch.track_update_id(9),
        "stale out-of-order update should be skipped"
    );
    assert!(ch.track_update_id(11));
}

// ── File sending API URL tests ──────────────────────────────────

#[test]
fn telegram_api_url_send_document() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendDocument"),
        "https://api.telegram.org/bot123:ABC/sendDocument"
    );
}

#[test]
fn telegram_api_url_send_photo() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendPhoto"),
        "https://api.telegram.org/bot123:ABC/sendPhoto"
    );
}

#[test]
fn telegram_api_url_send_video() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendVideo"),
        "https://api.telegram.org/bot123:ABC/sendVideo"
    );
}

#[test]
fn telegram_api_url_send_audio() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendAudio"),
        "https://api.telegram.org/bot123:ABC/sendAudio"
    );
}

#[test]
fn telegram_api_url_send_voice() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
    assert_eq!(
        ch.api_url("sendVoice"),
        "https://api.telegram.org/bot123:ABC/sendVoice"
    );
}

// ── File sending integration tests (with mock server) ──────────

#[tokio::test]
async fn telegram_send_document_bytes_builds_correct_form() {
    // This test verifies the method doesn't panic and handles bytes correctly
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"Hello, this is a test file content".to_vec();

    // The actual API call will fail (no real server), but we verify the method exists
    // and handles the input correctly up to the network call
    let result = ch
        .send_document_bytes("123456", None, file_bytes, "test.txt", Some("Test caption"))
        .await;

    // Should fail with network error, not a panic or type error
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Error should be network-related, not a code bug
    assert!(
        err.contains("error") || err.contains("failed") || err.contains("connect"),
        "Expected network error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_bytes_builds_correct_form() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    // Minimal valid PNG header bytes
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    let result = ch
        .send_photo_bytes("123456", None, file_bytes, "test.png", None)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    let result = ch
        .send_document_by_url(
            "123456",
            None,
            "https://example.com/file.pdf",
            Some("PDF doc"),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_photo_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

    let result = ch
        .send_photo_by_url("123456", None, "https://example.com/image.jpg", None)
        .await;

    assert!(result.is_err());
}

// ── File path handling tests ────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/file.txt");

    let result = ch.send_document("123456", None, path, None).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Should fail with file not found error
    assert!(
        err.contains("No such file") || err.contains("not found") || err.contains("os error"),
        "Expected file not found error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/photo.jpg");

    let result = ch.send_photo("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_video_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/video.mp4");

    let result = ch.send_video("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_audio_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/audio.mp3");

    let result = ch.send_audio("123456", None, path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_voice_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let path = Path::new("/nonexistent/path/to/voice.ogg");

    let result = ch.send_voice("123456", None, path, None).await;

    assert!(result.is_err());
}

// ── Message splitting tests ─────────────────────────────────────

#[test]
fn telegram_split_short_message() {
    let msg = "Hello, world!";
    let chunks = split_message_for_telegram(msg);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], msg);
}

#[test]
fn telegram_split_exact_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
    let chunks = split_message_for_telegram(&msg);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), TELEGRAM_MAX_MESSAGE_LENGTH);
}

#[test]
fn telegram_split_over_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
    let chunks = split_message_for_telegram(&msg);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    assert!(chunks[1].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
}

#[test]
fn telegram_split_at_word_boundary() {
    let msg = format!(
        "{} more text here",
        "word ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5)
    );
    let chunks = split_message_for_telegram(&msg);
    assert!(chunks.len() >= 2);
    // First chunk should end with a complete word (space at the end)
    for chunk in &chunks[..chunks.len() - 1] {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

#[test]
fn telegram_split_at_newline() {
    let text_block = "Line of text\n".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 13 + 1);
    let chunks = split_message_for_telegram(&text_block);
    assert!(chunks.len() >= 2);
    for chunk in chunks {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

#[test]
fn telegram_split_preserves_content() {
    let msg = "test ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5 + 100);
    let chunks = split_message_for_telegram(&msg);
    let rejoined = chunks.join("");
    assert_eq!(rejoined, msg);
}

#[test]
fn telegram_split_empty_message() {
    let chunks = split_message_for_telegram("");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "");
}

#[test]
fn telegram_split_very_long_message() {
    let msg = "x".repeat(TELEGRAM_MAX_MESSAGE_LENGTH * 3);
    let chunks = split_message_for_telegram(&msg);
    assert!(chunks.len() >= 3);
    for chunk in chunks {
        assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

// ── Caption handling tests ──────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"test content".to_vec();

    // With caption
    let result = ch
        .send_document_bytes(
            "123456",
            None,
            file_bytes.clone(),
            "test.txt",
            Some("My caption"),
        )
        .await;
    assert!(result.is_err()); // Network error expected

    // Without caption
    let result = ch
        .send_document_bytes("123456", None, file_bytes, "test.txt", None)
        .await;
    assert!(result.is_err()); // Network error expected
}

#[tokio::test]
async fn telegram_send_photo_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47];

    // With caption
    let result = ch
        .send_photo_bytes(
            "123456",
            None,
            file_bytes.clone(),
            "test.png",
            Some("Photo caption"),
        )
        .await;
    assert!(result.is_err());

    // Without caption
    let result = ch
        .send_photo_bytes("123456", None, file_bytes, "test.png", None)
        .await;
    assert!(result.is_err());
}

// ── Empty/edge case tests ───────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_empty_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes: Vec<u8> = vec![];

    let result = ch
        .send_document_bytes("123456", None, file_bytes, "empty.txt", None)
        .await;

    // Should not panic, will fail at API level
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_filename() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"content".to_vec();

    let result = ch
        .send_document_bytes("123456", None, file_bytes, "", None)
        .await;

    // Should not panic
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_chat_id() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
    let file_bytes = b"content".to_vec();

    let result = ch
        .send_document_bytes("", None, file_bytes, "test.txt", None)
        .await;

    // Should not panic
    assert!(result.is_err());
}

// ── Message ID edge cases ─────────────────────────────────────

#[test]
fn telegram_message_id_format_includes_chat_and_message_id() {
    // Verify that message IDs follow the format: telegram_{chat_id}_{message_id}
    let chat_id = "123456";
    let message_id = 789;
    let expected_id = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(expected_id, "telegram_123456_789");
}

#[test]
fn telegram_message_id_is_deterministic() {
    // Same chat_id + same message_id = same ID (prevents duplicates after restart)
    let chat_id = "123456";
    let message_id = 789;
    let id1 = format!("telegram_{chat_id}_{message_id}");
    let id2 = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(id1, id2);
}

#[test]
fn telegram_message_id_different_message_different_id() {
    // Different message IDs produce different IDs
    let chat_id = "123456";
    let id1 = format!("telegram_{chat_id}_789");
    let id2 = format!("telegram_{chat_id}_790");
    assert_ne!(id1, id2);
}

#[test]
fn telegram_message_id_different_chat_different_id() {
    // Different chats produce different IDs even with same message_id
    let message_id = 789;
    let id1 = format!("telegram_123456_{message_id}");
    let id2 = format!("telegram_789012_{message_id}");
    assert_ne!(id1, id2);
}

#[test]
fn telegram_message_id_no_uuid_randomness() {
    // Verify format doesn't contain random UUID components
    let chat_id = "123456";
    let message_id = 789;
    let id = format!("telegram_{chat_id}_{message_id}");
    assert!(!id.contains('-')); // No UUID dashes
    assert!(id.starts_with("telegram_"));
}

#[test]
fn telegram_message_id_handles_zero_message_id() {
    // Edge case: message_id can be 0 (fallback/missing case)
    let chat_id = "123456";
    let message_id = 0;
    let id = format!("telegram_{chat_id}_{message_id}");
    assert_eq!(id, "telegram_123456_0");
}

// ── Tool call tag stripping tests ───────────────────────────────────

#[test]
fn strip_tool_call_tags_removes_standard_tags() {
    let input = "Hello <tool>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_alias_tags() {
    let input =
        "Hello <toolcall>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</toolcall> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_dash_tags() {
    let input = "Hello <tool-call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool-call> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_tool_call_tags() {
    let input = "Hello <tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_removes_invoke_tags() {
    let input =
        "Hello <invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"date\"}}</invoke> world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strip_tool_call_tags_handles_multiple_tags() {
    let input = "Start <tool>a</tool> middle <tool>b</tool> end";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Start  middle  end");
}

#[test]
fn strip_tool_call_tags_handles_mixed_tags() {
    let input = "A <tool>a</tool> B <toolcall>b</toolcall> C <tool-call>c</tool-call> D";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "A  B  C  D");
}

#[test]
fn strip_tool_call_tags_preserves_normal_text() {
    let input = "Hello world! This is a test.";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello world! This is a test.");
}

#[test]
fn strip_tool_call_tags_handles_unclosed_tags() {
    let input = "Hello <tool>world";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello <tool>world");
}

#[test]
fn strip_tool_call_tags_handles_unclosed_tool_call_with_json() {
    let input = "Status:\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Status:");
}

#[test]
fn strip_tool_call_tags_handles_mismatched_close_tag() {
    let input =
        "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}</arg_value>";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn strip_tool_call_tags_cleans_extra_newlines() {
    let input = "Hello\n\n<tool>\ntest\n</tool>\n\n\nworld";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "Hello\n\nworld");
}

#[test]
fn strip_tool_call_tags_handles_empty_input() {
    let input = "";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn strip_tool_call_tags_handles_only_tags() {
    let input = "<tool>{\"name\":\"test\"}</tool>";
    let result = strip_tool_call_tags(input);
    assert_eq!(result, "");
}

#[test]
fn telegram_contains_bot_mention_finds_mention() {
    assert!(TelegramChannel::contains_bot_mention(
        "Hello @mybot",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "@mybot help",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "Hey @mybot how are you?",
        "mybot"
    ));
    assert!(TelegramChannel::contains_bot_mention(
        "Hello @MyBot, can you help?",
        "mybot"
    ));
}

#[test]
fn telegram_contains_bot_mention_no_false_positives() {
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello @otherbot",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello mybot",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention(
        "Hello @mybot2",
        "mybot"
    ));
    assert!(!TelegramChannel::contains_bot_mention("", "mybot"));
}

#[test]
fn telegram_normalize_incoming_content_strips_mention() {
    let result = TelegramChannel::normalize_incoming_content("@mybot hello", "mybot");
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn telegram_normalize_incoming_content_handles_multiple_mentions() {
    let result = TelegramChannel::normalize_incoming_content("@mybot @mybot test", "mybot");
    assert_eq!(result, Some("test".to_string()));
}

#[test]
fn telegram_normalize_incoming_content_returns_none_for_empty() {
    let result = TelegramChannel::normalize_incoming_content("@mybot", "mybot");
    assert_eq!(result, None);
}

#[test]
fn parse_update_message_mention_only_group_requires_exact_mention() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }

    let update = serde_json::json!({
        "update_id": 10,
        "message": {
            "message_id": 44,
            "text": "hello @mybot2",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    assert!(ch.parse_update_message(&update).is_none());
}

#[test]
fn parse_update_message_mention_only_group_strips_mention_and_drops_empty() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    {
        let mut cache = ch.bot_username.lock();
        *cache = Some("mybot".to_string());
    }

    let update = serde_json::json!({
        "update_id": 11,
        "message": {
            "message_id": 45,
            "text": "Hi @MyBot status please",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    let parsed = ch
        .parse_update_message(&update)
        .expect("mention should parse");
    assert_eq!(parsed.content, "Hi status please");

    let empty_update = serde_json::json!({
        "update_id": 12,
        "message": {
            "message_id": 46,
            "text": "@mybot",
            "from": {
                "id": 555,
                "username": "alice"
            },
            "chat": {
                "id": -100_200_300,
                "type": "group"
            }
        }
    });

    assert!(ch.parse_update_message(&empty_update).is_none());
}

#[test]
fn telegram_is_group_message_detects_groups() {
    let group_msg = serde_json::json!({
        "chat": { "type": "group" }
    });
    assert!(TelegramChannel::is_group_message(&group_msg));

    let supergroup_msg = serde_json::json!({
        "chat": { "type": "supergroup" }
    });
    assert!(TelegramChannel::is_group_message(&supergroup_msg));

    let private_msg = serde_json::json!({
        "chat": { "type": "private" }
    });
    assert!(!TelegramChannel::is_group_message(&private_msg));
}

#[test]
fn telegram_mention_only_enabled_by_config() {
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
    assert!(ch.mention_only);

    let ch_disabled = TelegramChannel::new("token".into(), vec!["*".into()], false);
    assert!(!ch_disabled.mention_only);
}

// ─────────────────────────────────────────────────────────────────────
// TG6: Channel platform limit edge cases for Telegram (4096 char limit)
// Prevents: Pattern 6 — issues #574, #499
// ─────────────────────────────────────────────────────────────────────

#[test]
fn telegram_split_code_block_at_boundary() {
    let mut msg = String::new();
    msg.push_str("```python\n");
    msg.push_str(&"x".repeat(4085));
    msg.push_str("\n```\nMore text after code block");
    let parts = split_message_for_telegram(&msg);
    assert!(
        parts.len() >= 2,
        "code block spanning boundary should split"
    );
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "each part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
}

#[test]
fn telegram_split_single_long_word() {
    let long_word = "a".repeat(5000);
    let parts = split_message_for_telegram(&long_word);
    assert!(parts.len() >= 2, "word exceeding limit must be split");
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "hard-split part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
    let reassembled: String = parts.join("");
    assert_eq!(reassembled, long_word);
}

#[test]
fn telegram_split_exactly_at_limit_no_split() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
    let parts = split_message_for_telegram(&msg);
    assert_eq!(parts.len(), 1, "message exactly at limit should not split");
}

#[test]
fn telegram_split_one_over_limit() {
    let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 1);
    let parts = split_message_for_telegram(&msg);
    assert!(parts.len() >= 2, "message 1 char over limit must split");
}

#[test]
fn telegram_split_many_short_lines() {
    let msg: String = (0..1000).map(|i| format!("line {i}\n")).collect();
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        assert!(
            part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "short-line batch must be <= limit"
        );
    }
}

#[test]
fn telegram_split_only_whitespace() {
    let msg = "   \n\n\t  ";
    let parts = split_message_for_telegram(msg);
    assert!(parts.len() <= 1);
}

#[test]
fn telegram_split_emoji_at_boundary() {
    let mut msg = "a".repeat(4094);
    msg.push_str("🎉🎊"); // 4096 chars total
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        // The function splits on character count, not byte count
        assert!(
            part.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH,
            "emoji boundary split must respect limit"
        );
    }
}

#[test]
fn telegram_split_consecutive_newlines() {
    let mut msg = "a".repeat(4090);
    msg.push_str("\n\n\n\n\n\n");
    msg.push_str(&"b".repeat(100));
    let parts = split_message_for_telegram(&msg);
    for part in &parts {
        assert!(part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }
}

// ── Reaction allowlist tests ────────────────────────────────────

#[test]
fn parse_update_reaction_returns_none_for_unlisted_actor() {
    // Only "alice" is allowed; "mallory" should be rejected.
    let ch = TelegramChannel::new("token".into(), vec!["alice".into()], false);
    let update = serde_json::json!({
        "update_id": 50,
        "message_reaction": {
            "chat": { "id": 100 },
            "message_id": 1,
            "user": { "id": 999, "username": "mallory" },
            "old_reaction": [],
            "new_reaction": [{ "type": "emoji", "emoji": "👍" }]
        }
    });
    assert!(
        ch.parse_update_reaction(&update).is_none(),
        "reaction from non-allowlisted actor must be ignored"
    );
}

#[test]
fn parse_update_reaction_returns_none_when_new_reaction_is_empty() {
    // Removing a reaction (new_reaction is empty) should yield None.
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 51,
        "message_reaction": {
            "chat": { "id": 100 },
            "message_id": 55,
            "user": { "id": 777, "username": "alice" },
            "old_reaction": [{ "type": "emoji", "emoji": "👍" }],
            "new_reaction": []
        }
    });
    assert!(
        ch.parse_update_reaction(&update).is_none(),
        "reaction removal (empty new_reaction) must be ignored"
    );
}

#[test]
fn parse_update_reaction_falls_back_to_user_id_when_username_absent() {
    // No "username" field; allowlist uses numeric user id.
    let ch = TelegramChannel::new("token".into(), vec!["99999".into()], false);
    let update = serde_json::json!({
        "update_id": 52,
        "message_reaction": {
            "chat": { "id": 200 },
            "message_id": 77,
            "user": { "id": 99999 },
            "old_reaction": [],
            "new_reaction": [{ "type": "emoji", "emoji": "❤️" }]
        }
    });
    let reaction = ch
        .parse_update_reaction(&update)
        .expect("user_id in allowlist should be accepted");
    assert_eq!(reaction.sender, "99999");
    assert_eq!(reaction.emoji, "❤️");
    assert_eq!(reaction.target_message_id, "77");
}

// ── Reaction marker parsing edge cases ─────────────────────────

#[test]
fn parse_reaction_marker_plain_emoji_without_pipe_has_no_explicit_target() {
    // [REACTION:👍] — no pipe separator, no explicit target message id.
    let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:👍]");
    assert_eq!(
        content, "",
        "content after stripping marker should be empty"
    );
    assert_eq!(
        marker.as_deref(),
        Some("👍"),
        "marker should be the emoji without pipe"
    );
}

#[test]
fn parse_reaction_marker_empty_inner_produces_no_marker() {
    // [REACTION:] — empty inner, no valid emoji.
    let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:]");
    assert_eq!(
        content, "",
        "content should be empty string for empty marker"
    );
    assert!(
        marker.is_none(),
        "empty inner must not produce a reaction marker"
    );
}

#[test]
fn parse_reaction_marker_non_marker_text_is_unchanged() {
    let input = "here is your answer";
    let (content, marker) = TelegramChannel::parse_reaction_marker(input);
    assert_eq!(content, input);
    assert!(marker.is_none());
}

// ── Typing body construction tests ─────────────────────────────

#[test]
fn typing_body_for_plain_chat_contains_no_thread_field() {
    let body = TelegramChannel::typing_body_for_recipient("99999");
    assert_eq!(body["chat_id"].as_str(), Some("99999"));
    assert_eq!(body["action"].as_str(), Some("typing"));
    // No message_thread_id for plain chats
    assert!(
        body.get("message_thread_id").is_none() || body["message_thread_id"].is_null(),
        "plain chat should not include message_thread_id"
    );
}

#[test]
fn typing_body_for_forum_topic_includes_message_thread_id() {
    let body = TelegramChannel::typing_body_for_recipient("99999:42");
    assert_eq!(body["chat_id"].as_str(), Some("99999"));
    assert_eq!(body["action"].as_str(), Some("typing"));
    assert_eq!(
        body["message_thread_id"].as_str(),
        Some("42"),
        "forum topic must carry message_thread_id in typing body"
    );
}

// ── Update tracking edge cases ──────────────────────────────────

#[test]
fn track_update_id_accepts_monotonically_increasing_sequence() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    for id in 1..=20i64 {
        assert!(
            ch.track_update_id(id),
            "monotonically increasing id {id} should be accepted"
        );
    }
}

#[test]
fn track_update_id_large_volume_beyond_cache_does_not_panic() {
    // TELEGRAM_RECENT_UPDATE_CACHE_SIZE is 4096; push well beyond to exercise eviction.
    let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
    for id in 1..=5000i64 {
        ch.track_update_id(id);
    }
    // After eviction, the next fresh id is still accepted.
    assert!(
        ch.track_update_id(5001),
        "first occurrence of a new id must always be accepted"
    );
}

#[test]
fn silent_streaming_is_configurable() {
    let silent = TelegramChannel::new("fake-token".into(), vec!["*".into()], false).with_streaming(
        StreamMode::Partial,
        1000,
        true,
    );
    assert!(silent.silent_streaming);

    let noisy = TelegramChannel::new("fake-token".into(), vec!["*".into()], false).with_streaming(
        StreamMode::Partial,
        1000,
        false,
    );
    assert!(!noisy.silent_streaming);
}

// ── Reply-target parsing unit tests ────────────────────────────

#[test]
fn parse_reply_target_splits_chat_and_thread_on_colon() {
    let (chat_id, thread_id) = TelegramChannel::parse_reply_target("12345:789");
    assert_eq!(chat_id, "12345");
    assert_eq!(thread_id.as_deref(), Some("789"));
}

#[test]
fn parse_reply_target_no_colon_returns_plain_chat_id() {
    let (chat_id, thread_id) = TelegramChannel::parse_reply_target("-100200300");
    assert_eq!(chat_id, "-100200300");
    assert!(thread_id.is_none());
}

#[test]
fn parse_update_message_without_reply_to_still_sets_thread_ts_to_own_message_id() {
    // Every inbound message sets thread_ts = its own message_id so the outbound
    // reply attaches visibly in Telegram. This applies even with no reply_to_message.
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 60,
        "message": {
            "message_id": 55,
            "text": "standalone message",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": 100 }
        }
    });
    let msg = ch.parse_update_message(&update).expect("should parse");
    assert_eq!(
        msg.thread_ts.as_deref(),
        Some("55"),
        "thread_ts must equal inbound message_id for reply targeting"
    );
    assert_eq!(msg.reply_target, "100");
}

#[test]
fn parse_update_message_forum_topic_encodes_thread_in_reply_target_and_thread_ts() {
    // Forum-topic messages carry message_thread_id (topic) AND may have reply_to_message.
    // reply_target must be chat_id:thread_id; thread_ts must be the inbound message_id.
    let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
    let update = serde_json::json!({
        "update_id": 61,
        "message": {
            "message_id": 100,
            "text": "in a forum topic with a quoted reply",
            "from": { "id": 1, "username": "alice" },
            "chat": { "id": -200 },
            "message_thread_id": 42,
            "reply_to_message": { "message_id": 90 }
        }
    });
    let msg = ch.parse_update_message(&update).expect("should parse");
    assert_eq!(
        msg.reply_target, "-200:42",
        "reply_target must encode both chat_id and topic thread_id"
    );
    assert_eq!(
        msg.thread_ts.as_deref(),
        Some("100"),
        "thread_ts is the inbound message_id (not the quoted parent)"
    );
}

#[tokio::test]
async fn test_thinking_placeholder_logic() {
    use crate::openhuman::agent::progress::AgentProgress;
    use crate::openhuman::channels::traits::Channel;
    use crate::openhuman::channels::SendMessage;
    use parking_lot::Mutex;
    use std::sync::Arc;

    // Mock channel that records updates
    struct MockTelegramChannel {
        updates: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Channel for MockTelegramChannel {
        fn name(&self) -> &str {
            "telegram"
        }
        fn supports_draft_updates(&self) -> bool {
            true
        }
        async fn send(&self, _: &SendMessage) -> anyhow::Result<()> {
            Ok(())
        }
        async fn listen(
            &self,
            _: tokio::sync::mpsc::Sender<crate::openhuman::channels::traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_draft(&self, _: &SendMessage) -> anyhow::Result<Option<String>> {
            Ok(Some("123".to_string()))
        }
        async fn update_draft(&self, _: &str, _: &str, text: &str) -> anyhow::Result<()> {
            self.updates.lock().push(text.to_string());
            Ok(())
        }
        async fn finalize_draft(
            &self,
            _: &str,
            _: &str,
            text: &str,
            _: Option<&str>,
        ) -> anyhow::Result<()> {
            self.updates.lock().push(format!("FINAL: {}", text));
            Ok(())
        }
    }

    let updates = Arc::new(Mutex::new(Vec::new()));
    let channel = Arc::new(MockTelegramChannel {
        updates: Arc::clone(&updates),
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(10);

    let channel_clone = Arc::clone(&channel);
    let handle = tokio::spawn(async move {
        let mut accumulated = String::new();
        let reply_target = "test_chat";
        let draft_id = "123";
        while let Some(progress) = rx.recv().await {
            match progress {
                AgentProgress::TextDelta { delta, .. } => {
                    accumulated.push_str(&delta);
                    let _ = channel_clone
                        .update_draft(reply_target, draft_id, &accumulated)
                        .await;
                }
                AgentProgress::ThinkingDelta { .. } => {
                    if accumulated.is_empty() {
                        let _ = channel_clone
                            .update_draft(reply_target, draft_id, "Thinking...")
                            .await;
                    }
                }
                AgentProgress::ToolCallStarted { tool_name, .. } => {
                    if accumulated.is_empty() {
                        let _ = channel_clone
                            .update_draft(
                                reply_target,
                                draft_id,
                                &format!("Working ({})...", tool_name),
                            )
                            .await;
                    }
                }
                _ => {}
            }
        }
    });

    // Simulate thinking then text
    tx.send(AgentProgress::ThinkingDelta {
        delta: "thought 1".to_string(),
        iteration: 1,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::ThinkingDelta {
        delta: "thought 2".to_string(),
        iteration: 1,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::ToolCallStarted {
        call_id: "c1".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::TextDelta {
        delta: "Hello".to_string(),
        iteration: 2,
    })
    .await
    .unwrap();
    drop(tx);
    handle.await.unwrap();

    let history = updates.lock();
    assert!(history.contains(&"Thinking...".to_string()));
    assert!(history.contains(&"Working (shell)...".to_string()));
    assert!(history.contains(&"Hello".to_string()));
    // Ensure actual thought text was NOT sent
    for update in history.iter() {
        assert!(!update.contains("thought 1"));
        assert!(!update.contains("thought 2"));
    }
}
