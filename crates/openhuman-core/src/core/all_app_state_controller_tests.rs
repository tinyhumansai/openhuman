use super::*;

#[test]
fn raw_app_state_desktop_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["app_state"],
        group: DomainGroup::Desktop,
        methods: &[
            ("app_state.snapshot", "openhuman.app_state_snapshot"),
            (
                "app_state.update_local_state",
                "openhuman.app_state_update_local_state",
            ),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::desktop::app_state::app_state_parameter_contracts(),
    )
    .expect("raw AppState entries retain their ordered wire contracts");
    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "app_state")
        .collect();
    let result = [("result", TypeSchema::Json, true)];
    assert_eq!(
        crate::core::all::verify_wire_fields(&entries[0].controller.schema.inputs, &[]),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(&entries[0].controller.schema.outputs, &result),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[1].controller.schema.inputs,
            &[
                ("encryptionKey", TypeSchema::Json, false),
                ("onboardingTasks", TypeSchema::Json, false),
                ("keyringConsent", TypeSchema::Json, false),
            ]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(&entries[1].controller.schema.outputs, &result),
        Ok(())
    );
}

#[test]
fn raw_app_state_decoders_ignore_unknown_outer_fields() {
    use serde_json::json;

    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "app_state")
        .map(|row| row.controller.clone())
        .collect();
    let snapshot = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "app_state.snapshot")
        .expect("raw snapshot entry");
    snapshot
        .decode_params(Map::from_iter([
            ("unknown_scalar".into(), json!(17)),
            ("unknown_nested".into(), json!({"ignored": [true, null]})),
        ]))
        .expect("snapshot ignores its entire parameter map");

    let update = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "app_state.update_local_state")
        .expect("raw update entry");
    update
        .decode_params(Map::from_iter([
            ("encryptionKey".into(), json!("unchanged")),
            ("ignored_scalar".into(), json!(false)),
            ("ignored_nested".into(), json!({"extra": 1})),
        ]))
        .expect("update ignores undeclared outer keys");
}
