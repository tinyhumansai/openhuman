use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
};

#[test]
fn update_schema_contracts_and_registration_are_preserved() {
    assert_eq!(all_controller_schemas().len(), 4);
    assert_eq!(all_registered_controllers().len(), 4);
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "version",
    );
    for function in ["version", "run", "check"] {
        let schema = schemas(function);
        expect_schema_identity(&schema, "update", function);
        assert!(schema.inputs.is_empty());
        assert!(!schema.outputs.is_empty());
    }
    let apply = schemas("apply");
    expect_schema_identity(&apply, "update", "apply");
    expect_schema_fields(
        &apply,
        &[
            ("download_url", Some(true)),
            ("asset_name", Some(true)),
            ("staging_dir", Some(false)),
        ],
    );
    let required: Vec<_> = apply
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert!(required.contains(&"download_url") && required.contains(&"asset_name"));
}
