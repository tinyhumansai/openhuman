use super::*;

#[test]
fn placeholder_title_is_always_replaceable() {
    assert!(is_replaceable_title("Chat Oct 6 3:04 PM", None, 0));
    assert!(is_replaceable_title("Chat Oct 6 3:04 PM", Some("hi"), 5));
}

#[test]
fn interim_title_from_first_message_is_replaceable_during_first_exchange() {
    let message = "Can you retrieve my latest 5 emails and summarize them?";
    let interim = title_from_user_message(message).expect("interim title");
    assert!(is_replaceable_title(&interim, Some(message), 0));
    assert!(is_replaceable_title(&interim, Some(message), 1));
}

#[test]
fn interim_title_is_kept_after_the_first_exchange() {
    let message = "Can you retrieve my latest 5 emails and summarize them?";
    let interim = title_from_user_message(message).expect("interim title");
    assert!(!is_replaceable_title(&interim, Some(message), 2));
}

#[test]
fn user_chosen_title_is_never_replaced() {
    let message = "Can you retrieve my latest 5 emails and summarize them?";
    assert!(!is_replaceable_title("Inbox triage", Some(message), 0));
    assert!(!is_replaceable_title("Inbox triage", None, 0));
}
