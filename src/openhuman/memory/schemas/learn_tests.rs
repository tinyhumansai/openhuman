use super::*;

#[test]
fn learn_schema_only_exposes_learn_all() {
    assert_eq!(FUNCTIONS, &["learn_all"]);
    assert_eq!(controllers().len(), 1);
}

#[test]
fn unknown_learn_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn learn_all_schema_has_optional_namespaces_input() {
    let schema = schema("learn_all").unwrap();
    assert_eq!(schema.inputs.len(), 1);
    assert_eq!(schema.inputs[0].name, "namespaces");
    assert!(!schema.inputs[0].required);
}
