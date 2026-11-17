use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_input_order, expect_schema_output_fields, expect_schema_required_inputs,
};
use serde_json::json;

#[test]
fn artifact_schema_contracts_and_registration_are_preserved() {
    const FUNCTIONS: &[&str] = &[
        "list_artifacts",
        "get_artifact",
        "delete_artifact",
        "regenerate",
        "save_artifact_version",
        "share_artifact",
        "read_artifact_body",
    ];
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, FUNCTIONS);
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "list_artifacts",
    );

    let list = schemas("list_artifacts");
    expect_schema_identity(&list, "ai", "list_artifacts");
    expect_schema_fields(&list, &[("offset", Some(false)), ("limit", Some(false))]);
    expect_schema_output_fields(&list, &["artifacts", "total", "offset", "limit"]);

    let get = schemas("get_artifact");
    expect_schema_identity(&get, "ai", "get_artifact");
    expect_schema_input_order(&get, &["artifact_id"]);
    expect_schema_required_inputs(&get, &["artifact_id"]);
    expect_schema_output_fields(
        &get,
        &[
            "id",
            "kind",
            "title",
            "path",
            "size_bytes",
            "status",
            "created_at",
            "absolute_path",
        ],
    );
    assert!(!get.outputs.iter().any(|field| field.name == "artifact"));

    let delete = schemas("delete_artifact");
    expect_schema_identity(&delete, "ai", "delete_artifact");
    expect_schema_input_order(&delete, &["artifact_id"]);
    expect_schema_required_inputs(&delete, &["artifact_id"]);
    assert_eq!(delete.outputs[0].name, "result");
    if let TypeSchema::Object { fields } = &delete.outputs[0].ty {
        assert!(fields.iter().any(|field| field.name == "artifact_id"));
        assert!(fields.iter().any(|field| field.name == "deleted"));
    } else {
        panic!("expected object output type");
    }

    let regenerate = schemas("regenerate");
    expect_schema_identity(&regenerate, "ai", "regenerate");
    expect_schema_input_order(&regenerate, &["artifact_id", "thread_id", "client_id"]);
    assert!(regenerate.inputs.iter().all(|input| input.required));
    if let TypeSchema::Object { fields } = &regenerate.outputs[0].ty {
        for name in ["artifact_id", "regenerated", "is_error"] {
            assert!(fields.iter().any(|field| field.name == name));
        }
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
