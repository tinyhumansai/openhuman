use super::*;

#[test]
fn all_schemas_returns_two() {
    assert_eq!(all_controller_schemas().len(), 2);
}

#[test]
fn all_controllers_returns_two() {
    assert_eq!(all_registered_controllers().len(), 2);
}

#[test]
fn encrypt_schema_requires_plaintext() {
    let s = schemas("encrypt_secret");
    assert_eq!(s.namespace, "encrypt");
    assert_eq!(s.function, "secret");
    assert_eq!(s.inputs.len(), 1);
    assert!(s.inputs[0].required);
    assert_eq!(s.inputs[0].name, "plaintext");
}

#[test]
fn decrypt_schema_requires_ciphertext() {
    let s = schemas("decrypt_secret");
    assert_eq!(s.namespace, "decrypt");
    assert_eq!(s.function, "secret");
    assert_eq!(s.inputs.len(), 1);
    assert!(s.inputs[0].required);
    assert_eq!(s.inputs[0].name, "ciphertext");
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "encryption");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn read_required_parses_string() {
    let mut m = Map::new();
    m.insert("key".into(), Value::String("value".into()));
    let result: String = read_required(&m, "key").unwrap();
    assert_eq!(result, "value");
}

#[test]
fn read_required_errors_on_missing_key() {
    let m = Map::new();
    let err = read_required::<String>(&m, "key").unwrap_err();
    assert!(err.contains("missing required param"));
}

#[test]
fn read_required_errors_on_wrong_type() {
    let mut m = Map::new();
    m.insert("key".into(), Value::Bool(true));
    let err = read_required::<String>(&m, "key").unwrap_err();
    assert!(err.contains("invalid"));
}
