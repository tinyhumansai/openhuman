use super::*;
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
            "graph_topologies",
            "registry_snapshot",
        ]
    );
    assert_eq!(schemas.len(), all_registered_controllers().len());
}

#[test]
fn schemas_expose_expected_inputs_and_unknown_fallback() {
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

    let unknown = schemas("nope");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.outputs[0].name, "error");
}

#[test]
fn deserialize_params_and_helpers_cover_success_and_failure_paths() {
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

    assert!(required_string("id", "x").required);
    assert!(matches!(
        optional_string("id", "x").ty,
        TypeSchema::Option(_)
    ));
    assert!(matches!(
        optional_f64("temperature", "x").ty,
        TypeSchema::Option(_)
    ));
    assert!(matches!(json_output("result", "x").ty, TypeSchema::Json));
}

#[tokio::test]
async fn graph_topologies_handler_exports_structural_reports() {
    let result = handle_graph_topologies(Map::new())
        .await
        .expect("topology export is infallible");
    let graphs = result
        .get("graphs")
        .and_then(Value::as_array)
        .expect("graphs array");
    assert!(!graphs.is_empty(), "at least one custom graph is exported");
    for g in graphs {
        assert!(g.get("name").and_then(Value::as_str).is_some());
        assert_eq!(g.get("ok").and_then(Value::as_bool), Some(true));
        assert!(g
            .get("mermaid")
            .and_then(Value::as_str)
            .unwrap()
            .contains("flowchart"));
        assert!(
            g.get("topology").map(|t| !t.is_null()).unwrap_or(false),
            "topology embeds as structured JSON"
        );
    }
    let names: Vec<_> = graphs
        .iter()
        .filter_map(|g| g.get("name").and_then(Value::as_str))
        .collect();
    assert!(names.contains(&"delegation"), "saw {names:?}");
    assert!(names.contains(&"workflow_runs:scheduler"), "saw {names:?}");
    let agents = result
        .get("agents")
        .and_then(Value::as_array)
        .expect("agents array");
    let orchestrator = agents
        .iter()
        .find(|agent| agent.get("id").and_then(Value::as_str) == Some("orchestrator"))
        .expect("orchestrator graph resolution");
    assert_eq!(
        orchestrator.get("graph").and_then(Value::as_str),
        Some("default")
    );
}

#[tokio::test]
async fn reload_and_definition_handlers_cover_missing_registry_paths() {
    let reload = handle_reload_definitions(Map::new())
        .await
        .expect("reload handler should always succeed");
    assert_eq!(reload.get("status").and_then(Value::as_str), Some("noop"));
    assert!(reload
        .get("note")
        .and_then(Value::as_str)
        .unwrap()
        .contains("Restart"));

    let list_result = handle_list_definitions(Map::new()).await;
    match list_result {
        Ok(value) => assert!(value.get("definitions").and_then(Value::as_array).is_some()),
        Err(err) => assert!(err.contains("AgentDefinitionRegistry not initialised")),
    }

    let get_err = handle_get_definition(Map::from_iter([(
        "id".into(),
        Value::String("__definitely_missing_definition__".into()),
    )]))
    .await
    .expect_err("missing or unknown definition should error");
    assert!(
        get_err.contains("AgentDefinitionRegistry not initialised")
            || get_err.contains("not found")
    );
}

#[tokio::test]
async fn triage_handler_rejects_unknown_source_and_to_json_maps_outcome() {
    let err = handle_triage_evaluate(Map::from_iter([
        ("source".into(), Value::String("__unknown_source__".into())),
        ("display_label".into(), Value::String("lbl".into())),
        ("payload".into(), json!({})),
    ]))
    .await
    .expect_err("unsupported source should fail before runtime dispatch");
    assert!(err.contains("unsupported trigger source"));

    let value = to_json(RpcOutcome::new(json!({ "ok": true }), Vec::new())).expect("json outcome");
    assert_eq!(value["ok"], json!(true));
}
