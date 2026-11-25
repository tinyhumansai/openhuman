use super::*;

// Fixed expectations carried over from the pre-existing memory specification
// tests and wallet setup regression. Never derive this oracle from live schemas.
fn independent_registered_schema_contract(schema: &ControllerSchema) -> Result<(), String> {
    let required: &[&str] = match (schema.namespace, schema.function) {
        ("memory", "engine_set") => &["engine"],
        ("memory", "recall") => &["question"],
        ("memory", "fetch" | "brain_search") => &["query"],
        ("memory", "learn") => &["text"],
        ("memory", "forget") => &["ids"],
        ("memory", "sources_add") => &["kind", "target"],
        ("memory", "sources_remove") => &["id"],
        ("memory", "import_start") => &["consent"],
        ("memory", "brain_forget") => &["source"],
        ("memory", "policy_set") => &[],
        ("wallet", "setup") => return required_fields_contract(schema, &["encryptedMnemonic"]),
        _ => return Ok(()),
    };
    required_input_contract(schema, required)
}

#[test]
fn registered_memory_contract_rejects_missing_or_optional_question() {
    let actual = all_registered_controllers()
        .into_iter()
        .map(|controller| controller.schema)
        .find(|schema| schema.method_name() == "memory.recall")
        .expect("memory.recall is registered");
    assert_eq!(independent_registered_schema_contract(&actual), Ok(()));
    for remove in [true, false] {
        let mut broken = actual.clone();
        if remove {
            broken.inputs.retain(|input| input.name != "question");
        } else {
            broken
                .inputs
                .iter_mut()
                .find(|input| input.name == "question")
                .expect("real recall schema declares question")
                .required = false;
        }
        assert!(independent_registered_schema_contract(&broken)
            .unwrap_err()
            .contains("memory.recall required inputs mismatch"));
    }
}

#[test]
fn registered_controller_schemas_are_consistent_and_serializable() {
    use std::collections::BTreeSet;

    let controllers = builtin_controller_snapshot();
    assert!(!controllers.is_empty(), "the controller registry is empty");
    let mut methods = BTreeSet::new();

    for controller in controllers {
        let schema = &controller.schema;
        let method = schema.method_name();
        assert!(methods.insert(method.clone()), "duplicate method {method}");
        assert!(
            !schema.namespace.trim().is_empty(),
            "{method} has no namespace"
        );
        assert!(
            !schema.function.trim().is_empty(),
            "{method} has no function name"
        );
        assert!(
            !schema.description.trim().is_empty(),
            "{method} has no description"
        );
        assert!(!schema.outputs.is_empty(), "{method} has no output fields");

        assert_eq!(
            independent_registered_schema_contract(schema),
            Ok(()),
            "{method} independent schema contract"
        );

        let serialized = serde_json::to_value(schema).expect("controller schema serializes");
        let serialized_inputs = serialized["inputs"]
            .as_array()
            .unwrap_or_else(|| panic!("{method} inputs are not serialized as an array"));
        assert_eq!(
            serialized_inputs.len(),
            schema.inputs.len(),
            "{method} serialized input count differs from its registered schema"
        );
        let mut params = Map::new();
        for input in &schema.inputs {
            let matching = serialized_inputs
                .iter()
                .find(|field| field["name"] == input.name);
            let matching = matching.unwrap_or_else(|| {
                panic!(
                    "{method} input `{}` is missing from serialized schema",
                    input.name
                )
            });
            assert_eq!(
                matching["required"], input.required,
                "{method} input `{}` requiredness differs in serialized schema",
                input.name
            );
            if input.required {
                // `validate_params` accepts null for every declared type, so this
                // safely checks required-key acceptance without executing a handler.
                params.insert(input.name.to_owned(), Value::Null);
            }
        }
        assert!(
            validate_params(schema, &params).is_ok(),
            "{method} rejects a map containing all required schema inputs"
        );
        for input in schema.inputs.iter().filter(|input| input.required) {
            let mut missing = params.clone();
            missing.remove(input.name);
            assert!(
                validate_params(schema, &missing).is_err(),
                "{method} accepts params without required input `{}`",
                input.name
            );
        }
    }
}

#[cfg(feature = "web3")]
#[test]
fn registered_wallet_schema_contract_rejects_missing_encrypted_mnemonic() {
    let actual = all_registered_controllers()
        .into_iter()
        .map(|controller| controller.schema)
        .find(|schema| schema.method_name() == "wallet.setup")
        .expect("wallet.setup is registered with web3 enabled");
    assert_eq!(independent_registered_schema_contract(&actual), Ok(()));

    let mut schema = actual;
    schema
        .inputs
        .retain(|input| input.name != "encryptedMnemonic");

    assert!(independent_registered_schema_contract(&schema)
        .unwrap_err()
        .contains("is missing required input `encryptedMnemonic`"));
}

/// Built-in/internal snapshots avoid ambient DomainSet filtering and global
/// hosted-extension mutation. Hosted/RPC contracts belong to their own crates.
fn builtin_controller_snapshot() -> Vec<ControllerEntry> {
    registry()
        .iter()
        .chain(internal_registry())
        .map(|grouped| grouped.controller.clone())
        .collect()
}

#[test]
#[cfg(not(any(feature = "modules", feature = "mcp")))]
fn optional_domain_controller_methods_are_absent_when_features_are_disabled() {
    let methods = builtin_controller_snapshot()
        .into_iter()
        .map(|controller| controller.schema.method_name())
        .collect::<Vec<_>>();
    assert!(!methods
        .iter()
        .any(|method| method == "mcp_clients.registry_get"));
    assert!(!methods
        .iter()
        .any(|method| method == "modules.computer_status"));
}
