mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn successful_turn_preserves_reported_charges_and_invalid_buyer_cost_is_unknown() {
    scenarios::successful_turn_preserves_reported_charges_and_invalid_buyer_cost_is_unknown();
}
