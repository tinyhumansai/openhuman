use super::{all_controller_schemas, all_registered_controllers, schemas, string_param};
use serde_json::{Map, Value};

#[test]
fn every_schema_is_in_the_modules_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(schema.namespace, "modules");
        assert_ne!(
            schema.function, "unknown",
            "an advertised function fell through to the unknown arm"
        );
    }
}

#[test]
fn registered_controllers_match_the_advertised_schemas() {
    // Two lists that must agree: one drives `/schema`, the other dispatch.
    let advertised: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    let registered: Vec<&str> = all_registered_controllers()
        .iter()
        .map(|c| c.schema.function)
        .collect();
    assert_eq!(advertised, registered);
}

#[test]
fn an_unknown_function_falls_through_to_the_unknown_arm() {
    assert_eq!(schemas("nope").function, "unknown");
}

#[test]
fn a_blank_or_missing_id_is_not_a_parameter() {
    let mut params = Map::new();
    assert_eq!(string_param(&params, "id"), None);
    params.insert("id".to_string(), Value::String("   ".to_string()));
    assert_eq!(string_param(&params, "id"), None);
    params.insert("id".to_string(), Value::String(" tinydocs ".to_string()));
    assert_eq!(string_param(&params, "id"), Some("tinydocs".to_string()));
}
