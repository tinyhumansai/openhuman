use super::*;

#[test]
fn inference_catalog_counts_match_and_nonempty() {
    let declared = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(declared.len(), registered.len());
    assert!(declared.len() >= 19);
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
    assert!(functions.contains(&"openai_oauth_start"));
    assert!(functions.contains(&"openai_oauth_complete"));
    assert!(functions.contains(&"openai_oauth_import_codex_cli"));
    assert!(functions.contains(&"openai_oauth_status"));
    assert!(functions.contains(&"openai_oauth_disconnect"));
    assert!(functions.contains(&"prompt"));
    assert!(functions.contains(&"vision_prompt"));
    // embed moved to the embeddings domain (openhuman.embeddings_embed)
    assert!(!functions.contains(&"embed"));
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
fn inference_openai_oauth_schemas_are_registered_with_expected_shapes() {
    let registered: Vec<&str> = all_registered_controllers()
        .into_iter()
        .map(|controller| controller.schema.function)
        .collect();
    for function in [
        "openai_oauth_start",
        "openai_oauth_complete",
        "openai_oauth_import_codex_cli",
        "openai_oauth_status",
        "openai_oauth_disconnect",
    ] {
        assert!(registered.contains(&function), "missing {function}");
        let schema = schemas(function);
        assert_eq!(schema.namespace, "inference");
        assert_eq!(schema.function, function);
        assert!(!schema.description.is_empty());
        assert!(!schema.outputs.is_empty());
    }

    let complete = schemas("openai_oauth_complete");
    assert_eq!(complete.inputs.len(), 1);
    assert_eq!(complete.inputs[0].name, "callback_url");
    assert!(complete.inputs[0].required);

    assert!(schemas("openai_oauth_start").inputs.is_empty());
    assert!(schemas("openai_oauth_import_codex_cli").inputs.is_empty());
    assert!(schemas("openai_oauth_status").inputs.is_empty());
    assert!(schemas("openai_oauth_disconnect").inputs.is_empty());
}

#[tokio::test]
async fn inference_openai_oauth_complete_handler_rejects_invalid_params() {
    let params = Map::from_iter([("callback_url".to_string(), Value::Bool(true))]);
    let err = handle_inference_openai_oauth_complete(params)
        .await
        .expect_err("invalid params");
    assert!(err.contains("invalid params"));
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
