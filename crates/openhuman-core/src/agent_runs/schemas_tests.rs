use super::*;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::core::TypeSchema;
use serde_json::json;

#[test]
fn controller_schema_inventory_is_stable() {
    let schemas = all_controller_schemas();
    let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
    assert_eq!(
        functions,
        vec![
            "chat",
            "chat_simple",
            "server_status",
            "list_definitions",
            "get_definition",
            "reload_definitions",
            "triage_evaluate",
            "registry_snapshot",
            "context_breakdown",
        ]
    );
    assert_eq!(schemas.len(), all_registered_controllers().len());
}

#[test]
fn schemas_expose_expected_inputs() {
    let chat = schemas("chat");
    assert_eq!(chat.namespace, "agent");
    assert_eq!(chat.inputs.len(), 5);
    assert!(matches!(chat.inputs[1].ty, TypeSchema::Option(_)));
    assert!(chat
        .inputs
        .iter()
        .any(|input| input.name == "thread_id" && !input.required));
}

#[test]
fn deserialize_params_preserves_chat_overrides_and_missing_definition_error() {
    let params = Map::from_iter([
        ("message".into(), Value::String("hello".into())),
        ("model_override".into(), Value::String("gpt".into())),
        ("temperature".into(), json!(0.2)),
    ]);
    let parsed = deserialize_params::<AgentChatParams>(params).expect("valid params");
    assert_eq!(parsed.message, "hello");
    assert_eq!(parsed.model_override.as_deref(), Some("gpt"));
    assert_eq!(parsed.temperature, Some(0.2));

    let err = deserialize_params::<GetDefinitionParams>(Map::new()).expect_err("missing id");
    assert!(err.contains("invalid params"));
}

#[tokio::test]
async fn reload_and_definition_handlers_cover_missing_registry_paths() {
    let reload = handle_reload_definitions(DispatchCtx::process_default(), Map::new())
        .await
        .expect("reload handler should always succeed");
    assert_eq!(reload.get("status").and_then(Value::as_str), Some("noop"));
    assert!(reload
        .get("note")
        .and_then(Value::as_str)
        .unwrap()
        .contains("Restart"));

    let list_result = handle_list_definitions(DispatchCtx::process_default(), Map::new()).await;
    match list_result {
        Ok(value) => assert!(value.get("definitions").and_then(Value::as_array).is_some()),
        Err(err) => assert!(err.contains("AgentDefinitionRegistry not initialised")),
    }

    let get_err = handle_get_definition(
        DispatchCtx::process_default(),
        Map::from_iter([(
            "id".into(),
            Value::String("__definitely_missing_definition__".into()),
        )]),
    )
    .await
    .expect_err("missing or unknown definition should error");
    assert!(
        get_err.contains("AgentDefinitionRegistry not initialised")
            || get_err.contains("not found")
    );
}

#[test]
fn to_json_maps_outcome() {
    let value = to_json(Outcome::new(json!({ "ok": true }), Vec::new())).expect("json outcome");
    assert_eq!(value["ok"], json!(true));
}
