use super::*;

#[test]
fn raw_devices_security_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["devices"],
        group: DomainGroup::Security,
        methods: &[
            ("devices.create_pairing", "openhuman.devices_create_pairing"),
            ("devices.import_legacy", "openhuman.devices_import_legacy"),
            ("devices.list", "openhuman.devices_list"),
            ("devices.revoke", "openhuman.devices_revoke"),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::security::devices::devices_parameter_contracts(),
    )
    .expect("raw Devices entries retain their ordered wire contracts");
    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "devices")
        .collect();
    use TypeSchema::{Array, Bool, Option, Ref, String};
    let create = &entries[0].controller.schema;
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &create.inputs,
            &[("label", Option(Box::new(String)), false)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &create.outputs,
            &[
                ("channel_id", String, true),
                ("pairing_token", String, true),
                ("core_pubkey", String, true),
                ("rpc_url", Option(Box::new(String)), false),
                ("expires_at", String, true),
            ]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[2].controller.schema.outputs,
            &[("devices", Array(Box::new(Ref("PairedDevice"))), true)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[3].controller.schema.inputs,
            &[("channel_id", String, true)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[3].controller.schema.outputs,
            &[("success", Bool, true)]
        ),
        Ok(())
    );
}

#[test]
fn raw_devices_decoders_ignore_extra_fields_and_list_parameters() {
    use serde_json::json;

    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "devices")
        .map(|row| row.controller.clone())
        .collect();

    entries[0]
        .decode_params(Map::from_iter([
            ("label".into(), json!("  preserve label  ")),
            ("unknown_scalar".into(), json!(true)),
            ("unknown_nested".into(), json!({"ignored": [1, null]})),
        ]))
        .expect("create_pairing ignores undeclared fields");
    entries[2]
        .decode_params(Map::from_iter([
            ("anything".into(), json!({"ignored": true})),
            ("label".into(), json!(false)),
        ]))
        .expect("list ignores its entire parameter map");
    entries[3]
        .decode_params(Map::from_iter([
            ("channel_id".into(), json!("  raw channel id  ")),
            ("extra".into(), json!(false)),
        ]))
        .expect("revoke ignores undeclared fields");
}
