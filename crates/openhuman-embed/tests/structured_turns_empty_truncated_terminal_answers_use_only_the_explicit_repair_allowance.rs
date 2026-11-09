mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn empty_truncated_terminal_answers_use_only_the_explicit_repair_allowance() {
    scenarios::empty_truncated_terminal_answers_use_only_the_explicit_repair_allowance();
}
