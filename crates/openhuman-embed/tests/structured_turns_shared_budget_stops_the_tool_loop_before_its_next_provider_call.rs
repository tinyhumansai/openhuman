mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn shared_budget_stops_the_tool_loop_before_its_next_provider_call() {
    scenarios::shared_budget_stops_the_tool_loop_before_its_next_provider_call();
}
