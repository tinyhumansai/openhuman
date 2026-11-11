mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn a_tool_loop_ends_in_a_structured_answer() {
    scenarios::a_tool_loop_ends_in_a_structured_answer();
}
