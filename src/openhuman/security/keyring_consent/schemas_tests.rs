use super::*;

#[test]
fn schemas_and_controllers_match() {
    let s = all_keyring_consent_controller_schemas();
    let c = all_keyring_consent_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
        assert_eq!(schema.namespace, ctrl.schema.namespace);
    }
}

#[test]
fn all_schemas_use_keyring_consent_namespace() {
    for s in all_keyring_consent_controller_schemas() {
        assert_eq!(s.namespace, "keyring_consent");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn decide_schema_requires_mode() {
    let s = keyring_consent_schema("decide");
    assert_eq!(s.inputs.len(), 1);
    assert!(s.inputs[0].required);
    assert_eq!(s.inputs[0].name, "mode");
}
