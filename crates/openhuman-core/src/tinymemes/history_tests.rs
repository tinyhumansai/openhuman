use super::*;

fn msg(id: &str, sender: &str, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_owned(),
        content: content.to_owned(),
        message_type: "text".to_owned(),
        extra_metadata: serde_json::Value::Null,
        sender: sender.to_owned(),
        created_at: "2026-10-09T00:00:00Z".to_owned(),
    }
}

#[test]
fn only_real_answers_are_remixed() {
    assert!(should_remix("a real answer", "budget exhausted"));
    assert!(!should_remix("budget exhausted", "budget exhausted"));
    assert!(!should_remix("  \n", "budget exhausted"));
}

#[test]
fn history_carries_message_ids_and_appends_the_pending_message() {
    let messages = vec![
        msg("u1", "user", "bhai build fail"),
        msg("r1", "agent", "Arre yaar, moye moye"),
        msg("r2", "agent", "Plain reply"),
        msg("s1", "system", "ignored"),
        msg("u2", "user", "  "),
    ];
    let turns = history_turns(&messages, "phir se fail");
    assert_eq!(turns.len(), 4);
    assert_eq!(turns[0].role, WireRole::User);
    assert_eq!(turns[1].role, WireRole::Assistant);
    // The module owns the ring of remixed ids; the host only names the turns.
    assert_eq!(turns[1].id.as_deref(), Some("r1"));
    assert_eq!(turns[2].id.as_deref(), Some("r2"));
    assert!(turns.iter().all(|t| !t.remixed));
    assert_eq!(turns[3].text, "phir se fail");
    assert_eq!(turns[3].id, None);
}

#[test]
fn pending_message_is_not_duplicated_when_already_stored() {
    let messages = vec![msg("u1", "user", "hello there")];
    let turns = history_turns(&messages, "hello there ");
    assert_eq!(turns.len(), 1);
}

#[test]
fn a_new_message_repeating_an_earlier_one_is_still_appended() {
    // user("hello"), agent("reply"), then a new "hello": a distinct message.
    let messages = vec![msg("u1", "user", "hello"), msg("r1", "agent", "reply")];
    let turns = history_turns(&messages, "hello");
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[2].text, "hello");
}
