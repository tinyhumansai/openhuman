use super::*;
use serde_json::{json, Map};

fn keyring_consent_entries() -> Vec<ControllerEntry> {
    registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "keyring_consent")
        .map(|row| row.controller.clone())
        .collect()
}

#[test]
fn raw_keyring_consent_registry_matches_its_independent_contract() {
    let contract = registry_test_support::BoundFamilyWireContract {
        namespaces: &["keyring_consent"],
        group: DomainGroup::Security,
        methods: &[
            ("keyring_consent.status", "openhuman.keyring_consent_status"),
            ("keyring_consent.decide", "openhuman.keyring_consent_decide"),
            (
                "keyring_consent.retry_probe",
                "openhuman.keyring_consent_retry_probe",
            ),
            (
                "keyring_consent.respond_operation",
                "openhuman.keyring_consent_respond_operation",
            ),
        ],
    };
    registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::security::keyring_consent::keyring_consent_parameter_contracts(),
    )
    .expect("raw Keyring Consent entries retain their ordered wire contracts");
}

#[test]
fn raw_keyring_consent_decoders_pin_their_safe_wire_boundary() {
    let entries = keyring_consent_entries();
    let status = entries
        .iter()
        .find(|entry| entry.schema.function == "status")
        .expect("raw status entry");
    let retry_probe = entries
        .iter()
        .find(|entry| entry.schema.function == "retry_probe")
        .expect("raw retry_probe entry");
    for entry in [status, retry_probe] {
        entry
            .decode_params(Map::from_iter([(
                "ignored".into(),
                json!({"nested": true}),
            )]))
            .unwrap_or_else(|error| {
                panic!(
                    "{} ignores its entire parameter map: {error}",
                    entry.schema.method_name()
                )
            });
    }

    let decide = entries
        .iter()
        .find(|entry| entry.schema.function == "decide")
        .expect("raw decide entry");
    for mode in [
        "local_encrypted",
        "declined",
        "",
        "unknown",
        " local_encrypted ",
    ] {
        decide
            .decode_params(Map::from_iter([
                ("mode".into(), json!(mode)),
                ("ignored".into(), json!({"extra": true})),
            ]))
            .unwrap_or_else(|error| panic!("string mode {mode:?} is passed unchanged: {error}"));
    }
    assert_eq!(
        decide.decode_params(Map::new()).unwrap_err(),
        "missing required param 'mode'"
    );
    for value in [json!(null), json!(false), json!(0.5), json!([]), json!({})] {
        assert_eq!(
            decide
                .decode_params(Map::from_iter([("mode".into(), value)]))
                .unwrap_err(),
            "missing required param 'mode'"
        );
    }
}

#[test]
fn raw_keyring_consent_decide_rejects_before_persistence() {
    let decide = keyring_consent_entries()
        .into_iter()
        .find(|entry| entry.schema.function == "decide")
        .expect("raw decide entry");
    let missing =
        futures::executor::block_on(decide.invoke(DispatchCtx::unbound(), Map::new())).unwrap_err();
    assert_eq!(missing, "missing required param 'mode'");
    let wrong_type = futures::executor::block_on(decide.invoke(
        DispatchCtx::unbound(),
        Map::from_iter([("mode".into(), json!(false))]),
    ))
    .unwrap_err();
    assert_eq!(wrong_type, "missing required param 'mode'");
    let unknown = futures::executor::block_on(decide.invoke(
        DispatchCtx::unbound(),
        Map::from_iter([("mode".into(), json!("unknown"))]),
    ))
    .unwrap_err();
    assert_eq!(
        unknown,
        "invalid mode 'unknown': expected 'local_encrypted' or 'declined'"
    );
}

#[test]
fn raw_keyring_consent_wire_fields_are_exact() {
    let entries = keyring_consent_entries();
    assert_eq!(entries.len(), 3);
    for entry in [&entries[0], &entries[2]] {
        assert_eq!(
            crate::core::all::verify_wire_fields(&entry.schema.inputs, &[]),
            Ok(()),
            "{} inputs",
            entry.schema.method_name()
        );
        assert_eq!(
            crate::core::all::verify_wire_fields(
                &entry.schema.outputs,
                &[("result", TypeSchema::Json, true)]
            ),
            Ok(()),
            "{} outputs",
            entry.schema.method_name()
        );
    }
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[1].schema.inputs,
            &[("mode", TypeSchema::String, true)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[1].schema.outputs,
            &[("result", TypeSchema::Json, true)]
        ),
        Ok(())
    );
}
