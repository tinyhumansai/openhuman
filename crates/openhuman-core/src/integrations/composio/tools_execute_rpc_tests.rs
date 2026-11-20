use super::*;

use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
};

#[test]
fn composio_execute_wire_contract_is_preserved() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "composio_execute",
    );
    assert_eq!(all_controller_schemas().len(), 1);
    let schema = schemas("tools_composio_execute");
    expect_schema_identity(&schema, "tools", "composio_execute");
    expect_schema_fields(&schema, &[("action", Some(true))]);
}

#[test]
fn unknown_tools_function_resolves_to_the_unknown_schema() {
    assert_eq!(schemas("tools_web_search").function, "unknown");
}
