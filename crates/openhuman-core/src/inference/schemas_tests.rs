use super::*;
use crate::core::all::assert_schema_controller_parity;
use crate::core::runtime::DispatchCtx;

use crate::core::TypeSchema;
#[test]
fn inference_schema_function_names_are_stable() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "inference");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "status",
    );
    let functions: Vec<&str> = all_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();
    assert!(functions.contains(&"status"));
    assert!(functions.contains(&"get_client_config"));
    assert!(functions.contains(&"update_model_settings"));
    assert!(functions.contains(&"update_local_settings"));
    assert!(functions.contains(&"list_models"));
    // Local model presets, device profiling and tiering were removed: the
    // user runs their own local runtime and picks models there.
    assert!(!functions.contains(&"device_profile"));
    assert!(!functions.contains(&"presets"));
    assert!(!functions.contains(&"apply_preset"));
    assert!(functions.contains(&"diagnostics"));
    assert!(!functions.contains(&"openai_oauth_start"));
    assert!(!functions.contains(&"openai_oauth_complete"));
    assert!(!functions.contains(&"openai_oauth_import_codex_cli"));
    assert!(!functions.contains(&"openai_oauth_status"));
    assert!(!functions.contains(&"openai_oauth_disconnect"));
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
fn inference_update_local_settings_schema_allows_json_base_url() {
    let schema = schemas("update_local_settings");
    let field = schema
        .inputs
        .iter()
        .find(|field| field.name == "base_url")
        .expect("base_url field");
    match &field.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::Json)),
        other => panic!("expected Option<Json>, got {other:?}"),
    }
}

#[test]
fn inference_update_local_settings_schema_accepts_api_key() {
    // Regression: the OMLX Bearer key flows through `inference.update_local_settings`.
    // The serde struct + handler carried `api_key`, but the ControllerSchema inputs
    // did not — so `validate_params` rejected it as "unknown param 'api_key'" before
    // dispatch. The field must be declared here for the RPC to accept it.
    let schema = schemas("update_local_settings");
    let field = schema
        .inputs
        .iter()
        .find(|field| field.name == "api_key")
        .expect("api_key field must be declared so validate_params accepts it");
    match &field.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::String)),
        other => panic!("expected Option<String>, got {other:?}"),
    }
}

#[tokio::test]
async fn inference_status_handler_returns_cli_json() {
    let value = handle_inference_status(DispatchCtx::process_default(), Map::new())
        .await
        .expect("handler value");
    assert!(value.get("result").is_some() || value.get("logs").is_some());
}

#[tokio::test]
async fn inference_prompt_handler_rejects_invalid_shape() {
    let params = Map::from_iter([("prompt".to_string(), Value::Bool(true))]);
    let err = handle_inference_prompt(DispatchCtx::process_default(), params)
        .await
        .expect_err("invalid params");
    assert!(err.contains("invalid params"));
}
