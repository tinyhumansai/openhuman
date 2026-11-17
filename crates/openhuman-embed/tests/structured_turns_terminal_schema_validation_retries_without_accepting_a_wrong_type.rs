mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn terminal_schema_validation_retries_without_accepting_a_wrong_type() {
    scenarios::terminal_schema_validation_retries_without_accepting_a_wrong_type();
}
