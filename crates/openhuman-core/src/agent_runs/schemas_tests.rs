use super::*;
use crate::core::runtime::DispatchCtx;
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

    let triage = schemas("triage_evaluate");
    assert_eq!(triage.inputs.len(), 7);
    assert!(triage
        .inputs
        .iter()
        .any(|input| input.name == "payload" && input.required));
    assert!(triage
        .inputs
        .iter()
        .any(|input| input.name == "dry_run" && !input.required));
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
async fn registry_snapshot_omits_retired_graphs() {
    let result = handle_registry_snapshot(DispatchCtx::process_default(), Map::new())
        .await
        .expect("registry snapshot is available");
    let components = result
        .get("components")
        .and_then(Value::as_array)
        .expect("component snapshot");
    let ids: Vec<_> = components
        .iter()
        .filter_map(|component| component.get("id").and_then(Value::as_str))
        .collect();
    assert!(
        !ids.contains(&"delegation"),
        "snapshot includes the retired delegation graph: {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("agent_teams:")),
        "snapshot includes a retired team graph: {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("workflow_runs:")),
        "snapshot includes a retired workflow graph: {ids:?}"
    );
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

#[tokio::test]
async fn triage_handler_rejects_unknown_source_and_to_json_maps_outcome() {
    let err = handle_triage_evaluate(
        DispatchCtx::process_default(),
        Map::from_iter([
            ("source".into(), Value::String("__unknown_source__".into())),
            ("display_label".into(), Value::String("lbl".into())),
            ("payload".into(), json!({})),
        ]),
    )
    .await
    .expect_err("unsupported source should fail before runtime dispatch");
    assert!(err.contains("unsupported trigger source"));

    let value = to_json(Outcome::new(json!({ "ok": true }), Vec::new())).expect("json outcome");
    assert_eq!(value["ok"], json!(true));
}
