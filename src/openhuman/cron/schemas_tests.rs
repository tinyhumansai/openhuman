use super::*;
use serde_json::json;

// ── schemas() branch coverage ───────────────────────────────────

#[test]
fn schemas_list_has_no_inputs_and_jobs_output() {
    let s = schemas("list");
    assert_eq!(s.namespace, "cron");
    assert_eq!(s.function, "list");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "jobs");
}

#[test]
fn schemas_update_requires_job_id_and_patch() {
    let s = schemas("update");
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"job_id"));
    assert!(names.contains(&"patch"));
    assert!(s.inputs.iter().all(|f| f.required));
}

#[test]
fn schemas_remove_has_job_id_input_and_result_output() {
    let s = schemas("remove");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "job_id");
    assert_eq!(s.outputs[0].name, "result");
}

#[test]
fn schemas_run_result_declares_only_the_enqueue_acknowledgement() {
    let s = schemas("run");
    // `cron_run` spawns the execution and returns immediately, so the declared
    // output is the enqueue acknowledgement — `job_id` plus a `status` that is
    // always "queued". `duration_ms` / `output` belong to the terminal record
    // in `cron_runs` and must not be promised here.
    let TypeSchema::Object { fields } = &s.outputs[0].ty else {
        panic!("expected object output type");
    };
    let names: Vec<_> = fields.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["job_id", "status"]);

    let status = fields.iter().find(|f| f.name == "status").unwrap();
    assert!(
        matches!(&status.ty, TypeSchema::Enum { variants } if variants.as_slice() == ["queued"]),
        "status must declare exactly the variant the handler emits, got {:?}",
        status.ty
    );
}

#[test]
fn schemas_run_description_points_at_cron_runs_for_the_terminal_record() {
    let s = schemas("run");
    assert!(
        s.description.contains("cron_runs"),
        "the description must tell a client where the terminal outcome lives, got: {}",
        s.description
    );
}

#[test]
fn schemas_runs_limit_is_optional() {
    let s = schemas("runs");
    let limit = s.inputs.iter().find(|f| f.name == "limit").unwrap();
    assert!(!limit.required);
}

#[test]
fn schemas_unknown_function_returns_placeholder_with_error_output() {
    // The `_other` branch is used when a caller requests a schema
    // for a function that does not exist — it should not panic.
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

// ── registry helpers ────────────────────────────────────────────

#[test]
fn schemas_add_requires_schedule_and_returns_job() {
    let s = schemas("add");
    assert_eq!(s.namespace, "cron");
    assert_eq!(s.function, "add");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["schedule"]);
    assert_eq!(s.outputs[0].name, "job");
}

#[test]
fn all_controller_schemas_covers_every_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec!["add", "list", "update", "remove", "run", "runs"]
    );
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 6);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        names,
        vec!["add", "list", "update", "remove", "run", "runs"]
    );
}

// ── read_required ───────────────────────────────────────────────

#[test]
fn read_required_returns_value_for_present_key() {
    let mut params = Map::new();
    params.insert("job_id".into(), json!("abc"));
    let got: String = read_required(&params, "job_id").unwrap();
    assert_eq!(got, "abc");
}

#[test]
fn read_required_errors_when_key_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "job_id").unwrap_err();
    assert!(err.contains("missing required param 'job_id'"));
}

#[test]
fn read_required_errors_when_deserialization_fails() {
    let mut params = Map::new();
    params.insert("job_id".into(), json!(42));
    let err = read_required::<String>(&params, "job_id").unwrap_err();
    assert!(err.contains("invalid 'job_id'"));
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
    params.insert("limit".into(), json!(42));
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(42));
}

#[test]
fn read_optional_u64_rejects_negative_number() {
    let mut params = Map::new();
    params.insert("limit".into(), json!(-1));
    let err = read_optional_u64(&params, "limit").unwrap_err();
    assert!(err.contains("expected unsigned integer"));
}

#[test]
fn read_optional_u64_rejects_non_number_types() {
    for (tag, v) in [
        ("string", json!("ten")),
        ("bool", json!(true)),
        ("array", json!([1, 2])),
        ("object", json!({"k": 1})),
    ] {
        let mut params = Map::new();
        params.insert("limit".into(), v);
        let err = read_optional_u64(&params, "limit").unwrap_err();
        assert!(
            err.contains("expected unsigned integer"),
            "tag={tag} err={err}"
        );
    }
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
