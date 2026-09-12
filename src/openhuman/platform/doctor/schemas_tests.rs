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
fn report_schema() {
    let s = schemas("report");
    assert_eq!(s.namespace, "doctor");
    assert_eq!(s.function, "report");
    assert!(s.inputs.is_empty());
}

#[test]
fn models_schema_has_optional_use_cache() {
    let s = schemas("models");
    assert_eq!(s.function, "models");
    let use_cache = s.inputs.iter().find(|f| f.name == "use_cache");
    assert!(use_cache.is_some_and(|f| !f.required));
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn read_optional_returns_none_for_missing() {
    let m = Map::new();
    let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_optional_returns_none_for_null() {
    let mut m = Map::new();
    m.insert("use_cache".into(), Value::Null);
    let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_optional_returns_some_for_value() {
    let mut m = Map::new();
    m.insert("use_cache".into(), Value::Bool(true));
    let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
    assert_eq!(result, Some(true));
}

#[test]
fn read_optional_errors_on_wrong_type() {
    let mut m = Map::new();
    m.insert("use_cache".into(), Value::String("yes".into()));
    let err = read_optional::<bool>(&m, "use_cache").unwrap_err();
    assert!(err.contains("invalid"));
}
