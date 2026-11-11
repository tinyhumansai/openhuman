use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_input_order, expect_schema_output_names, expect_schema_required_inputs,
};
// Additional names from the extracted schema handler modules.
use super::handlers::*;
use serde_json::json;

#[test]
fn flow_schema_wire_contracts_are_preserved() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "create",
    );
    // None preserves optional-field-only tests; Some(&[]) asserts no required inputs.
    let contracts = [
        ("import", Some(&["graph"][..]), &["format"][..]),
        (
            "create",
            Some(&["name", "graph"][..]),
            &["require_approval"][..],
        ),
        ("duplicate", Some(&["id"][..]), &[][..]),
        ("prune_runs", Some(&["id"][..]), &[][..]),
        ("run", None, &["input", "inputs"][..]),
        ("resume", Some(&["id", "thread_id"][..]), &["approvals"][..]),
        ("list_runs", None, &["limit"][..]),
        ("get_run", Some(&["run_id"][..]), &[][..]),
        (
            "build",
            Some(&["mode"][..]),
            &["thread_id", "request_id"][..],
        ),
        (
            "build_cancel",
            Some(&["thread_id"][..]),
            &["request_id"][..],
        ),
        ("discover", Some(&[][..]), &["thread_id", "request_id"][..]),
    ];
    for (function, required, optional) in contracts {
        let schema = schemas(function);
        expect_schema_identity(&schema, "flows", function);
        if let Some(required) = required {
            expect_schema_required_inputs(&schema, required);
        }
        expect_schema_fields(
            &schema,
            &optional
                .iter()
                .map(|name| (*name, Some(false)))
                .collect::<Vec<_>>(),
        );
    }
    let discover = schemas("discover");
    expect_schema_input_order(&discover, &["thread_id", "request_id"]);
    expect_schema_output_names(&schemas("import"), &["graph", "warnings"]);
    expect_schema_output_names(&schemas("duplicate"), &["flow"]);
    assert_eq!(schemas("prune_runs").outputs[0].name, "result");
}

#[test]
fn run_detached_schema_description_has_normal_spacing_and_input_rules() {
    let rendered = serde_json::to_value(schemas("run_detached")).unwrap();
    let inputs = rendered["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == "inputs")
        .unwrap();
    let comment = inputs["comment"].as_str().unwrap();

    assert!(comment.contains("graph.inputs"), "{comment}");
    assert!(comment.contains("Validated synchronously"), "{comment}");
    assert!(comment.contains("Distinct from `input`"), "{comment}");
    assert!(
        !comment.contains("                               "),
        "the rendered schema must not contain continuation indentation: {:?}",
        comment
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
fn read_required_errors_when_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "id").unwrap_err();
    assert!(err.contains("missing required param 'id'"));
}

// R-m7: absent leaves the link untouched; null/blank explicitly unlink.
// Invalid types must not silently become an explicit unlink.
#[test]
fn draft_update_flow_link_preserves_all_three_states_and_rejects_wrong_types() {
    for (case, input, expected) in [
        ("absent leaves link untouched", None, None),
        ("null explicitly unlinks", Some(Value::Null), Some(None)),
        (
            "string links flow",
            Some(json!("flow-123")),
            Some(Some("flow-123".to_string())),
        ),
        ("blank explicitly unlinks", Some(json!("   ")), Some(None)),
    ] {
        let mut params = Map::new();
        if let Some(input) = input {
            params.insert("flow_id".into(), input);
        }
        assert_eq!(
            parse_draft_update_flow_id(&params).unwrap(),
            expected,
            "{case}"
        );
    }
    for (case, input) in [("number", json!(42)), ("object", json!({"id":"flow-1"}))] {
        let params = Map::from_iter([("flow_id".into(), input)]);
        let err = parse_draft_update_flow_id(&params).unwrap_err();
        assert!(err.contains("invalid 'flow_id'"), "{case}: {err}");
    }
}
