use super::*;
use serde_json::json;

#[test]
fn all_controller_schemas_advertises_both_vendors() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, vec!["openclaw", "hermes"]);
}

#[test]
fn all_registered_controllers_has_two_handlers() {
    let ctrl = all_registered_controllers();
    assert_eq!(ctrl.len(), 2);
    assert_eq!(ctrl[0].schema.function, "openclaw");
    assert_eq!(ctrl[1].schema.function, "hermes");
}

#[test]
fn openclaw_schema_describes_optional_source_and_dry_run() {
    let s = schemas("openclaw");
    assert_eq!(s.namespace, "migrate");
    assert_eq!(s.function, "openclaw");
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"source_workspace"));
    assert!(names.contains(&"dry_run"));
    for f in &s.inputs {
        assert!(!f.required, "input `{}` must be optional", f.name);
    }
    assert_eq!(s.outputs[0].name, "report");
}

#[test]
fn unknown_function_returns_unknown_placeholder() {
    let s = schemas("bogus");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "migrate");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn migrate_openclaw_params_tolerates_empty_object() {
    let params: MigrateOpenClawParams = serde_json::from_value(json!({})).unwrap();
    assert!(params.source_workspace.is_none());
    assert!(params.dry_run.is_none());
}

#[test]
fn migrate_openclaw_params_parses_both_fields() {
    let params: MigrateOpenClawParams = serde_json::from_value(json!({
        "source_workspace": "/tmp/old",
        "dry_run": false
    }))
    .unwrap();
    assert_eq!(params.source_workspace.as_deref(), Some("/tmp/old"));
    assert_eq!(params.dry_run, Some(false));
}

#[test]
fn to_json_wraps_rpc_outcome_result_envelope() {
    let v = to_json(RpcOutcome::single_log(json!({"done": true}), "done")).unwrap();
    assert!(v.get("logs").is_some() || v.get("result").is_some());
}

// ── Hermes schema tests ─────────────────────────────────────────

#[test]
fn hermes_schema_describes_optional_source_and_dry_run() {
    let s = schemas("hermes");
    assert_eq!(s.namespace, "migrate");
    assert_eq!(s.function, "hermes");
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"source_workspace"));
    assert!(names.contains(&"dry_run"));
    for f in &s.inputs {
        assert!(!f.required, "input `{}` must be optional", f.name);
    }
    assert_eq!(s.outputs[0].name, "report");
}

#[test]
fn migrate_hermes_params_tolerates_empty_object() {
    let params: MigrateHermesParams = serde_json::from_value(json!({})).unwrap();
    assert!(params.source_workspace.is_none());
    assert!(params.dry_run.is_none());
}

#[test]
fn migrate_hermes_params_parses_both_fields() {
    let params: MigrateHermesParams = serde_json::from_value(json!({
        "source_workspace": "/home/u/.hermes",
        "dry_run": false
    }))
    .unwrap();
    assert_eq!(params.source_workspace.as_deref(), Some("/home/u/.hermes"));
    assert_eq!(params.dry_run, Some(false));
}
