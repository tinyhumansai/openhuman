use super::*;
#[test]
fn registration_entry_points_are_empty() {
    assert!(all_x402_registered_controllers().is_empty());
    assert!(all_x402_controller_schemas().is_empty());
}
