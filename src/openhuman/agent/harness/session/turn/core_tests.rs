//! What the cap path must not send to a provider (#6068).
//!
//! Sibling `*_tests.rs` file rather than an inline module, per the Rust layout
//! gate. `is_empty_assistant_chat` is `pub(super)` inside a private `core`
//! module, so this is the one place it is reachable from.

use super::*;

#[test]
fn an_empty_assistant_reply_is_recognised_for_removal() {
    let empty = ConversationMessage::Chat(ChatMessage::assistant(String::new()));
    let blank = ConversationMessage::Chat(ChatMessage::assistant("   \n".to_string()));
    let real =
        ConversationMessage::Chat(ChatMessage::assistant("here is what I found".to_string()));
    let user = ConversationMessage::Chat(ChatMessage::user("do the thing".to_string()));

    assert!(
        is_empty_assistant_chat(&empty),
        "empty content must be dropped"
    );
    assert!(
        is_empty_assistant_chat(&blank),
        "whitespace-only content is empty to a provider too"
    );
    assert!(
        !is_empty_assistant_chat(&real),
        "a real conclusion must never be dropped"
    );
    assert!(
        !is_empty_assistant_chat(&user),
        "only the assistant's own empty turn is at issue"
    );
}
