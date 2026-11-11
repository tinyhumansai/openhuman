use super::*;

#[test]
fn raw_encryption_security_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["encrypt", "decrypt"],
        group: DomainGroup::Security,
        methods: &[
            ("encrypt.secret", "openhuman.encrypt_secret"),
            ("decrypt.secret", "openhuman.decrypt_secret"),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::security::encryption::encryption_parameter_contracts(),
    )
    .expect("raw Encryption entries retain their ordered wire contracts");
}

#[test]
fn bound_encryption_decoders_preserve_required_strings_and_ignore_extra_fields() {
    use serde_json::json;

    let entries = registry()
        .iter()
        .filter(|row| matches!(row.controller.schema.namespace, "encrypt" | "decrypt"))
        .map(|row| row.controller.clone())
        .collect::<Vec<_>>();

    for (method, field) in [
        ("encrypt.secret", "plaintext"),
        ("decrypt.secret", "ciphertext"),
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry.schema.method_name() == method)
            .expect("raw encryption entry exists");
        let mut params = Map::from_iter([(field.into(), json!("unchanged"))]);
        params.insert("extra_scalar".into(), json!(true));
        params.insert("extra_nested".into(), json!({"ignored":true}));
        entry
            .decode_params(params)
            .unwrap_or_else(|error| panic!("{method} decoder rejected extra fields: {error}"));

        assert_eq!(
            entry.decode_params(Map::new()).unwrap_err(),
            format!("missing required param '{field}'"),
            "{method} missing-field error"
        );
        for value in [json!(null), json!(true), json!(42)] {
            let params = Map::from_iter([(field.into(), value)]);
            let error = entry
                .decode_params(params)
                .expect_err("non-string secret input is rejected");
            assert!(
                error.starts_with(&format!("invalid '{field}':")),
                "{method} invalid-field prefix: {error}"
            );
        }
    }
}
