use super::*;

#[test]
fn all_controller_schemas_lists_three_functions() {
    let schemas = all_controller_schemas();
    let names: Vec<&'static str> = schemas.iter().map(|s| s.function).collect();
    assert_eq!(schemas.len(), 4);
    assert!(names.contains(&"get_dashboard"));
    assert!(names.contains(&"get_daily_history"));
    assert!(names.contains(&"get_summary"));
    assert!(names.contains(&"get_usage_log"));
    for schema in &schemas {
        assert_eq!(schema.namespace, "cost");
    }
}

#[test]
fn all_registered_controllers_has_handlers_matching_schemas() {
    let registered = all_registered_controllers();
    assert_eq!(registered.len(), 4);
    let schema_fns: Vec<&'static str> = registered.iter().map(|r| r.schema.function).collect();
    assert!(schema_fns.contains(&"get_dashboard"));
    assert!(schema_fns.contains(&"get_daily_history"));
    assert!(schema_fns.contains(&"get_summary"));
    assert!(schema_fns.contains(&"get_usage_log"));
}

#[test]
fn schema_for_dashboard_has_no_inputs_and_one_output() {
    let s = schema_for("cost_get_dashboard");
    assert_eq!(s.function, "get_dashboard");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "dashboard");
}

#[test]
fn schema_for_daily_history_has_optional_days_input() {
    let s = schema_for("cost_get_daily_history");
    assert_eq!(s.function, "get_daily_history");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "days");
    assert!(!s.inputs[0].required);
}

#[test]
fn schema_for_summary_returns_summary_output() {
    let s = schema_for("cost_get_summary");
    assert_eq!(s.function, "get_summary");
    assert_eq!(s.outputs[0].name, "summary");
}

#[test]
fn schema_for_usage_log_has_days_and_limit_inputs() {
    let s = schema_for("cost_get_usage_log");
    assert_eq!(s.function, "get_usage_log");
    assert_eq!(s.inputs.len(), 2);
    assert_eq!(s.inputs[0].name, "days");
    assert_eq!(s.inputs[1].name, "limit");
    assert_eq!(s.outputs[0].name, "usage_log");
}

#[test]
fn schema_for_unknown_returns_error_shape() {
    let s = schema_for("cost_get_nonexistent");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn new_correlation_id_returns_eight_hex_chars() {
    let cid = new_correlation_id();
    assert_eq!(cid.len(), 8);
    assert!(cid.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn new_correlation_id_is_unique_across_calls() {
    let a = new_correlation_id();
    let b = new_correlation_id();
    // Collision probability for 8 hex chars (32 bits) per call is
    // ~1/4B — virtually zero for a unit test.
    assert_ne!(a, b);
}
