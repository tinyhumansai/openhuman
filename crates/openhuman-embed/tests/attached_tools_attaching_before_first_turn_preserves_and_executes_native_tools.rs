mod common;
#[path = "support/attached_tools.rs"]
mod scenarios;

#[test]
fn attaching_before_first_turn_preserves_and_executes_native_tools() {
    scenarios::attaching_before_first_turn_preserves_and_executes_native_tools();
}
