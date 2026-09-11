use super::*;
use serde_json::json;

#[test]
fn run_schema_advertises_both_input_channels() {
    let run = all_controller_schemas()
        .into_iter()
        .find(|s| s.function == "run")
        .expect("the run controller is registered");
    let names: Vec<_> = run.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"input"), "trigger payload, got {names:?}");
    assert!(names.contains(&"inputs"), "declared inputs, got {names:?}");

    let declared = run.inputs.iter().find(|f| f.name == "inputs").unwrap();
    assert!(
        !declared.required,
        "a flow with no declared inputs must still be runnable without the param"
    );
}

#[test]
fn read_declared_inputs_accepts_absent_null_and_object() {
    let mut params = Map::new();
    assert!(read_declared_inputs(&params).unwrap().is_empty(), "absent");

    params.insert("inputs".into(), Value::Null);
    assert!(read_declared_inputs(&params).unwrap().is_empty(), "null");

    params.insert("inputs".into(), json!({ "repo": "acme/api" }));
    assert_eq!(
        read_declared_inputs(&params).unwrap()["repo"],
        json!("acme/api")
    );
}

#[test]
fn read_declared_inputs_rejects_a_non_object_naming_the_param() {
    // A caller sending an array or scalar has mis-shaped the call; say so
    // here rather than letting it read as "you supplied no inputs".
    for bad in [json!([1, 2]), json!("repo=acme"), json!(7), json!(true)] {
        let mut params = Map::new();
        params.insert("inputs".into(), bad.clone());
        let err =
            read_declared_inputs(&params).expect_err("a non-object `inputs` must be rejected");
        assert!(err.contains("'inputs'"), "got: {err} (for {bad})");
    }
}

#[test]
fn all_controller_schemas_covers_every_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec![
            "create",
            "duplicate",
            "validate",
            "import",
            "get",
            "list",
            "list_connections",
            "update",
            "delete",
            "set_enabled",
            "run",
            "run_detached",
            "resume",
            "cancel_run",
            "list_runs",
            "list_all_runs",
            "get_run",
            "prune_runs",
            "build",
            "build_cancel",
            "discover",
            "list_suggestions",
            "dismiss_suggestion",
            "mark_suggestion_built",
            "draft_create",
            "draft_get",
            "draft_update",
            "draft_list",
            "draft_delete",
            "draft_promote",
            "get_history",
            "rollback",
            "search_tool_catalog",
            "get_tool_contract",
            "required_connections",
            "approval_manifest",
        ]
    );
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 36);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        names,
        vec![
            "create",
            "duplicate",
            "validate",
            "import",
            "get",
            "list",
            "list_connections",
            "update",
            "delete",
            "set_enabled",
            "run",
            "run_detached",
            "resume",
            "cancel_run",
            "list_runs",
            "list_all_runs",
            "get_run",
            "prune_runs",
            "build",
            "build_cancel",
            "discover",
            "list_suggestions",
            "dismiss_suggestion",
            "mark_suggestion_built",
            "draft_create",
            "draft_get",
            "draft_update",
            "draft_list",
            "draft_delete",
            "draft_promote",
            "get_history",
            "rollback",
            "search_tool_catalog",
            "get_tool_contract",
            "required_connections",
            "approval_manifest",
        ]
    );
}

#[test]
fn schemas_import_requires_graph_and_optional_format() {
    let s = schemas("import");
    assert_eq!(s.namespace, "flows");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["graph"]);
    let format = s.inputs.iter().find(|f| f.name == "format").unwrap();
    assert!(!format.required);
    let names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["graph", "warnings"]);
}

#[test]
fn schemas_list_connections_has_no_inputs_and_secret_free_outputs() {
    let s = schemas("list_connections");
    assert_eq!(s.namespace, "flows");
    assert!(s.inputs.is_empty());
    // The only output is the `connections` array.
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "connections");
    // No field on a FlowConnection element may resemble secret material.
    if let TypeSchema::Array(inner) = &s.outputs[0].ty {
        if let TypeSchema::Object { fields } = inner.as_ref() {
            let names: Vec<_> = fields.iter().map(|f| f.name).collect();
            assert_eq!(
                names,
                vec![
                    "connection_ref",
                    "kind",
                    "display",
                    "toolkit",
                    "scheme",
                    "platform_user_id"
                ]
            );
            for f in fields {
                let n = f.name.to_ascii_lowercase();
                assert!(
                    !n.contains("secret")
                        && !n.contains("token")
                        && !n.contains("password")
                        && !n.contains("key"),
                    "flow_connection field '{}' looks secret-bearing",
                    f.name
                );
            }
        } else {
            panic!("connections element type is not an Object");
        }
    } else {
        panic!("connections output is not an Array");
    }
}

#[test]
fn schemas_create_requires_name_and_graph() {
    let s = schemas("create");
    assert_eq!(s.namespace, "flows");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["name", "graph"]);
}

#[test]
fn schemas_create_require_approval_is_optional() {
    let s = schemas("create");
    let field = s
        .inputs
        .iter()
        .find(|f| f.name == "require_approval")
        .unwrap();
    assert!(!field.required);
}

#[test]
fn schemas_duplicate_requires_id_and_outputs_flow() {
    let s = schemas("duplicate");
    assert_eq!(s.namespace, "flows");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["id"]);
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "flow");
}

#[test]
fn schemas_prune_runs_requires_id_and_reports_counts() {
    let s = schemas("prune_runs");
    assert_eq!(s.namespace, "flows");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["id"]);
    assert_eq!(s.outputs[0].name, "result");
}

#[test]
fn schemas_run_input_is_optional() {
    let s = schemas("run");
    let input = s.inputs.iter().find(|f| f.name == "input").unwrap();
    assert!(!input.required);
}

#[test]
fn schemas_resume_requires_id_and_thread_id_but_not_approvals() {
    let s = schemas("resume");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["id", "thread_id"]);
    let approvals = s.inputs.iter().find(|f| f.name == "approvals").unwrap();
    assert!(!approvals.required);
}

#[test]
fn schemas_list_runs_limit_is_optional() {
    let s = schemas("list_runs");
    let limit = s.inputs.iter().find(|f| f.name == "limit").unwrap();
    assert!(!limit.required);
}

#[test]
fn schemas_get_run_requires_run_id() {
    let s = schemas("get_run");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["run_id"]);
}

#[test]
fn schemas_build_exposes_optional_stream_params() {
    let s = schemas("build");
    assert_eq!(s.namespace, "flows");
    // The only structurally required build input is `mode`.
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["mode"]);
    // The streaming params are present and optional.
    let thread = s.inputs.iter().find(|f| f.name == "thread_id").unwrap();
    assert!(!thread.required);
    let request = s.inputs.iter().find(|f| f.name == "request_id").unwrap();
    assert!(!request.required);
}

#[test]
fn schemas_build_cancel_requires_thread_id_but_not_request_id() {
    let s = schemas("build_cancel");
    assert_eq!(s.namespace, "flows");
    assert_eq!(s.function, "build_cancel");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["thread_id"]);
    let request = s.inputs.iter().find(|f| f.name == "request_id").unwrap();
    assert!(!request.required);
}

#[test]
fn schemas_discover_exposes_optional_stream_params() {
    let s = schemas("discover");
    assert_eq!(s.namespace, "flows");
    // Discover has no required inputs — the two stream params are optional.
    assert!(s.inputs.iter().all(|f| !f.required));
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["thread_id", "request_id"]);
}

#[test]
fn read_flow_stream_target_none_without_thread_id() {
    let mut params = Map::new();
    // request_id alone is not enough — streaming needs a thread.
    params.insert("request_id".to_string(), Value::String("r-1".to_string()));
    assert!(read_flow_stream_target(&params).is_none());
    // Blank thread id is also treated as absent.
    params.insert("thread_id".to_string(), Value::String("   ".to_string()));
    assert!(read_flow_stream_target(&params).is_none());
}

#[test]
fn read_flow_stream_target_uses_thread_and_request() {
    let mut params = Map::new();
    params.insert("thread_id".to_string(), Value::String("t-42".to_string()));
    params.insert("request_id".to_string(), Value::String("r-9".to_string()));
    let target = read_flow_stream_target(&params).expect("stream target");
    assert_eq!(target.thread_id, "t-42");
    assert_eq!(target.request_id, "r-9");
}

#[test]
fn read_flow_stream_target_generates_request_id_when_absent() {
    let mut params = Map::new();
    params.insert("thread_id".to_string(), Value::String("t-7".to_string()));
    let target = read_flow_stream_target(&params).expect("stream target");
    assert_eq!(target.thread_id, "t-7");
    // A uuid was minted — non-empty and not the thread id.
    assert!(!target.request_id.is_empty());
    assert_ne!(target.request_id, target.thread_id);
}

#[test]
fn schemas_unknown_function_returns_placeholder() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn read_required_errors_when_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "id").unwrap_err();
    assert!(err.contains("missing required param 'id'"));
}

// ── R-m7: parse_draft_update_flow_id ─────────────────────────────────────

#[test]
fn parse_draft_update_flow_id_absent_leaves_link_untouched() {
    let params = Map::new();
    assert_eq!(parse_draft_update_flow_id(&params).unwrap(), None);
}

#[test]
fn parse_draft_update_flow_id_null_is_an_explicit_unlink() {
    let mut params = Map::new();
    params.insert("flow_id".to_string(), Value::Null);
    assert_eq!(parse_draft_update_flow_id(&params).unwrap(), Some(None));
}

#[test]
fn parse_draft_update_flow_id_string_links_to_that_flow() {
    let mut params = Map::new();
    params.insert("flow_id".to_string(), Value::String("flow-123".to_string()));
    assert_eq!(
        parse_draft_update_flow_id(&params).unwrap(),
        Some(Some("flow-123".to_string()))
    );
}

#[test]
fn parse_draft_update_flow_id_empty_string_is_an_explicit_unlink() {
    let mut params = Map::new();
    params.insert("flow_id".to_string(), Value::String("   ".to_string()));
    assert_eq!(parse_draft_update_flow_id(&params).unwrap(), Some(None));
}

// Regression for R-m7: a number must be REJECTED, not silently coerced
// into `Some(None)` (an explicit unlink) the way `Value::as_str()`
// returning `None` on a type mismatch used to produce.
#[test]
fn parse_draft_update_flow_id_rejects_a_number() {
    let mut params = Map::new();
    params.insert("flow_id".to_string(), Value::from(42));
    let err = parse_draft_update_flow_id(&params).unwrap_err();
    assert!(err.contains("invalid 'flow_id'"), "{err}");
}

#[test]
fn parse_draft_update_flow_id_rejects_an_object() {
    let mut params = Map::new();
    params.insert("flow_id".to_string(), serde_json::json!({ "id": "flow-1" }));
    let err = parse_draft_update_flow_id(&params).unwrap_err();
    assert!(err.contains("invalid 'flow_id'"), "{err}");
}
