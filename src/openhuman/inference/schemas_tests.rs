use super::*;

#[test]
fn inference_catalog_counts_match_and_nonempty() {
    let declared = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(declared.len(), registered.len());
    assert!(declared.len() >= 16);
}

#[test]
fn inference_schemas_use_inference_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(
            schema.namespace, "inference",
            "function {}",
            schema.function
        );
        assert!(!schema.description.is_empty());
        assert!(!schema.outputs.is_empty());
    }
}

#[test]
fn inference_schema_function_names_are_stable() {
    let functions: Vec<&str> = all_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();
    assert!(functions.contains(&"status"));
    assert!(functions.contains(&"get_client_config"));
    assert!(functions.contains(&"update_model_settings"));
    assert!(functions.contains(&"update_local_settings"));
    assert!(functions.contains(&"list_models"));
    assert!(functions.contains(&"device_profile"));
    assert!(functions.contains(&"presets"));
    assert!(functions.contains(&"apply_preset"));
    assert!(functions.contains(&"diagnostics"));
    assert!(functions.contains(&"prompt"));
    assert!(functions.contains(&"vision_prompt"));
    assert!(functions.contains(&"embed"));
    assert!(functions.contains(&"chat"));
    assert!(!functions.contains(&"should_send_gif"));
    assert!(!functions.contains(&"tenor_search"));
}

#[test]
fn inference_prompt_schema_reuses_local_ai_shape_with_new_namespace() {
    let schema = schemas("prompt");
    assert_eq!(schema.namespace, "inference");
    assert_eq!(schema.function, "prompt");
    assert!(schema.inputs.iter().any(|field| field.name == "prompt"));
    assert!(schema.inputs.iter().any(|field| field.name == "max_tokens"));
}

#[test]
fn inference_chat_schema_requires_messages() {
    let schema = schemas("chat");
    assert_eq!(schema.namespace, "inference");
    assert_eq!(schema.function, "chat");
    assert!(schema
        .inputs
        .iter()
        .any(|field| field.name == "messages" && field.required));
}

#[test]
fn inference_unknown_schema_panics() {
    let panic = std::panic::catch_unwind(|| schemas("no_such_function"));
    assert!(panic.is_err());
}

#[tokio::test]
async fn inference_status_handler_returns_cli_json() {
    let value = handle_inference_status(Map::new())
        .await
        .expect("handler value");
    assert!(value.get("result").is_some() || value.get("logs").is_some());
}

#[tokio::test]
async fn inference_prompt_handler_rejects_invalid_shape() {
    let params = Map::from_iter([("prompt".to_string(), Value::Bool(true))]);
    let err = handle_inference_prompt(params)
        .await
        .expect_err("invalid params");
    assert!(err.contains("invalid params"));
}
