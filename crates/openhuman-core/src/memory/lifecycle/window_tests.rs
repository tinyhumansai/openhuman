use super::*;

#[test]
fn starts_where_the_pre_turn_step_put_it_and_never_past_the_turn() {
    assert_eq!(InPromptFrom::new(6, 20).get(), 6);
    assert_eq!(InPromptFrom::new(30, 20).get(), 20);
}

#[test]
fn a_fold_moves_it_two_turns_per_folded_user_message() {
    let window = InPromptFrom::new(6, 20);
    assert_eq!(
        window.advance_past(["first question", "second question"]),
        10
    );
    assert_eq!(window.get(), 10);
    assert_eq!(window.advance_past(["third question"]), 12);
    assert_eq!(window.advance_past(Vec::<String>::new()), 12);
}

#[test]
fn identical_messages_are_separate_turns() {
    let window = InPromptFrom::new(0, 40);
    assert_eq!(window.advance_past(["yes", "yes"]), 4);
    // A later fold holding the same text is new turns too.
    assert_eq!(window.advance_past(["ok", "yes"]), 8);
}

#[test]
fn the_same_slice_folded_again_moves_nothing() {
    let window = InPromptFrom::new(0, 40);
    assert_eq!(window.advance_past(["a", "b"]), 4);
    assert_eq!(window.advance_past(["c"]), 6);
    assert_eq!(window.advance_past(["c"]), 6);
}

#[test]
fn a_fold_from_the_start_of_the_turn_is_counted_from_there() {
    let window = InPromptFrom::new(2, 40);
    assert_eq!(window.advance_past(["a", "b"]), 6);
    // The transcript folded again from its start, one message further:
    // three messages past the turn's start, not five past the last value.
    assert_eq!(window.advance_past(["a", "b", "c"]), 8);
}

#[test]
fn it_stops_at_the_turn_being_answered() {
    let window = InPromptFrom::new(16, 20);
    assert_eq!(window.advance_past(["a", "b", "c", "d", "e"]), 20);
}
