use super::*;

fn msg(role: &str, content: &str) -> ChatMessage {
    match role {
        "system" => ChatMessage::system(content),
        "user" => ChatMessage::user(content),
        "assistant" => ChatMessage::assistant(content),
        _ => ChatMessage::tool(content),
    }
}

/// Every row's `message.role` must be `"user"`. The CLI validates this
/// before invoking the model and exits 1 with
/// `Expected message role 'user', got 'assistant'` otherwise (#5711).
/// Verified against Claude Code CLI 2.1.221.
fn assert_every_row_is_a_user_role(payload: &str) {
    for (i, line) in payload.lines().enumerate() {
        let row: Value = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!(
                "row {i} is not valid JSON: {e}
{line}"
            )
        });
        assert_eq!(
            row["message"]["role"], "user",
            "row {i} must carry role=user, the only role CC stdin accepts
{line}"
        );
        assert_eq!(row["type"], "user", "row {i} envelope type");
    }
}

#[test]
fn new_session_never_emits_an_assistant_role() {
    let history = vec![
        msg("system", "you are helpful"),
        msg("user", "first user"),
        msg("assistant", "prior assistant"),
        msg("user", "latest user"),
    ];
    let s = String::from_utf8(build_stdin(&history, true)).unwrap();
    assert_every_row_is_a_user_role(&s);
    assert!(
        !s.contains("\"role\":\"assistant\""),
        "an assistant role row is what the CLI rejects:
{s}"
    );
}

#[test]
fn new_session_carries_prior_turns_as_one_labelled_transcript() {
    let history = vec![
        msg("system", "you are helpful"),
        msg("user", "hi"),
        msg("assistant", "hello"),
        msg("user", "how are you?"),
    ];
    let s = String::from_utf8(build_stdin(&history, true)).unwrap();
    let lines: Vec<_> = s.lines().collect();

    // One transcript row + the latest user turn. The system row is still
    // filtered out — it rides `--append-system-prompt`.
    assert_eq!(
        lines.len(),
        2,
        "got:
{s}"
    );
    assert!(lines[0].contains("User: hi"));
    assert!(lines[0].contains("Assistant: hello"));
    assert!(
        !lines[0].contains("you are helpful"),
        "the system message must not leak into the transcript"
    );

    // The prompt itself is passed through untouched, not folded in.
    let latest: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(latest["message"]["content"][0]["text"], "how are you?");
}

#[test]
fn a_history_ending_on_an_assistant_turn_is_all_context() {
    // Switching an existing conversation to the CC provider can leave the
    // assistant speaking last; none of it is a fresh instruction.
    let history = vec![msg("user", "hi"), msg("assistant", "hello")];
    let s = String::from_utf8(build_stdin(&history, true)).unwrap();
    let lines: Vec<_> = s.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "got:
{s}"
    );
    assert_every_row_is_a_user_role(&s);
    assert!(lines[0].contains("User: hi"));
    assert!(lines[0].contains("Assistant: hello"));
}

#[test]
fn a_single_user_turn_is_sent_verbatim_with_no_transcript() {
    let history = vec![msg("user", "just this")];
    let s = String::from_utf8(build_stdin(&history, true)).unwrap();
    let lines: Vec<_> = s.lines().collect();
    assert_eq!(lines.len(), 1);
    let row: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row["message"]["content"][0]["text"], "just this");
}

#[test]
fn resume_pipes_only_last_user_turn() {
    let history = vec![
        msg("user", "earlier turn"),
        msg("assistant", "earlier reply"),
        msg("user", "follow-up"),
    ];
    let bytes = build_stdin(&history, false);
    let s = String::from_utf8(bytes).unwrap();
    let lines: Vec<_> = s.lines().collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"follow-up\""));
    assert_every_row_is_a_user_role(&s);
}

#[test]
fn empty_history_yields_empty_bytes() {
    let bytes = build_stdin(&[], true);
    assert!(bytes.is_empty());
}

#[test]
fn interleaved_text_and_images_keep_source_order() {
    let s = String::from_utf8(build_stdin(
        &[msg("user", "before [IMAGE:data:image/png;base64,QUJD] between [IMAGE:data:image/jpeg;base64,REVG] after")],
        true,
    )).unwrap();
    let row: Value = serde_json::from_str(s.lines().next().unwrap()).unwrap();
    let content = row["message"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 5);
    assert_eq!(content[0]["text"], "before ");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[2]["text"], " between ");
    assert_eq!(content[3]["type"], "image");
    assert_eq!(content[4]["text"], " after");
}

#[test]
fn percent_encoded_data_uri_emits_an_image_block() {
    let s = String::from_utf8(build_stdin(
        &[msg("user", "see [IMAGE:data:image/png,%89PNG%0D%0A]")],
        true,
    ))
    .unwrap();
    let row: Value = serde_json::from_str(s.lines().next().unwrap()).unwrap();
    let content = row["message"]["content"].as_array().unwrap();
    assert!(content.len() >= 2, "expected text and image blocks: {s}");
    let block = &content[1];
    assert_eq!(block["type"], "image");
    assert_eq!(block["source"]["media_type"], "image/png");
    assert_eq!(
        block["source"]["data"],
        base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n")
    );
}

#[test]
fn readable_managed_image_file_emits_an_image_block() {
    let dir = crate::openhuman::agent::multimodal::managed_attachments_dir_for_tests();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("input-builder-test-{}.png", std::process::id()));
    std::fs::write(&path, b"PNG").unwrap();
    let s = String::from_utf8(build_stdin(
        &[msg("user", &format!("file [IMAGE:{}]", path.display()))],
        true,
    ))
    .unwrap();
    let row: Value = serde_json::from_str(s.lines().next().unwrap()).unwrap();
    let content = row["message"]["content"].as_array().unwrap();
    assert!(content.len() >= 2, "expected text and image blocks: {s}");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["data"], "UE5H");
    let _ = std::fs::remove_file(path);
}

#[test]
fn unmanaged_and_unreadable_images_degrade_without_reading_paths() {
    let s = String::from_utf8(build_stdin(
        &[msg(
            "user",
            "before [IMAGE:/etc/passwd] after [IMAGE:/definitely/missing.png]",
        )],
        true,
    ))
    .unwrap();
    assert!(s.contains("before ") && s.contains(" after"));
    assert_eq!(s.matches("an attached image could not be read").count(), 2);
}

#[test]
fn invalid_inline_images_use_the_text_fallback() {
    assert!(image_block("data:image/svg+xml;base64,PHN2Zz4=").is_none());
    assert!(image_block("data:image/png;base64,not-base64").is_none());
    assert!(image_block("data:image/png,%ZZ").is_none());
}

#[test]
fn image_count_is_capped_at_sixteen() {
    let marker = "[IMAGE:data:image/png;base64,QQ==]";
    let raw = std::iter::repeat_n(marker, 17).collect::<String>();
    let blocks = content_blocks(&raw);
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block["type"] == "image")
            .count(),
        16
    );
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block["text"] == "[an attached image could not be read]")
            .count(),
        1
    );
}

#[test]
fn oversized_inline_images_use_the_text_fallback() {
    let payload = "A".repeat(20 * 1024 * 1024 + 1);
    assert!(image_block(&format!("data:image/png;base64,{payload}")).is_none());
}
