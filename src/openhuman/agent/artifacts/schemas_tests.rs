use super::*;
use serde_json::json;

// ── schemas() branch coverage ───────────────────────────────────

#[test]
fn schemas_list_artifacts_has_pagination_inputs_and_correct_outputs() {
    let s = schemas("list_artifacts");
    assert_eq!(s.namespace, "ai");
    assert_eq!(s.function, "list_artifacts");
    let input_names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert!(input_names.contains(&"offset"));
    assert!(input_names.contains(&"limit"));
    assert!(s.inputs.iter().all(|f| !f.required));
    let output_names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
    assert!(output_names.contains(&"artifacts"));
    assert!(output_names.contains(&"total"));
    assert!(output_names.contains(&"offset"));
    assert!(output_names.contains(&"limit"));
}

#[test]
fn schemas_get_artifact_requires_artifact_id() {
    let s = schemas("get_artifact");
    assert_eq!(s.namespace, "ai");
    assert_eq!(s.function, "get_artifact");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "artifact_id");
    assert!(s.inputs[0].required);
    // Output should be flat fields matching the actual JSON response shape
    let output_names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
    assert!(output_names.contains(&"id"));
    assert!(output_names.contains(&"kind"));
    assert!(output_names.contains(&"title"));
    assert!(output_names.contains(&"path"));
    assert!(output_names.contains(&"size_bytes"));
    assert!(output_names.contains(&"status"));
    assert!(output_names.contains(&"created_at"));
    assert!(output_names.contains(&"absolute_path"));
    // Must NOT have an opaque "artifact" wrapper
    assert!(!output_names.contains(&"artifact"));
}

#[test]
fn schemas_delete_artifact_has_artifact_id_input_and_result_output() {
    let s = schemas("delete_artifact");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "artifact_id");
    assert!(s.inputs[0].required);
    assert_eq!(s.outputs[0].name, "result");
    if let TypeSchema::Object { fields } = &s.outputs[0].ty {
        let names: Vec<_> = fields.iter().map(|f| f.name).collect();
        assert!(names.contains(&"artifact_id"));
        assert!(names.contains(&"deleted"));
    } else {
        panic!("expected object output type");
    }
}

#[test]
fn schemas_unknown_function_returns_placeholder_with_error_output() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

// ── registry helpers ────────────────────────────────────────────

#[test]
fn all_controller_schemas_covers_every_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec![
            "list_artifacts",
            "get_artifact",
            "delete_artifact",
            "regenerate"
        ]
    );
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 4);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        names,
        vec![
            "list_artifacts",
            "get_artifact",
            "delete_artifact",
            "regenerate"
        ]
    );
}

#[test]
fn schemas_regenerate_requires_artifact_id_thread_and_client() {
    let s = schemas("regenerate");
    assert_eq!(s.function, "regenerate");
    let input_names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert_eq!(input_names, vec!["artifact_id", "thread_id", "client_id"]);
    assert!(s.inputs.iter().all(|f| f.required));
    if let TypeSchema::Object { fields } = &s.outputs[0].ty {
        let names: Vec<_> = fields.iter().map(|f| f.name).collect();
        assert!(names.contains(&"artifact_id"));
        assert!(names.contains(&"regenerated"));
        assert!(names.contains(&"is_error"));
    } else {
        panic!("expected object output type");
    }
}

// ── read_required ───────────────────────────────────────────────

#[test]
fn read_required_returns_value_for_present_key() {
    let mut params = Map::new();
    params.insert("artifact_id".into(), json!("abc"));
    let got: String = read_required(&params, "artifact_id").unwrap();
    assert_eq!(got, "abc");
}

#[test]
fn read_required_errors_when_key_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "artifact_id").unwrap_err();
    assert!(err.contains("missing required param 'artifact_id'"));
}

// ── read_optional_u64 ───────────────────────────────────────────

#[test]
fn read_optional_u64_absent_key_is_none() {
    assert_eq!(read_optional_u64(&Map::new(), "limit").unwrap(), None);
}

#[test]
fn read_optional_u64_explicit_null_is_none() {
    let mut params = Map::new();
    params.insert("limit".into(), Value::Null);
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), None);
}

#[test]
fn read_optional_u64_accepts_unsigned_integer() {
    let mut params = Map::new();
    params.insert("limit".into(), json!(50));
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(50));
}

#[test]
fn read_optional_u64_rejects_negative_number() {
    let mut params = Map::new();
    params.insert("limit".into(), json!(-1));
    let err = read_optional_u64(&params, "limit").unwrap_err();
    assert!(err.contains("expected unsigned integer"));
}

// ── type_name ───────────────────────────────────────────────────

#[test]
fn type_name_reports_each_json_variant() {
    assert_eq!(type_name(&Value::Null), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(1)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}
