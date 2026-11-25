use super::*;

#[test]
fn a_done_event_without_legacy_response_text_returns_an_empty_reply() {
    assert_eq!(chat_reply_text(None), "");
}

#[test]
fn a_done_event_keeps_its_response_text() {
    assert_eq!(chat_reply_text(Some("answer".into())), "answer");
}

#[test]
fn an_error_event_without_legacy_message_gets_safe_user_facing_copy() {
    assert_eq!(
        chat_failure_message(None),
        "The turn failed without an error message."
    );
}

#[test]
fn an_error_event_keeps_its_message() {
    assert_eq!(
        chat_failure_message(Some("provider failed".into())),
        "provider failed"
    );
}
