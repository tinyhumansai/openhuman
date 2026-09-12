use super::*;

#[test]
fn all_schemas_returns_two() {
    assert_eq!(all_app_state_controller_schemas().len(), 2);
}

#[test]
fn all_controllers_returns_two() {
    assert_eq!(all_app_state_registered_controllers().len(), 2);
}

#[test]
fn snapshot_schema() {
    let s = app_state_schemas("snapshot");
    assert_eq!(s.namespace, "app_state");
    assert_eq!(s.function, "snapshot");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
}

#[test]
fn update_local_state_schema() {
    let s = app_state_schemas("update_local_state");
    assert_eq!(s.namespace, "app_state");
    assert_eq!(s.function, "update_local_state");
    assert_eq!(s.inputs.len(), 3);
    for input in &s.inputs {
        assert!(!input.required, "input '{}' should be optional", input.name);
    }
}

#[test]
fn unknown_function_returns_unknown() {
    let s = app_state_schemas("nonexistent");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "app_state");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_app_state_controller_schemas();
    let c = all_app_state_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
        assert_eq!(schema.namespace, ctrl.schema.namespace);
    }
}

#[test]
fn all_schemas_use_app_state_namespace() {
    for s in all_app_state_controller_schemas() {
        assert_eq!(s.namespace, "app_state");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn optional_json_helper() {
    let f = optional_json("key", "desc");
    assert_eq!(f.name, "key");
    assert!(!f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

#[test]
fn deserialize_update_local_state_params_empty() {
    let params: UpdateLocalStateParams =
        serde_json::from_value(serde_json::Value::Object(Map::new())).unwrap();
    assert!(params.encryption_key.is_none());
    assert!(params.onboarding_tasks.is_none());
}

#[test]
fn deserialize_update_local_state_params_with_values() {
    let mut m = Map::new();
    // encryption_key is Option<Option<String>> — sending a string value sets Some(Some("..."))
    m.insert("encryptionKey".into(), serde_json::json!("my-key"));
    let params: UpdateLocalStateParams =
        serde_json::from_value(serde_json::Value::Object(m)).unwrap();
    assert!(params.encryption_key.is_some());
}

#[test]
fn deserialize_update_local_state_params_with_null_clears_value() {
    let mut m = Map::new();
    m.insert("encryptionKey".into(), serde_json::Value::Null);
    m.insert("onboardingTasks".into(), serde_json::Value::Null);
    let params: UpdateLocalStateParams =
        serde_json::from_value(serde_json::Value::Object(m)).unwrap();
    assert_eq!(params.encryption_key, Some(None));
    assert!(matches!(params.onboarding_tasks, Some(None)));
}
