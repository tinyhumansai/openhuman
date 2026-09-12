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
