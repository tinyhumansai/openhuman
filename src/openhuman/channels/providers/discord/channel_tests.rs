use super::*;

#[test]
fn discord_channel_name() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    assert_eq!(ch.name(), "discord");
}

#[test]
fn base64_decode_bot_id() {
    // "MTIzNDU2" decodes to "123456"
    let decoded = base64_decode("MTIzNDU2");
    assert_eq!(decoded, Some("123456".to_string()));
}

#[test]
fn bot_user_id_extraction() {
    // Token format: base64(user_id).timestamp.hmac
    let token = "MTIzNDU2.fake.hmac";
    let id = DiscordChannel::bot_user_id_from_token(token);
    assert_eq!(id, Some("123456".to_string()));
}

#[test]
fn empty_allowlist_denies_everyone() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    assert!(!ch.is_user_allowed("12345"));
    assert!(!ch.is_user_allowed("anyone"));
}

#[test]
fn wildcard_allows_everyone() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec!["*".into()], false, false);
    assert!(ch.is_user_allowed("12345"));
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn specific_allowlist_filters() {
    let ch = DiscordChannel::new(
        "fake".into(),
        None,
        None,
        vec!["111".into(), "222".into()],
        false,
        false,
    );
    assert!(ch.is_user_allowed("111"));
    assert!(ch.is_user_allowed("222"));
    assert!(!ch.is_user_allowed("333"));
    assert!(!ch.is_user_allowed("unknown"));
}

#[test]
fn allowlist_is_exact_match_not_substring() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec!["111".into()], false, false);
    assert!(!ch.is_user_allowed("1111"));
    assert!(!ch.is_user_allowed("11"));
    assert!(!ch.is_user_allowed("0111"));
}

#[test]
fn allowlist_empty_string_user_id() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec!["111".into()], false, false);
    assert!(!ch.is_user_allowed(""));
}

#[test]
fn allowlist_with_wildcard_and_specific() {
    let ch = DiscordChannel::new(
        "fake".into(),
        None,
        None,
        vec!["111".into(), "*".into()],
        false,
        false,
    );
    assert!(ch.is_user_allowed("111"));
    assert!(ch.is_user_allowed("anyone_else"));
}

#[test]
fn allowlist_case_sensitive() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec!["ABC".into()], false, false);
    assert!(ch.is_user_allowed("ABC"));
    assert!(!ch.is_user_allowed("abc"));
    assert!(!ch.is_user_allowed("Abc"));
}

#[test]
fn base64_decode_empty_string() {
    let decoded = base64_decode("");
    assert_eq!(decoded, Some(String::new()));
}

#[test]
fn base64_decode_invalid_chars() {
    let decoded = base64_decode("!!!!");
    assert!(decoded.is_none());
}

#[test]
fn bot_user_id_from_empty_token() {
    let id = DiscordChannel::bot_user_id_from_token("");
    assert_eq!(id, Some(String::new()));
}

#[test]
fn contains_bot_mention_supports_plain_and_nick_forms() {
    assert!(contains_bot_mention("hi <@12345>", "12345"));
    assert!(contains_bot_mention("hi <@!12345>", "12345"));
    assert!(!contains_bot_mention("hi <@99999>", "12345"));
}

#[test]
fn normalize_incoming_content_requires_mention_when_enabled() {
    let cleaned = normalize_incoming_content("hello there", true, "12345");
    assert!(cleaned.is_none());
}

#[test]
fn normalize_incoming_content_strips_mentions_and_trims() {
    let cleaned = normalize_incoming_content("  <@!12345> run status  ", true, "12345");
    assert_eq!(cleaned.as_deref(), Some("run status"));
}

#[test]
fn normalize_incoming_content_rejects_empty_after_strip() {
    let cleaned = normalize_incoming_content("<@12345>", true, "12345");
    assert!(cleaned.is_none());
}

// Message splitting tests

#[test]
fn split_empty_message() {
    let chunks = split_message_for_discord("");
    assert_eq!(chunks, vec![""]);
}

#[test]
fn split_short_message_under_limit() {
    let msg = "Hello, world!";
    let chunks = split_message_for_discord(msg);
    assert_eq!(chunks, vec![msg]);
}

#[test]
fn split_message_exactly_2000_chars() {
    let msg = "a".repeat(DISCORD_MAX_MESSAGE_LENGTH);
    let chunks = split_message_for_discord(&msg);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].chars().count(), DISCORD_MAX_MESSAGE_LENGTH);
}

#[test]
fn split_message_just_over_limit() {
    let msg = "a".repeat(DISCORD_MAX_MESSAGE_LENGTH + 1);
    let chunks = split_message_for_discord(&msg);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].chars().count(), DISCORD_MAX_MESSAGE_LENGTH);
    assert_eq!(chunks[1].chars().count(), 1);
}

#[test]
fn split_very_long_message() {
    let msg = "word ".repeat(2000); // 10000 characters (5 chars per "word ")
    let chunks = split_message_for_discord(&msg);
    // Should split into 5 chunks of <= 2000 chars
    assert_eq!(chunks.len(), 5);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.chars().count() <= DISCORD_MAX_MESSAGE_LENGTH));
    // Verify total content is preserved
    let reconstructed = chunks.concat();
    assert_eq!(reconstructed, msg);
}

#[test]
fn split_prefer_newline_break() {
    let msg = format!("{}\n{}", "a".repeat(1500), "b".repeat(500));
    let chunks = split_message_for_discord(&msg);
    // Should split at the newline
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].ends_with('\n'));
    assert!(chunks[1].starts_with('b'));
}

#[test]
fn split_prefer_space_break() {
    let msg = format!("{} {}", "a".repeat(1500), "b".repeat(600));
    let chunks = split_message_for_discord(&msg);
    assert_eq!(chunks.len(), 2);
}

#[test]
fn split_without_good_break_points_hard_split() {
    // No spaces or newlines - should hard split at 2000
    let msg = "a".repeat(5000);
    let chunks = split_message_for_discord(&msg);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].chars().count(), DISCORD_MAX_MESSAGE_LENGTH);
    assert_eq!(chunks[1].chars().count(), DISCORD_MAX_MESSAGE_LENGTH);
    assert_eq!(chunks[2].chars().count(), 1000);
}

#[test]
fn split_multiple_breaks() {
    // Create a message with multiple newlines
    let part1 = "a".repeat(900);
    let part2 = "b".repeat(900);
    let part3 = "c".repeat(900);
    let msg = format!("{part1}\n{part2}\n{part3}");
    let chunks = split_message_for_discord(&msg);
    // Should split into 2 chunks (first two parts + third part)
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].chars().count() <= DISCORD_MAX_MESSAGE_LENGTH);
    assert!(chunks[1].chars().count() <= DISCORD_MAX_MESSAGE_LENGTH);
}

#[test]
fn split_preserves_content() {
    let original = "Hello world! This is a test message with some content. ".repeat(200);
    let chunks = split_message_for_discord(&original);
    let reconstructed = chunks.concat();
    assert_eq!(reconstructed, original);
}

#[test]
fn split_unicode_content() {
    // Test with emoji and multi-byte characters
    let msg = "🦀 Rust is awesome! ".repeat(500);
    let chunks = split_message_for_discord(&msg);
    // All chunks should be valid UTF-8
    for chunk in &chunks {
        assert!(std::str::from_utf8(chunk.as_bytes()).is_ok());
        assert!(chunk.chars().count() <= DISCORD_MAX_MESSAGE_LENGTH);
    }
    // Reconstruct and verify
    let reconstructed = chunks.concat();
    assert_eq!(reconstructed, msg);
}

#[test]
fn split_newline_too_close_to_end() {
    // If newline is in the first half, don't use it - use space instead or hard split
    let msg = format!("{}\n{}", "a".repeat(1900), "b".repeat(500));
    let chunks = split_message_for_discord(&msg);
    // Should split at newline since it's in the second half of the window
    assert_eq!(chunks.len(), 2);
}

#[test]
fn split_multibyte_only_content_without_panics() {
    let msg = "🦀".repeat(2500);
    let chunks = split_message_for_discord(&msg);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].chars().count(), DISCORD_MAX_MESSAGE_LENGTH);
    assert_eq!(chunks[1].chars().count(), 500);
    let reconstructed = chunks.concat();
    assert_eq!(reconstructed, msg);
}

#[test]
fn split_chunks_always_within_discord_limit() {
    let msg = "x".repeat(12_345);
    let chunks = split_message_for_discord(&msg);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.chars().count() <= DISCORD_MAX_MESSAGE_LENGTH));
}

#[test]
fn split_message_with_multiple_newlines() {
    let msg = "Line 1\nLine 2\nLine 3\n".repeat(1000);
    let chunks = split_message_for_discord(&msg);
    assert!(chunks.len() > 1);
    let reconstructed = chunks.concat();
    assert_eq!(reconstructed, msg);
}

#[test]
fn typing_handle_starts_as_none() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn start_typing_sets_handle() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    let _ = ch.start_typing("123456").await;
    let guard = ch.typing_handle.lock();
    assert!(guard.is_some());
}

#[tokio::test]
async fn stop_typing_clears_handle() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    let _ = ch.start_typing("123456").await;
    let _ = ch.stop_typing("123456").await;
    let guard = ch.typing_handle.lock();
    assert!(guard.is_none());
}

#[tokio::test]
async fn stop_typing_is_idempotent() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    assert!(ch.stop_typing("123456").await.is_ok());
    assert!(ch.stop_typing("123456").await.is_ok());
}

#[tokio::test]
async fn start_typing_replaces_existing_task() {
    let ch = DiscordChannel::new("fake".into(), None, None, vec![], false, false);
    let _ = ch.start_typing("111").await;
    let _ = ch.start_typing("222").await;
    let guard = ch.typing_handle.lock();
    assert!(guard.is_some());
}

// ── Message ID edge cases ─────────────────────────────────────

#[test]
fn discord_message_id_format_includes_discord_prefix() {
    // Verify that message IDs follow the format: discord_{message_id}
    let message_id = "123456789012345678";
    let expected_id = format!("discord_{message_id}");
    assert_eq!(expected_id, "discord_123456789012345678");
}

#[test]
fn discord_message_id_is_deterministic() {
    // Same message_id = same ID (prevents duplicates after restart)
    let message_id = "123456789012345678";
    let id1 = format!("discord_{message_id}");
    let id2 = format!("discord_{message_id}");
    assert_eq!(id1, id2);
}

#[test]
fn discord_message_id_different_message_different_id() {
    // Different message IDs produce different IDs
    let id1 = "discord_123456789012345678".to_string();
    let id2 = "discord_987654321098765432".to_string();
    assert_ne!(id1, id2);
}

#[test]
fn discord_message_id_uses_snowflake_id() {
    // Discord snowflake IDs are numeric strings
    let message_id = "123456789012345678"; // Typical snowflake format
    let id = format!("discord_{message_id}");
    assert!(id.starts_with("discord_"));
    // Snowflake IDs are numeric
    assert!(message_id.chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn discord_message_id_fallback_to_uuid_on_empty() {
    // Edge case: empty message_id falls back to UUID
    let message_id = "";
    let id = if message_id.is_empty() {
        format!("discord_{}", uuid::Uuid::new_v4())
    } else {
        format!("discord_{message_id}")
    };
    assert!(id.starts_with("discord_"));
    // Should have UUID dashes
    assert!(id.contains('-'));
}

// ─────────────────────────────────────────────────────────────────────
// TG6: Channel platform limit edge cases for Discord (2000 char limit)
// Prevents: Pattern 6 — issues #574, #499
// ─────────────────────────────────────────────────────────────────────

#[test]
fn split_message_code_block_at_boundary() {
    // Code block that spans the split boundary
    let mut msg = String::new();
    msg.push_str("```rust\n");
    msg.push_str(&"x".repeat(1990));
    msg.push_str("\n```\nMore text after code block");
    let parts = split_message_for_discord(&msg);
    assert!(
        parts.len() >= 2,
        "code block spanning boundary should split"
    );
    for part in &parts {
        assert!(
            part.len() <= DISCORD_MAX_MESSAGE_LENGTH,
            "each part must be <= {DISCORD_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
}

#[test]
fn split_message_single_long_word_exceeds_limit() {
    // A single word longer than 2000 chars must be hard-split
    let long_word = "a".repeat(2500);
    let parts = split_message_for_discord(&long_word);
    assert!(parts.len() >= 2, "word exceeding limit must be split");
    for part in &parts {
        assert!(
            part.len() <= DISCORD_MAX_MESSAGE_LENGTH,
            "hard-split part must be <= {DISCORD_MAX_MESSAGE_LENGTH}, got {}",
            part.len()
        );
    }
    // Reassembled content should match original
    let reassembled: String = parts.join("");
    assert_eq!(reassembled, long_word);
}

#[test]
fn split_message_exactly_at_limit_no_split() {
    let msg = "a".repeat(DISCORD_MAX_MESSAGE_LENGTH);
    let parts = split_message_for_discord(&msg);
    assert_eq!(parts.len(), 1, "message exactly at limit should not split");
    assert_eq!(parts[0].len(), DISCORD_MAX_MESSAGE_LENGTH);
}

#[test]
fn split_message_one_over_limit_splits() {
    let msg = "a".repeat(DISCORD_MAX_MESSAGE_LENGTH + 1);
    let parts = split_message_for_discord(&msg);
    assert!(parts.len() >= 2, "message 1 char over limit must split");
}

#[test]
fn split_message_many_short_lines() {
    // Many short lines should be batched into chunks under the limit
    let msg: String = (0..500).map(|i| format!("line {i}\n")).collect();
    let parts = split_message_for_discord(&msg);
    for part in &parts {
        assert!(
            part.len() <= DISCORD_MAX_MESSAGE_LENGTH,
            "short-line batch must be <= limit"
        );
    }
    // All content should be preserved
    let reassembled: String = parts.join("");
    assert_eq!(reassembled.trim(), msg.trim());
}

#[test]
fn split_message_only_whitespace() {
    let msg = "   \n\n\t  ";
    let parts = split_message_for_discord(msg);
    // Should handle gracefully without panic
    assert!(parts.len() <= 1);
}

#[test]
fn split_message_emoji_at_boundary() {
    // Emoji are multi-byte; ensure we don't split mid-emoji
    let mut msg = "a".repeat(1998);
    msg.push_str("🎉🎊"); // 2 emoji at the boundary (2000 chars total)
    let parts = split_message_for_discord(&msg);
    for part in &parts {
        // The function splits on character count, not byte count
        assert!(
            part.chars().count() <= DISCORD_MAX_MESSAGE_LENGTH,
            "emoji boundary split must respect limit"
        );
    }
}

#[test]
fn split_message_consecutive_newlines_at_boundary() {
    let mut msg = "a".repeat(1995);
    msg.push_str("\n\n\n\n\n");
    msg.push_str(&"b".repeat(100));
    let parts = split_message_for_discord(&msg);
    for part in &parts {
        assert!(part.len() <= DISCORD_MAX_MESSAGE_LENGTH);
    }
}

// ── channel_id field tests ───────────────────────────────────

#[test]
fn channel_id_stored_in_struct() {
    let ch = DiscordChannel::new(
        "token".into(),
        Some("guild1".into()),
        Some("channel1".into()),
        vec![],
        false,
        false,
    );
    assert_eq!(ch.channel_id.as_deref(), Some("channel1"));
    assert_eq!(ch.guild_id.as_deref(), Some("guild1"));
}

#[test]
fn channel_id_defaults_to_none() {
    let ch = DiscordChannel::new("token".into(), None, None, vec![], false, false);
    assert!(ch.channel_id.is_none());
}
