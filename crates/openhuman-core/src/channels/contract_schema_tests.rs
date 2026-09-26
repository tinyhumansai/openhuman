use super::*;

#[test]
fn converts_the_contract_schema_with_optional_types() {
    let s = contract_controller_schema("telegram_login_check");
    assert_eq!(s.namespace, "channels");
    assert_eq!(s.function, "telegram_login_check");
    let token = s.inputs.iter().find(|f| f.name == "linkToken").unwrap();
    assert!(token.required);
    assert!(matches!(token.ty, TypeSchema::String));

    let threads = contract_controller_schema("list_threads");
    let active = threads.inputs.iter().find(|f| f.name == "active").unwrap();
    assert!(!active.required);
    assert!(matches!(
        &active.ty,
        TypeSchema::Option(inner) if matches!(inner.as_ref(), TypeSchema::Bool)
    ));
}
