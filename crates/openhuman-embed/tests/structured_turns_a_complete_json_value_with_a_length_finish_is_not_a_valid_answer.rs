mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn a_complete_json_value_with_a_length_finish_is_not_a_valid_answer() {
    scenarios::a_complete_json_value_with_a_length_finish_is_not_a_valid_answer();
}
