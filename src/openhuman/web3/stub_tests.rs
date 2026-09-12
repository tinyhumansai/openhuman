use super::*;

#[test]
fn registration_entry_points_are_empty() {
    assert!(all_web3_registered_controllers().is_empty());
    assert!(all_web3_controller_schemas().is_empty());
}

#[test]
fn agent_tools_are_absent() {
    assert!(all_web3_agent_tools().is_empty());
}
